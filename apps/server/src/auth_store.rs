//! Persistent sessions and dual-dimensional fixed-window admission.
use crate::auth;
use sqlx::{PgPool, Postgres, Transaction};
pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub async fn account(
    pool: &PgPool,
    username: &str,
    password: &str,
    reset: bool,
) -> Result<(), Error> {
    if !auth::valid_username(username) {
        return Err("invalid username".into());
    }
    let password = password.to_owned();
    let hash = tokio::task::spawn_blocking(move || auth::hash(&password)).await??;
    let mut tx = pool.begin().await?;
    if reset {
        replace_password(&mut tx, username, hash).await?;
    } else {
        sqlx::query("INSERT INTO platform_account(username,password_hash) VALUES($1,$2)")
            .bind(username)
            .bind(hash)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn session(
    pool: &PgPool,
    token: &str,
    now: i64,
) -> Result<Option<Option<String>>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT username FROM platform_session WHERE digest=$1 AND NOT revoked AND expires_at>$2",
    )
    .bind(auth::digest(token))
    .bind(now)
    .fetch_optional(pool)
    .await
}
pub async fn issue(pool: &PgPool, now: i64) -> Result<String, Error> {
    let token = auth::random()?;
    sqlx::query("INSERT INTO platform_session(digest,expires_at) VALUES($1,$2)")
        .bind(auth::digest(&token))
        .bind(now + 600)
        .execute(pool)
        .await?;
    Ok(token)
}
pub async fn revoke(pool: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE platform_session SET revoked=true WHERE digest=$1")
        .bind(auth::digest(token))
        .execute(pool)
        .await?;
    Ok(())
}
async fn limit(
    tx: &mut Transaction<'_, Postgres>,
    dimension: &str,
    value: &str,
    now: i64,
    max: i32,
) -> Result<bool, sqlx::Error> {
    let attempts: i32 = sqlx::query_scalar("INSERT INTO platform_login_limit(dimension,digest,window_start,attempts) VALUES($1,$2,$3,1) ON CONFLICT(dimension,digest) DO UPDATE SET attempts=CASE WHEN platform_login_limit.window_start+900<=$3 THEN 1 ELSE LEAST(platform_login_limit.attempts+1,1000000) END, window_start=CASE WHEN platform_login_limit.window_start+900<=$3 THEN $3 ELSE platform_login_limit.window_start END RETURNING attempts")
        .bind(dimension).bind(auth::digest(value)).bind(now).fetch_one(&mut **tx).await?;
    Ok(attempts <= max)
}
pub enum Login {
    Success(String),
    Invalid,
    Limited,
}
pub async fn login(
    pool: &PgPool,
    username: &str,
    password: &str,
    source: &str,
    old: &str,
    now: i64,
) -> Result<Login, Error> {
    let mut tx = pool.begin().await?;
    if !admitted(&mut tx, username, source, now).await? {
        tx.commit().await?;
        return Ok(Login::Limited);
    }
    if !credentials_match(&mut tx, username, password).await? {
        tx.commit().await?;
        return Ok(Login::Invalid);
    }
    rotate_session(tx, username, old, now).await
}
async fn admitted(
    tx: &mut Transaction<'_, Postgres>,
    username: &str,
    source: &str,
    now: i64,
) -> Result<bool, sqlx::Error> {
    // Stable account/source lock order; both dimensions count every attempt.
    let account = limit(tx, "account", username, now, 10).await?;
    let source = limit(tx, "source", source, now, 50).await?;
    Ok(account && source)
}
async fn credentials_match(
    tx: &mut Transaction<'_, Postgres>,
    username: &str,
    password: &str,
) -> Result<bool, Error> {
    let hash: Option<String> = sqlx::query_scalar(
        "SELECT password_hash FROM platform_account WHERE username=$1 FOR UPDATE",
    )
    .bind(username)
    .fetch_optional(&mut **tx)
    .await?;
    let password = password.to_owned();
    Ok(tokio::task::spawn_blocking(move || {
        // Unknown accounts perform the same password KDF work; never report existence.
        match hash {
            Some(hash) => auth::verify(&password, &hash),
            None => {
                let _ = auth::hash("unknown-account-dummy-password");
                false
            }
        }
    })
    .await?)
}
async fn rotate_session(
    mut tx: Transaction<'_, Postgres>,
    username: &str,
    old: &str,
    now: i64,
) -> Result<Login, Error> {
    let token = auth::random()?;
    sqlx::query("UPDATE platform_session SET revoked=true WHERE digest=$1")
        .bind(auth::digest(old))
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO platform_session(digest,username,expires_at) VALUES($1,$2,$3)")
        .bind(auth::digest(&token))
        .bind(username)
        .bind(now + auth::SESSION_SECONDS)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Login::Success(token))
}

async fn replace_password(
    tx: &mut Transaction<'_, Postgres>,
    username: &str,
    hash: String,
) -> Result<(), Error> {
    let changed = sqlx::query("UPDATE platform_account SET password_hash=$2 WHERE username=$1")
        .bind(username)
        .bind(hash)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    if changed != 1 {
        return Err("account does not exist".into());
    }
    sqlx::query("UPDATE platform_session SET revoked=true WHERE username=$1")
        .bind(username)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
