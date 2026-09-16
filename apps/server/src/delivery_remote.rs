//! GitHub App adapter. Exact repo+branch reconciliation includes closed PRs.
use crate::{
    delivery_store::Pending,
    delivery_worker::Remote,
    git_broker::GitBroker,
    github::Policy,
    github_http::{AppClient, Error, invalid},
    workspace::Manifest,
};
use serde_json::{Value, json};
pub struct Github<'a> {
    pub client: &'a mut AppClient,
    pub policy: Policy,
    pub broker: &'a GitBroker,
    pub now: i64,
}
impl Github<'_> {
    fn bound(&self, job: &Pending) -> Result<(), Error> {
        if self.policy.repository_id != job.repository_id as u64
            || self.policy.repository != job.repository
            || self.policy.default_branch != job.base_branch
        {
            return Err(invalid());
        }
        Ok(())
    }
    fn candidate(&self, job: &Pending) -> Result<std::path::PathBuf, Error> {
        let manifest: Manifest =
            serde_json::from_value(job.manifest.clone()).map_err(|_| invalid())?;
        if manifest.head != job.head_sha
            || manifest.workspace.branch != job.branch
            || manifest.workspace.requirement != job.requirement_id
            || manifest.workspace.revision != job.revision
        {
            return Err(invalid());
        }
        self.broker
            .delivery_repository(&manifest)
            .map_err(|_| invalid())
    }
    fn query(&self, job: &Pending) -> Result<String, Error> {
        let mut path = self.path("pulls");
        let owner = self
            .policy
            .repository
            .split('/')
            .next()
            .ok_or_else(invalid)?;
        let query = reqwest::Url::parse_with_params(
            "https://api.github.com/",
            [
                ("state", "all"),
                ("head", &format!("{owner}:{}", job.branch)),
            ],
        )
        .map_err(|_| invalid())?;
        path.push('?');
        path.push_str(query.query().ok_or_else(invalid)?);
        Ok(path)
    }
    fn path(&self, suffix: &str) -> String {
        format!("/repos/{}/{}", self.policy.repository, suffix)
    }
}
impl Remote for Github<'_> {
    async fn find(&mut self, job: &Pending) -> Result<Option<Value>, Error> {
        self.bound(job)?;
        let path = self.query(job)?;
        let prs = self
            .client
            .pages(&self.policy, &path, None, self.now)
            .await?;
        let Some(number) = unique_number(job, &prs)? else {
            return Ok(None);
        };
        self.client
            .get(
                &self.policy,
                &self.path(&format!("pulls/{number}")),
                self.now,
            )
            .await
            .map(Some)
    }
    async fn head(&mut self, job: &Pending) -> Result<Option<String>, Error> {
        self.bound(job)?;
        // First verify repository accessibility, so a ref 404 has a known context.
        let repo = self
            .client
            .get(
                &self.policy,
                &format!("/repos/{}", job.repository),
                self.now,
            )
            .await?;
        if repo["id"] != job.repository_id {
            return Err(invalid());
        }
        match self
            .client
            .get(
                &self.policy,
                &self.path(&format!("git/ref/heads/{}", job.branch)),
                self.now,
            )
            .await
        {
            Ok(value) => Ok(Some(
                value["object"]["sha"].as_str().ok_or_else(invalid)?.into(),
            )),
            Err(Error {
                status: Some(404), ..
            }) => Ok(None),
            Err(error) => Err(error),
        }
    }
    async fn push(&mut self, job: &Pending) -> Result<Value, Error> {
        self.bound(job)?;
        let repository = self.candidate(job)?;
        self.client
            .push(
                &self.policy,
                &repository,
                &job.head_sha,
                &job.branch,
                self.now,
            )
            .await
    }
    async fn create(&mut self, job: &Pending) -> Result<Value, Error> {
        self.bound(job)?;
        self.candidate(job)?;
        self.client.write(&self.policy,reqwest::Method::POST,&self.path("pulls"),json!({"title":format!("Requirement {} revision {}",job.requirement_id,job.revision),"head":job.branch,"base":job.base_branch,"body":job.identity().marker()}),self.now).await
    }
    async fn close(&mut self, job: &Pending, number: u64) -> Result<Value, Error> {
        self.bound(job)?;
        // Re-read immediately before close. Merge races preserve GitHub's fact.
        let path = self.path(&format!("pulls/{number}"));
        let pr = self.client.get(&self.policy, &path, self.now).await?;
        if crate::delivery::pr_fact(&job.identity(), &pr) != crate::delivery::PrFact::Open {
            return Ok(pr);
        }
        self.client
            .write(
                &self.policy,
                reqwest::Method::PATCH,
                &path,
                json!({"state":"closed"}),
                self.now,
            )
            .await
    }
}

fn unique_number(job: &Pending, prs: &[Value]) -> Result<Option<u64>, Error> {
    if prs.len() > 1 {
        return Err(invalid());
    }
    let number = match prs.first() {
        Some(pr) => pr["number"].as_u64().ok_or_else(invalid)?,
        None => return Ok(None),
    };
    if job.pr_number.is_some_and(|n| n as u64 != number) {
        return Err(invalid());
    }
    Ok(Some(number))
}
