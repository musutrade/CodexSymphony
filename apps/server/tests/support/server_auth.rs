//! Real process fixtures authenticate through the public login API.
#![allow(dead_code)]
use serde_json::Value;

pub fn config(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("test-auth.json");
    std::fs::write(
        &path,
        r#"{"public_origin":"https://localhost:4200","trusted_proxies":[]}"#,
    )
    .unwrap();
    path
}

pub async fn client(address: &str, database: &str) -> reqwest::Client {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(database)
        .await
        .unwrap();
    let username = format!(
        "http-{}",
        codexsymphony_server::process::new_identity().unwrap()
    );
    codexsymphony_server::auth_store::account(&pool, &username, "synthetic-http-password", false)
        .await
        .unwrap();
    pool.close().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(format!("{address}/api/auth/csrf"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response_cookie(&response);
    let proof: Value = response.json().await.unwrap();
    let response = client
        .post(format!("{address}/api/auth/login"))
        .header("cookie", cookie)
        .header("origin", "https://localhost:4200")
        .header(
            "x-codexsymphony-csrf",
            proof["csrf_token"].as_str().unwrap(),
        )
        .json(&serde_json::json!({"username":username,"password":"synthetic-http-password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response_cookie(&response);
    let proof: Value = response.json().await.unwrap();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("cookie", cookie.parse().unwrap());
    headers.insert("origin", "https://localhost:4200".parse().unwrap());
    headers.insert(
        "x-codexsymphony-csrf",
        proof["csrf_token"].as_str().unwrap().parse().unwrap(),
    );
    reqwest::Client::builder()
        .no_proxy()
        .default_headers(headers)
        .build()
        .unwrap()
}
fn response_cookie(response: &reqwest::Response) -> String {
    response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into()
}
