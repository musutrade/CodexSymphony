//! Bounded in-place material cleanup; intents and verified references precede
//! deletion. Failed scans cannot mutate business, execution or validation facts.
use crate::{
    preparation::{Failure, Retry},
    run_store,
    storage_archive::{self, Package},
    storage_consumers,
    storage_files::{Directory, Entry, FileIdentity},
    storage_inventory,
    storage_lifecycle::{self as domain, Category, Kind},
    storage_measure,
    storage_store::{self as store, Deployment, Result, Tx},
};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::path::{Path, PathBuf};

#[derive(sqlx::FromRow)]
struct Material {
    id: String,
    run_id: String,
    path: String,
    directory_identity: Value,
    kind: String,
    category: String,
    status: String,
    expires_at: i64,
    manifest: Option<Value>,
    archive: Option<Value>,
    retry: Value,
    protection: Option<String>,
}

pub async fn scan(pool: &PgPool, now: i64) -> Result<()> {
    static SCAN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _scan = SCAN.lock().await;
    let mut retry = scan_retry(pool, now).await?;
    if !retry.begin(now, false) {
        persist_scan(pool, &retry).await?;
        return Ok(());
    }
    persist_scan(pool, &retry).await?;
    let result = scan_registered(pool, now).await;
    match &result {
        Ok(()) => {
            retry.authorize_retry_group(now);
            retry.last_failure = None;
        }
        Err(error) => {
            let mut failure = failure();
            failure.evidence = "storage_guard.scan_retry".into();
            failure.detail = crate::operator_view::redact_text(&error.to_string())
                .chars()
                .take(1024)
                .collect();
            retry.fail(failure, now);
        }
    }
    persist_scan(pool, &retry).await?;
    result
}
async fn scan_retry(pool: &PgPool, now: i64) -> Result<Retry> {
    let value: Option<Value> =
        sqlx::query_scalar("SELECT scan_retry FROM storage_guard WHERE id=1")
            .fetch_one(pool)
            .await?;
    let mut retry = value
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_else(|| Retry::new("cleanup", now));
    if retry.active() {
        retry.fail(failure(), now);
    } else if retry.last_failure.is_none() && !retry.todo {
        retry.authorize_retry_group(now);
    }
    Ok(retry)
}
async fn persist_scan(pool: &PgPool, retry: &Retry) -> Result<()> {
    sqlx::query("UPDATE storage_guard SET scan_retry=$1 WHERE id=1")
        .bind(json!(retry))
        .execute(pool)
        .await?;
    Ok(())
}
async fn scan_registered(pool: &PgPool, now: i64) -> Result<()> {
    let Some((config, ids)) = catalog(pool, now).await? else {
        return Ok(());
    };
    crate::storage_db::collect(pool, &config, now).await?;
    for id in ids {
        if let Err(error) = cleanup(pool, &config, &id, now).await {
            failed(pool, &id, now, &error.to_string()).await?;
        }
    }
    Ok(())
}

async fn catalog(pool: &PgPool, now: i64) -> Result<Option<(Deployment, Vec<String>)>> {
    let mut tx = run_store::lock(pool).await?;
    let Some(config) = store::deployment(&mut tx).await? else {
        return Ok(None);
    };
    refresh_catalog(&mut tx, &config, now).await?;
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM storage_material WHERE status<>'deleted' AND expires_at<=$1 ORDER BY created_at,id")
        .bind(now).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some((config, ids)))
}
async fn refresh_catalog(tx: &mut Tx<'_>, config: &Deployment, now: i64) -> Result<()> {
    config.execution.open()?;
    config.cold.open()?;
    storage_inventory::discover(tx, config, now).await?;
    storage_consumers::resolve(tx).await?;
    reconcile(tx, config).await?;
    Ok(())
}

async fn load(tx: &mut Tx<'_>, id: &str) -> Result<Material> {
    Ok(sqlx::query_as("SELECT id,run_id,path,directory_identity,kind,category,status,expires_at,manifest,archive,retry,protection FROM storage_material WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_one(&mut **tx).await?)
}

async fn cleanup(pool: &PgPool, config: &Deployment, id: &str, now: i64) -> Result<()> {
    let Some(item) = prepare_attempt(pool, config, id, now).await? else {
        return Ok(());
    };
    prepare_delete(pool, config, item, now).await?;
    delete(pool, config, id, now).await
}
async fn prepare_attempt(
    pool: &PgPool,
    config: &Deployment,
    id: &str,
    now: i64,
) -> Result<Option<Material>> {
    let mut tx = run_store::lock(pool).await?;
    let item = load(&mut tx, id).await?;
    if !allowed(&mut tx, config, &item, now).await? {
        tx.commit().await?;
        return Ok(None);
    }
    let (retry, ready) = retry_transition(&item.retry, now)?;
    save_retry(&mut tx, id, &retry).await?;
    tx.commit().await?;
    Ok(ready.then_some(item))
}
fn retry_transition(value: &Value, now: i64) -> Result<(Retry, bool)> {
    let mut retry: Retry = serde_json::from_value(value.clone())?;
    if retry.attempts == 0 {
        retry = Retry::new("cleanup", now);
    }
    if retry.active() {
        retry.fail(failure(), now);
        return Ok((retry, false));
    }
    let ready = retry.begin(now, false);
    Ok((retry, ready))
}
async fn allowed(tx: &mut Tx<'_>, config: &Deployment, item: &Material, now: i64) -> Result<bool> {
    if item.status == "deleted" {
        return Ok(false);
    }
    let protection = material_protection(tx, config, item).await?;
    sqlx::query("UPDATE storage_material SET protection=CASE WHEN protection='partial archive; reconcile' OR starts_with(protection,'unknown') THEN protection ELSE $2 END WHERE id=$1")
        .bind(&item.id)
        .bind(protection.reason())
        .execute(&mut **tx)
        .await?;
    Ok(domain::expired(now, item.expires_at, &protection))
}
async fn material_protection(
    tx: &mut Tx<'_>,
    config: &Deployment,
    item: &Material,
) -> Result<domain::Protection> {
    let mut protection = storage_consumers::protection(
        tx,
        config,
        &item.run_id,
        if item.status == "deleting" {
            "rebuildable"
        } else {
            &item.kind
        },
    )
    .await?;
    // A verified cold package retains the current successful retrospective;
    // redundant hot originals may be removed after that package is committed.
    if item.category == "hot" {
        protection.current_success = false;
    }
    if item.protection.as_deref() == Some("partial archive; reconcile") {
        protection.unreconciled = true;
    }
    if item
        .protection
        .as_deref()
        .is_some_and(|reason| reason.starts_with("unknown"))
    {
        protection.unknown = true;
    }
    protection.consumer |= sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM storage_material WHERE status<>'deleted' AND archive->>'path'=$1)")
        .bind(&item.path).fetch_one(&mut **tx).await?;
    protection.unreconciled |= preparation_pending(item);
    Ok(protection)
}
fn preparation_pending(item: &Material) -> bool {
    if item.id.starts_with(".preparation-") {
        let stopped = crate::workspace_files::read(&Path::new(&item.path).join("quiescent.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        return stopped
            .as_ref()
            .is_none_or(|value| value["quiescent"] != true);
    }
    false
}

fn failure() -> Failure {
    Failure { code:"cleanup_failed".into(),phase:"cleanup".into(),
        detail:"material operation failed or interrupted; original identity retained; inspect storage_material and retry after reconciliation".into(),
        exit_code:None,evidence:"storage_material".into() }
}
async fn save_retry(tx: &mut Tx<'_>, id: &str, retry: &Retry) -> Result<()> {
    sqlx::query("UPDATE storage_material SET retry=$2 WHERE id=$1")
        .bind(id)
        .bind(json!(retry))
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn failed(pool: &PgPool, id: &str, now: i64, error: &str) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    let item = load(&mut tx, id).await?;
    let mut retry: Retry = serde_json::from_value(item.retry)?;
    let mut failure = failure();
    failure.detail = crate::operator_view::redact_text(error)
        .chars()
        .take(1024)
        .collect();
    retry.fail(failure, now);
    save_retry(&mut tx, id, &retry).await?;
    tx.commit().await?;
    Ok(())
}

fn directory(item: &Material, config: &Deployment) -> Result<Directory> {
    let path = Path::new(&item.path);
    let root = if item.category == "cold" {
        &config.cold
    } else {
        &config.execution
    };
    let parent = root.open()?;
    if !path.starts_with(&root.path) || path == root.path {
        return Err("material escaped product root".into());
    }
    let directory = parent.child(path.strip_prefix(&root.path)?)?;
    directory.matches(&serde_json::from_value::<FileIdentity>(
        item.directory_identity.clone(),
    )?)?;
    Ok(directory)
}

async fn prepare_delete(
    pool: &PgPool,
    config: &Deployment,
    item: Material,
    now: i64,
) -> Result<()> {
    if item.status == "deleting" {
        return Ok(());
    }
    let source = directory(&item, config)?;
    let files = source.inventory(config.policy.entry_count)?;
    let archive = if item.kind == "retrospective" && item.category == "hot" {
        Some(archive(pool, config, &item, &source, &files, now).await?)
    } else {
        None
    };
    persist_deletion(pool, config, &item, &source, &files, archive).await
}
async fn persist_deletion(
    pool: &PgPool,
    config: &Deployment,
    item: &Material,
    source: &Directory,
    files: &[Entry],
    archive: Option<Package>,
) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    let mut protection =
        storage_consumers::protection(&mut tx, config, &item.run_id, &item.kind).await?;
    if archive.is_some() {
        protection.current_success = false;
    }
    if protection.reason().is_some() {
        return Err("consumer acquired material before deletion".into());
    }
    retain_key_logs(&mut tx, item, source, files, config.policy.entry_bytes).await?;
    sqlx::query("UPDATE storage_material SET status='deleting',manifest=$2,archive=$3,deletion_reason='retention expired; verified replacement or reconstructable output' WHERE id=$1")
        .bind(&item.id).bind(json!(files)).bind(archive.map(|package|json!(package))).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

async fn retain_key_logs(
    tx: &mut Tx<'_>,
    item: &Material,
    source: &Directory,
    files: &[Entry],
    limit: u64,
) -> Result<()> {
    if item.kind != "retrospective" || item.category != "hot" {
        return Ok(());
    }
    let logs = files
        .iter()
        .filter(|entry| !entry.directory && entry.link.is_none())
        .take(4)
        .map(|entry| excerpt(source, entry, limit))
        .collect::<Result<Vec<_>>>()?;
    let existing: Value = sqlx::query_scalar("SELECT summary FROM storage_attempt WHERE run_id=$1")
        .bind(&item.run_id)
        .fetch_one(&mut **tx)
        .await?;
    let mut summary = existing;
    summary["key_logs"][&item.id] = json!(logs);
    if serde_json::to_vec(&summary)?.len() as u64 > limit {
        return Err(
            "long-term decision record limit; retain originals and export or adjust policy".into(),
        );
    }
    sqlx::query("UPDATE storage_attempt SET summary=$2 WHERE run_id=$1")
        .bind(&item.run_id)
        .bind(summary)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn excerpt(source: &Directory, entry: &Entry, limit: u64) -> Result<Value> {
    use std::io::Read;
    let mut bytes = Vec::new();
    source
        .read(&entry.path)?
        .take(limit.min(1024))
        .read_to_end(&mut bytes)?;
    let text = crate::operator_view::redact_text(&String::from_utf8_lossy(&bytes));
    Ok(json!({"file":entry.path,"sha256":entry.sha256,"text":text,
            "kept_range":[0,bytes.len()],"original_bytes":entry.logical_bytes,
            "truncated":entry.logical_bytes>bytes.len() as u64,"reason":"bounded retrospective excerpt; full raw material may expire"}))
}

async fn authorize_scan(tx: &mut Tx<'_>, now: i64) -> Result<()> {
    let value: Option<Value> =
        sqlx::query_scalar("SELECT scan_retry FROM storage_guard WHERE id=1")
            .fetch_one(&mut **tx)
            .await?;
    if let Some(value) = value {
        let mut retry: Retry = serde_json::from_value(value)?;
        retry.authorize_retry_group(now);
        sqlx::query("UPDATE storage_guard SET scan_retry=$1 WHERE id=1")
            .bind(json!(retry))
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}
pub async fn authorize(tx: &mut Tx<'_>, requirement: i64, now: i64) -> Result<()> {
    authorize_scan(tx, now).await?;
    let rows:Vec<(String,Value)>=sqlx::query_as("SELECT m.id,m.retry FROM storage_material m JOIN storage_attempt a ON a.run_id=m.run_id WHERE a.requirement_id=$1 AND (m.retry->>'todo')::boolean FOR UPDATE OF m")
        .bind(requirement).fetch_all(&mut **tx).await?;
    for (id, value) in rows {
        let mut retry: Retry = serde_json::from_value(value)?;
        retry.authorize_retry_group(now);
        save_retry(tx, &id, &retry).await?;
    }
    crate::storage_db::authorize(tx, requirement, now).await?;
    sqlx::query("SELECT pg_notify('storage_phase_ended','')")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn archive(
    pool: &PgPool,
    config: &Deployment,
    item: &Material,
    source: &Directory,
    files: &[Entry],
    now: i64,
) -> Result<Package> {
    let id = reserve_archive(pool, config, item, files).await?;
    let (target, package) = create_archive(pool, config, item, &id, files, now).await?;
    storage_archive::write(source, &target, files, config.policy.entry_bytes)?;
    storage_archive::verify(&package)?;
    finish_archive(pool, config, &id, &target).await?;
    Ok(package)
}
async fn reserve_archive(
    pool: &PgPool,
    config: &Deployment,
    item: &Material,
    files: &[Entry],
) -> Result<String> {
    let id = format!("archive-{}-{}", item.id, crate::process::new_identity()?);
    let bytes = files
        .iter()
        .try_fold(65536u64, |total, file| {
            total.checked_add(file.logical_bytes)?.checked_add(65536)
        })
        .ok_or("archive reservation overflow")?
        .max(config.policy.categories[&Category::Cold].reserve_bytes);
    let mut tx = run_store::lock(pool).await?;
    let measured = storage_measure::measure(&mut tx, config).await?;
    if !store::reserve(
        &mut tx,
        &id,
        &item.run_id,
        Category::Cold,
        bytes,
        measured.usage(Category::Cold),
        &config.policy,
    )
    .await?
    {
        return Err("archive capacity unavailable; preserve original".into());
    }
    tx.commit().await?;
    Ok(id)
}
async fn create_archive(
    pool: &PgPool,
    config: &Deployment,
    item: &Material,
    id: &str,
    files: &[Entry],
    now: i64,
) -> Result<(Directory, Package)> {
    let path = config.cold.path.join(id);
    let target = config.cold.open()?.create_directory(Path::new(&id))?;
    let package = Package {
        path: path.clone(),
        identity: target.identity()?,
        files: files.to_vec(),
    };
    let mut tx = run_store::lock(pool).await?;
    store::material(
        &mut tx,
        store::Material {
            id,
            run: &item.run_id,
            path: &path,
            kind: Kind::Retrospective,
            category: Category::Cold,
            now,
        },
        &config.policy,
    )
    .await?;
    sqlx::query("UPDATE storage_material SET protection='partial archive; reconcile' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((target, package))
}
async fn finish_archive(
    pool: &PgPool,
    config: &Deployment,
    id: &str,
    target: &Directory,
) -> Result<()> {
    let actual = target.usage(config.policy.entry_count)?;
    let mut tx = run_store::lock(pool).await?;
    sqlx::query(
        "UPDATE storage_material SET protection=NULL,manifest=$2,actual_bytes=$3 WHERE id=$1",
    )
    .bind(id)
    .bind(json!(target.inventory(config.policy.entry_count)?))
    .bind(i64::try_from(actual)?)
    .execute(&mut *tx)
    .await?;
    store::reconcile(&mut tx, id, actual, true).await?;
    tx.commit().await?;
    Ok(())
}

async fn delete(pool: &PgPool, config: &Deployment, id: &str, now: i64) -> Result<()> {
    let mut tx = run_store::lock(pool).await?;
    let item = load(&mut tx, id).await?;
    let protection =
        storage_consumers::protection(&mut tx, config, &item.run_id, "rebuildable").await?;
    if protection.reason().is_some() {
        return Err("active material consumer".into());
    }
    remove_original(&item, config)?;
    sqlx::query(
        "UPDATE storage_material SET status='deleted',actual_bytes=0,deleted_at=$2 WHERE id=$1",
    )
    .bind(id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE storage_material SET status='deleted',actual_bytes=0,deleted_at=$2,deletion_reason='parent material retired after verified deletion' WHERE starts_with(path,$1) AND status<>'deleted'")
        .bind(format!("{}/",item.path)).bind(now).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

fn remove_original(item: &Material, config: &Deployment) -> Result<()> {
    if item.status != "deleting" {
        return Err("deletion intent missing".into());
    }
    if let Some(archive) = &item.archive {
        storage_archive::verify(&serde_json::from_value(archive.clone())?)?;
    }
    let manifest: Vec<Entry> =
        serde_json::from_value(item.manifest.clone().ok_or("deletion manifest missing")?)?;
    directory(item, config)?.remove(&manifest)?;
    Ok(())
}

pub async fn reconcile(tx: &mut Tx<'_>, config: &Deployment) -> Result<()> {
    measure_materials(tx, config).await?;
    adopt_existing(tx, config).await?;
    let allocations: Vec<(String,String,String)>=sqlx::query_as("SELECT request_id,run_id,category FROM storage_allocation WHERE NOT settled AND category<>'cold'")
        .fetch_all(&mut **tx).await?;
    for (request, run, category) in allocations {
        let actual = attributed_usage(tx, &request, &run, &category).await?;
        let protection = storage_consumers::protection(tx, config, &run, "rebuildable").await?;
        if !store::reconcile(tx, &request, actual as u64, protection.reason().is_none()).await? {
            tracing::warn!(
                run,
                request,
                "Run storage allocation exceeded; retain originals and stop affected Run"
            );
            sqlx::query("UPDATE agent_run SET stop_requested=true WHERE id=$1 AND NOT quiescent")
                .bind(&run)
                .execute(&mut **tx)
                .await?;
            sqlx::query("UPDATE integration_validation SET state='unknown',blocker='storage allocation exceeded; retain originals' WHERE id=$1 AND NOT quiescent")
                .bind(&run).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

async fn adopt_existing(tx: &mut Tx<'_>, config: &Deployment) -> Result<()> {
    let materials: Vec<(String,String,String)>=sqlx::query_as("SELECT DISTINCT allocation_request,run_id,category FROM storage_material WHERE category<>'cold' UNION SELECT run_id || '-database',run_id,'database' FROM storage_attempt UNION SELECT run_id || '-record',run_id,'record' FROM storage_attempt")
        .fetch_all(&mut **tx).await?;
    for (request, run, category) in materials {
        let bytes = attributed_usage(tx, &request, &run, &category).await?;
        if bytes > 0 {
            sqlx::query("INSERT INTO storage_allocation(request_id,run_id,category,policy_version,requested,allocated,outstanding) VALUES($1,$2,$3,$4,$5,$5,0) ON CONFLICT DO NOTHING")
                .bind(request).bind(run).bind(category).bind(&config.policy.version).bind(bytes).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

async fn measure_materials(tx: &mut Tx<'_>, config: &Deployment) -> Result<()> {
    let materials: Vec<(String, String, Value)> = sqlx::query_as(
        "SELECT id,path,directory_identity FROM storage_material WHERE status<>'deleted'",
    )
    .fetch_all(&mut **tx)
    .await?;
    for (id, path, identity) in materials {
        let directory = Directory::open(&PathBuf::from(path))?;
        directory.matches(&serde_json::from_value(identity)?)?;
        sqlx::query("UPDATE storage_material SET actual_bytes=$2 WHERE id=$1")
            .bind(id)
            .bind(i64::try_from(directory.usage(config.policy.entry_count)?)?)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}
async fn attributed_usage(
    tx: &mut Tx<'_>,
    request: &str,
    run: &str,
    category: &str,
) -> Result<i64> {
    let (sql, key) = match category {
        "database" => (
            "SELECT COALESCE(SUM(pg_column_size(e)+64),0)::bigint FROM runtime_evidence_chunk e WHERE run_id=$1",
            run,
        ),
        "record" => (
            "SELECT COALESCE(SUM(pg_column_size(e)+64),0)::bigint FROM run_event e WHERE run_id=$1",
            run,
        ),
        _ => (
            "SELECT COALESCE(SUM(m.actual_bytes),0)::bigint FROM storage_material m WHERE m.allocation_request=$1 AND NOT EXISTS(SELECT 1 FROM storage_material parent WHERE parent.allocation_request=m.allocation_request AND parent.status<>'deleted' AND starts_with(m.path,parent.path || '/'))",
            request,
        ),
    };
    Ok(sqlx::query_scalar(sql)
        .bind(key)
        .fetch_one(&mut **tx)
        .await?)
}
