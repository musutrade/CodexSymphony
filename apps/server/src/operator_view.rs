//! Read models contain independent durable facts; no view result advances a lifecycle.
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, sqlx::Error>;

pub async fn detail(pool: &PgPool, id: i64) -> Result<Value> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let requirement:Value=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'version',version,'revision',revision,'state',state,'paused',paused,'cancel_requested',cancel_requested,'cleanup_complete',cleanup_complete) FROM requirement WHERE id=$1")
        .bind(id).fetch_one(&mut *tx).await?;
    let mut value = timeline(&mut tx, id).await?;
    let (preparation, storage, storage_usage) = environment(&mut tx, id).await?;
    tx.commit().await?;
    value["requirement"] = requirement;
    value["preparation"] = json!(preparation);
    value["storage"] = storage;
    value["storage_lifecycle"] = json!(if storage_usage["configured"] == true {
        "configured"
    } else {
        "not_configured"
    });
    value["storage_usage"] = storage_usage;
    value["metrics"] = metrics(pool, id).await?;
    redact(&mut value);
    Ok(value)
}
async fn environment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
) -> Result<(Vec<Value>, Value, Value)> {
    let preparation:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('run_id',p.run_id,'phase',p.retry->>'phase','attempts',p.retry->'attempts','todo',p.retry->'todo','code',p.retry#>>'{last_failure,code}','detail',p.retry#>>'{last_failure,detail}') FROM preparation_record p JOIN requirement r ON r.id=p.requirement_id AND r.revision=p.revision WHERE p.requirement_id=$1 ORDER BY p.checked_at DESC")
        .bind(id).fetch_all(&mut **tx).await?;
    let storage: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('blocked',blocked,'error',error) FROM storage_guard WHERE id=1",
    )
    .fetch_one(&mut **tx)
    .await?;
    let storage_usage = crate::storage_view::detail(tx, id).await?;
    Ok((preparation, storage, storage_usage))
}

async fn timeline(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, id: i64) -> Result<Value> {
    let mut recoveries: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('event_key',f.event_key,'phase',f.phase,'decision',f.decision,'reason',f.reason,'log_ref',f.facts->>'log_ref','candidate_sha',f.facts->>'candidate_sha','attempts',COALESCE(t.attempts,0),'next_attempt_at',t.next_attempt_at) FROM recovery_failure f LEFT JOIN recovery_retry t USING(event_key) WHERE f.requirement_id=$1 ORDER BY f.created_at,f.event_key")
        .bind(id).fetch_all(&mut **tx).await?;
    let merges: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('event_key',action_key,'phase',CASE WHEN merged_sha IS NULL THEN 'merge' ELSE 'post_merge' END,'decision',state,'reason',COALESCE(blocker,'waiting for confirmed merge and applicable acceptance'),'log_ref','merge-operation:'||action_key,'candidate_sha',COALESCE(merged_sha,intent->>'head'),'attempts',jsonb_array_length(receipts),'next_attempt_at',next_attempt_at) FROM merge_operation WHERE requirement_id=$1 ORDER BY created_at,action_key")
        .bind(id).fetch_all(&mut **tx).await?;
    recoveries.extend(merges);
    let runs:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'revision',revision,'state',state,'phase',phase,'blocker',blocker,'quiescent',quiescent,'created_at',created_at,'waiting',waiting::text) FROM agent_run WHERE requirement_id=$1 ORDER BY created_at,id")
        .bind(id).fetch_all(&mut **tx).await?;
    let validations:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'revision',revision,'source_run_id',source_run_id,'candidate_sha',candidate_sha,'stage',stage,'result',result,'failure',failure) FROM candidate_validation WHERE requirement_id=$1 ORDER BY revision,id")
        .bind(id).fetch_all(&mut **tx).await?;
    let external:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('repository',d.repository,'pr_number',d.pr_number,'head_sha',d.head_sha,'revision',d.revision,'observation',g.observation::text,'stale',COALESCE(g.stale,true)) FROM delivery d LEFT JOIN github_pr_observation g ON g.repository_id=d.repository_id AND g.number=d.pr_number AND g.requirement_id=d.requirement_id WHERE d.requirement_id=$1 ORDER BY d.revision")
        .bind(id).fetch_all(&mut **tx).await?;
    let questions:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'version',version,'revision',revision,'run_id',run_id,'questions',(SELECT jsonb_agg(jsonb_build_object('id',q->>'id','question',q->>'question','options',COALESCE((SELECT jsonb_agg(o->>'label') FROM jsonb_array_elements(q->'options') o),'[]'::jsonb))) FROM jsonb_array_elements(original->'params'->'questions') q),'answered',answer IS NOT NULL,'resume_state',resume_state) FROM runtime_question WHERE requirement_id=$1 ORDER BY created_at,id")
        .bind(id).fetch_all(&mut **tx).await?;
    let events:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('kind',kind,'version',version,'created_at',created_at) FROM business_event WHERE object_id=$1 ORDER BY id DESC LIMIT 100")
        .bind(format!("requirement:{id}")).fetch_all(&mut **tx).await?;
    let materials:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('run_id',e.run_id,'channel',e.channel,'kept_bytes',e.kept_bytes,'discarded_bytes',e.discarded_bytes,'status',CASE WHEN e.expired_at IS NOT NULL THEN 'expired; retrospective available' WHEN e.truncated THEN 'truncated' ELSE 'available' END) FROM runtime_evidence e JOIN agent_run a ON a.id=e.run_id WHERE a.requirement_id=$1 ORDER BY e.run_id,e.channel")
        .bind(id).fetch_all(&mut **tx).await?;
    Ok(
        json!({"runs":runs,"recoveries":recoveries,"validations":validations,"external":external,"questions":questions,"events":events,"materials":materials}),
    )
}
async fn metrics(pool: &PgPool, id: i64) -> Result<Value> {
    let mut result:Value=sqlx::query_scalar("SELECT jsonb_build_object('model_calls',count(*),'input',CASE WHEN count(*)=count(usage->>'input') THEN SUM((usage->>'input')::bigint) END,'cached',CASE WHEN count(*)=count(usage->>'cached') THEN SUM((usage->>'cached')::bigint) END,'output',CASE WHEN count(*)=count(usage->>'output') THEN SUM((usage->>'output')::bigint) END,'repair_count',(SELECT count(*) FROM repair_reservation WHERE requirement_id=$1),'human_seconds',(SELECT SUM((waiting->>'human_seconds')::bigint) FROM agent_run WHERE requirement_id=$1),'interventions',(SELECT count(*) FROM operator_intervention WHERE requirement_id=$1)) FROM model_call WHERE requirement_id=$1")
        .bind(id).fetch_one(pool).await?;
    result["reasons"] = json!(
        sqlx::query_scalar::<_, String>(
            "SELECT reason FROM operator_intervention WHERE requirement_id=$1 ORDER BY id"
        )
        .bind(id)
        .fetch_all(pool)
        .await?
    );
    result["phases"]=json!(sqlx::query_scalar::<_,Value>("SELECT jsonb_build_object('phase',phase,'seconds',extract(epoch FROM COALESCE(finished_at,now())-started_at)::bigint,'complete',finished_at IS NOT NULL) FROM operator_phase WHERE requirement_id=$1 ORDER BY id").bind(id).fetch_all(pool).await?);
    result["to_pr_seconds"]=json!(sqlx::query_scalar::<_,Option<i64>>("SELECT extract(epoch FROM MIN(t.created_at)-r.created_at)::bigint FROM requirement r LEFT JOIN delivery d ON d.requirement_id=r.id AND d.pr_number IS NOT NULL LEFT JOIN delivery_attempt t ON t.action_key=d.action_key AND t.operation='create' AND t.result IS NOT NULL WHERE r.id=$1 GROUP BY r.created_at").bind(id).fetch_one(pool).await?);
    result["zero_intervention"] = zero_intervention(pool).await?;
    Ok(result)
}
async fn zero_intervention(pool: &PgPool) -> Result<Value> {
    sqlx::query_scalar("WITH samples AS (SELECT r.id FROM requirement r WHERE r.state='Submitted' AND EXISTS(SELECT 1 FROM business_event e WHERE e.object_id='requirement:'||r.id AND e.kind='reviewed_ready' AND e.created_at>(SELECT started_at FROM operator_measurement_epoch WHERE id=1))) SELECT jsonb_build_object('phase','reviewed_to_submitted','denominator',count(*),'numerator',count(*) FILTER(WHERE NOT EXISTS(SELECT 1 FROM operator_intervention i WHERE i.requirement_id=s.id))) FROM samples s").fetch_one(pool).await
}
pub fn redact(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact_text(text),
        Value::Array(values) => values.iter_mut().for_each(redact),
        Value::Object(values) => values.values_mut().for_each(redact),
        _ => (),
    }
}
pub fn redact_text(text: &str) -> String {
    // PEM bodies must never survive line-by-line header filtering.
    if text.to_ascii_lowercase().contains("private key") {
        return "[redacted]".into();
    }
    text.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "bearer ",
                "authorization",
                "credential",
                "passwd",
                "pwd=",
                "token\"",
                "token=",
                "token:",
                "password",
                "secret",
                "api_key",
                "api-key",
                "ghp_",
                "ghs_",
                "github_pat_",
                "sk-",
                "private key",
                "://",
            ]
            .iter()
            .any(|marker| lower.contains(marker))
            {
                "[redacted]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
pub async fn evidence(pool: &PgPool, id: i64, run: &str, channel: &str) -> Result<Value> {
    let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_evidence e JOIN agent_run a ON a.id=e.run_id WHERE a.requirement_id=$1 AND e.run_id=$2 AND e.channel=$3)")
        .bind(id).bind(run).bind(channel).fetch_one(pool).await?;
    if !exists {
        return Err(sqlx::Error::RowNotFound);
    }
    let retrospective: Option<String> = sqlx::query_scalar(
        "SELECT retrospective FROM runtime_evidence WHERE run_id=$1 AND channel=$2",
    )
    .bind(run)
    .bind(channel)
    .fetch_one(pool)
    .await?;
    if let Some(text) = retrospective {
        return Ok(json!({"text":redact_text(&text),"preview_only":true}));
    }
    let chunks:Vec<Vec<u8>>=sqlx::query_scalar("SELECT payload FROM runtime_evidence_chunk WHERE run_id=$1 AND channel=$2 ORDER BY sequence LIMIT 16")
        .bind(run).bind(channel).fetch_all(pool).await?;
    let bytes = chunks.concat();
    Ok(json!({"text":redact_text(&String::from_utf8_lossy(&bytes)),"preview_only":true}))
}
