//! Host-only resource authorization. Never resumes or replaces an existing Run.
use std::io::Read;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn input(args: &[String]) -> Result<crate::budget_store::Increase> {
    if args != ["increase", "--stdin-json"] {
        return Err("usage: budget increase --stdin-json".into());
    }
    let mut bytes = Vec::new();
    std::io::stdin().take(8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err("budget input exceeds limit".into());
    }
    Ok(serde_json::from_slice(&bytes).or(Err("invalid budget authorization"))?)
}

pub async fn run(args: &[String]) -> Result<()> {
    let input = input(args)?;
    let url = std::env::var("DATABASE_URL").or(Err("DATABASE_URL is required"))?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .or(Err("database unavailable"))?;
    crate::budget_store::increase(&pool, &input).await?;
    Ok(())
}
