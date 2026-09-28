//! Host-only resource authorization. Never resumes or replaces an existing Run.
use std::io::Read;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn input(args: &[String]) -> Result<Vec<u8>> {
    if args != ["increase", "--stdin-json"] && args != ["group-increase", "--stdin-json"] {
        return Err("usage: budget {increase|group-increase} --stdin-json".into());
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

async fn connect() -> Result<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL").or(Err("DATABASE_URL is required"))?;
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .or(Err("database unavailable".into()))
}

async fn dispatch(args: &[String], input: &[u8], pool: &sqlx::PgPool) -> Result<()> {
    if args[0] == "group-increase" {
        let grant: crate::group_budget_increase::GroupIncrease =
            serde_json::from_slice(input).or(Err("invalid group budget authorization"))?;
        crate::group_budget_increase::increase(pool, &grant).await?;
    } else {
        let grant: crate::budget_store::Increase =
            serde_json::from_slice(input).or(Err("invalid budget authorization"))?;
        crate::budget_store::increase(pool, &grant).await?;
    }
    Ok(())
}
