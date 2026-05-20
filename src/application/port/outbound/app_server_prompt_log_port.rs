use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppServerPromptInputRecord {
    pub kind: String,
    pub label: String,
    pub content: String,
}

impl AppServerPromptInputRecord {
    pub fn new(
        kind: impl Into<String>,
        label: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            label: label.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppServerPromptOutputRecord {
    pub item_id: String,
    pub phase: Option<String>,
    pub text: String,
}

impl AppServerPromptOutputRecord {
    pub fn new(item_id: impl Into<String>, phase: Option<String>, text: impl Into<String>) -> Self {
        Self {
            item_id: item_id.into(),
            phase,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppServerPromptInteractionRecord {
    #[serde(default)]
    pub sequence: i64,
    pub interaction_id: String,
    pub session_kind: String,
    pub operation: String,
    pub status: String,
    pub workspace_dir: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub service_name: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub developer_instructions: Option<String>,
    pub input_items: Vec<AppServerPromptInputRecord>,
    pub output_items: Vec<AppServerPromptOutputRecord>,
    pub error_message: Option<String>,
    pub started_at: String,
    pub completed_at: String,
}

impl AppServerPromptInteractionRecord {
    pub fn input_chars(&self) -> usize {
        self.input_items
            .iter()
            .map(|item| item.content.chars().count())
            .sum()
    }

    pub fn output_chars(&self) -> usize {
        self.output_items
            .iter()
            .map(|item| item.text.chars().count())
            .sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppServerPromptInteractionSnapshot {
    pub records: Vec<AppServerPromptInteractionRecord>,
}

impl AppServerPromptInteractionSnapshot {
    pub fn empty() -> Self {
        Self {
            records: Vec::new(),
        }
    }
}

pub trait AppServerPromptLogPort: Send + Sync {
    fn append_app_server_prompt_interaction(
        &self,
        workspace_dir: &str,
        record: AppServerPromptInteractionRecord,
    ) -> Result<()>;

    fn load_recent_app_server_prompt_interactions(
        &self,
        workspace_dir: &str,
        limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot>;
}

#[derive(Debug, Default)]
pub struct NoopAppServerPromptLogPort;

impl AppServerPromptLogPort for NoopAppServerPromptLogPort {
    fn append_app_server_prompt_interaction(
        &self,
        _workspace_dir: &str,
        _record: AppServerPromptInteractionRecord,
    ) -> Result<()> {
        Ok(())
    }

    fn load_recent_app_server_prompt_interactions(
        &self,
        _workspace_dir: &str,
        _limit: usize,
    ) -> Result<AppServerPromptInteractionSnapshot> {
        Ok(AppServerPromptInteractionSnapshot::empty())
    }
}
