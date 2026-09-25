//! Deployment-owned GitHub credential provider. Only the GitHub adapter loads
//! key bytes; generic delivery requests and candidate processes carry references.
use crate::{
    github::{Policy, Source},
    github_http::AppClient,
};
use std::path::Path;
type Result<T> = crate::delivery_extension::Result<T>;

pub trait Provider {
    fn client(&self) -> Result<AppClient>;
}
pub struct FileProvider<'a> {
    pub api_url: &'a str,
    pub app_id: u64,
    pub private_key: &'a Path,
}
impl Provider for FileProvider<'_> {
    fn client(&self) -> Result<AppClient> {
        if !self.private_key.is_absolute()
            || !std::fs::symlink_metadata(self.private_key)?
                .file_type()
                .is_file()
        {
            return Err("deployment App key must be an absolute regular file".into());
        }
        let key = std::fs::read(self.private_key)?;
        Ok(AppClient::new(self.api_url, self.app_id, &key)?)
    }
}

pub fn separate_check_identity(policy: &Policy, delivery_app: u64) -> bool {
    let mut sources = Vec::new();
    for check in &policy.required {
        sources.push(&check.source);
    }
    if let Some(contract) = &policy.delivery {
        for check in contract.all_checks() {
            sources.push(&check.selector.source);
        }
    }
    for source in sources {
        if matches!(source, Source::CheckRun { app_id } if *app_id == delivery_app) {
            return false;
        }
    }
    true
}
