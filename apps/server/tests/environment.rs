#[path = "support/auth.rs"]
mod auth_client;
#[path = "support/delivery_hooks.rs"]
mod delivery_hook_fixture;
#[path = "support/environment_preparation.rs"]
mod preparation_fixture;
// GH-119: real local processes/tools; database/CI/provider delivery are separate.
use codexsymphony_server::{
    controlled_contract::{ControlledConfig, EnvironmentBinding, Operation, Registration, Verdict},
    environment::{Cache, Plan, Role, Service},
    environment_host::{Profile, Registry},
    environment_probe, process,
    validation::sha256,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

fn temporary() -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("environment-{}", process::new_identity().unwrap()));
    fs::create_dir(&root).unwrap();
    root
}
fn tool(name: &str) -> (String, String) {
    let path = Command::new("python3")
        .args([
            "-c",
            "import shutil,sys;print(shutil.which(sys.argv[1]))",
            name,
        ])
        .output()
        .unwrap();
    let path = String::from_utf8(path.stdout).unwrap().trim().to_owned();
    let version = Command::new(&path).arg("--version").output().unwrap();
    (
        String::from_utf8(version.stdout).unwrap().trim().into(),
        sha256(fs::read(path).unwrap()),
    )
}
fn fixture(root: &Path, name: &str, language: &str, cached: bool) -> (Plan, Profile) {
    let resources = root.join(name);
    fs::create_dir(&resources).unwrap();
    fs::write(
        resources.join("README.md"),
        format!("Controlled {language} repository fixture\n"),
    )
    .unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "README.md"],
        vec![
            "-c",
            "user.name=Environment Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "initial fixture",
        ],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&resources)
                .status()
                .unwrap()
                .success()
        );
    }
    let cache = cached.then(|| Cache {
        identity: sha256(name),
        scope: name.into(),
        capacity_bytes: 4096,
        writable: true,
        seed: None,
    });
    fs::write(
        resources.join("settings.json"),
        serde_json::to_vec(
            &json!({"tool":language,"threads":1,"image":"none","memory":64,"cache":cache}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::write(resources.join("state"), "ready").unwrap();
    let executable = root.join(format!("{name}-probe"));
    fs::write(&executable, include_str!("fixtures/environment/probe.py")).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let registration = Registration {
        id: name.into(),
        implementation_digest: sha256(fs::read(&executable).unwrap()),
        operations: vec![Operation::EnvironmentCheck],
        scope_ref: name.into(),
        config_ref: format!("{name}-config"),
        credential_provider_ref: None,
    };
    let (version, digest) = tool(language);
    let role = Role {
        expected: BTreeMap::from([
            ("tool.version".into(), json!(version)),
            ("tool.digest".into(), json!(digest)),
            ("schedule.threads".into(), json!(1)),
            ("image".into(), json!("none")),
            ("memory".into(), json!(64)),
        ]),
        services: BTreeMap::new(),
        runtime: BTreeMap::from([("runtime.state".into(), vec![json!("ready"), json!("idle")])]),
        checks: vec!["environment".into()],
        differences: "Same tool/resource contract; independent data per repository".into(),
    };
    let mut plan = Plan {
        host_profile_digest: String::new(),
        controlled: ControlledConfig {
            protocol_version: 1,
            environment: EnvironmentBinding {
                repository_revision: format!("{name}@1"),
                contract_digest: "0".repeat(64),
                host_profile_ref: name.into(),
                role: "test".into(),
            },
            extensions: vec![registration.clone()],
        },
        extension_id: name.into(),
        lockfiles: vec![],
        roles: BTreeMap::from([("dev".into(), role.clone()), ("test".into(), role)]),
        ci: false,
        cache,
    };
    plan.controlled.environment.contract_digest = plan.contract_digest();
    let mut profile = Profile {
        extensions: vec![],
        registration,
        executable,
        approved_plans: vec![plan.digest()],
        resource_root: resources,
        // Include durable supervisor startup on the shared development disk.
        timeout_seconds: 60,
    };
    plan.host_profile_digest = profile.identity(&root.join("evidence"));
    plan.controlled.environment.contract_digest = plan.contract_digest();
    profile.approved_plans = vec![plan.digest()];
    (plan, profile)
}
fn registry(root: &Path, entries: Vec<(String, Profile)>) -> Registry {
    Registry {
        evidence_root: root.join("evidence"),
        profiles: entries.into_iter().collect(),
    }
}
fn approve(registry: &mut Registry, plan: &mut Plan) {
    plan.controlled.environment.contract_digest = plan.contract_digest();
    registry
        .profiles
        .get_mut(&plan.extension_id)
        .unwrap()
        .approved_plans = vec![plan.digest()];
}
async fn check(registry: &Registry, plan: &Plan, stage: &str) -> environment_probe::Report {
    environment_probe::check(registry, plan, stage, "test", None)
        .await
        .unwrap()
}

#[tokio::test]
async fn real_two_project_admission_detects_drift_and_preserves_isolation() {
    // This test binary alone owns this environment variable, before any supervisor spawn.
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temporary();
    let (python, python_host) = fixture(&root, "python", "python3", false);
    let (node, node_host) = fixture(&root, "node", "node", true);
    let registry = registry(
        &root,
        vec![("python".into(), python_host), ("node".into(), node_host)],
    );
    registry.validate().unwrap();
    for stage in [
        "enable",
        "startup",
        "preparation",
        "launch",
        "validation",
        "delivery",
        "recovery",
    ] {
        assert!(check(&registry, &python, stage).await.passed());
        assert!(check(&registry, &node, stage).await.passed());
    }
    // Actual validation commands for two different environments, no project DB.
    fs::write(root.join("python/test.py"), "import unittest\nclass Test(unittest.TestCase):\n def test_value(self): self.assertEqual(2+2,4)\nif __name__=='__main__': unittest.main()\n").unwrap();
    assert!(
        Command::new("python3")
            .current_dir(root.join("python"))
            .arg(root.join("python/test.py"))
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.join("node/test.mjs"), "import test from 'node:test';import assert from 'node:assert/strict';test('value',()=>assert.equal(2+2,4));").unwrap();
    assert!(
        Command::new("node")
            .current_dir(root.join("node"))
            .arg("--test")
            .arg(root.join("node/test.mjs"))
            .status()
            .unwrap()
            .success()
    );
    assert!(!root.join("python/cache").exists());
    let node_cache = root
        .join("node/cache")
        .join(&node.cache.as_ref().unwrap().identity);
    assert!(node_cache.is_dir());
    fs::write(node_cache.join("data"), "node-only").unwrap();
    let settings = root.join("python/settings.json");
    let original: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    for (field, value, fact) in [
        ("threads", json!(2), "schedule.threads"),
        ("image", json!("changed"), "image"),
        ("memory", json!(128), "memory"),
        ("tool", json!("node"), "tool.digest"),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        fs::write(&settings, serde_json::to_vec(&changed).unwrap()).unwrap();
        let report = check(&registry, &python, "recovery").await;
        assert!(!report.passed());
        assert!(report.differences.iter().any(|diff| diff.field == fact));
        assert!(check(&registry, &node, "validation").await.passed());
    }
    fs::write(&settings, serde_json::to_vec(&original).unwrap()).unwrap();
    fs::write(root.join("python/state"), "idle").unwrap();
    assert!(check(&registry, &python, "preparation").await.passed());
    fs::write(root.join("python/state"), "broken").unwrap();
    assert!(!check(&registry, &python, "preparation").await.passed());
    fs::write(node_cache.join("overflow"), vec![0_u8; 4097]).unwrap();
    assert!(
        environment_probe::check(&registry, &node, "validation", "test", None)
            .await
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(node_cache.join("data")).unwrap(),
        "node-only"
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn installed_new_binary_does_not_approve_an_old_running_process() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temporary();
    let (mut plan, profile) = fixture(&root, "service", "python3", false);
    let mut registry = registry(&root, vec![("service".into(), profile)]);
    let resources = root.join("service");
    let mut child = Command::new("python3")
        .args(["-c", include_str!("fixtures/environment/service.py")])
        .arg(&resources)
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if resources.join("service.sock").exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    fs::write(resources.join("service.pid"), child.id().to_string()).unwrap();
    let running = fs::read(format!("/proc/{}/exe", child.id())).unwrap();
    fs::write(resources.join("installed"), &running).unwrap();
    let service = Service {
        installation_digest: sha256(&running),
        config_digest: sha256(fs::read(resources.join("settings.json")).unwrap()),
    };
    for role in plan.roles.values_mut() {
        role.services.insert("fixture".into(), service.clone());
    }
    approve(&mut registry, &mut plan);
    assert!(check(&registry, &plan, "startup").await.passed());
    let mut settings: Value =
        serde_json::from_slice(&fs::read(resources.join("settings.json")).unwrap()).unwrap();
    settings["threads"] = json!(2);
    fs::write(
        resources.join("settings.json"),
        serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    for role in plan.roles.values_mut() {
        role.expected.insert("schedule.threads".into(), json!(2));
        role.services.get_mut("fixture").unwrap().config_digest =
            sha256(fs::read(resources.join("settings.json")).unwrap());
    }
    approve(&mut registry, &mut plan);
    assert!(
        check(&registry, &plan, "recovery")
            .await
            .differences
            .iter()
            .any(|d| d.field == "service.fixture.effective_config")
    );
    let new = fs::read("/bin/sleep").unwrap();
    fs::write(resources.join("installed"), &new).unwrap();
    for role in plan.roles.values_mut() {
        role.services
            .get_mut("fixture")
            .unwrap()
            .installation_digest = sha256(&new);
    }
    approve(&mut registry, &mut plan);
    let report = check(&registry, &plan, "startup").await;
    assert!(
        report
            .differences
            .iter()
            .any(|d| d.field == "service.fixture.process")
    );
    assert!(
        !report
            .differences
            .iter()
            .any(|d| d.field == "service.fixture.installed")
    );
    fs::write(resources.join("settings.json"), "{").unwrap();
    assert!(check(&registry, &plan, "recovery").await.error.is_some());
    child.kill().unwrap();
    child.wait().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reviewed_identity_role_and_scope_reject_unsupported_changes() {
    let root = temporary();
    let (mut plan, profile) = fixture(&root, "python", "python3", false);
    let mut registry = registry(&root, vec![("python".into(), profile)]);
    registry.resolve(&plan).unwrap();
    plan.roles
        .get_mut("test")
        .unwrap()
        .expected
        .insert("memory".into(), json!(128));
    assert!(registry.resolve(&plan).is_err());
    approve(&mut registry, &mut plan);
    registry.resolve(&plan).unwrap();
    let mut changed = plan.clone();
    changed.lockfiles.push("../Cargo.lock".into());
    approve(&mut registry, &mut changed);
    assert!(registry.resolve(&changed).is_err());
    let mut changed = plan.clone();
    changed.roles.remove("dev");
    approve(&mut registry, &mut changed);
    assert!(registry.resolve(&changed).is_err());
    let mut changed = plan.clone();
    changed
        .roles
        .get_mut("test")
        .unwrap()
        .runtime
        .insert("tool.version".into(), vec![json!("anything")]);
    approve(&mut registry, &mut changed);
    assert!(registry.resolve(&changed).is_err());
    let mut changed = plan.clone();
    changed
        .roles
        .get_mut("test")
        .unwrap()
        .checks
        .push("environment".into());
    approve(&mut registry, &mut changed);
    assert!(registry.resolve(&changed).is_err());
    let mut changed = plan.clone();
    changed.controlled.protocol_version = 0;
    assert!(
        changed
            .validate(&[registry.profiles["python"].registration.clone()])
            .is_err()
    );
    let mut changed = plan.clone();
    changed.cache = Some(Cache {
        identity: sha256("cache"),
        scope: "python".into(),
        capacity_bytes: 1,
        writable: true,
        seed: Some("invalid".into()),
    });
    approve(&mut registry, &mut changed);
    assert!(registry.resolve(&changed).is_err());
    changed.cache.as_mut().unwrap().seed = None;
    changed.cache.as_mut().unwrap().capacity_bytes = 0;
    approve(&mut registry, &mut changed);
    assert!(registry.resolve(&changed).is_err());
    let config_path = root.join("bad-registry.json");
    assert!(Registry::read(&config_path).is_err());
    fs::write(&config_path, "not JSON").unwrap();
    assert!(Registry::read(&config_path).is_err());
    let mut empty = registry.clone();
    empty.profiles.clear();
    fs::write(&config_path, serde_json::to_vec(&empty).unwrap()).unwrap();
    assert!(Registry::read(&config_path).is_err());
    let profile = registry.profiles["python"].clone();
    registry.profiles.insert("overlap".into(), profile.clone());
    assert!(registry.validate().is_err());
    registry.profiles.remove("overlap");
    fs::write(&profile.executable, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(profile.verify_installation().is_err());
    assert!(registry.resolve(&plan).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn forged_missing_or_failed_evidence_never_passes() {
    use codexsymphony_server::controlled_contract::{CheckResult, EvidenceRef};
    let request = environment_probe::Request {
        resource: codexsymphony_server::controlled_contract::ResourceCall {
            protocol_version: 1,
            invocation_id: "one".into(),
            attempt: 1,
            resource_id: "python".into(),
            controlled_config_digest: sha256("controlled"),
            environment: EnvironmentBinding {
                repository_revision: "python@1".into(),
                contract_digest: sha256("plan"),
                host_profile_ref: "python".into(),
                role: "test".into(),
            },
            extension_id: "python".into(),
            implementation_digest: sha256("extension"),
            deadline_unix_ms: i64::MAX,
        },
        call: None,
        invocation_id: "one".into(),
        stage: "test".into(),
        role: "test".into(),
        plan_digest: sha256("plan"),
        host_profile: "python".into(),
        resource_root: "/tmp/resources".into(),
        workspace: None,
        required_checks: vec!["environment".into()],
    };
    let actual = BTreeMap::from([("cache".into(), Value::Null)]);
    let mut response = environment_probe::Response {
        evaluation: None,
        request: request.clone(),
        actual: actual.clone(),
        checks: vec![CheckResult {
            id: "environment".into(),
            verdict: Verdict::Pass,
            evidence: vec![EvidenceRef {
                artifact_id: "actual".into(),
                sha256: sha256(serde_json::to_vec(&actual).unwrap()),
            }],
        }],
    };
    environment_probe::verify_response(&request, &response).unwrap();
    response.request.invocation_id = "stale".into();
    assert!(environment_probe::verify_response(&request, &response).is_err());
    response.request = request.clone();
    response.checks[0].verdict = Verdict::Unknown;
    assert!(environment_probe::verify_response(&request, &response).is_err());
    response.checks[0].verdict = Verdict::Pass;
    response.actual.insert("cache".into(), json!(true));
    assert!(environment_probe::verify_response(&request, &response).is_err());
    response.actual = actual;
    response.checks.push(response.checks[0].clone());
    assert!(environment_probe::verify_response(&request, &response).is_err());
    response.checks.clear();
    assert!(environment_probe::verify_response(&request, &response).is_err());
}

async fn database(repository: &Value) -> sqlx::PgPool {
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    let options: PgConnectOptions = std::env::var("TEST_DATABASE_URL").unwrap().parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "environment_{}",
        process::new_identity().unwrap().replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(options.options([("search_path", schema)]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,$1)")
        .bind(repository)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO requirement(id,version,state,contract,revision) OVERRIDING SYSTEM VALUE VALUES(1,1,'Ready','{}',1)").execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO requirement_revision(requirement_id,revision,document) VALUES(1,1,$1)",
    )
    .bind(json!({"repository":repository,"repository_id":1,"repository_version":1}))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO requirement_budget(requirement_id,version,limits) VALUES(1,7,'{\"tokens\":100,\"turns\":3,\"model_seconds\":60}')").execute(&pool).await.unwrap();
    sqlx::query("UPDATE execution_control SET requirement_id=1 WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

#[tokio::test]
async fn immutable_review_admission_startup_recovery_ci_and_budget_continuity() {
    use codexsymphony_server::environment_service as service;
    let root = temporary();
    let (mut plan, profile) = fixture(&root, "python", "python3", false);
    let mut registry = registry(&root, vec![("python".into(), profile)]);
    let config = root.join("registry.json");
    registry
        .profiles
        .get_mut("python")
        .unwrap()
        .registration
        .scope_ref = "repository:1".into();
    plan.controlled.extensions = vec![registry.profiles["python"].registration.clone()];
    plan.controlled.environment.repository_revision = "repository:1@1".into();
    plan.host_profile_digest = registry.profiles["python"].identity(&registry.evidence_root);
    approve(&mut registry, &mut plan);
    assert!(plan.matches_repository(1, 1));
    assert!(!plan.matches_repository(2, 1));
    assert!(!plan.matches_repository(1, 2));
    fs::write(&config, serde_json::to_vec(&registry).unwrap()).unwrap();
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
        std::env::set_var("ENVIRONMENT_CONFIG", &config);
    }
    let pool = database(&json!({"environment":serde_json::to_string(&plan).unwrap(),"revoked":false,"project":"Python","remote":"owner/python","github_repository_id":1,"base_branch":"main","reason":"environment test","policy":{"allowed_checks":["npm_test"],"max_timeout_seconds":60,"token_limit":100,"turn_limit":3,"model_work_seconds":60,"gate_recovery_policy":"one_code_repair"}})).await;
    let repository: codexsymphony_server::contract::Repository = serde_json::from_value(
        sqlx::query_scalar::<_, Value>("SELECT document FROM repository WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap(),
    )
    .unwrap();
    let serialized = serde_json::to_vec(&repository).unwrap();
    for remaining in 0..serialized.len() {
        assert!(serde_json::to_writer(LimitedWriter(remaining), &repository).is_err());
    }
    let encoded = serde_json::to_value(&repository).unwrap();
    assert!(encoded["environment"].is_string());
    assert_eq!(
        serde_json::from_value::<codexsymphony_server::contract::Repository>(encoded).unwrap(),
        repository
    );
    service::enable(None).await.unwrap();
    service::enable(Some(&plan)).await.unwrap();
    service::startup(&pool).await.unwrap();
    service::recovery(&pool).await.unwrap();
    for stage in ["preparation", "validation", "delivery"] {
        service::admit(&pool, 1, 1, stage, None).await.unwrap();
    }
    let latest: Value =
        sqlx::query_scalar("SELECT report FROM environment_observation ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let report: environment_probe::Report = serde_json::from_value(latest).unwrap();
    let mut response = report.response.unwrap();
    response.checks.push(response.checks[0].clone());
    assert!(environment_probe::verify_response(&report.request, &response).is_err());
    response.evaluation = None;
    assert!(environment_probe::verify_response(&report.request, &response).is_err());
    preparation_fixture::exercise(&pool, &root).await;
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 601;
    let retry = json!({"phase":"preparation","started_at":started,"todo":false,"next_attempt_at":null,"attempts":2});
    sqlx::query("INSERT INTO preparation_record(run_id,requirement_id,revision,launch,retry) VALUES('deadline',1,1,'{}',$1)").bind(&retry).execute(&pool).await.unwrap();
    assert!(
        service::admit(&pool, 1, 1, "preparation", None)
            .await
            .is_err()
    );
    let deadline_report: Value =
        sqlx::query_scalar("SELECT report FROM environment_observation ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        deadline_report["error"]
            .as_str()
            .unwrap()
            .contains("deadline exhausted")
    );
    assert!(
        Path::new(deadline_report["evidence"].as_str().unwrap())
            .join("not-started.json")
            .exists()
    );
    service::admit(&pool, 1, 1, "validation", None)
        .await
        .unwrap();
    let retained: Value =
        sqlx::query_scalar("SELECT retry FROM preparation_record WHERE run_id='deadline'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(retained, retry);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM environment_observation")
        .fetch_one(&pool)
        .await
        .unwrap();
    service::admit(&pool, 1, 1, "ci", None).await.unwrap();
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM environment_observation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, after);
    // A newly edited repository default cannot upgrade an already reviewed task.
    sqlx::query("UPDATE repository SET document='{}',version=2 WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(service::plan(&pool, 1, 1).await.unwrap().unwrap(), plan);
    fs::write(root.join("python/state"), "broken").unwrap();
    for stage in ["preparation", "validation", "delivery", "recovery"] {
        assert!(service::admit(&pool, 1, 1, stage, None).await.is_err());
    }
    assert!(
        codexsymphony_server::coordinator::recover(&pool, &root, "current")
            .await
            .is_err()
    );
    let repository: Value = sqlx::query_scalar("SELECT document->'repository' FROM execution_revision WHERE requirement_id=1 AND revision=1").fetch_one(&pool).await.unwrap();
    let app = auth_client::router(
        pool.clone(),
        codexsymphony_server::security::RequestPolicy::new(
            "127.0.0.1:3081".parse().unwrap(),
            "https://localhost:4200".into(),
        )
        .unwrap(),
    );
    use tower::ServiceExt;
    for (version, status) in [(1, 422), (0, 409)] {
        let response = app.clone().oneshot(axum::http::Request::builder().method("PUT").uri("/api/multi/repository")
                    .header("host", "127.0.0.1:3081")
                    .header("x-codexsymphony-csrf", "1")
                    .header("origin", "https://localhost:4200").header("content-type","application/json").body(axum::body::Body::from(serde_json::to_vec(&json!({"request_id":format!("environment-{version}"),"repository_id":1,"version":version,"repository":repository})).unwrap())).unwrap()).await.unwrap();
        assert_eq!(response.status().as_u16(), status);
    }
    let budget:Value=sqlx::query_scalar("SELECT jsonb_build_object('version',version,'limits',limits) FROM requirement_budget WHERE requirement_id=1").fetch_one(&pool).await.unwrap();
    assert_eq!(
        budget,
        json!({"version":7,"limits":{"tokens":100,"turns":3,"model_seconds":60}})
    );
    let calls: i64 = sqlx::query_scalar("SELECT count(*) FROM model_call")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(calls, 0);
    let attempts: i64 = sqlx::query_scalar("SELECT count(*) FROM repair_reservation")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(attempts, 0);
    let stored: Value =
        sqlx::query_scalar("SELECT report FROM environment_observation ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored["differences"][0]["field"], "runtime.state");
    let path = root.join("plan.json");
    fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let cli = |stage: &str| {
        Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
            .arg("--environment-check")
            .arg(&path)
            .arg(stage)
            .env("ENVIRONMENT_CONFIG", &config)
            .output()
            .unwrap()
    };
    let skipped = cli("ci");
    assert!(skipped.status.success());
    assert!(
        String::from_utf8(skipped.stdout)
            .unwrap()
            .contains("not_applicable")
    );
    assert!(!cli("validation").status.success());
    assert!(!cli("invalid").status.success());
    plan.ci = true;
    approve(&mut registry, &mut plan);
    fs::write(&config, serde_json::to_vec(&registry).unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
    assert!(!cli("ci").status.success());
    fs::write(root.join("python/state"), "ready").unwrap();
    assert!(cli("ci").status.success());
    // The old policy stays frozen, and the new host approval does not bless it.
    assert!(
        service::admit(&pool, 1, 1, "validation", None)
            .await
            .is_err()
    );
    pool.close().await;
    unsafe {
        std::env::remove_var("ENVIRONMENT_CONFIG");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cache_permissions_capacity_seed_identity_and_symlinks_are_checked() {
    use codexsymphony_server::environment_cache;
    let root = temporary();
    let (mut plan, profile) = fixture(&root, "cached", "python3", true);
    let registry = registry(&root, vec![("cached".into(), profile.clone())]);
    let mut cache = plan.cache.take().unwrap();
    environment_cache::check(&registry, &profile, Some(&cache)).unwrap();
    let directory = profile.resource_root.join("cache").join(&cache.identity);
    cache.writable = false;
    assert!(environment_cache::check(&registry, &profile, Some(&cache)).is_err());
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
    environment_cache::check(&registry, &profile, Some(&cache)).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    cache.writable = true;
    std::os::unix::fs::symlink(root.join("cached/settings.json"), directory.join("link")).unwrap();
    assert!(environment_cache::check(&registry, &profile, Some(&cache)).is_err());
    fs::remove_file(directory.join("link")).unwrap();
    let manifest =
        serde_json::to_vec(&BTreeMap::from([("nested/data", sha256("trusted"))])).unwrap();
    let identity = sha256(&manifest);
    let seed = registry.evidence_root.join("seeds").join(&identity);
    fs::create_dir_all(&seed).unwrap();
    fs::write(seed.join("manifest.json"), manifest).unwrap();
    fs::create_dir(seed.join("nested")).unwrap();
    fs::write(seed.join("nested/data"), "trusted").unwrap();
    cache.seed = Some(identity);
    assert!(environment_cache::check(&registry, &profile, Some(&cache)).is_err());
    for path in [seed.join("manifest.json"), seed.join("nested/data")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o400)).unwrap();
    }
    fs::set_permissions(seed.join("nested"), fs::Permissions::from_mode(0o500)).unwrap();
    fs::set_permissions(&seed, fs::Permissions::from_mode(0o500)).unwrap();
    environment_cache::check(&registry, &profile, Some(&cache)).unwrap();
    fs::set_permissions(seed.join("nested/data"), fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(seed.join("nested/data"), "tampered").unwrap();
    fs::set_permissions(seed.join("nested/data"), fs::Permissions::from_mode(0o400)).unwrap();
    assert!(environment_cache::check(&registry, &profile, Some(&cache)).is_err());
    fs::set_permissions(seed.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(seed, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn timeout_stops_descendants_and_unreconciled_intent_blocks_another_probe() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = temporary();
    let (mut plan, mut profile) = fixture(&root, "timeout", "python3", false);
    fs::write(&profile.executable, "#!/bin/sh\nsleep 90\n").unwrap();
    profile.timeout_seconds = 20;
    profile.registration.implementation_digest = sha256(fs::read(&profile.executable).unwrap());
    plan.controlled.extensions = vec![profile.registration.clone()];
    plan.host_profile_digest = profile.identity(&root.join("evidence"));
    let mut registry = registry(&root, vec![("timeout".into(), profile)]);
    approve(&mut registry, &mut plan);
    let regular = plan.clone();
    plan.roles
        .get_mut("test")
        .unwrap()
        .checks
        .push("large".repeat(15000));
    approve(&mut registry, &mut plan);
    assert!(
        environment_probe::check(&registry, &plan, "preparation", "test", None)
            .await
            .unwrap_err()
            .to_string()
            .contains("64 KiB")
    );
    plan = regular;
    approve(&mut registry, &mut plan);
    let report = check(&registry, &plan, "preparation").await;
    assert!(
        report.error.as_ref().unwrap().contains("timeout"),
        "{report:?}"
    );
    assert!(report.evidence.join("quiescent.json").exists());
    let unknown = registry.evidence_root.join("unknown");
    fs::create_dir(&unknown).unwrap();
    fs::write(unknown.join("input.json"), "{}").unwrap();
    let error = environment_probe::check(&registry, &plan, "preparation", "test", None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("reconcile"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resource_envelopes_and_host_profiles_cannot_relabel_approval() {
    use codexsymphony_server::controlled_contract::ResourceCall;
    let root = temporary();
    let (plan, profile) = fixture(&root, "python", "python3", false);
    let approved = vec![profile.registration.clone()];
    let call = ResourceCall {
        protocol_version: 1,
        invocation_id: "check".into(),
        attempt: 1,
        resource_id: "python".into(),
        controlled_config_digest: plan.controlled.freeze(&approved).unwrap(),
        environment: plan.controlled.environment.clone(),
        extension_id: "python".into(),
        implementation_digest: profile.registration.implementation_digest.clone(),
        deadline_unix_ms: 100,
    };
    call.validate(&plan.controlled, &approved).unwrap();
    let mut wrong = call.clone();
    wrong.protocol_version = 2;
    assert!(wrong.validate(&plan.controlled, &approved).is_err());
    let mut wrong = call.clone();
    wrong.attempt = 0;
    assert!(wrong.validate(&plan.controlled, &approved).is_err());
    let mut wrong = call.clone();
    wrong.environment.role = "ci".into();
    assert!(wrong.validate(&plan.controlled, &approved).is_err());
    let mut wrong = call;
    wrong.implementation_digest = sha256("replaced");
    assert!(wrong.validate(&plan.controlled, &approved).is_err());
    let mut registry = registry(&root, vec![("python".into(), profile)]);
    let original_evidence = registry.evidence_root.clone();
    fs::write(&original_evidence, "not a directory").unwrap();
    assert!(registry.validate().is_err());
    fs::remove_file(&original_evidence).unwrap();
    registry.evidence_root = root.join("missing-parent/evidence");
    assert!(registry.validate().is_err());
    registry.evidence_root = registry.profiles["python"].resource_root.clone();
    assert!(registry.validate().is_err());
    fs::create_dir(root.join("extra")).unwrap();
    registry.evidence_root = root.join("extra/../evidence");
    assert!(registry.validate().is_err());
    registry.evidence_root = original_evidence;
    let approved_registry = registry.clone();
    registry.profiles.get_mut("python").unwrap().timeout_seconds = 0;
    assert!(registry.validate().is_err());
    registry = approved_registry.clone();
    registry
        .profiles
        .get_mut("python")
        .unwrap()
        .registration
        .credential_provider_ref = Some("delivery-secret".into());
    assert!(registry.validate().is_err());
    registry = approved_registry.clone();
    let original_root = registry.profiles["python"].resource_root.clone();
    registry.profiles.get_mut("python").unwrap().resource_root = original_root.join("..");
    assert!(registry.validate().is_err());
    registry = approved_registry.clone();
    let embedded = original_root.join("embedded-probe");
    fs::copy(&registry.profiles["python"].executable, &embedded).unwrap();
    registry.profiles.get_mut("python").unwrap().executable = embedded;
    assert!(registry.validate().is_err());
    registry = approved_registry;
    registry.profiles.get_mut("python").unwrap().timeout_seconds = 600;
    assert!(registry.resolve(&plan).is_err());
    fs::remove_dir_all(root).unwrap();
}

/// An interrupted output must never be mistaken for a complete configuration.
struct LimitedWriter(usize);
impl std::io::Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0 == 0 {
            return Err(std::io::Error::other("controlled output failure"));
        }
        let written = self.0.min(bytes.len());
        self.0 -= written;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn p9_validation_is_bound_to_task_environment_and_retained_source_at_delivery() {
    use codexsymphony_server::{
        environment_service, validation_context, validation_runner, validation_service,
    };
    let root = temporary();
    let (mut environment, profile) = fixture(&root, "python", "python3", false);
    let mut registry = registry(&root, vec![("python".into(), profile)]);
    registry
        .profiles
        .get_mut("python")
        .unwrap()
        .registration
        .scope_ref = "repository:1".into();
    environment.controlled.extensions = vec![registry.profiles["python"].registration.clone()];
    environment.controlled.environment.repository_revision = "repository:1@1".into();
    environment.host_profile_digest = registry.profiles["python"].identity(&registry.evidence_root);
    delivery_hook_fixture::install(&root, &mut environment, &mut registry);
    environment.host_profile_digest = registry.profiles["python"].identity(&registry.evidence_root);
    approve(&mut registry, &mut environment);
    let config = root.join("registry.json");
    fs::write(&config, serde_json::to_vec(&registry).unwrap()).unwrap();
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
        std::env::set_var("ENVIRONMENT_CONFIG", &config);
    }
    let repository = json!({"environment":serde_json::to_string(&environment).unwrap(),"revoked":false,"project":"Python","remote":"owner/python","github_repository_id":1,"base_branch":"main","reason":"reviewed complete validation","policy":{"allowed_checks":["validate"],"max_timeout_seconds":60,"token_limit":100,"turn_limit":3,"model_work_seconds":60,"gate_recovery_policy":"one_code_repair"}});
    let pool = database(&repository).await;
    let contract = json!({"title":"project test","description":"candidate check","acceptance_criteria":[{"description":"passes","verification_ref":"test"}],"validation_plan":[{"id":"test","check":"validate","selector":"test","expected_result":"pass","timeout_seconds":30}],"network_access":[]});
    let typed_contract: codexsymphony_server::contract::Contract =
        serde_json::from_value(contract.clone()).unwrap();
    let mut typed_repository: codexsymphony_server::contract::Repository =
        serde_json::from_value(repository.clone()).unwrap();
    codexsymphony_server::contract::authorize(&typed_contract, &typed_repository).unwrap();
    typed_repository.environment = None;
    assert!(codexsymphony_server::contract::authorize(&typed_contract, &typed_repository).is_err());
    sqlx::query("UPDATE requirement_revision SET document=document || jsonb_build_object('contract',$1::jsonb) WHERE requirement_id=1").bind(contract).execute(&pool).await.unwrap();
    sqlx::raw_sql("UPDATE requirement SET state='Running' WHERE id=1; UPDATE execution_control SET recovery_complete=true,paused=false WHERE id=1;").execute(&pool).await.unwrap();
    let checkout = root.join("candidate");
    assert!(
        Command::new("git")
            .args(["clone", "-q"])
            .arg(root.join("python"))
            .arg(&checkout)
            .status()
            .unwrap()
            .success()
    );
    let candidate = validation_runner::candidate(&checkout).unwrap();
    sqlx::query("INSERT INTO agent_run(id,requirement_id,revision,incarnation,request_id,workspace,workspace_identity,launch,state,quiescent) VALUES('source',1,1,'boot','request',$1,'source','{}','Succeeded',true)").bind(checkout.to_str().unwrap()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspace_snapshot(run_id,manifest,candidate) VALUES('source',$1,true)").bind(json!({"head":candidate.sha,"workspace":{"branch":"ai/req-1-source","baseline":candidate.sha}})).execute(&pool).await.unwrap();
    let entry = root.join("project-validation");
    let script = "#!/bin/sh\ntest -f README.md && printf 'complete project test passed\\n'\n";
    fs::write(&entry, script).unwrap();
    fs::set_permissions(&entry, fs::Permissions::from_mode(0o700)).unwrap();
    let plan = validation_runner::Plan {
        entry,
        entry_sha256: sha256(script),
        steps: vec![validation_runner::Step {
            id: "test".into(),
            command: vec!["/gate-entry".into()],
            timeout_seconds: 10,
            code_failure: true,
        }],
    };
    let runtime_config = root.join("runtime.json");
    let runtime_value = json!({"validation":plan,"settings":{"startup_seconds":10,"response_seconds":10,"stall_seconds":10,"reservation":{"tokens":1,"turns":1,"model_seconds":1},"codex_config":""},"preparation_adapter":"/bin/true","preparation":{"launcher":["/bin/true"]}});
    fs::write(&runtime_config, serde_json::to_vec(&runtime_value).unwrap()).unwrap();
    unsafe {
        std::env::set_var("RUNTIME_CONFIG", &runtime_config);
    }
    let directory = root.join("validate");
    let request = || validation_service::Request {
        id: "validate",
        source_run: "source",
        requirement: 1,
        revision: 1,
        checkout: &checkout,
        directory: &directory,
        candidate: &candidate,
        plan: &plan,
    };
    // No Agent test declaration is consumed: the service executes the approved hook.
    assert!(
        validation_service::validate(&pool, request())
            .await
            .unwrap()
    );
    let action: String =
        sqlx::query_scalar("SELECT action_key FROM delivery WHERE validation_id='validate'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let evaluation: Value =
        sqlx::query_scalar("SELECT hook_evaluation FROM candidate_validation WHERE id='validate'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(evaluation["verdict"], "pass");
    assert_eq!(evaluation["call"]["identity"]["requirement_id"], 1);
    assert!(
        validation_service::validate(&pool, request())
            .await
            .unwrap()
    );
    environment_service::admit(&pool, 1, 1, "delivery", None)
        .await
        .unwrap();
    validation_context::delivery(&pool, &action).await.unwrap();
    delivery_hook_fixture::exercise(&root, &pool, &action).await;
    let source_identity = codexsymphony_server::controlled_contract::SourceIdentity {
        commit: candidate.sha.clone(),
        tree: candidate.tree.clone(),
    };
    validation_context::admit(&pool, "validate", 1, 1, &source_identity)
        .await
        .unwrap();
    // A saved PASS cannot reconstruct missing raw evidence or rerun its Hook.
    let binding = fs::read(directory.join("binding.json")).unwrap();
    fs::remove_file(directory.join("binding.json")).unwrap();
    let error = validation_service::validate(&pool, request())
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("accepted validation evidence unavailable")
    );
    assert!(!directory.join("binding.json").exists());
    fs::write(directory.join("binding.json"), binding).unwrap();
    reject_damaged_validation_records(&pool, &action).await;
    reject_interrupted_validation_recovery(&pool, &directory).await;
    validation_context::delivery(&pool, &action).await.unwrap();
    let wrong_source = codexsymphony_server::controlled_contract::SourceIdentity {
        commit: "other".into(),
        tree: candidate.tree.clone(),
    };
    assert!(
        validation_context::admit(&pool, "validate", 1, 1, &wrong_source)
            .await
            .is_err()
    );
    assert!(
        validation_context::admit(&pool, "validate", 1, 2, &source_identity)
            .await
            .is_err()
    );
    let mut changed_runtime = runtime_value.clone();
    changed_runtime["validation"]["steps"][0]["command"] = json!(["/gate-entry", "replacement"]);
    fs::write(
        &runtime_config,
        serde_json::to_vec(&changed_runtime).unwrap(),
    )
    .unwrap();
    assert!(validation_context::delivery(&pool, &action).await.is_err());
    fs::write(&runtime_config, serde_json::to_vec(&runtime_value).unwrap()).unwrap();
    validation_context::delivery(&pool, &action).await.unwrap();
    unsafe {
        std::env::remove_var("RUNTIME_CONFIG");
    }
    if let Some(path) = std::env::var_os("GH120_EVIDENCE_DIR") {
        let target = PathBuf::from(path);
        fs::create_dir_all(&target).unwrap();
        for name in [
            "input.json",
            "invocation.json",
            "binding.json",
            "evaluation.json",
            "result.json",
            "step-0.log",
            "identity.json",
            "quiescent.json",
            "exit.json",
        ] {
            fs::copy(directory.join(name), target.join(name)).unwrap();
        }
        fs::write(
            target.join("environment-plan.json"),
            serde_json::to_vec_pretty(&environment).unwrap(),
        )
        .unwrap();
        fs::write(
            target.join("source.json"),
            serde_json::to_vec_pretty(&candidate).unwrap(),
        )
        .unwrap();
        fs::copy(
            checkout.join("README.md"),
            target.join("candidate-README.md"),
        )
        .unwrap();
    }
    fs::write(checkout.join("uncommitted"), "after PASS").unwrap();
    assert!(validation_context::delivery(&pool, &action).await.is_err());
    fs::remove_file(checkout.join("uncommitted")).unwrap();
    fs::write(root.join("python/state"), "idle").unwrap();
    environment_service::admit(&pool, 1, 1, "delivery", None)
        .await
        .unwrap();
    assert!(validation_context::delivery(&pool, &action).await.is_err());
    fs::write(root.join("python/state"), "ready").unwrap();
    environment_service::admit(&pool, 1, 1, "delivery", None)
        .await
        .unwrap();
    validation_context::delivery(&pool, &action).await.unwrap();
    fs::write(&plan.entry, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(validation_context::delivery(&pool, &action).await.is_err());
    fs::write(&plan.entry, script).unwrap();
    validation_context::delivery(&pool, &action).await.unwrap();
    let stop_proof = fs::read(directory.join("quiescent.json")).unwrap();
    fs::remove_file(directory.join("quiescent.json")).unwrap();
    assert!(validation_context::delivery(&pool, &action).await.is_err());
    fs::write(directory.join("quiescent.json"), stop_proof).unwrap();
    validation_context::delivery(&pool, &action).await.unwrap();
    sqlx::query("UPDATE repository SET document=jsonb_set(document,'{revoked}','true') WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(validation_context::delivery(&pool, &action).await.is_err());
    sqlx::query(
        "UPDATE repository SET document=jsonb_set(document,'{revoked}','false') WHERE id=1",
    )
    .execute(&pool)
    .await
    .unwrap();
    validation_context::delivery(&pool, &action).await.unwrap();
    delivery_hook_fixture::cancel_running_hook(&root, &pool, &action).await;
    sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(validation_context::delivery(&pool, &action).await.is_err());
    sqlx::query("UPDATE requirement SET paused=false WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    assert!(validation_context::delivery(&pool, &action).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM agent_run WHERE id='source'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "Succeeded"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM requirement WHERE id=1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "Running"
    );
    seed_hook_retry(&pool, "validation-failure", &plan).await;
    fs::set_permissions(&plan.entry, fs::Permissions::from_mode(0o600)).unwrap();
    let failed_directory = root.join("validation-failure");
    let failed = validation_service::Request {
        id: "validation-failure",
        directory: &failed_directory,
        ..request()
    };
    assert!(validation_service::validate(&pool, failed).await.is_err());
    let unknown: Value = sqlx::query_scalar(
        "SELECT hook_evaluation FROM candidate_validation WHERE id='validation-failure'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(unknown["verdict"], "unknown");
    assert!(failed_directory.join("stderr.log").exists());
    let slow_entry = root.join("slow-validation");
    let slow_script = "#!/bin/sh\nprintf 'starting real check\\n'\nsleep 30\n";
    fs::write(&slow_entry, slow_script).unwrap();
    fs::set_permissions(&slow_entry, fs::Permissions::from_mode(0o700)).unwrap();
    let slow_plan = validation_runner::Plan {
        entry: slow_entry,
        entry_sha256: sha256(slow_script),
        steps: plan.steps.clone(),
    };
    seed_hook_retry(&pool, "validation-cancel", &slow_plan).await;
    let cancelled_directory = root.join("validation-cancel");
    let cancelled = validation_service::Request {
        id: "validation-cancel",
        directory: &cancelled_directory,
        plan: &slow_plan,
        ..request()
    };
    let stop = async {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !cancelled_directory.join("step-0.log").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "validation failed to start"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        sqlx::query("UPDATE requirement SET paused=true WHERE id=1")
            .execute(&pool)
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(validation_service::validate(&pool, cancelled), stop);
    assert!(result.is_err());
    assert!(cancelled_directory.join("quiescent.json").exists());
    assert!(cancelled_directory.join("stop.json").exists());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM delivery")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    unsafe {
        std::env::remove_var("ENVIRONMENT_CONFIG");
    }
    pool.close().await;
    fs::remove_dir_all(root).unwrap();
}

async fn reject_damaged_validation_records(pool: &sqlx::PgPool, action: &str) {
    use codexsymphony_server::validation_context;
    let saved: (Value, Value, Value) = sqlx::query_as(
        "SELECT hook_context,hook_evaluation,approved_plan FROM candidate_validation WHERE id='validate'",
    ).fetch_one(pool).await.unwrap();
    for (context, evaluation, plan) in [
        (json!({}), Some(saved.1.clone()), Some(saved.2.clone())),
        (saved.0.clone(), None, Some(saved.2.clone())),
        (saved.0.clone(), Some(json!({})), Some(saved.2.clone())),
        (saved.0.clone(), Some(saved.1.clone()), None),
        (saved.0.clone(), Some(saved.1.clone()), Some(json!({}))),
    ] {
        sqlx::query("UPDATE candidate_validation SET hook_context=$1,hook_evaluation=$2,approved_plan=$3 WHERE id='validate'")
            .bind(context).bind(evaluation).bind(plan).execute(pool).await.unwrap();
        assert!(validation_context::delivery(pool, action).await.is_err());
    }
    sqlx::query("UPDATE candidate_validation SET hook_context=$1,hook_evaluation=$2,approved_plan=$3 WHERE id='validate'")
        .bind(saved.0).bind(saved.1).bind(saved.2).execute(pool).await.unwrap();
    validation_context::delivery(pool, action).await.unwrap();
}

async fn reject_interrupted_validation_recovery(pool: &sqlx::PgPool, directory: &Path) {
    use codexsymphony_server::{process, validation_supervisor};
    let job =
        || process::read::<validation_supervisor::Job>(&directory.join("input.json")).unwrap();
    let before = fs::read(directory.join("result.json")).unwrap();
    validation_supervisor::execute(pool, job(), false)
        .await
        .unwrap();
    let stopped = fs::read(directory.join("quiescent.json")).unwrap();
    fs::remove_file(directory.join("quiescent.json")).unwrap();
    let error = validation_supervisor::execute(pool, job(), false)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("validation process outcome unknown")
    );
    assert!(directory.join("stop.json").exists());
    assert!(!directory.join("quiescent.json").exists());
    assert_eq!(fs::read(directory.join("result.json")).unwrap(), before);
    // Restore the fixture's original proof; even then the stop intent forbids admission.
    fs::write(directory.join("quiescent.json"), &stopped).unwrap();
    assert!(
        validation_supervisor::execute(pool, job(), false)
            .await
            .is_err()
    );
    fs::remove_file(directory.join("stop.json")).unwrap();
    fs::write(directory.join("quiescent.json"), b"{}").unwrap();
    assert!(
        validation_supervisor::execute(pool, job(), false)
            .await
            .is_err()
    );
    fs::write(directory.join("quiescent.json"), stopped).unwrap();
}

async fn seed_hook_retry(
    pool: &sqlx::PgPool,
    id: &str,
    plan: &codexsymphony_server::validation_runner::Plan,
) {
    sqlx::query("INSERT INTO candidate_validation(id,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,trusted,required_steps,source_before,source_after,entry_before,entry_after,stage,result,retry_of) SELECT $1,requirement_id,revision,source_run_id,candidate_sha,candidate_tree,$2,required_steps,source_before,source_before,$3,$3,'declaration','pending',id FROM candidate_validation WHERE id='validate'")
        .bind(id).bind(json!(plan.identity().unwrap())).bind(&plan.entry_sha256).execute(pool).await.unwrap();
}
