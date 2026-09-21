//! Launch the real service: migrations, HTTP wiring and orderly shutdown.
#![cfg(unix)]
#[path = "support/auth_admin.rs"]
mod auth_admin;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Service(Child, std::path::PathBuf);
static STARTUP: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

#[test]
fn serves_a_real_request_and_shuts_down_cleanly() {
    let _serial = STARTUP.lock().unwrap();
    for configured in [false, true] {
        serve_and_shutdown(configured);
    }
}

fn serve_and_shutdown(configured: bool) {
    let database =
        std::env::var("TEST_DATABASE_URL").expect("disposable TEST_DATABASE_URL required");
    let root = std::env::temp_dir().join(codexsymphony_server::process::new_identity().unwrap());
    let execution = root.join("execution");
    std::fs::create_dir_all(&execution).unwrap();
    let auth_config = root.join("auth.json");
    std::fs::write(
        &auth_config,
        r#"{"public_origin":"https://localhost:4200","trusted_proxies":[]}"#,
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"));
    command
        .env("AUTH_CONFIG", &auth_config)
        .env("WEB_ORIGIN", "https://localhost:4200")
        .env_remove("RUNTIME_CONFIG")
        .env_remove("STORAGE_CONFIG");
    if configured {
        // No execution is admitted; provide only the trusted broker metadata.
        let canonical = execution.join("workspaces/canonical.git");
        std::fs::create_dir_all(&canonical).unwrap();
        std::fs::write(
            canonical.join("config"),
            "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = true\n",
        )
        .unwrap();
        let path = execution.join("runtime.json");
        std::fs::write(&path, serde_json::json!({"validation":null,"settings":{"startup_seconds":10,"response_seconds":10,"stall_seconds":10,"reservation":{"tokens":100,"turns":1,"model_seconds":10},"codex_config":""},"preparation_adapter":"/bin/true","preparation":{"launcher":["/bin/true"],"baseline":"fixture"}}).to_string()).unwrap();
        command.env("RUNTIME_CONFIG", path);
        let path = root.join("storage.json");
        std::fs::write(
            &path,
            storage_fixture(&execution, &root.join("cold")).to_string(),
        )
        .unwrap();
        command.env("STORAGE_CONFIG", path);
    }
    let mut child = Service(
        command
            .env("DATABASE_URL", database)
            .env("BIND_ADDRESS", "127.0.0.1:0")
            .env("RUST_LOG", "info")
            .env("EXECUTION_DIRECTORY", &execution)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
        root,
    );
    let stdout = child.0.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some((_, address)) = line.split_once("API listening at http://") {
                let _ = send.send(address.trim().parse::<SocketAddr>());
                break;
            }
        }
    });
    let address = receive
        .recv_timeout(Duration::from_secs(15))
        .unwrap()
        .unwrap();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .write_all(
            format!("GET /api/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains(r#"{"status":"ok","database":"ok"}"#));
    let anonymous = request(address, "GET", "/api/multi/repository", "", "");
    assert!(anonymous.starts_with("HTTP/1.1 401"));
    let username = format!(
        "startup-{}",
        codexsymphony_server::process::new_identity().unwrap()
    );
    let mut admin = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .args(["auth", "init", "--stdin-json"])
        .env("DATABASE_URL", std::env::var("TEST_DATABASE_URL").unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    admin
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::json!({"username":username,"password":"synthetic-startup-password"})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    assert!(admin.wait().unwrap().success());
    let bootstrap = request(address, "GET", "/api/auth/csrf", "", "");
    let (cookie, proof) = session_headers(&bootstrap);
    let headers = format!(
        "Cookie: {cookie}\r\nOrigin: https://localhost:4200\r\nx-codexsymphony-csrf: {proof}\r\n"
    );
    let login = request(
        address,
        "POST",
        "/api/auth/login",
        &headers,
        &serde_json::json!({"username":username,"password":"synthetic-startup-password"})
            .to_string(),
    );
    assert!(login.starts_with("HTTP/1.1 200"));
    let (cookie, _) = session_headers(&login);
    let response = request(
        address,
        "GET",
        "/api/multi/repository",
        &format!("Cookie: {cookie}\r\n"),
        "",
    );
    assert!(response.starts_with("HTTP/1.1 200"));
    let body: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(body["runtime_ready"], configured);
    let duplicate = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", std::env::var("TEST_DATABASE_URL").unwrap())
        .env("BIND_ADDRESS", "127.0.0.1:0")
        .env("WEB_ORIGIN", "https://localhost:4200")
        .env("AUTH_CONFIG", &auth_config)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(
        !duplicate.status.success(),
        "second cwd/port must not bypass host instance lock"
    );
    assert!(!String::from_utf8_lossy(&duplicate.stdout).contains("API listening"));
    // Keep the API alive across a second coordinator tick before shutdown.
    thread::sleep(Duration::from_millis(300));
    assert!(
        Command::new("kill")
            .args(["-INT", &child.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "service did not shut down");
        thread::sleep(Duration::from_millis(20));
    }
    reader.join().unwrap();
}

#[test]
fn missing_database_configuration_fails_startup() {
    let _serial = STARTUP.lock().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env_remove("DATABASE_URL")
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("API listening"));
}

#[test]
fn invalid_bind_address_fails_without_announcing_readiness() {
    let _serial = STARTUP.lock().unwrap();
    let database =
        std::env::var("TEST_DATABASE_URL").expect("disposable TEST_DATABASE_URL required");
    let result = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", database)
        .env("BIND_ADDRESS", "invalid address")
        .env("RUST_LOG", "info")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("API listening"));
}

#[test]
fn connection_and_bind_failures_do_not_announce_readiness() {
    let _serial = STARTUP.lock().unwrap();
    let database =
        std::env::var("TEST_DATABASE_URL").expect("disposable TEST_DATABASE_URL required");
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    for (url, address) in [
        (
            "postgres://synthetic@127.0.0.1:1/unavailable_test",
            "127.0.0.1:0".into(),
        ),
        (
            database.as_str(),
            occupied.local_addr().unwrap().to_string(),
        ),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
            .env("DATABASE_URL", url)
            .env("BIND_ADDRESS", address)
            .env("RUST_LOG", "info")
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!String::from_utf8_lossy(&result.stdout).contains("API listening"));
    }
}

fn storage_fixture(execution: &std::path::Path, cold: &std::path::Path) -> serde_json::Value {
    use codexsymphony_server::{storage_files::Directory, storage_lifecycle::CATEGORIES};
    use serde_json::json;
    std::fs::create_dir_all(cold).unwrap();
    let root = |path: &std::path::Path| json!({"path":path.canonicalize().unwrap(),"identity":Directory::open(path).unwrap().identity().unwrap()});
    let categories: serde_json::Map<String, serde_json::Value> = CATEGORIES
        .into_iter()
        .map(|category| {
            (
                serde_json::to_value(category)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned(),
                json!({"bytes":2147483648_u64,"seconds":3600,"reserve_bytes":1048576}),
            )
        })
        .collect();
    json!({"policy":{"version":codexsymphony_server::process::new_identity().unwrap(),"reason":"isolated startup fixture","global_bytes":4294967296_u64,"control_bytes":268435456,"run_bytes":33554432,"requirement_bytes":134217728,"entry_bytes":1048576,"entry_count":10000,"categories":categories},"execution":root(execution),"cold":root(cold),"database_filesystem":root(cold),"database_extras":[]})
}

fn request(address: SocketAddr, method: &str, path: &str, headers: &str, body: &str) -> String {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n{body}", body.len()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}
fn session_headers(response: &str) -> (String, String) {
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let cookie = headers
        .lines()
        .find_map(|line| line.strip_prefix("set-cookie: "))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let body: serde_json::Value = serde_json::from_str(body).unwrap();
    (cookie, body["csrf_token"].as_str().unwrap().into())
}

#[test]
fn occupied_listener_fails_without_starting_workers() {
    let _serial = STARTUP.lock().unwrap();
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let root = std::env::temp_dir().join(codexsymphony_server::process::new_identity().unwrap());
    std::fs::create_dir_all(&root).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", std::env::var("TEST_DATABASE_URL").unwrap())
        .env("BIND_ADDRESS", occupied.local_addr().unwrap().to_string())
        .env("WEB_ORIGIN", "https://localhost:4200")
        .env("EXECUTION_DIRECTORY", root.join("execution"))
        .env_remove("RUNTIME_CONFIG")
        .env_remove("STORAGE_CONFIG")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("AddrInUse"));
    assert!(!root.join("execution").exists());
    std::fs::remove_dir_all(root).unwrap();
}
