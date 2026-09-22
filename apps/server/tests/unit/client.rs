pub fn client(root: &std::path::Path) -> crate::github_http::AppClient {
    let key = root.join("unit-key.pem");
    let output = std::process::Command::new("openssl")
        .args(["genrsa", "-out"])
        .arg(&key)
        .arg("2048")
        .output()
        .unwrap();
    assert!(output.status.success());
    crate::github_http::AppClient::new("http://127.0.0.1:1/", 42, &std::fs::read(key).unwrap())
        .unwrap()
}
