//! Runtime decisions and validation, independent of SQL, processes and Git.
use crate::{budget::Usage, runtime_protocol::DynamicToolCallResponse};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_REQUEST: usize = 65536;
pub const MAX_FRAME: usize = 2 * 1024 * 1024;
pub const MAX_EVIDENCE: usize = 1024 * 1024;
pub const MAX_RECORDS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Commit {
    pub message: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub candidate_sha: String,
    pub summary: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Blocker {
    pub reason: String,
    pub requires_permission: bool,
}

pub fn text_valid(text: &str, limit: usize) -> bool {
    !text.trim().is_empty() && text.len() <= limit && !text.contains('\0')
}
pub fn rpc_id_valid(id: &Value) -> bool {
    id.as_i64().is_some() || id.as_str().is_some_and(|id| text_valid(id, 200))
}
pub fn reply(success: bool, text: &str) -> Value {
    json!(DynamicToolCallResponse {
        success,
        content_items: Vec::from([json!({"type":"inputText","text":text})]),
    })
}
pub fn tools() -> Vec<Value> {
    [
        ("create_local_commit", "Create a local candidate through the platform Git Broker.", json!({"message":{"type":"string","maxLength":4096}})),
        ("report_completion", "Declare completion for an existing candidate; the platform stops and preserves the execution group.", json!({"candidate_sha":{"type":"string"},"summary":{"type":"string","maxLength":8192}})),
        ("report_blocker", "Stop and preserve when progress needs intervention. Use request_user_input for answerable business questions.", json!({"reason":{"type":"string","maxLength":8192},"requires_permission":{"type":"boolean"}})),
    ].into_iter().map(|(name, description, properties)| {
        let required: Vec<_> = properties.as_object().unwrap().keys().cloned().collect();
        json!({"type":"function","name":name,"description":description,"inputSchema":{
            "type":"object","properties":properties,"required":required,"additionalProperties":false}})
    }).collect()
}

/// All deadlines use original persisted wall-clock times. A transport restart
/// does not create a new question or move a Run's absolute deadline.
pub fn expired(now: i64, created: i64, waiting: Option<i64>) -> bool {
    now.saturating_sub(created) >= crate::budget::RUN_LIFETIME_SECONDS
        || waiting.is_some_and(|at| now.saturating_sub(at) >= 2 * 60 * 60)
}

pub fn token_usage(value: &Value) -> Result<Usage, &'static str> {
    // `total` is cumulative for this thread, not this turn. `last` is the
    // current model response; the adapter accumulates distinct responses.
    let last = &value["last"];
    let usage = Usage {
        input: last["inputTokens"].as_i64(),
        cached: last["cachedInputTokens"].as_i64(),
        output: last["outputTokens"].as_i64(),
        model_seconds: None,
        complete: false,
    };
    if !usage.valid() {
        return Err("invalid token counters");
    }
    Ok(usage)
}

pub fn validate_answers(original: &Value, answer: &Value) -> bool {
    let Some(questions) = original["params"]["questions"].as_array() else {
        return false;
    };
    let Some(answers) = answer["answers"].as_object() else {
        return false;
    };
    questions.len() == answers.len()
        && questions.iter().all(|question| {
            let Some(id) = question["id"].as_str() else {
                return false;
            };
            answers
                .get(id)
                .and_then(|a| a["answers"].as_array())
                .is_some_and(|values| {
                    !values.is_empty()
                        && values.len() <= 16
                        && values
                            .iter()
                            .all(|v| v.as_str().is_some_and(|s| text_valid(s, 8192)))
                })
        })
}
