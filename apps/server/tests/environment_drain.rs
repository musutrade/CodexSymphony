//! Stop drain with a real supervisor. The drain gate is process-global, so the
//! library-level case lives in its own test binary; the service cases run the
//! real binary as a child process.
use codexsymphony_server::{
    controlled_contract::{ControlledConfig, EnvironmentBinding, Operation, Registration},
    environment::{Plan, Role},
    environment_host::{Profile, Registry},
    environment_probe, process,
    validation::sha256,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

// One real service at a time: it takes the host controller instance lock.
static SERVICE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

/// Same controlled probe as `tests/environment.rs`, held for `seconds` after
/// the supervisor has launched it. A `state` other than `ready`/`idle` makes
/// the probe complete with a difference, i.e. a real failed admission.
fn fixture(root: &Path, seconds: f64, state: &str) -> (Plan, Registry) {
    let name = "drain";
    let resources = root.join(name);
    fs::create_dir(&resources).unwrap();
    fs::write(
        resources.join("settings.json"),
        serde_json::to_vec(
            &json!({"tool":"python3","threads":1,"image":"none","memory":64,"cache":null}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::write(resources.join("state"), state).unwrap();
    let executable = root.join(format!("{name}-probe"));
    let script = include_str!("fixtures/environment/probe.py").replace(
        "request = json.load(sys.stdin)",
        &format!("request = json.load(sys.stdin)\nimport time\ntime.sleep({seconds})"),
    );
    fs::write(&executable, script).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let registration = Registration {
        id: name.into(),
        implementation_digest: sha256(fs::read(&executable).unwrap()),
        operations: vec![Operation::EnvironmentCheck],
        scope_ref: "all".into(),
        config_ref: format!("{name}-config"),
        credential_provider_ref: None,
    };
    let (version, digest) = tool("python3");
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
            protocol_version: 2,
            environment: EnvironmentBinding {
                repository_revision: "repository:1@1".into(),
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
        cache: None,
    };
    let mut profile = Profile {
        extensions: vec![],
        registration,
        executable,
        approved_plans: vec![],
        resource_root: resources,
        timeout_seconds: 60,
    };
    plan.host_profile_digest = profile.identity(&root.join("evidence"));
    plan.controlled.environment.contract_digest = plan.contract_digest();
    profile.approved_plans = vec![plan.digest()];
    let registry = Registry {
        evidence_root: root.join("evidence"),
        profiles: BTreeMap::from([(name.into(), profile)]),
    };
    (plan, registry)
}

fn directories(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return vec![];
    };
    let mut found: Vec<_> = entries.map(|entry| entry.unwrap().path()).collect();
    found.sort();
    found
}

#[tokio::test]
async fn drain_waits_for_real_quiescence_and_admits_no_new_probe() {
    unsafe {
        std::env::set_var(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        );
    }
    let root = std::env::temp_dir().join(format!(
        "environment-drain-{}",
        process::new_identity().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    let (plan, registry) = fixture(&root, 1.0, "ready");
    let evidence = registry.evidence_root.clone();
    let check = |stage: &'static str| {
        let (registry, plan) = (registry.clone(), plan.clone());
        tokio::spawn(async move {
            environment_probe::check(&registry, &plan, stage, "test", None).await
        })
    };

    // One probe in flight under the real supervisor, another queued on the lock.
    let running = check("recovery");
    let deadline = Instant::now() + Duration::from_secs(30);
    let launched = loop {
        let started: Vec<_> = directories(&evidence)
            .into_iter()
            .filter(|directory| directory.join("identity.json").exists())
            .collect();
        if let [directory] = started.as_slice() {
            break directory.clone();
        }
        assert!(Instant::now() < deadline, "probe never launched");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(!launched.join("quiescent.json").exists());
    let queued = check("recovery");
    tokio::time::sleep(Duration::from_millis(100)).await;

    environment_probe::drain_within(&evidence, environment_probe::drain_limit(&registry))
        .await
        .unwrap();
    assert_eq!(
        fs::read(launched.join("identity.json")).unwrap(),
        fs::read(launched.join("quiescent.json")).unwrap()
    );
    let report = running.await.unwrap().unwrap();
    assert!(report.passed(), "{report:?}");
    assert_eq!(report.evidence, launched);

    // Neither the queued probe nor a later one creates evidence or a supervisor.
    let late = check("recovery");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!queued.is_finished());
    assert!(!late.is_finished());
    assert_eq!(directories(&evidence), vec![launched.clone()]);
    queued.abort();
    late.abort();
    environment_probe::drain_within(&evidence, Duration::from_secs(1))
        .await
        .unwrap();
    fs::remove_dir_all(root).unwrap();
}

/// Isolated schema holding one repository whose reviewed environment makes
/// normal startup run a real probe. Returns a URL bound to that schema.
async fn database(plan: &Plan) -> String {
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    let base = std::env::var("TEST_DATABASE_URL").expect("disposable TEST_DATABASE_URL required");
    let options: PgConnectOptions = base.parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!(
        "environment_drain_{}",
        process::new_identity().unwrap().replace('-', "")
    );
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.options([("search_path", schema.clone())]))
        .await
        .unwrap();
    sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
    let repository = json!({"environment":serde_json::to_string(plan).unwrap(),"revoked":false,"project":"Drain","remote":"owner/drain","github_repository_id":1,"base_branch":"main","reason":"environment drain test","policy":{"allowed_checks":["npm_test"],"max_timeout_seconds":60,"token_limit":100,"turn_limit":3,"model_work_seconds":60,"gate_recovery_policy":"one_code_repair"}});
    sqlx::query("INSERT INTO repository(id,version,document) VALUES(1,1,$1)")
        .bind(repository)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let mut url = reqwest::Url::parse(&base).unwrap();
    url.query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    url.into()
}

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts the real server; its startup admission probes `registry`.
async fn service(root: &Path, plan: &Plan, registry: &Registry) -> Service {
    let config = root.join("registry.json");
    fs::write(&config, serde_json::to_vec(registry).unwrap()).unwrap();
    let auth = root.join("auth.json");
    fs::write(
        &auth,
        r#"{"public_origin":"https://localhost:4200","trusted_proxies":[]}"#,
    )
    .unwrap();
    let execution = root.join("execution");
    fs::create_dir_all(&execution).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", database(plan).await)
        .env("BIND_ADDRESS", "127.0.0.1:0")
        .env("WEB_ORIGIN", "https://localhost:4200")
        .env("AUTH_CONFIG", &auth)
        .env("EXECUTION_DIRECTORY", &execution)
        .env("ENVIRONMENT_CONFIG", &config)
        .env(
            "SYMPHONY_SUPERVISOR",
            env!("CARGO_BIN_EXE_codexsymphony-server"),
        )
        .env("RUST_LOG", "info")
        .env_remove("RUNTIME_CONFIG")
        .env_remove("STORAGE_CONFIG")
        .env_remove("GITHUB_APP_CONFIG")
        .stdout(fs::File::create(root.join("stdout.log")).unwrap())
        .stderr(fs::File::create(root.join("stderr.log")).unwrap())
        .spawn()
        .unwrap();
    Service(child)
}

fn launched(evidence: &Path) -> PathBuf {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let started: Vec<_> = directories(evidence)
            .into_iter()
            .filter(|directory| directory.join("identity.json").exists())
            .collect();
        if let [directory] = started.as_slice() {
            assert!(!directory.join("quiescent.json").exists());
            return directory.clone();
        }
        assert!(Instant::now() < deadline, "startup probe never launched");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn terminate(service: &Service) {
    assert!(
        Command::new("kill")
            .args(["-TERM", &service.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
}

/// Bounded by the registry drain limit plus process start/exit slack.
fn exit(service: &mut Service, registry: &Registry) -> std::process::ExitStatus {
    let deadline =
        Instant::now() + environment_probe::drain_limit(registry) + Duration::from_secs(15);
    loop {
        if let Some(status) = service.0.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "service did not stop");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn quiescent(directory: &Path) {
    assert_eq!(
        fs::read(directory.join("identity.json")).unwrap(),
        fs::read(directory.join("quiescent.json")).unwrap()
    );
}

fn temporary() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "environment-drain-service-{}",
        process::new_identity().unwrap()
    ));
    fs::create_dir(&root).unwrap();
    root
}

/// SIGTERM while the startup probe is in flight: the probe finishes under its
/// real supervisor, the next startup probe is held at the gate (startup never
/// completes), and the process still exits within the drain limit.
#[tokio::test]
async fn sigterm_during_startup_probe_drains_real_supervisor_and_exits() {
    let _serial = SERVICE.lock().await;
    let root = temporary();
    let (plan, registry) = fixture(&root, 2.0, "ready");
    let mut service = service(&root, &plan, &registry).await;
    let directory = launched(&registry.evidence_root);
    terminate(&service);
    let status = exit(&mut service, &registry);
    let stdout = fs::read_to_string(root.join("stdout.log")).unwrap();
    let stderr = fs::read_to_string(root.join("stderr.log")).unwrap();
    assert!(status.success(), "{status:?}\n{stdout}\n{stderr}");
    quiescent(&directory);
    // Only the in-flight probe exists; the gated one created nothing.
    assert_eq!(directories(&registry.evidence_root), vec![directory]);
    assert!(!stdout.contains("API listening"), "{stdout}");
    assert!(stdout.contains("environment probes drained"), "{stdout}");
    fs::remove_dir_all(root).unwrap();
}

/// A startup error is never turned into a successful exit by a successful
/// drain, with or without a stop signal during the failing probe.
#[tokio::test]
async fn startup_error_is_propagated_after_drain() {
    let _serial = SERVICE.lock().await;
    for signalled in [false, true] {
        let root = temporary();
        let (plan, registry) = fixture(&root, 2.0, "broken");
        let mut service = service(&root, &plan, &registry).await;
        let directory = launched(&registry.evidence_root);
        if signalled {
            terminate(&service);
        }
        let status = exit(&mut service, &registry);
        let stdout = fs::read_to_string(root.join("stdout.log")).unwrap();
        let stderr = fs::read_to_string(root.join("stderr.log")).unwrap();
        assert!(!status.success(), "{status:?}\n{stdout}\n{stderr}");
        assert!(
            stderr.contains("environment admission blocked"),
            "{stdout}\n{stderr}"
        );
        quiescent(&directory);
        assert_eq!(directories(&registry.evidence_root), vec![directory]);
        assert!(!stdout.contains("API listening"), "{stdout}");
        fs::remove_dir_all(root).unwrap();
    }
}
