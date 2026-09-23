//! Operator-owned per-repository Runtime configuration; one worker and owner.
use crate::runtime_service::Config;
use serde::Deserialize;
use sqlx::PgPool;
use std::{collections::BTreeMap, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Routes {
    repositories: BTreeMap<i64, Route>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub github_repository_id: i64,
    pub remote: String,
    pub base_branch: String,
    pub version: i64,
    pub runtime: Config,
}
pub enum Deployment {
    Legacy(Box<Config>),
    Multiple(BTreeMap<i64, Route>),
}
impl Deployment {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value.get("repositories").is_none() {
            let config: Config = serde_json::from_value(value)?;
            config.launcher()?;
            return Ok(Self::Legacy(Box::new(config)));
        }
        Self::multiple(serde_json::from_value(value)?)
    }
    fn multiple(routes: Routes) -> Result<Self> {
        if routes.repositories.is_empty() {
            return Err("at least one Runtime repository route required".into());
        }
        for (id, route) in &routes.repositories {
            route.validate(*id)?;
        }
        Ok(Self::Multiple(routes.repositories))
    }
    pub async fn selected(&self, pool: &PgPool) -> Result<Option<((i64, i64), &Config)>> {
        crate::group_queue_store::materialize(pool).await?;
        // An occupied requirement always wins, including pause/CI/manual blocks.
        // Never scan past the queue head to find a configured repository.
        let row: Option<(i64, i64, i64)> = sqlx::query_as("SELECT r.id,r.revision,COALESCE((v.document->>'repository_id')::bigint,r.repository_id::bigint) FROM requirement r JOIN execution_revision v ON v.requirement_id=r.id AND v.revision=r.revision WHERE r.id=COALESCE((SELECT requirement_id FROM execution_control WHERE id=1),(SELECT requirement_id FROM execution_queue ORDER BY queued_at,queue_key,item_order LIMIT 1))")
            .fetch_optional(pool).await?;
        let Some((requirement, revision, repository)) = row else {
            return Ok(None);
        };
        match self {
            Self::Legacy(config) => {
                Ok((repository == 1).then_some(((requirement, revision), config.as_ref())))
            }
            Self::Multiple(routes) => {
                let Some(route) = routes.get(&repository) else {
                    return Ok(None);
                };
                if route.matches(pool, requirement, repository).await? {
                    Ok(Some(((requirement, revision), &route.runtime)))
                } else {
                    Ok(None)
                }
            }
        }
    }
}
impl Route {
    fn validate(&self, id: i64) -> Result<()> {
        if id <= 0 || self.github_repository_id <= 0 || self.version <= 0 {
            return Err("invalid Runtime repository identity".into());
        }
        self.runtime.launcher()?;
        Ok(())
    }

    async fn matches(&self, pool: &PgPool, requirement: i64, repository: i64) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM requirement r JOIN execution_revision v ON v.requirement_id=r.id AND v.revision=r.revision JOIN repository p ON p.id=(v.document->>'repository_id')::bigint WHERE r.id=$1 AND p.id=$2 AND p.version=$3 AND (v.document->>'repository_version')::bigint=$3 AND (p.document->>'github_repository_id')::bigint=$4 AND p.document->>'remote'=$5 AND p.document->>'base_branch'=$6 AND v.document->'repository'=p.document)")
            .bind(requirement).bind(repository).bind(self.version).bind(self.github_repository_id).bind(&self.remote).bind(&self.base_branch).fetch_one(pool).await?)
    }
}
