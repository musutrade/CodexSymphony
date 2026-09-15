//! Launch the real service: migrations, HTTP wiring and orderly shutdown.
#![cfg(unix)]
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Service(Child);
static STARTUP: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn serves_a_real_request_and_shuts_down_cleanly() {
    let _serial = STARTUP.lock().unwrap();
    let database =
        std::env::var("TEST_DATABASE_URL").expect("disposable TEST_DATABASE_URL required");
    let mut child = Service(
        Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
            .env("DATABASE_URL", database)
            .env("BIND_ADDRESS", "127.0.0.1:0")
            .env("RUST_LOG", "info")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
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
    let duplicate = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .env("DATABASE_URL", std::env::var("TEST_DATABASE_URL").unwrap())
        .env("BIND_ADDRESS", "127.0.0.1:0")
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
