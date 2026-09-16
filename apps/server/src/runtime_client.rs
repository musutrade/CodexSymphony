//! One supervised app-server per Run; no process/thread reuse after restart.
use crate::{
    budget::{Amount, Purpose, Usage, Waiting},
    budget_store::{self, Admission, CallIntent},
    execution::Launch,
    git_broker::GitBroker,
    process, run_store, runtime, runtime_protocol as wire, runtime_questions, runtime_store,
    runtime_tools,
    runtime_transport::{Record, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub startup_seconds: u64,
    pub response_seconds: u64,
    pub stall_seconds: u64,
    pub reservation: Amount,
    /// Operator-owned configuration, never supplied by an Agent tool. Each Run
    /// gets fresh writable state; authentication is provisioned by deployment.
    pub codex_config: String,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        if [
            self.startup_seconds,
            self.response_seconds,
            self.stall_seconds,
        ]
        .into_iter()
        .any(|n| n == 0 || n > 28800)
            || !self.reservation.positive()
            || self.reservation.turns != 1
            || self.codex_config.len() > 65536
        {
            return Err("invalid Runtime deployment settings".into());
        }
        Ok(())
    }
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

struct Client<'a> {
    pool: &'a PgPool,
    launch: &'a Launch,
    broker: &'a GitBroker,
    transport: Transport,
    settings: &'a Settings,
    directory: PathBuf,
    thread: String,
    turn: String,
    call: String,
    started: i64,
    last_progress: i64,
    waiting_start: Option<i64>,
    human_seconds: i64,
    baseline: Usage,
    cumulative: Usage,
    usage: Usage,
}

pub async fn execute(
    pool: &PgPool,
    root: &Path,
    supervisor: &Path,
    broker: &GitBroker,
    launch: &Launch,
    settings: &Settings,
) -> Result<()> {
    settings.validate()?;
    runtime_store::open(pool, &launch.key, now()).await?;
    let mut child =
        crate::coordinator::start_runtime(pool, root, supervisor, launch, &settings.codex_config)
            .await?;
    let directory = process::run_directory(root, &launch.key.run_id)?;
    let transport = Transport::new(&mut child);
    // Reap the supervisor regardless of protocol outcome. Only its ECHILD
    // receipt (read by Coordinator) establishes quiescence.
    tokio::task::spawn_blocking(move || child.wait());
    let result = match transport {
        Ok(transport) => {
            let mut client = Client {
                pool,
                launch,
                broker,
                transport,
                settings,
                directory: directory.clone(),
                thread: String::new(),
                turn: String::new(),
                call: String::new(),
                started: now(),
                last_progress: now(),
                waiting_start: None,
                human_seconds: 0,
                baseline: Usage::default(),
                cumulative: Usage::default(),
                usage: Usage::default(),
            };
            let result = client.run().await;
            let _ = client.interrupt().await;
            result
        }
        Err(error) => Err(error.into()),
    };
    // RPC receipt, turn completion and app-server exit are not group receipts.
    let _ = runtime_store::stop(
        pool,
        &launch.key,
        "Runtime exited; stop and preserve execution group",
    )
    .await;
    process::durable_write(&directory.join("stop.json"), &launch.key)?;
    result
}
impl Client<'_> {
    async fn run(&mut self) -> Result<()> {
        self.initialize().await?;
        self.start_turn(runtime_store::input(self.pool, &self.launch.key).await?)
            .await?;
        loop {
            if !self.guard().await? {
                return Ok(());
            }
            self.step().await?;
        }
    }
    async fn step(&mut self) -> Result<()> {
        self.answers().await?;
        if let Some(message) = self.transport.pending() {
            self.event(message).await?;
            return Ok(());
        }
        if let Ok(message) = tokio::time::timeout(Duration::from_millis(100), self.next()).await {
            self.event(message?).await?;
        }
        Ok(())
    }
    async fn initialize(&mut self) -> Result<()> {
        let params = wire::InitializeParams {
            client_info: json!({"name":"codexsymphony","version":"0.1.0"}),
            capabilities: Some(json!({"experimentalApi":true})),
        };
        let initialized = self
            .rpc("initialize", &params, self.settings.startup_seconds)
            .await?;
        runtime_store::require(
            initialized["userAgent"]
                .as_str()
                .is_some_and(|s| s.contains("/0.154.0 ")),
            "app-server version mismatch",
        )?;
        self.transport
            .send(&json!({"method":"initialized"}))
            .await?;
        self.start_thread().await
    }
    async fn start_thread(&mut self) -> Result<()> {
        let model: String = sqlx::query_scalar("SELECT model FROM agent_run WHERE id=$1")
            .bind(&self.launch.key.run_id)
            .fetch_one(self.pool)
            .await?;
        let params = wire::ThreadStartParams {
            cwd: Some(self.launch.workspace.clone()),
            model: Some(model),
            approval_policy: Some(json!("never")),
            sandbox: Some(json!("danger-full-access")),
            ephemeral: Some(true),
            dynamic_tools: Some(runtime::tools()),
            allow_provider_model_fallback: Some(false),
            ..Default::default()
        };
        let started = self
            .rpc("thread/start", &params, self.settings.startup_seconds)
            .await?;
        runtime_store::require(
            started["cwd"] == self.launch.workspace
                && started["thread"]["cwd"] == self.launch.workspace,
            "thread cwd differs from worktree",
        )?;
        self.thread = started["thread"]["id"]
            .as_str()
            .ok_or("missing thread id")?
            .to_owned();
        runtime_store::thread(self.pool, &self.launch.key, &self.thread, now()).await?;
        Ok(())
    }
    async fn start_turn(&mut self, input: String) -> Result<()> {
        let (call, id) = self.dispatch_turn(input).await?;
        let result = self.response(&id, self.settings.response_seconds).await?;
        self.turn = result["turn"]["id"]
            .as_str()
            .ok_or("missing turn id")?
            .to_owned();
        self.call = call;
        self.started = now();
        self.last_progress = now();
        self.human_seconds = 0;
        self.baseline = self.cumulative.clone();
        self.usage = Usage::default();
        runtime_store::turn(self.pool, &self.launch.key, &self.turn, &self.call, now()).await?;
        Ok(())
    }
    async fn dispatch_turn(&mut self, input: String) -> Result<(String, Value)> {
        let call = process::new_identity()?;
        let intent = CallIntent {
            key: self.launch.key.clone(),
            turn_id: call.clone(),
            purpose: Purpose::Coding,
            reserve: self.settings.reservation,
        };
        runtime_store::require(
            budget_store::reserve(self.pool, &intent).await? == Admission::Reserved,
            "turn budget not admitted",
        )?;
        let mut tx = run_store::lock(self.pool).await?;
        runtime_store::require(
            runtime_store::allowed(&mut tx, &self.launch.key).await?,
            "authorization changed before dispatch",
        )?;
        let params = wire::TurnStartParams {
            thread_id: self.thread.clone(),
            input: Vec::from([json!({"type":"text","text":input,"text_elements":[]})]),
            ..Default::default()
        };
        let id = self.transport.request("turn/start", &params).await?;
        tx.commit().await?;
        Ok((call, id))
    }
    async fn rpc(&mut self, method: &str, params: &impl Serialize, seconds: u64) -> Result<Value> {
        let id = self.transport.request(method, params).await?;
        self.response(&id, seconds).await
    }
    async fn response(&mut self, id: &Value, seconds: u64) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        loop {
            let message = tokio::time::timeout_at(deadline, self.next()).await??;
            if message.get("method").is_none() && message.get("id") == Some(id) {
                if message.get("error").is_some() {
                    return Err("app-server RPC rejected".into());
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or("missing RPC result".into());
            }
            self.transport.defer(message)?;
        }
    }
    async fn next(&mut self) -> Result<Value> {
        loop {
            match self.transport.receive().await? {
                Record::Protocol(bytes) => {
                    runtime_store::evidence(self.pool, &self.launch.key, "stdout", &bytes).await?;
                    return Ok(serde_json::from_slice(&bytes)?);
                }
                Record::Diagnostic(bytes) => {
                    runtime_store::evidence(self.pool, &self.launch.key, "stderr", &bytes).await?
                }
                Record::Closed => return Err("app-server protocol closed".into()),
            }
        }
    }
    async fn guard(&mut self) -> Result<bool> {
        runtime_questions::expire(self.pool, now()).await?;
        let mut tx = run_store::lock(self.pool).await?;
        let allowed = runtime_store::allowed(&mut tx, &self.launch.key).await?;
        tx.commit().await?;
        if !allowed {
            return Ok(false);
        };
        self.check_home()?;
        if self.waiting_start.is_none()
            && now() - self.last_progress >= self.settings.stall_seconds as i64
        {
            return Err("Runtime stalled".into());
        }
        Ok(true)
    }
    fn check_home(&self) -> Result<()> {
        // App-server's own SQLite/log files are also bounded; ephemeral threads
        // avoid rollout attachments. Exceeding limits stops the entire group.
        let mut pending = Vec::from([self.directory.join("codex-home")]);
        let mut bytes = 0u64;
        let mut count = 0;
        while let Some(path) = pending.pop() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                count += 1;
                bytes = bytes.saturating_add(meta.len());
                state_entry(entry, &mut pending, bytes, count)?;
            }
        }
        Ok(())
    }
    async fn event(&mut self, message: Value) -> Result<()> {
        let method = message["method"].as_str().unwrap_or("");
        if message.get("id").is_some() && !method.is_empty() {
            return self.server_request(&message).await;
        }
        self.notification(method, &message["params"]).await
    }
    async fn notification(&mut self, method: &str, params: &Value) -> Result<()> {
        if params["threadId"] != self.thread {
            return Ok(());
        }
        match method {
            "thread/tokenUsage/updated" => self.tokens(params).await?,
            "turn/completed" => self.completed(params).await?,
            "item/started" | "item/completed" | "item/agentMessage/delta"
                if params["turnId"] == self.turn =>
            {
                self.last_progress = now();
            }
            _ => {}
        }
        Ok(())
    }
    async fn server_request(&mut self, message: &Value) -> Result<()> {
        let result = match message["method"].as_str().unwrap_or("") {
            "item/tool/call" => {
                runtime_tools::handle(self.pool, self.broker, &self.launch.key, message).await?
            }
            "item/tool/requestUserInput" => {
                runtime_questions::ask(self.pool, &self.launch.key, message, now()).await?;
                self.waiting_start.get_or_insert(now());
                return Ok(());
            }
            method => runtime_tools::deny(method),
        };
        let response = if result.get("error").is_some() {
            json!({"id":message["id"],"error":result["error"]})
        } else {
            json!({"id":message["id"],"result":result})
        };
        self.transport.send(&response).await?;
        self.last_progress = now();
        Ok(())
    }
    async fn answers(&mut self) -> Result<()> {
        for question in runtime_questions::live_answers(self.pool, &self.launch.key, now()).await? {
            deliver_answer(self.pool, &self.launch.key, &mut self.transport, &question).await?;
        }
        self.waiting().await
    }
    async fn waiting(&mut self) -> Result<()> {
        let waiting: Option<i64> =
            sqlx::query_scalar("SELECT waiting_since FROM runtime_session WHERE run_id=$1")
                .bind(&self.launch.key.run_id)
                .fetch_one(self.pool)
                .await?;
        if waiting.is_none()
            && let Some(start) = self.waiting_start.take()
        {
            self.human_seconds += (now() - start).max(0);
            self.last_progress = now();
            budget_store::record_waiting(
                self.pool,
                &self.launch.key,
                &Waiting {
                    human_seconds: self.human_seconds,
                    ..Default::default()
                },
            )
            .await?;
        }
        Ok(())
    }
    async fn tokens(&mut self, params: &Value) -> Result<()> {
        let event: wire::ThreadTokenUsageUpdatedNotification =
            serde_json::from_value(params.clone())?;
        if event.turn_id != self.turn {
            return Ok(());
        }
        let total = runtime::token_usage(&json!({"last":event.token_usage["total"]}))?;
        self.cumulative = self.cumulative.merge(&total);
        self.usage.input = difference(self.cumulative.input, self.baseline.input)?;
        self.usage.cached = difference(self.cumulative.cached, self.baseline.cached)?;
        self.usage.output = difference(self.cumulative.output, self.baseline.output)?;
        self.settle(false).await?;
        self.last_progress = now();
        Ok(())
    }
    async fn settle(&mut self, complete: bool) -> Result<()> {
        let waiting = self.waiting_start.map_or(0, |start| (now() - start).max(0));
        self.usage.model_seconds =
            Some((now() - self.started - self.human_seconds - waiting).max(0));
        self.usage.complete |= complete;
        // Same cumulative values share an event identity, bounding duplicate use.
        let identity = serde_json::to_string(&self.usage)?;
        budget_store::settle(
            self.pool,
            &self.launch.key,
            &self.call,
            &identity,
            &self.usage,
        )
        .await?;
        Ok(())
    }
    async fn completed(&mut self, params: &Value) -> Result<()> {
        let event: wire::TurnCompletedNotification = serde_json::from_value(params.clone())?;
        if event.turn["id"] != self.turn {
            return Ok(());
        }
        self.settle(true).await?;
        if event.turn["status"] != "completed" {
            return Err("turn interrupted or failed".into());
        }
        if !runtime_store::can_continue(self.pool, &self.launch.key, now()).await? {
            return Err("continuation blocked; preserve work and questions".into());
        }
        self.start_turn("Continue the reviewed task; declare completion or a blocker using the registered tools.".into()).await
    }
    async fn interrupt(&mut self) -> Result<()> {
        if self.thread.is_empty() || self.turn.is_empty() {
            return Ok(());
        }
        self.transport
            .request(
                "turn/interrupt",
                &wire::TurnInterruptParams {
                    thread_id: self.thread.clone(),
                    turn_id: self.turn.clone(),
                },
            )
            .await?;
        // Best effort protocol signal followed by mandatory process-group stop.
        self.settle(false).await?;
        Ok(())
    }
}
fn difference(total: Option<i64>, baseline: Option<i64>) -> Result<Option<i64>> {
    total
        .map(|total| {
            total
                .checked_sub(baseline.unwrap_or(0))
                .filter(|n| *n >= 0)
                .ok_or("token total regressed".into())
        })
        .transpose()
}

fn state_entry(
    entry: std::fs::DirEntry,
    pending: &mut Vec<PathBuf>,
    bytes: u64,
    count: usize,
) -> Result<()> {
    if count > 128 || bytes > 32 * 1024 * 1024 {
        return Err("app-server state quota exceeded".into());
    }
    if entry.file_type()?.is_dir() {
        pending.push(entry.path());
    }
    Ok(())
}

/// Serialize the actual reply write against pause and revocation.
pub async fn deliver_answer(
    pool: &PgPool,
    key: &crate::execution::RunKey,
    transport: &mut Transport,
    question: &runtime_questions::Question,
) -> Result<()> {
    // Serialize the actual write against pause/revocation as well as CAS.
    let mut tx = run_store::lock(pool).await?;
    runtime_store::require(
        runtime_store::allowed(&mut tx, key).await?,
        "answer delivery authorization changed",
    )?;
    transport
        .send(&json!({"id":question.rpc_id,"result":question.answer}))
        .await?;
    tx.commit().await?;
    runtime_questions::delivered(pool, &question.id).await?;
    Ok(())
}
