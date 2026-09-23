//! A Runtime reservation is not a spawned process. Its session is durable first.
use crate::{process, run_store};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::Path;

/// Retire only known Runtime intents blocked before `runtime_store::open`.
/// Session creation takes this same lock and precedes every Runtime spawn. A
/// launch directory, unknown origin or session leaves the normal stop-proof
/// requirement intact, including an uncertain spawn after session creation.
pub async fn retire(pool: &PgPool, root: &Path) -> Result<(), sqlx::Error> {
    let mut tx = run_store::lock(pool).await?;
    let runs: Vec<(String, String, String, Value)> = sqlx::query_as(
        "SELECT a.id,a.request_id,a.incarnation,a.launch FROM agent_run a WHERE a.state='Created' AND NOT a.quiescent AND a.process_identity IS NULL AND a.launch->'args'->>-1='app-server' AND (SELECT blocked FROM storage_guard WHERE id=1) AND NOT EXISTS(SELECT 1 FROM runtime_session s WHERE s.run_id=a.id) AND (EXISTS(SELECT 1 FROM initial_run n WHERE n.launch=a.launch) OR EXISTS(SELECT 1 FROM runtime_resume r WHERE r.job->'launch'=a.launch) OR EXISTS(SELECT 1 FROM repair_reservation p WHERE p.launch=a.launch))",
    ).fetch_all(&mut *tx).await?;
    for (id, request, incarnation, launch) in runs {
        if !absent(root, &id) {
            continue;
        }
        sqlx::query("UPDATE agent_run SET state='Interrupted',quiescent=true,stop_requested=true WHERE id=$1")
            .bind(&id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO run_event(run_id,request_id,incarnation,payload,accepted) VALUES($1,$2,$3,$4,true)")
            .bind(id).bind(request).bind(incarnation)
            .bind(json!({"kind":"runtime_never_dispatched","reason":"storage blocked before durable Runtime session or process launch","launch":launch}))
            .execute(&mut *tx).await?;
    }
    tx.commit().await
}

fn absent(root: &Path, id: &str) -> bool {
    let Ok(directory) = process::run_directory(root, id) else {
        return false;
    };
    matches!(std::fs::symlink_metadata(directory), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}
