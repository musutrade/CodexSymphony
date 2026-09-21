//! An isolated restored database can only enter this read-only inspection path.
//! No migrations, coordinator, model runtime, GitHub worker or business routes.
use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{error::Error, net::SocketAddr};

type RecoveryError = Box<dyn Error + Send + Sync>;

pub async fn guarded(pool: &PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT to_regclass('public.symphony_recovery_guard') IS NOT NULL")
        .fetch_one(pool)
        .await
}

pub async fn refuse_normal_start(pool: &PgPool) -> Result<(), RecoveryError> {
    if guarded(pool).await? {
        return Err(
            "isolated recovery database: normal startup forbidden; use --recovery-drill".into(),
        );
    }
    Ok(())
}

pub fn router(pool: PgPool) -> Router {
    Router::new()
        .route("/api/recovery", get(summary))
        .with_state(pool)
}

async fn summary(State(pool): State<PgPool>) -> Result<Json<Value>, axum::http::StatusCode> {
    let facts: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('requirements',(SELECT count(*) FROM requirement),
         'paused',(SELECT count(*) FROM requirement WHERE paused),
         'revoked_sessions',(SELECT count(*) FROM platform_session WHERE revoked),
         'expired_sessions',(SELECT count(*) FROM platform_session WHERE expires_at<=extract(epoch FROM now())),
         'pending_questions',(SELECT count(*) FROM runtime_question WHERE answer IS NULL))",
    )
    .fetch_one(&pool)
    .await
    .map_err(|_| axum::http::StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(
        json!({"status":"isolated_read_only", "external_actions":false, "facts":facts}),
    ))
}

pub async fn serve(url: &str, address: SocketAddr) -> Result<(), RecoveryError> {
    if !address.ip().is_loopback() {
        return Err("recovery drill requires loopback".into());
    }
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET default_transaction_read_only=on")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await?;
    if !guarded(&pool).await? {
        return Err("recovery drill requires restored database guard".into());
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("recovery drill listening at {}", listener.local_addr()?);
    axum::serve(listener, router(pool))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
