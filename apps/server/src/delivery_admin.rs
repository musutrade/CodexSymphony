//! Trusted operator entry; unavailable to the Runtime and HTTP task APIs.
use std::io::Read;
type Result<T> = crate::delivery_extension::Result<T>;

pub async fn run(args: &[String]) -> Result<()> {
    if args != ["acceptance-recheck", "--stdin-json"] {
        return Err("usage: delivery acceptance-recheck --stdin-json".into());
    }
    let mut input = Vec::new();
    std::io::stdin().take(32769).read_to_end(&mut input)?;
    if input.len() > 32768 {
        return Err("acceptance correction input exceeds limit".into());
    }
    let command: crate::local_acceptance_recheck::Command =
        serde_json::from_slice(&input).or(Err("invalid acceptance correction"))?;
    let pool = crate::budget_admin::connect().await?;
    let result = crate::local_acceptance_recheck::request(&pool, &command).await?;
    std::io::Write::write_all(
        &mut std::io::stdout(),
        format!("{}\n", serde_json::to_string(&result)?).as_bytes(),
    )?;
    Ok(())
}
