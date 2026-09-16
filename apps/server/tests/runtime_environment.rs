//! Real supervisor child boundary, with synthetic credentials only.
use codexsymphony_server::{
    execution::{Launch, RunKey},
    process,
};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

#[test]
fn runtime_child_receives_only_deployment_environment() {
    let root =
        std::env::temp_dir().join(format!("runtime-env-{}", process::new_identity().unwrap()));
    fs::create_dir_all(root.join("codex-home")).unwrap();
    let key = RunKey {
        run_id: "environment-probe".into(),
        request_id: "request".into(),
        incarnation: "fixture".into(),
    };
    let script = r#"import os,json,sys
from pathlib import Path
blocked=['GITHUB_TOKEN','GH_TOKEN','GITHUB_ENTERPRISE_TOKEN','GH_ENTERPRISE_TOKEN','GITLAB_PAT','GITLAB_ACCESS_TOKEN','GITLAB_TOKEN','OAUTH_TOKEN','CUSTOM_TRACKER_SECRET','OPENAI_API_KEY','DATABASE_URL','PYTHONPATH']
assert all(k not in os.environ for k in blocked)
assert os.environ['PATH']=='/usr/bin:/bin'
assert os.environ['LANG']=='C.UTF-8'
assert Path(os.environ['CODEX_HOME']).resolve()==Path.cwd()/'codex-home'
assert Path(os.environ['TMPDIR']).resolve()==Path.cwd()/'codex-home/tmp'
Path(os.environ['TMPDIR'],'write-probe').write_text('ok')
print('environment-boundary-PASS')
"#;
    let launch = Launch {
        key: key.clone(),
        workspace: root.to_str().unwrap().into(),
        workspace_identity: "fixture".into(),
        program: "/usr/bin/python3".into(),
        args: vec!["-c".into(), script.into()],
    };
    for name in ["runtime.json", "start.json", "storage-heartbeat.json"] {
        process::durable_write(&root.join(name), &key).unwrap();
    }
    process::durable_write(&root.join("launch.json"), &launch).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"));
    command
        .arg("--supervise")
        .arg(&root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8");
    for name in [
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITLAB_PAT",
        "GITLAB_ACCESS_TOKEN",
        "GITLAB_TOKEN",
        "OAUTH_TOKEN",
        "CUSTOM_TRACKER_SECRET",
        "OPENAI_API_KEY",
        "DATABASE_URL",
        "PYTHONPATH",
    ] {
        command.env(name, "synthetic-secret-must-not-cross");
    }
    command
        .env("CODEX_HOME", "/synthetic/host-auth")
        .env("TMPDIR", "/synthetic/host-tmp");
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("supervisor timeout");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "environment-boundary-PASS"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-secret"));
    assert!(root.join("quiescent.json").exists());
    fs::remove_dir_all(root).unwrap();
}
