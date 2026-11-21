use anyhow::Result;
use futures_util::future::BoxFuture;
use serde::Deserialize;

use crate::domain::delegation::{BackendStatus, ChatSession, Delivery, Phase, Recovery};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Observation {
    pub delivery: Delivery,
    pub phase: Phase,
    pub reason: Option<String>,
    pub conversation_url: Option<String>,
    #[serde(default)]
    pub recovery_exhausted: bool,
    pub recovery: Option<Recovery>,
    pub resumes: Option<u32>,
    pub retry_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackendHealth {
    #[serde(flatten)]
    pub status: BackendStatus,
    pub retry_at: Option<i64>,
}

pub(crate) struct Dispatch<'a> {
    pub attempt_id: &'a str,
    pub job_id: &'a str,
    pub instruction: &'a str,
    pub max_resumes: Option<u32>,
    pub chat_session: Option<(&'a ChatSession, &'a str)>,
    pub callback_token: &'a str,
    pub callback_workspace: &'a str,
    pub model: Option<&'a str>,
    pub profile: Option<TextDispatchProfile>,
    pub timeout: std::time::Duration,
}

#[derive(Clone, Copy)]
pub(crate) struct TextDispatchProfile {
    pub developer_instructions: &'static str,
    pub accepts_reply: fn(&str) -> bool,
}

pub(crate) struct BackendCapabilities {
    /// Enforced tool and network isolation for untrusted inputs.
    pub defensive_isolation: bool,
}

pub(crate) trait AgentBackendPort: Send + Sync {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            defensive_isolation: false,
        }
    }
    fn health(&self) -> BoxFuture<'_, BackendHealth>;
    fn dispatch<'a>(&'a self, request: Dispatch<'a>) -> BoxFuture<'a, Result<Observation>>;
    fn observe<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<Observation>>;
    fn collect<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<String>>;
    fn stop<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<()>>;
    fn forget(&self, _attempt_id: &str) {}
}
