use codexsymphony_server::{auth, auth_store};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn command(url: Option<&str>, args: &[&str], input: &[u8]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"));
    command
        .arg("auth")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env_remove("DATABASE_URL");
    if let Some(url) = url {
        command.env("DATABASE_URL", url);
    }
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(input).ok();
    child.wait_with_output().unwrap()
}
#[tokio::test]
async fn controlled_admin_input_and_reset_are_durable_and_secret_free() {
    let base = std::env::var("TEST_DATABASE_URL").unwrap();
    let options: PgConnectOptions = base.parse().unwrap();
    let admin = PgPoolOptions::new()
        .connect_with(options.clone())
        .await
        .unwrap();
    let schema = format!("admin_{}", auth::random().unwrap());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut url = reqwest::Url::parse(&base).unwrap();
    url.query_pairs_mut()
        .append_pair("options", &format!("-csearch_path={schema}"));
    let pool = PgPoolOptions::new()
        .connect_with(options.options([("search_path", schema.clone())]))
        .await
        .unwrap();
    let secret = "synthetic-admin-password";
    let input = serde_json::json!({"username":"operator","password":secret}).to_string();
    let result = command(
        Some(url.as_str()),
        &["init", "--stdin-json"],
        input.as_bytes(),
    );
    assert!(result.status.success());
    assert!(!String::from_utf8_lossy(&result.stdout).contains(secret));
    assert!(!String::from_utf8_lossy(&result.stderr).contains(secret));
    let initial = auth_store::issue(&pool, 1_800_000_000).await.unwrap();
    let session = match auth_store::login(
        &pool,
        "operator",
        secret,
        "192.0.2.1",
        &initial,
        1_800_000_000,
    )
    .await
    .unwrap()
    {
        auth_store::Login::Success(token) => token,
        _ => panic!("test account must authenticate"),
    };
    for action in ["change", "reset"] {
        assert!(
            command(
                Some(url.as_str()),
                &[action, "--stdin-json"],
                input.as_bytes()
            )
            .status
            .success()
        );
        assert!(
            auth_store::session(&pool, &session, 1_800_000_000)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(
        !command(
            Some(url.as_str()),
            &["init", "--stdin-json"],
            input.as_bytes()
        )
        .status
        .success()
    );
    for bad in [b"invalid-json".as_slice(), b"{}", &[b'x'; 8193]] {
        assert!(
            !command(Some(url.as_str()), &["init", "--stdin-json"], bad)
                .status
                .success()
        );
    }
    assert!(
        !command(None, &["init", "--stdin-json"], input.as_bytes())
            .status
            .success()
    );
    assert!(
        !command(
            Some("postgres://127.0.0.1:1/unavailable"),
            &["init", "--stdin-json"],
            input.as_bytes()
        )
        .status
        .success()
    );
    assert!(
        !command(Some(url.as_str()), &["init", "--password"], b"")
            .status
            .success()
    );
    let invalid = serde_json::json!({"username":"invalid name","password":secret}).to_string();
    assert!(
        !command(
            Some(url.as_str()),
            &["init", "--stdin-json"],
            invalid.as_bytes()
        )
        .status
        .success()
    );
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("admin-input-{}", auth::random().unwrap()));
    std::fs::write(&path, &input).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
        .args(["auth", "reset", "--stdin-json"])
        .env("DATABASE_URL", url.as_str())
        .stdin(std::fs::File::open(&path).unwrap())
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        Command::new(env!("CARGO_BIN_EXE_codexsymphony-server"))
            .args(["auth", "reset", "--stdin-json"])
            .env("DATABASE_URL", url.as_str())
            .stdin(std::fs::File::open(&path).unwrap())
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::remove_file(path).unwrap();
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
