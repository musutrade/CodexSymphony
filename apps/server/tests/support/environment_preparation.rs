//! Controlled legacy adapter, real Broker/worktree and environment admission.
use codexsymphony_server::{
    execution::{Launch, RunKey},
    git_broker::GitBroker,
    preparation_service::{self, Request},
    process,
    workspace::Workspace,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{fs, path::Path, process::Command};

pub async fn exercise(pool: &PgPool, root: &Path) {
    let seed = root.join("python");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&seed)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let baseline = git(&["rev-parse", "HEAD"]);
    let bundle = root.join("preparation.bundle");
    git(&["bundle", "create", bundle.to_str().unwrap(), "--all"]);
    let broker = GitBroker::initialize(&root.join("broker"), &bundle).unwrap();
    let key = RunKey {
        run_id: process::new_identity().unwrap(),
        request_id: "environment-preparation".into(),
        incarnation: "current".into(),
    };
    let launch = Launch {
        workspace: broker.path(&key.run_id).unwrap().to_str().unwrap().into(),
        workspace_identity: "environment-worktree".into(),
        program: "/no-model-call".into(),
        args: vec!["app-server".into()],
        key,
    };
    let workspace = Workspace {
        key: launch.key.clone(),
        identity: launch.workspace_identity.clone(),
        requirement: 1,
        revision: 1,
        phase: "preparation".into(),
        baseline,
        branch: format!("ai/req-1-{}", launch.key.run_id),
        path: launch.workspace.clone(),
    };
    broker.prepare(&workspace, true).unwrap();
    sqlx::query("UPDATE execution_control SET requirement_id=NULL, incarnation='current', recovery_complete=true WHERE id=1").execute(pool).await.unwrap();
    let adapter = root.join("preparation-adapter.py");
    let marker = root.join("adapter-ran");
    fs::write(&adapter, format!("import sys,json,pathlib\nc=json.load(sys.stdin)\nassert c['environment_extension_verified'] is True\nassert c['dependencies']==[]\npathlib.Path({:?}).write_text('verified')\nprint(json.dumps({{'deployment_identity':'fixture','execution_identity':'controlled-adapter','network':{{'configuration_identity':'fixture','reachable':True}},'failures':[],'sample':{{'controlled':True}}}}))\n",marker.to_str().unwrap())).unwrap();
    let request = |now| Request {
        launch: &launch,
        requirement: 1,
        revision: 1,
        phase: "preparation",
        now,
        adapter: &adapter,
        control_directory: root,
        broker: &broker,
        workspace: &workspace,
        config: json!({"deployment_identity":"fixture","launcher":[launch.program],"dependencies":["legacy-unavailable"]}),
    };
    let now = codexsymphony_server::github_service::now();
    fs::write(seed.join("state"), "broken").unwrap();
    assert!(
        !preparation_service::prepare(pool, request(now))
            .await
            .unwrap()
    );
    assert!(!marker.exists());
    let retry: Value = sqlx::query_scalar("SELECT retry FROM preparation_record WHERE run_id=$1")
        .bind(&launch.key.run_id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(retry["last_failure"]["code"], "environment_mismatch");
    assert_eq!(retry["attempts"], 1);
    assert_eq!(retry["todo"], true);
    assert!(retry["next_attempt_at"].is_null());
    let next = now + 1;
    codexsymphony_server::preparation_store::authorize_retry(
        pool,
        &launch.key.run_id,
        next,
        "controlled environment repaired and explicitly authorized",
    )
    .await
    .unwrap();
    fs::write(seed.join("state"), "ready").unwrap();
    assert!(
        preparation_service::prepare(pool, request(next))
            .await
            .unwrap()
    );
    assert_eq!(fs::read_to_string(marker).unwrap(), "verified");
    let retry: Value = sqlx::query_scalar("SELECT retry FROM preparation_record WHERE run_id=$1")
        .bind(&launch.key.run_id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(retry["attempts"], 2);
    sqlx::query("UPDATE execution_control SET requirement_id=1 WHERE id=1")
        .execute(pool)
        .await
        .unwrap();
}
