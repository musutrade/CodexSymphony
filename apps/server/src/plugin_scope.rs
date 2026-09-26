//! Deployment-owned repository admission; this never selects an implementation.
use sqlx::PgPool;

pub async fn admit(
    pool: &PgPool,
    plugin: &str,
    invocation: &str,
    requirement: i64,
    revision: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT plugin_scope_admit($1,$2,$3,$4,plugin_scope_repository($3,$4))")
        .bind(plugin)
        .bind(invocation)
        .bind(requirement)
        .bind(revision)
        .execute(pool)
        .await?;
    Ok(())
}

/// Existing reviewed scope references retain their wire format and hashes.
/// A list must be canonical, positive and unique; no path or workspace matching.
pub fn contains(scope: &str, repository: i64) -> bool {
    if repository <= 0 {
        return false;
    }
    if scope == "all" {
        return true;
    }
    if let Some(id) = scope.strip_prefix("repository:") {
        return id == repository.to_string();
    }
    let Some(list) = scope.strip_prefix("repositories:") else {
        return false;
    };
    let mut ids = std::collections::BTreeSet::new();
    for value in list.split(',') {
        let Ok(id) = value.parse::<i64>() else {
            return false;
        };
        if id <= 0 || value != id.to_string() || !ids.insert(id) {
            return false;
        }
    }
    ids.contains(&repository)
}

pub fn repository_revision(value: &str) -> Option<(i64, i64)> {
    let (repository, revision) = value.strip_prefix("repository:")?.split_once('@')?;
    let repository = repository.parse().ok()?;
    let revision = revision.parse().ok()?;
    if repository <= 0 || revision <= 0 {
        return None;
    }
    Some((repository, revision))
}
