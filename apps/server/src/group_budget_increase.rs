//! Host-authorized additions to an already bound group account.
//! Reviewed Run inputs and cumulative usage remain immutable.
use crate::{
    budget::Amount,
    budget_store::{decode, require},
    run_store,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, sqlx::Error>;
type Tx<'a> = Transaction<'a, Postgres>;
struct BudgetRow {
    limits: Amount,
    used: Amount,
    reserved: Amount,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChildIncrease {
    pub child_id: String,
    pub requirement_id: i64,
    pub expected_version: i64,
    pub delta: Amount,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GroupIncrease {
    pub request_id: String,
    pub draft_id: String,
    pub expected_queue_version: i64,
    pub actor: String,
    pub reason: String,
    pub parent_delta: Amount,
    pub children: Vec<ChildIncrease>,
}

pub async fn increase(pool: &PgPool, input: &GroupIncrease) -> Result<()> {
    validate(input)?;
    let mut tx = run_store::lock(pool).await?;
    if replay(&mut tx, input).await? {
        return Ok(());
    }
    apply(&mut tx, input).await?;
    sqlx::query(
        "INSERT INTO group_queue_event(request_id,draft_id,input,result) VALUES($1,$2,$3,$4)",
    )
    .bind(&input.request_id)
    .bind(&input.draft_id)
    .bind(json!(input))
    .bind(json!({"authorized":true}))
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

fn validate(input: &GroupIncrease) -> Result<()> {
    validate_identity(input)?;
    validate_resources(input)?;
    validate_children(input)
}

fn validate_identity(input: &GroupIncrease) -> Result<()> {
    require(
        !input.request_id.trim().is_empty()
            && input.request_id.len() <= 160
            && !input.draft_id.trim().is_empty(),
        "group authorization request and draft required",
    )?;
    require(
        !input.actor.trim().is_empty()
            && !input.reason.trim().is_empty()
            && input.expected_queue_version > 0,
        "group authorization actor, reason and version required",
    )
}

fn validate_resources(input: &GroupIncrease) -> Result<()> {
    require(
        input.parent_delta.nonnegative()
            && input.parent_delta != Amount::default()
            && !input.children.is_empty()
            && input.children.len() <= 32,
        "positive bounded group increment required",
    )?;
    Ok(())
}

fn validate_children(input: &GroupIncrease) -> Result<()> {
    let mut names = BTreeSet::new();
    let mut sum = Amount::default();
    for child in &input.children {
        validate_child(child, names.insert(&child.child_id))?;
        sum = sum
            .checked_add(child.delta)
            .ok_or_else(accounting_overflow)?;
    }
    require(
        sum == input.parent_delta,
        "group increment must equal child increments",
    )
}

fn validate_child(child: &ChildIncrease, unique: bool) -> Result<()> {
    require(
        !child.child_id.trim().is_empty()
            && child.requirement_id > 0
            && child.expected_version > 0
            && child.delta.nonnegative()
            && unique,
        "invalid or duplicate child increment",
    )
}

fn accounting_overflow() -> sqlx::Error {
    sqlx::Error::Protocol("group budget authorization overflow".into())
}

async fn replay(tx: &mut Tx<'_>, input: &GroupIncrease) -> Result<bool> {
    let previous: Option<Value> =
        sqlx::query_scalar("SELECT input FROM group_queue_event WHERE request_id=$1")
            .bind(&input.request_id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(previous) = previous {
        require(
            previous == json!(input),
            "group authorization request identity conflict",
        )?;
        return Ok(true);
    }
    Ok(false)
}

async fn apply(tx: &mut Tx<'_>, input: &GroupIncrease) -> Result<()> {
    check_queue(tx, input).await?;
    let mut budgets = load_budgets(tx, input).await?;
    let new_parent = increase_parent(input, &mut budgets)?;
    let sum = increase_children(tx, input, &mut budgets).await?;
    require(
        budgets.is_empty() && sum == new_parent,
        "group limits do not sum to parent",
    )?;
    sqlx::query("UPDATE group_budget SET limits=$2 WHERE draft_id=$1 AND item_id=''")
        .bind(&input.draft_id)
        .bind(json!(new_parent))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn check_queue(tx: &mut Tx<'_>, input: &GroupIncrease) -> Result<()> {
    let queue: Option<(i64, String)> =
        sqlx::query_as("SELECT version,state FROM group_queue WHERE draft_id=$1 FOR UPDATE")
            .bind(&input.draft_id)
            .fetch_optional(&mut **tx)
            .await?;
    require(
        queue == Some((input.expected_queue_version, "waiting_scheduler".into())),
        "group queue authorization changed",
    )?;
    let pending: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM group_edit WHERE draft_id=$1)")
            .bind(&input.draft_id)
            .fetch_one(&mut **tx)
            .await?;
    require(!pending, "pending group edit must be resolved")?;
    Ok(())
}

async fn load_budgets(
    tx: &mut Tx<'_>,
    input: &GroupIncrease,
) -> Result<BTreeMap<String, BudgetRow>> {
    let rows: Vec<(String, Value, Value, Value)> = sqlx::query_as(
        "SELECT item_id,limits,used,reserved FROM group_budget WHERE draft_id=$1 ORDER BY item_id FOR UPDATE",
    )
    .bind(&input.draft_id)
    .fetch_all(&mut **tx)
    .await?;
    require(
        rows.len() == input.children.len() + 1,
        "group budget membership changed",
    )?;
    let mut budgets = BTreeMap::new();
    for (id, limit, used, reserved) in rows {
        budgets.insert(
            id,
            BudgetRow {
                limits: decode(limit)?,
                used: decode(used)?,
                reserved: decode(reserved)?,
            },
        );
    }
    Ok(budgets)
}

fn increase_parent(
    input: &GroupIncrease,
    budgets: &mut BTreeMap<String, BudgetRow>,
) -> Result<Amount> {
    let parent = budgets.remove("").ok_or_else(accounting_overflow)?;
    let new_parent = parent
        .limits
        .checked_add(input.parent_delta)
        .ok_or_else(accounting_overflow)?;
    require(
        exposure_fits(parent.used, parent.reserved, new_parent)?,
        "parent exposure exceeds increase",
    )?;
    Ok(new_parent)
}

async fn increase_children(
    tx: &mut Tx<'_>,
    input: &GroupIncrease,
    budgets: &mut BTreeMap<String, BudgetRow>,
) -> Result<Amount> {
    let mut sum = Amount::default();
    for child in &input.children {
        let row = budgets
            .remove(&child.child_id)
            .ok_or_else(accounting_overflow)?;
        let next = row
            .limits
            .checked_add(child.delta)
            .ok_or_else(accounting_overflow)?;
        require(
            exposure_fits(row.used, row.reserved, next)?,
            "child exposure exceeds increase",
        )?;
        apply_child(tx, input, child, row.limits, next).await?;
        sum = sum.checked_add(next).ok_or_else(accounting_overflow)?;
    }
    Ok(sum)
}

fn exposure_fits(used: Amount, reserved: Amount, limit: Amount) -> Result<bool> {
    Ok(used
        .checked_add(reserved)
        .ok_or_else(accounting_overflow)?
        .fits(limit))
}

async fn apply_child(
    tx: &mut Tx<'_>,
    input: &GroupIncrease,
    child: &ChildIncrease,
    old: Amount,
    next: Amount,
) -> Result<()> {
    check_child_account(tx, input, child, old).await?;
    write_child(tx, input, child, next).await
}

async fn check_child_account(
    tx: &mut Tx<'_>,
    input: &GroupIncrease,
    child: &ChildIncrease,
    old: Amount,
) -> Result<()> {
    let bound: Option<i64> = sqlx::query_scalar("SELECT requirement_id FROM group_execution_item WHERE draft_id=$1 AND child_id=$2 AND NOT removed")
        .bind(&input.draft_id).bind(&child.child_id).fetch_optional(&mut **tx).await?;
    require(
        bound == Some(child.requirement_id),
        "child account binding changed",
    )?;
    let current: (i64, Value) = sqlx::query_as(
        "SELECT version,limits FROM requirement_budget WHERE requirement_id=$1 FOR UPDATE",
    )
    .bind(child.requirement_id)
    .fetch_one(&mut **tx)
    .await?;
    require(
        current.0 == child.expected_version && decode::<Amount>(current.1)? == old,
        "child authorization version or limit changed",
    )
}

async fn write_child(
    tx: &mut Tx<'_>,
    input: &GroupIncrease,
    child: &ChildIncrease,
    next: Amount,
) -> Result<()> {
    let request = format!("{}:{}", input.request_id, child.child_id);
    require(
        request.len() <= 200,
        "child authorization request identity too long",
    )?;
    sqlx::query("INSERT INTO budget_authorization(requirement_id,version,request_id,actor,reason,delta,limits) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(child.requirement_id).bind(child.expected_version + 1).bind(request).bind(&input.actor).bind(&input.reason).bind(json!(child.delta)).bind(json!(next)).execute(&mut **tx).await?;
    sqlx::query("UPDATE requirement_budget SET limits=$2,version=version+1,exhausted=false WHERE requirement_id=$1")
        .bind(child.requirement_id).bind(json!(next)).execute(&mut **tx).await?;
    sqlx::query("UPDATE group_budget SET limits=$3 WHERE draft_id=$1 AND item_id=$2")
        .bind(&input.draft_id)
        .bind(&child.child_id)
        .bind(json!(next))
        .execute(&mut **tx)
        .await?;
    Ok(())
}
