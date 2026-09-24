//! Same approved test-role contract for local and CI admission, without a DB.
use crate::{
    environment::Plan,
    environment_host::{Registry, Result},
    environment_probe,
};

pub async fn run(arguments: &[String]) -> Result<()> {
    let (plan, stage) = input(arguments)?;
    let registry = Registry::load()?;
    registry.resolve(&plan)?;
    if stage == "ci" && !plan.ci {
        output(
            &serde_json::json!({"stage":"ci","status":"not_applicable","plan_digest":plan.digest()}),
        )?;
        return Ok(());
    }
    checked_report(&registry, &plan, stage).await
}

fn input(arguments: &[String]) -> Result<(Plan, &str)> {
    let [path, stage] = arguments else {
        return Err("usage: --environment-check PLAN.json STAGE".into());
    };
    if !matches!(
        stage.as_str(),
        "enable"
            | "startup"
            | "preparation"
            | "launch"
            | "validation"
            | "delivery"
            | "ci"
            | "recovery"
    ) {
        return Err("unknown environment stage".into());
    }
    let plan: Plan = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok((plan, stage))
}

async fn checked_report(registry: &Registry, plan: &Plan, stage: &str) -> Result<()> {
    let role = crate::environment_service::role(stage);
    let report =
        environment_probe::check(registry, plan, stage, role, Some(&std::env::current_dir()?))
            .await?;
    output(&report)?;
    if !report.passed() {
        return Err("environment admission blocked; see report".into());
    }
    Ok(())
}

fn output(value: &impl serde::Serialize) -> Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    out.write_all(b"\n")?;
    Ok(())
}
