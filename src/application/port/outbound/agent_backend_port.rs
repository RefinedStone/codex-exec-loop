use std::future::Future;

use anyhow::Result;
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
    pub issue_draft: bool,
    pub timeout: std::time::Duration,
}

pub(crate) struct BackendCapabilities {
    /// Enforced tool and network isolation for untrusted inputs.
    pub defensive_isolation: bool,
}

pub(crate) trait AgentBackendPort: Sync {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            defensive_isolation: false,
        }
    }
    fn health(&self) -> impl Future<Output = BackendHealth> + Send;
    fn dispatch(&self, request: Dispatch<'_>) -> impl Future<Output = Result<Observation>> + Send;
    fn observe(&self, attempt_id: &str) -> impl Future<Output = Result<Observation>> + Send;
    fn collect(&self, attempt_id: &str) -> impl Future<Output = Result<String>> + Send;
    fn stop(&self, attempt_id: &str) -> impl Future<Output = Result<()>> + Send;
}
