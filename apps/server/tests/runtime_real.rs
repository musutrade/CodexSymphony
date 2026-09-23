//! Real pinned Runtime integration in the ordinary trusted development environment.
use codexsymphony_server::{
    execution::{Launch, RunKey},
    process, runtime, runtime_protocol as wire,
    runtime_transport::{Record, Transport},
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Notify;

struct Worker {
    child: std::process::Child,
    directory: PathBuf,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = process::durable_write(&self.directory.join("stop.json"), &true);
        for _ in 0..100 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
async fn message(
    transport: &mut Transport,
    saved: &mut Vec<Value>,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            if let Some(i) = saved.iter().position(&predicate) {
                return saved.remove(i);
            }
            match transport.receive().await.unwrap() {
                Record::Protocol(bytes) => saved.push(serde_json::from_slice(&bytes).unwrap()),
                Record::Diagnostic(bytes) => eprintln!("{}", String::from_utf8_lossy(&bytes)),
                Record::Closed => panic!("app-server closed before expected response"),
            }
            assert!(saved.len() < 500);
        }
    })
    .await
    .expect("RPC timeout")
}
async fn call(
    transport: &mut Transport,
    saved: &mut Vec<Value>,
    method: &str,
    params: Value,
) -> Value {
    let id = transport.request(method, &params).await.unwrap();
    let reply = message(transport, saved, |v| {
        v.get("method").is_none() && v.get("id") == Some(&id)
    })
    .await;
    assert!(reply.get("error").is_none(), "{reply}");
    reply["result"].clone()
}

#[tokio::test]
async fn real_runtime_transport_and_supervision() {
    let root = std::env::current_dir().unwrap();
    let directory =
        std::env::temp_dir().join(format!("real-runtime-{}", process::new_identity().unwrap()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = requests.clone();
    let finish = Arc::new(Notify::new());
    let finished = finish.clone();
    let application=axum::Router::new().route("/responses",axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
        let requests=captured.clone(); let finish=finished.clone();
        async move {
            let first={let mut values=requests.lock().unwrap();values.push(body);values.len()==1};
            if !first { finish.notified().await; }
            let events=[json!({"type":"response.created","response":{"id":"fixture-1"}}),
                json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"fixture-call","name":"report_blocker","arguments":"{\"reason\":\"fixture\",\"requires_permission\":false}"}}),
                json!({"type":"response.completed","response":{"id":"fixture-1","usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}}})];
            let body=events.iter().map(|v|format!("data: {v}\n\n")).collect::<String>();
            ([("content-type","text/event-stream")],body)
        }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, application).await.unwrap() });
    let config = format!(
        r#"
model = "gpt-6-astra"
model_provider = "runtime_fixture"
approval_policy = "never"
sandbox_mode = "danger-full-access"
[model_providers.runtime_fixture]
name = "Local scripted fixture"
base_url = "http://127.0.0.1:{port}"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
"#
    );
    let key = RunKey {
        run_id: process::new_identity().unwrap(),
        request_id: process::new_identity().unwrap(),
        incarnation: "fixture".into(),
    };
    let launch = Launch {
        key: key.clone(),
        workspace: root.to_str().unwrap().into(),
        workspace_identity: "reviewed".into(),
        program: std::env::var("CODEX_BINARY").unwrap_or_else(|_| {
            String::from_utf8(
                std::process::Command::new("which")
                    .arg("codex")
                    .output()
                    .unwrap()
                    .stdout,
            )
            .unwrap()
            .trim()
            .into()
        }),
        args: vec!["app-server".into()],
    };
    let supervisor = PathBuf::from(env!("CARGO_BIN_EXE_codexsymphony-server"));
    let child =
        process::spawn_with_transport(&supervisor, &directory, &launch, Some(&config)).unwrap();
    let mut worker = Worker {
        child,
        directory: directory.clone(),
    };
    let mut transport = Transport::new(&mut worker.child).unwrap();
    process::durable_write(&directory.join("start.json"), &key).unwrap();
    let heart_directory = directory.clone();
    let heartbeat = tokio::spawn(async move {
        loop {
            process::durable_write(&heart_directory.join("storage-heartbeat.json"), &true).unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    let mut saved = vec![];
    let initialized = call(
        &mut transport,
        &mut saved,
        "initialize",
        json!(wire::InitializeParams {
            client_info: json!({"name":"rust-runtime-integration","version":"1"}),
            capabilities: Some(json!({"experimentalApi":true}))
        }),
    )
    .await;
    assert!(
        initialized["userAgent"]
            .as_str()
            .unwrap()
            .contains("/0.156.1 ")
    );
    transport
        .send(&json!({"method":"initialized"}))
        .await
        .unwrap();
    let code = r#"import os,json,pathlib,tempfile
root=pathlib.Path.cwd()
assert 'GITHUB_TOKEN' not in os.environ
assert 'GH_TOKEN' not in os.environ
with tempfile.TemporaryDirectory() as d:
 p=pathlib.Path(d)/'ordinary-command';p.write_text('ok');assert p.read_text()=='ok'
print(json.dumps({'cwd':str(root),'ordinary_command':True}))
"#;
    let command=call(&mut transport,&mut saved,"command/exec",json!({"command":["python3","-c",code],"cwd":root,"sandboxPolicy":{"type":"dangerFullAccess"},"timeoutMs":20000})).await;
    assert_eq!(command["exitCode"], 0, "{command}");
    let proof: Value = serde_json::from_str(command["stdout"].as_str().unwrap()).unwrap();
    assert_eq!(proof["cwd"], root.to_str().unwrap());
    let started = call(
        &mut transport,
        &mut saved,
        "thread/start",
        json!(wire::ThreadStartParams {
            cwd: Some(root.to_string_lossy().into_owned()),
            approval_policy: Some(json!("never")),
            sandbox: Some(json!("danger-full-access")),
            ephemeral: Some(true),
            dynamic_tools: Some(runtime::tools()),
            allow_provider_model_fallback: Some(false),
            ..Default::default()
        }),
    )
    .await;
    assert_eq!(started["cwd"], root.to_str().unwrap());
    let thread = started["thread"]["id"].clone();
    let turn = call(
        &mut transport,
        &mut saved,
        "turn/start",
        json!(wire::TurnStartParams {
            thread_id: thread.as_str().unwrap().into(),
            input: vec![json!({"type":"text","text":"Deterministic test.","text_elements":[]})],
            ..Default::default()
        }),
    )
    .await["turn"]["id"]
        .clone();
    let tool = message(&mut transport, &mut saved, |v| {
        v["method"] == "item/tool/call"
    })
    .await;
    let params: codexsymphony_server::runtime_protocol::DynamicToolCallParams =
        serde_json::from_value(tool["params"].clone()).unwrap();
    assert_eq!(params.tool, "report_blocker");
    assert_eq!(tool["params"]["threadId"], thread);
    assert_eq!(tool["params"]["turnId"], turn);
    transport
        .send(&json!({"id":tool["id"],"result":runtime::reply(true,"rust-adapter-reply")}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if requests.lock().unwrap().len() > 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        requests.lock().unwrap()[1]["input"]
            .to_string()
            .contains("rust-adapter-reply")
    );
    call(
        &mut transport,
        &mut saved,
        "turn/interrupt",
        json!({"threadId":thread,"turnId":turn}),
    )
    .await;
    let completed = message(&mut transport, &mut saved, |v| {
        v["method"] == "turn/completed"
    })
    .await;
    assert_eq!(completed["params"]["turn"]["status"], "interrupted");
    assert!(
        !directory.join("quiescent.json").exists(),
        "RPC completion is not group quiescence"
    );
    process::durable_write(&directory.join("stop.json"), &key).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if worker.child.try_wait().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let receipt: codexsymphony_server::execution::Receipt =
        process::read(&directory.join("quiescent.json")).unwrap();
    assert_eq!(receipt.key, key);
    heartbeat.abort();
    server.abort();
    finish.notify_waiters();
    std::fs::remove_dir_all(&directory).unwrap();
    println!("real Rust transport/supervisor + locked app-server PASS; zero external model calls");
}
