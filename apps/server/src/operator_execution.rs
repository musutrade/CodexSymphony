//! Read-only projections of frozen inputs, repository scopes and cumulative accounts.
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
type Tx<'a> = Transaction<'a, Postgres>;
type Result<T> = std::result::Result<T, sqlx::Error>;

pub async fn detail(tx: &mut Tx<'_>, id: i64) -> Result<Value> {
    let revisions = revisions(tx, id).await?;
    let scopes = scopes(tx, id).await?;
    let budget = budget(tx, id).await?;
    let group = group(tx, id).await?;
    Ok(json!({"revisions":revisions,"scopes":scopes,"budget":budget,"group":group}))
}

async fn revisions(tx: &mut Tx<'_>, id: i64) -> Result<Vec<Value>> {
    sqlx::query_scalar("SELECT jsonb_build_object('revision',revision,'repository_id',document->'repository_id','repository_version',document->'repository_version','kind',COALESCE((SELECT input#>>'{child,kind}' FROM group_execution_item WHERE requirement_id=$1),'coding'),'environment',document#>>'{repository,environment}','hooks',COALESCE(document#>'{repository,hooks}','[]'::jsonb)) FROM execution_revision WHERE requirement_id=$1 ORDER BY revision")
        .bind(id).fetch_all(&mut **tx).await
}

async fn scopes(tx: &mut Tx<'_>, id: i64) -> Result<Vec<Value>> {
    sqlx::query_scalar("SELECT jsonb_build_object('plugin_id',i.plugin_id,'invocation_id',i.invocation_id,'revision',i.revision,'repository_id',i.repository_id,'scope_version',i.scope_version,'current_version',p.version,'kind',p.kind,'repository_ids',p.repository_ids,'allowed',plugin_scope_allows(i.plugin_id,i.repository_id)) FROM plugin_scope_invocation i LEFT JOIN plugin_scope p USING(plugin_id) WHERE i.requirement_id=$1 ORDER BY i.revision,i.plugin_id,i.invocation_id")
        .bind(id).fetch_all(&mut **tx).await
}

async fn budget(tx: &mut Tx<'_>, id: i64) -> Result<Value> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM requirement_budget WHERE requirement_id=$1)",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    if !exists {
        return Ok(Value::Null);
    }
    Ok(json!(crate::budget_store::balance(tx, id).await?))
}

async fn group(tx: &mut Tx<'_>, id: i64) -> Result<Option<Value>> {
    sqlx::query_scalar("SELECT jsonb_build_object('draft_id',i.draft_id,'child_id',i.child_id,'authorization_id',i.authorization_id,'parent_revision',a.snapshot->'parent_revision','review_version',a.review_version,'budgets',(SELECT COALESCE(jsonb_agg(jsonb_build_object('item_id',b.item_id,'limits',b.limits,'used',b.used,'reserved',b.reserved) ORDER BY b.item_id),'[]'::jsonb) FROM group_budget b WHERE b.draft_id=i.draft_id AND b.item_id IN ('',i.child_id)),'dependencies',COALESCE(c.dependencies,'[]'::jsonb)::text,'completion',(SELECT fact::text FROM group_completion WHERE requirement_id=$1)) FROM group_execution_item i JOIN group_authorization a ON a.id=i.authorization_id LEFT JOIN group_claim_input c ON c.requirement_id=i.requirement_id WHERE i.requirement_id=$1")
        .bind(id).fetch_optional(&mut **tx).await
}
