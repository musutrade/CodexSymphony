//! Requirement-wide accounting decisions, independent of persistence and runtime.
use serde::{Deserialize, Serialize};

pub const RUN_LIFETIME_SECONDS: i64 = 8 * 60 * 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Amount {
    pub tokens: i64,
    pub turns: i64,
    pub model_seconds: i64,
}
impl Amount {
    pub fn positive(self) -> bool {
        self.tokens > 0 && self.turns > 0 && self.model_seconds > 0
    }
    pub fn nonnegative(self) -> bool {
        self.tokens >= 0 && self.turns >= 0 && self.model_seconds >= 0
    }
    pub fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            tokens: self.tokens.checked_add(other.tokens)?,
            turns: self.turns.checked_add(other.turns)?,
            model_seconds: self.model_seconds.checked_add(other.model_seconds)?,
        })
    }
    pub fn fits(self, limit: Self) -> bool {
        self.tokens <= limit.tokens
            && self.turns <= limit.turns
            && self.model_seconds <= limit.model_seconds
    }
    pub fn reached(self, limit: Self) -> bool {
        self.tokens >= limit.tokens
            || self.turns >= limit.turns
            || self.model_seconds >= limit.model_seconds
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Coding,
    Repair,
    Summary,
}

/// The adapter supplies cumulative counters for ONE turn, never per-message deltas.
/// Cached input is a subset of input, not an additional token charge. Missing
/// counters stay unknown. Finality can arrive before the final counters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input: Option<i64>,
    pub cached: Option<i64>,
    pub output: Option<i64>,
    pub model_seconds: Option<i64>,
    pub complete: bool,
}
impl Usage {
    pub fn valid(&self) -> bool {
        [self.input, self.cached, self.output, self.model_seconds]
            .into_iter()
            .flatten()
            .all(|n| n >= 0)
            && self
                .cached
                .zip(self.input)
                .is_none_or(|(cached, input)| cached <= input)
    }
    pub fn merge(&self, incoming: &Self) -> Self {
        Self {
            input: self.input.max(incoming.input),
            cached: self.cached.max(incoming.cached),
            output: self.output.max(incoming.output),
            model_seconds: self.model_seconds.max(incoming.model_seconds),
            complete: self.complete || incoming.complete,
        }
    }
    pub fn settled(&self) -> bool {
        self.complete
            && self.input.is_some()
            && self.output.is_some()
            && self.model_seconds.is_some()
    }
    pub fn actual(&self) -> Option<Amount> {
        let observed = self.complete
            || [self.input, self.cached, self.output, self.model_seconds]
                .iter()
                .any(Option::is_some);
        Some(Amount {
            tokens: self
                .input
                .or(self.cached)
                .unwrap_or(0)
                .checked_add(self.output.unwrap_or(0))?,
            turns: i64::from(observed),
            model_seconds: self.model_seconds.unwrap_or(0),
        })
    }
    /// Until reconciliation finishes, retain at least the reservation in every
    /// dimension; partial observations can increase exposure but never release it.
    pub fn exposure(&self, reserved: Amount) -> Option<Amount> {
        let actual = self.actual()?;
        if self.settled() {
            return Some(actual);
        }
        Some(Amount {
            tokens: actual.tokens.max(reserved.tokens),
            turns: 1,
            model_seconds: actual.model_seconds.max(reserved.model_seconds),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Waiting {
    pub human_seconds: i64,
    pub paused_seconds: i64,
    pub ci_seconds: i64,
    pub network_seconds: i64,
}
impl Waiting {
    pub fn valid(&self) -> bool {
        [
            self.human_seconds,
            self.paused_seconds,
            self.ci_seconds,
            self.network_seconds,
        ]
        .into_iter()
        .all(|n| n >= 0)
    }
    pub fn merge(&self, next: &Self) -> Self {
        Self {
            human_seconds: self.human_seconds.max(next.human_seconds),
            paused_seconds: self.paused_seconds.max(next.paused_seconds),
            ci_seconds: self.ci_seconds.max(next.ci_seconds),
            network_seconds: self.network_seconds.max(next.network_seconds),
        }
    }
}
