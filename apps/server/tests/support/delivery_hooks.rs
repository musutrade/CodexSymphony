use codexsymphony_server::{
    controlled_contract::{Operation, Registration},
    delivery_hooks::{self, Stage},
    environment::Plan,
    environment_host::Registry,
    validation::sha256,
    validation_runner,
};
use serde_json::json;
use sqlx::PgPool;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

pub fn install(root: &Path, environment: &mut Plan, registry: &mut Registry) {
    let entry = root.join("delivery-check");
    let count = root.join("delivery-effects");
    let script = format!(
        "#!/bin/sh\nprintf x >> '{}'\ncase \"$(cat '{}' 2>/dev/null)\" in\nfail) printf 'rejected\\n'; exit 1;;\nmutate) printf changed >> README.md;;\ntimeout) sleep 20;;\noutput) head -c 1100000 /dev/zero;;\nesac\nprintf 'delivery check passed\\n'\n",
        count.display(),
        root.join("delivery-case").display()
    );
    fs::write(&entry, &script).unwrap();
    fs::set_permissions(&entry, fs::Permissions::from_mode(0o700)).unwrap();
    let plan = validation_runner::Plan {
        entry,
        entry_sha256: sha256(&script),
        steps: vec![validation_runner::Step {
            id: "delivery-check".into(),
            command: vec!["/gate-entry".into()],
            timeout_seconds: 1,
            code_failure: false,
        }],
    };
    let stages = vec![
        Stage::BeforeDeliver,
        Stage::BeforePublish,
        Stage::BeforeMerge,
    ];
    let registration = Registration {
        id: "auxiliary-delivery".into(),
        implementation_digest: plan.entry_sha256.clone(),
        operations: vec![Operation::BeforeDeliver],
        scope_ref: "repository:1".into(),
        config_ref: sha256(serde_json::to_vec(&(&stages, &plan)).unwrap()),
        credential_provider_ref: None,
    };
    environment.controlled.extensions.push(registration.clone());
    registry
        .profiles
        .get_mut("python")
        .unwrap()
        .extensions
        .push(registration.clone());
    let path = root.join("delivery-registry.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!([{"registration":registration,"stages":stages,"plan":plan}]))
            .unwrap(),
    )
    .unwrap();
    unsafe {
        std::env::set_var("DELIVERY_HOOK_REGISTRY", path);
    }
}

pub async fn exercise(root: &Path, pool: &PgPool, action: &str) {
    let repository: serde_json::Value = sqlx::query_scalar("SELECT document->'repository' FROM execution_revision WHERE requirement_id=1 AND revision=1").fetch_one(pool).await.unwrap();
    let repository: codexsymphony_server::contract::Repository =
        serde_json::from_value(repository).unwrap();
    let frozen = codexsymphony_server::extension_contract::ExtensionConfig::from_legacy_repository(
        &repository,
    )
    .freeze(&codexsymphony_server::extension_contract::Capabilities::legacy_codex(None))
    .unwrap();
    sqlx::query("INSERT INTO project_hook_run(run_id,requirement_id,revision,resource_id,workspace,role,frozen) VALUES('source',1,1,'source',$1,'coding',$2)")
        .bind(root.join("candidate").to_str().unwrap()).bind(json!(frozen)).execute(pool).await.unwrap();
    for stage in [
        Stage::BeforeDeliver,
        Stage::BeforePublish,
        Stage::BeforeMerge,
    ] {
        for _ in 0..2 {
            delivery_hooks::run(pool, action, "publish", stage)
                .await
                .unwrap();
        }
    }
    assert_eq!(fs::read(root.join("delivery-effects")).unwrap(), b"xxx");
    // Lost result of a non-idempotent hook must not rerun, even after repeated
    // observations. Restore the original artifact to reconcile this invocation.
    let directory: String = sqlx::query_scalar(
        "SELECT output_dir FROM project_hook_invocation WHERE event='before_merge'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    reconcile_missing_stop(pool, Path::new(&directory)).await;
    reject_changed_inputs(Path::new(&directory));
    let evaluation = Path::new(&directory).join("evaluation.json");
    let original = fs::read(&evaluation).unwrap();
    fs::remove_file(&evaluation).unwrap();
    for _ in 0..2 {
        assert!(
            delivery_hooks::run(pool, action, "publish", Stage::BeforeMerge)
                .await
                .is_err()
        );
    }
    assert_eq!(fs::read(root.join("delivery-effects")).unwrap(), b"xxx");
    delivery_hooks::reconcile(pool).await.unwrap();
    fs::write(&evaluation, original).unwrap();
    delivery_hooks::reconcile(pool).await.unwrap();
    delivery_hooks::run(pool, action, "publish", Stage::BeforeMerge)
        .await
        .unwrap();
    let entry = root.join("delivery-check");
    let script = fs::read(&entry).unwrap();
    fs::write(&entry, "#!/bin/sh\nexit 0\n").unwrap();
    assert!(
        delivery_hooks::run(pool, action, "publish", Stage::BeforeMerge)
            .await
            .is_err()
    );
    fs::write(entry, script).unwrap();
    let config = root.join("delivery-registry.json");
    let saved = fs::read(&config).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&saved).unwrap();
    value[0]["stages"] = json!([]);
    fs::write(&config, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        delivery_hooks::run(pool, action, "publish", Stage::BeforeMerge)
            .await
            .is_err()
    );
    fs::write(&config, saved).unwrap();
    assert_eq!(fs::read(root.join("delivery-effects")).unwrap(), b"xxx");
    fault_cases(root, pool, action).await;
    unsafe {
        std::env::remove_var("DELIVERY_HOOK_REGISTRY");
    }
}

async fn fault_cases(root: &Path, pool: &PgPool, action: &str) {
    let readme = root.join("candidate/README.md");
    let original = fs::read(&readme).unwrap();
    let mut count = 3;
    for scenario in ["fail", "timeout", "output", "mutate"] {
        fs::write(root.join("delivery-case"), scenario).unwrap();
        assert!(
            delivery_hooks::run(pool, action, scenario, Stage::BeforeMerge)
                .await
                .is_err()
        );
        count += 1;
        fs::write(&readme, &original).unwrap();
        fs::remove_file(root.join("delivery-case")).unwrap();
        assert!(
            delivery_hooks::run(pool, action, scenario, Stage::BeforeMerge)
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(root.join("delivery-effects")).unwrap().len(),
            count
        );
    }
    let failures: i64 =
        sqlx::query_scalar("SELECT count(*) FROM project_hook_invocation WHERE status='failed'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(failures >= 1);
}

fn reject_changed_inputs(directory: &Path) {
    let path = directory.join("input.json");
    let original = fs::read(&path).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&original).unwrap();
    for pointer in [
        "/candidate/sha",
        "/call/implementation_digest",
        "/directory",
    ] {
        let mut changed = saved.clone();
        *changed.pointer_mut(pointer).unwrap() = json!("wrong");
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(codexsymphony_server::delivery_hook_process::run(directory).is_err());
    }
    let mut expired = saved;
    expired["call"]["deadline_unix_ms"] = json!(1);
    fs::write(&path, serde_json::to_vec(&expired).unwrap()).unwrap();
    assert!(codexsymphony_server::delivery_hook_process::run(directory).is_err());
    fs::write(&path, original).unwrap();
}

async fn reconcile_missing_stop(pool: &PgPool, directory: &Path) {
    let saved: serde_json::Value =
        sqlx::query_scalar("SELECT input FROM project_hook_invocation WHERE output_dir=$1")
            .bind(directory.to_str().unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
    let mut wrong = saved.clone();
    wrong["call"]["identity"]["invocation_id"] = json!("different-invocation");
    sqlx::query("UPDATE project_hook_invocation SET status='unknown',input=$2 WHERE output_dir=$1")
        .bind(directory.to_str().unwrap())
        .bind(&wrong)
        .execute(pool)
        .await
        .unwrap();
    assert!(delivery_hooks::reconcile(pool).await.is_err());
    sqlx::query("UPDATE project_hook_invocation SET input=$2 WHERE output_dir=$1")
        .bind(directory.to_str().unwrap())
        .bind(&saved)
        .execute(pool)
        .await
        .unwrap();
    let quiescent = directory.join("quiescent.json");
    let proof = fs::read(&quiescent).unwrap();
    fs::remove_file(&quiescent).unwrap();
    delivery_hooks::reconcile(pool).await.unwrap();
    let (status, stopped): (String, bool) = sqlx::query_as(
        "SELECT status,stop_confirmed FROM project_hook_invocation WHERE output_dir=$1",
    )
    .bind(directory.to_str().unwrap())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(status, "unknown");
    assert!(!stopped);
    assert!(directory.join("stop.json").is_file());
    // Restore the retained proof from the already completed synthetic invocation;
    // no real process is running and recovery has not repeated the hook.
    fs::write(quiescent, proof).unwrap();
    fs::remove_file(directory.join("stop.json")).unwrap();
    delivery_hooks::reconcile(pool).await.unwrap();
}

pub async fn cancel_running_hook(root: &Path, pool: &PgPool, action: &str) {
    unsafe {
        std::env::set_var(
            "DELIVERY_HOOK_REGISTRY",
            root.join("delivery-registry.json"),
        );
    }
    let before = fs::read(root.join("delivery-effects")).unwrap().len();
    fs::write(root.join("delivery-case"), "timeout").unwrap();
    let cancel = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while fs::read(root.join("delivery-effects")).unwrap().len() == before {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        sqlx::query("UPDATE requirement SET cancel_requested=true WHERE id=1")
            .execute(pool)
            .await
            .unwrap();
    };
    let (outcome, ()) = tokio::join!(
        delivery_hooks::run(pool, action, "cancel-running", Stage::BeforeMerge),
        cancel
    );
    assert!(outcome.is_err());
    let (status, stopped, directory): (String, bool, String) = sqlx::query_as(
        "SELECT status,stop_confirmed,output_dir FROM project_hook_invocation WHERE resource_id=$1",
    )
    .bind(format!("{action}:cancel-running"))
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(status, "unknown");
    assert!(stopped);
    assert!(Path::new(&directory).join("stop.json").is_file());
    sqlx::query("UPDATE requirement SET cancel_requested=false WHERE id=1")
        .execute(pool)
        .await
        .unwrap();
    fs::remove_file(root.join("delivery-case")).unwrap();
    assert!(
        delivery_hooks::run(pool, action, "cancel-running", Stage::BeforeMerge)
            .await
            .is_err()
    );
    assert_eq!(
        fs::read(root.join("delivery-effects")).unwrap().len(),
        before + 1
    );
    // Cancellation invalidates the old validation durably even after the control
    // flag is cleared. This test belongs after all successful-admission assertions.
    assert!(
        codexsymphony_server::validation_context::delivery(pool, action)
            .await
            .is_err()
    );
    unsafe {
        std::env::remove_var("DELIVERY_HOOK_REGISTRY");
    }
}
