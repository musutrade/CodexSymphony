//! Provider-specific completion facts retain their wire identities. Local
//! completions never invent PR, CI or merge facts to satisfy dependencies.
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Completion {
    Github(crate::group_dependency::Fact),
    Local(Local),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Local {
    pub source: String,
    pub requirement_id: i64,
    pub authorization_id: i64,
    pub child_revision: i64,
    pub repository_id: i64,
    pub delivery_version: String,
    pub acceptance_sha: String,
    pub acceptance_plan: Value,
    pub action_key: String,
    pub target: crate::local_git::Binding,
    pub evidence_sha256: String,
    pub artifact: String,
}
impl Completion {
    pub fn repository_id(&self) -> i64 {
        match self {
            Self::Github(f) => f.repository_id,
            Self::Local(f) => f.repository_id,
        }
    }
    pub fn github_id(&self) -> i64 {
        match self {
            Self::Github(f) => f.github_repository_id,
            Self::Local(_) => 0,
        }
    }
    pub fn commit(&self) -> &str {
        match self {
            Self::Github(f) => &f.merged_sha,
            Self::Local(f) => &f.delivery_version,
        }
    }
    pub fn artifact(&self) -> &str {
        match self {
            Self::Github(f) => &f.artifact,
            Self::Local(f) => &f.artifact,
        }
    }
}

pub async fn decode(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    value: Value,
) -> Result<Completion, sqlx::Error> {
    let fact: Completion = crate::budget_store::decode(value)?;
    if let Completion::Local(local) = &fact {
        verify_local(tx, local).await?;
    }
    Ok(fact)
}

async fn verify_local(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    local: &Local,
) -> Result<(), sqlx::Error> {
    let evidence: Option<Value> = sqlx::query_scalar("SELECT d.local_acceptance->'evidence' FROM delivery d JOIN group_execution_item i ON i.requirement_id=d.requirement_id WHERE d.action_key=$1 AND d.mode='local_git' AND d.released AND d.local_acceptance_quiescent AND d.requirement_id=$2 AND d.internal_repository_id=$3 AND d.head_sha=$4 AND d.local_binding=$5 AND d.local_acceptance->>'passed'='true' AND d.local_acceptance#>>'{evidence,candidate,sha}'=$4 AND i.authorization_id=$6 AND i.input#>'{review,revision}'=$7 AND i.input#>'{review,verification}'=$8 AND i.input#>'{child,repository_id}'=to_jsonb($3::bigint)")
        .bind(&local.action_key).bind(local.requirement_id).bind(local.repository_id).bind(&local.delivery_version).bind(serde_json::json!(local.target)).bind(local.authorization_id).bind(serde_json::json!(local.child_revision)).bind(&local.acceptance_plan).fetch_optional(&mut **tx).await?;
    let evidence = evidence.ok_or(sqlx::Error::Protocol(
        "local dependency completion identity unavailable".into(),
    ))?;
    let evidence: crate::validation::ValidationEvidence = crate::budget_store::decode(evidence)?;
    let bytes = serde_json::to_vec(&evidence).map_err(sqlx::Error::decode)?;
    crate::budget_store::require(
        local.source == "platform-local-delivery/v1"
            && local.acceptance_sha == local.delivery_version
            && local.evidence_sha256 == crate::validation::sha256(bytes)
            && local.artifact == format!("local-delivery:{}", local.action_key),
        "local dependency provenance differs",
    )
}
