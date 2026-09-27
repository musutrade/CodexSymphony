//! Resolve choices at review, independently of the mutable execution defaults.
use crate::{
    contract::Repository,
    draft::Document,
    group_review::{RepositorySnapshot, Review},
    model_selection::{self, Frozen, Registration, Selection},
    runtime_routes::Deployment,
};
use std::path::Path;
type Result<T> = std::result::Result<T, &'static str>;

fn deployed(repository: i64, version: i64, input: &Repository) -> Result<Registration> {
    registration(&deployment()?, repository, version, input)
}

fn deployment_error(_: impl std::fmt::Display) -> &'static str {
    "Runtime deployment configuration unavailable"
}

pub fn registration(
    deployment: &Deployment,
    repository: i64,
    version: i64,
    input: &Repository,
) -> Result<Registration> {
    let config = match deployment {
        Deployment::Legacy(config) if repository == 1 => config.as_ref(),
        Deployment::Multiple(routes) => {
            let route = routes
                .get(&repository)
                .ok_or("Runtime repository route unavailable")?;
            if route.github_repository_id.unwrap_or(0) != input.github_repository_id
                || route.remote != input.remote
                || route.base_branch != input.base_branch
                || route.version != version
            {
                return Err("Runtime repository route differs from reviewed scope");
            }
            &route.runtime
        }
        _ => return Err("Runtime repository route unavailable"),
    };
    config
        .settings
        .model_capabilities
        .clone()
        .ok_or("model capability registration unavailable")
}

pub fn freeze(
    selected: Option<&Selection>,
    repository: i64,
    version: i64,
    input: &Repository,
) -> Result<Option<Frozen>> {
    if selected.is_none() && input.model_selection.is_none() {
        return Ok(None);
    }
    let registration = deployed(repository, version, input)?;
    model_selection::freeze(
        selected,
        input.model_selection.as_ref(),
        repository,
        version,
        Some(&registration),
    )
}

pub fn group(
    document: &Document,
    review: &mut Review,
    repositories: &[RepositorySnapshot],
) -> Result<()> {
    let defaults = has_defaults(repositories);
    for item in &mut review.items {
        if item.model_selection.is_none() && !defaults {
            item.frozen_model = None;
            continue;
        }
        let repository = child_repository(document, &item.child_id, repositories)?;
        item.frozen_model = freeze(
            item.model_selection.as_ref(),
            repository.id,
            repository.version,
            &repository.repository,
        )?;
    }
    Ok(())
}

fn child_repository<'a>(
    document: &Document,
    child: &str,
    repositories: &'a [RepositorySnapshot],
) -> Result<&'a RepositorySnapshot> {
    for item in &document.children {
        if item.id == child {
            for repository in repositories {
                if Some(repository.id) == item.repository_id {
                    return Ok(repository);
                }
            }
        }
    }
    Err("model selection references an unknown child or repository")
}

pub fn verify_group(
    document: &Document,
    review: &Review,
    repositories: &[RepositorySnapshot],
) -> Result<()> {
    let mut expected = review.clone();
    group(document, &mut expected, repositories)?;
    for (actual, expected) in review.items.iter().zip(&expected.items) {
        if actual.frozen_model != expected.frozen_model {
            return Err("model choice changed after review; save and review again");
        }
    }
    Ok(())
}

fn has_defaults(repositories: &[RepositorySnapshot]) -> bool {
    for repository in repositories {
        if repository.repository.model_selection.is_some() {
            return true;
        }
    }
    false
}

fn deployment() -> Result<Deployment> {
    let path = std::env::var_os("RUNTIME_CONFIG").ok_or("Runtime deployment unavailable")?;
    Deployment::load(Path::new(&path)).map_err(deployment_error)
}

pub fn runtime_registration(repository: i64, input: &Repository) -> Result<Registration> {
    let deployment = deployment()?;
    let version = match &deployment {
        Deployment::Legacy(_) => 1,
        Deployment::Multiple(routes) => {
            routes
                .get(&repository)
                .ok_or("Runtime repository route unavailable")?
                .version
        }
    };
    registration(&deployment, repository, version, input)
}
