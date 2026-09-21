//! Attribution uses cumulative existing model-call facts, never a new Run budget.
use crate::{
    budget::Amount,
    budget_store::{decode, require},
    group_queue_store::{Result, Tx},
};
use serde_json::{Value, json};
fn subtract(a: Amount, b: Amount) -> Result<Amount> {
    Ok(Amount {
        tokens: a.tokens.checked_sub(b.tokens).ok_or_else(overflow)?,
        turns: a.turns.checked_sub(b.turns).ok_or_else(overflow)?,
        model_seconds: a
            .model_seconds
            .checked_sub(b.model_seconds)
            .ok_or_else(overflow)?,
    })
}
fn overflow() -> sqlx::Error {
    sqlx::Error::Protocol("group accounting overflow".into())
}
pub(crate) async fn allowed(tx: &mut Tx<'_>, id: i64, reserve: Amount) -> Result<bool> {
    capacity(tx, id, reserve, false).await
}
pub(crate) async fn prepaid_fits(tx: &mut Tx<'_>, id: i64) -> Result<bool> {
    capacity(tx, id, Amount::default(), true).await
}
async fn capacity(tx: &mut Tx<'_>, id: i64, reserve: Amount, prepaid: bool) -> Result<bool> {
    let rows:Vec<(Value,Value,Value)>=sqlx::query_as("SELECT b.limits,b.used,b.reserved FROM group_budget b JOIN group_execution_item i ON i.draft_id=b.draft_id AND (b.item_id='' OR b.item_id=i.child_id) WHERE i.requirement_id=$1")
        .bind(id).fetch_all(&mut **tx).await?;
    for (limit, used, reserved) in rows {
        let limit: Amount = decode(limit)?;
        let exposure = exposure(used, reserved)?;
        if (!prepaid && exposure.reached(limit))
            || !exposure
                .checked_add(reserve)
                .ok_or_else(overflow)?
                .fits(limit)
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn exposure(used: Value, reserved: Value) -> Result<Amount> {
    decode::<Amount>(used)?
        .checked_add(decode(reserved)?)
        .ok_or_else(overflow)
}
pub(crate) async fn sync(tx: &mut Tx<'_>, id: i64) -> Result<()> {
    let item: Option<(String, String)> = sqlx::query_as(
        "SELECT draft_id,child_id FROM group_execution_item WHERE requirement_id=$1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((draft, child)) = item else {
        return Ok(());
    };
    let balance = crate::budget_store::balance(tx, id).await?;
    let reserved = subtract(balance.exposure, balance.used)?;
    let (delta_used, delta_reserved) = accounted_delta(tx, id, balance.used, reserved).await?;
    for key in ["", child.as_str()] {
        update(tx, &draft, key, delta_used, delta_reserved).await?;
    }
    sqlx::query("UPDATE group_accounted SET used=$2,reserved=$3 WHERE requirement_id=$1")
        .bind(id)
        .bind(json!(balance.used))
        .bind(json!(reserved))
        .execute(&mut **tx)
        .await?;
    Ok(())
}
async fn accounted_delta(
    tx: &mut Tx<'_>,
    id: i64,
    used: Amount,
    reserved: Amount,
) -> Result<(Amount, Amount)> {
    sqlx::query("INSERT INTO group_accounted(requirement_id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    let (old_used, old_reserved): (Value, Value) =
        sqlx::query_as("SELECT used,reserved FROM group_accounted WHERE requirement_id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    let delta_used = subtract(used, decode(old_used)?)?;
    let delta_reserved = subtract(reserved, decode(old_reserved)?)?;
    Ok((delta_used, delta_reserved))
}
async fn update(
    tx: &mut Tx<'_>,
    draft: &str,
    key: &str,
    delta_used: Amount,
    delta_reserved: Amount,
) -> Result<()> {
    let (used, reserved): (Value, Value) = sqlx::query_as(
        "SELECT used,reserved FROM group_budget WHERE draft_id=$1 AND item_id=$2 FOR UPDATE",
    )
    .bind(draft)
    .bind(key)
    .fetch_one(&mut **tx)
    .await?;
    let used = decode::<Amount>(used)?
        .checked_add(delta_used)
        .ok_or_else(overflow)?;
    let reserved = decode::<Amount>(reserved)?
        .checked_add(delta_reserved)
        .ok_or_else(overflow)?;
    require(
        used.nonnegative() && reserved.nonnegative(),
        "negative group accounting",
    )?;
    sqlx::query("UPDATE group_budget SET used=$3,reserved=$4 WHERE draft_id=$1 AND item_id=$2")
        .bind(draft)
        .bind(key)
        .bind(json!(used))
        .bind(json!(reserved))
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub(crate) async fn exhausted(tx: &mut Tx<'_>, id: i64) -> Result<bool> {
    let rows: Vec<(Value,Value,Value)>=sqlx::query_as("SELECT b.limits,b.used,b.reserved FROM group_budget b JOIN group_execution_item i ON i.draft_id=b.draft_id AND (b.item_id='' OR b.item_id=i.child_id) WHERE i.requirement_id=$1").bind(id).fetch_all(&mut **tx).await?;
    for (limits, used, reserved) in rows {
        let limits = decode(limits)?;
        let used: Amount = decode(used)?;
        let exposure = used.checked_add(decode(reserved)?).ok_or_else(overflow)?;
        if used.execution_exhausted(limits) || !exposure.fits(limits) {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(crate) async fn stop_group(tx: &mut Tx<'_>, id: i64) -> Result<()> {
    let ids: Vec<i64>=sqlx::query_scalar("SELECT peer.requirement_id FROM group_execution_item own JOIN group_execution_item peer USING(draft_id) WHERE own.requirement_id=$1 AND peer.requirement_id IS NOT NULL")
        .bind(id).fetch_all(&mut **tx).await?;
    for peer in ids {
        crate::budget_store::stop(tx, peer).await?;
    }
    Ok(())
}
