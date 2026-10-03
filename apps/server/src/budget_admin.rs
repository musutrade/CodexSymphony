//! Host-only resource authorization. Never resumes or replaces an existing Run.
use std::io::Read;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn input(args: &[String]) -> Result<Vec<u8>> {
    if args != ["increase", "--stdin-json"]
        && args != ["group-increase", "--stdin-json"]
        && args != ["repair-recheck", "--stdin-json"]
        && args != ["repair-source-recheck", "--stdin-json"]
    {
        return Err("usage: budget {increase|group-increase|repair-recheck|repair-source-recheck} --stdin-json".into());
    }
    let mut bytes = Vec::new();
    std::io::stdin().take(8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err("budget input exceeds limit".into());
    }
    Ok(bytes)
}

pub async fn run(args: &[String]) -> Result<()> {
    let input = input(args)?;
    let pool = connect().await?;
    dispatch(args, &input, &pool).await
}

pub(crate) async fn connect() -> Result<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").or(Err("DATABASE_URL is required"))?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .or(Err("database unavailable".into()))
}

pub(crate) async fn dispatch(args: &[String], input: &[u8], pool: &sqlx::PgPool) -> Result<()> {
    match args[0].as_str() {
        "repair-source-recheck" => source_recheck(input, pool).await?,
        "repair-recheck" => repair_recheck(input, pool).await?,
        "group-increase" => group_increase(input, pool).await?,
        _ => increase(input, pool).await?,
    }
    Ok(())
}

async fn source_recheck(input: &[u8], pool: &sqlx::PgPool) -> Result<()> {
    let command = serde_json::from_slice(input).or(Err("invalid linked source recovery"))?;
    crate::linked_source_recovery::recheck(pool, &command).await?;
    Ok(())
}
async fn repair_recheck(input: &[u8], pool: &sqlx::PgPool) -> Result<()> {
    let command = serde_json::from_slice(input).or(Err("invalid linked repair recovery"))?;
    crate::linked_budget_recovery::recheck(pool, &command).await?;
    Ok(())
}
async fn group_increase(input: &[u8], pool: &sqlx::PgPool) -> Result<()> {
    let grant = serde_json::from_slice(input).or(Err("invalid group budget authorization"))?;
    crate::group_budget_increase::increase(pool, &grant).await?;
    Ok(())
}
async fn increase(input: &[u8], pool: &sqlx::PgPool) -> Result<()> {
    let grant = serde_json::from_slice(input).or(Err("invalid budget authorization"))?;
    crate::budget_store::increase(pool, &grant).await?;
    Ok(())
}
