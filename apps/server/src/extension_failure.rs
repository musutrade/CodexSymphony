//! Persist plugin feedback without guessing root causes or authorizing repair.
use crate::{extension_feedback, validation::StepEvidence};
use serde_json::{Value, json};
use sqlx::PgPool;
type Result<T> = std::result::Result<T, sqlx::Error>;

pub async fn record(pool: &PgPool, validation: &str, steps: &[StepEvidence]) -> Result<bool> {
    let feedback = collect_feedback(steps);
    if feedback.is_empty() {
        return Ok(false);
    }
    let mut tx = crate::run_store::lock(pool).await?;
    sqlx::query("UPDATE candidate_validation SET extension_feedback=$2 WHERE id=$1 AND extension_feedback IS NULL")
        .bind(validation).bind(json!(feedback)).execute(&mut *tx).await?;
    for item in feedback {
        persist_feedback(&mut tx, validation, &item).await?;
    }
    tx.commit().await?;
    Ok(true)
}

pub fn normalized(step: &StepEvidence) -> Value {
    match extension_feedback::decode(step) {
        Ok(Some(feedback)) => {
            json!({"check_id":step.id,"verdict":feedback.verdict,"fault":feedback.fault,"exit_code":step.exit_code,"log_ref":step.log_ref,"output_sha256":step.output_sha256,"authorized_code_check":step.code_failure,"source":"reviewed_plugin"})
        }
        _ => {
            json!({"check_id":step.id,"verdict":"unknown","fault":{"class":"protocol","code":"invalid_feedback","message":"Missing, invalid or contradictory feedback","owner":"plugin_maintainer","scope":[step.id],"resume_condition":"Restore the approved feedback contract and reconcile original execution"},"exit_code":step.exit_code,"log_ref":step.log_ref,"output_sha256":step.output_sha256,"source":"host"})
        }
    }
}

pub async fn crashed(pool: &PgPool, validation: &str) -> Result<()> {
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason) SELECT 'extension:'||id||':execution',requirement_id,id,'local',jsonb_build_object('phase','local','candidate_sha',candidate_sha,'feedback',jsonb_build_object('verdict','unknown','source','host','fault',jsonb_build_object('class','unknown','code','execution_unknown','owner','operator','resume_condition','Reconcile the original supervisor and retained evidence'))),id,'blocked','validation execution or evidence incomplete; root cause unknown' FROM candidate_validation WHERE id=$1 AND result='blocked' ON CONFLICT DO NOTHING")
        .bind(validation).execute(pool).await?;
    Ok(())
}

fn collect_feedback(steps: &[StepEvidence]) -> Vec<Value> {
    let mut feedback = Vec::new();
    for step in steps {
        if extension_feedback::negotiated(step) {
            feedback.push(normalized(step));
        }
    }
    feedback
}

/// Older exit-code adapters can retain an unknown result without a structured
/// fault. Recover only the persisted failure, never infer successful execution.
pub(crate) async fn reconcile_legacy(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    requirement: i64,
    validation: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason) SELECT 'extension:'||v.id||':legacy:'||s.step_id,v.requirement_id,v.id,'local',jsonb_build_object('phase','local','candidate_sha',v.candidate_sha,'feedback',jsonb_build_object('check_id',s.step_id,'verdict','unknown','source','host','exit_code',s.exit_code,'output_sha256',s.output_sha256,'log_ref',s.log_ref,'fault',jsonb_build_object('class','unknown','code','legacy_evidence_unknown','owner','operator','resume_condition','Reconcile original execution and approve a complete validation plan'))),s.output_sha256,'blocked','retained legacy validation evidence is unknown; no code fault inferred' FROM candidate_validation v JOIN validation_step s ON s.validation_id=v.id WHERE v.id=$1 AND v.requirement_id=$2 AND v.result='blocked' AND s.status='unknown' AND COALESCE(s.command->>1,'')<>'--symphony-feedback-v1' ON CONFLICT DO NOTHING")
        .bind(validation).bind(requirement).execute(&mut **tx).await?;
    Ok(())
}
async fn persist_feedback(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    validation: &str,
    item: &Value,
) -> Result<()> {
    if item["verdict"] == "pass" {
        return Ok(());
    }
    let event = format!(
        "extension:{validation}:{}",
        item["check_id"].as_str().unwrap_or("unknown")
    );
    let decision = if item["verdict"] == "fail" && item["authorized_code_check"] == true {
        "code"
    } else {
        "blocked"
    };
    sqlx::query("INSERT INTO recovery_failure(event_key,requirement_id,source_validation_id,phase,facts,fingerprint,decision,reason) SELECT $1,requirement_id,id,'local',jsonb_build_object('phase','local','candidate_sha',candidate_sha,'feedback',$2),$3,$4,'structured extension feedback; inspect evidence and resume condition' FROM candidate_validation WHERE id=$5 ON CONFLICT DO NOTHING")
            .bind(&event).bind(item).bind(crate::validation::sha256(item.to_string())).bind(decision).bind(validation).execute(&mut **tx).await?;
    Ok(())
}
