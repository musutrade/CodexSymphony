//! Controlled host command. No credential arguments, echo, or credential logs.
use serde::Deserialize;
use std::{
    io::Read,
    os::unix::fs::{FileTypeExt, PermissionsExt},
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    username: String,
    password: String,
}
fn reset_requested(args: &[String]) -> Result<bool, crate::auth_store::Error> {
    Ok(match args {
        [command, flag] if flag == "--stdin-json" && command == "init" => false,
        [command, flag]
            if flag == "--stdin-json" && matches!(command.as_str(), "reset" | "change") =>
        {
            true
        }
        _ => {
            return Err(
                "usage: auth init|reset|change --stdin-json (restricted stdin only)".into(),
            );
        }
    })
}
fn read_input() -> Result<Input, crate::auth_store::Error> {
    let meta = std::fs::metadata("/proc/self/fd/0")?;
    if !meta.file_type().is_fifo() && !(meta.is_file() && meta.permissions().mode() & 0o077 == 0) {
        return Err("stdin must be a pipe or an owner-only regular file".into());
    }
    let mut input = String::new();
    std::io::stdin().take(8193).read_to_string(&mut input)?;
    if input.len() > 8192 {
        return Err("input exceeds limit".into());
    }
    Ok(serde_json::from_str(&input).or(Err("invalid account input"))?)
}
pub async fn run(args: &[String]) -> Result<(), crate::auth_store::Error> {
    let reset = reset_requested(args)?;
    let input = read_input()?;
    let url = std::env::var("DATABASE_URL").or(Err("DATABASE_URL is required"))?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .or(Err("database unavailable"))?;
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .or(Err("migration failed"))?;
    crate::auth_store::account(&pool, &input.username, &input.password, reset)
        .await
        .or(Err("account operation rejected"))?;
    Ok(())
}
