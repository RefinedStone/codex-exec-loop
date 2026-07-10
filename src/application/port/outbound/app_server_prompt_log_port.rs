use anyhow::Result;
use serde::{Deserialize, Serialize};

pub(crate) const APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION: usize = 16;
pub(crate) const APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS: usize = 16_384;
pub(crate) const APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS: usize = 1_024;
const APP_SERVER_PROMPT_LOG_TRUNCATION_MARKER: &str =
    "\n[truncated by Akra prompt-log retention policy]";

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

    pub(crate) fn into_bounded(mut self) -> Self {
        truncate_prompt_log_string(&mut self.kind, APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS);
        truncate_prompt_log_string(&mut self.label, APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS);
        truncate_prompt_log_string(&mut self.content, APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS);
        self
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

    pub(crate) fn into_bounded(mut self) -> Self {
        truncate_prompt_log_string(&mut self.item_id, APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS);
        truncate_optional_prompt_log_string(
            &mut self.phase,
            APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS,
        );
        truncate_prompt_log_string(&mut self.text, APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS);
        self
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

    pub(crate) fn into_bounded(mut self) -> Self {
        truncate_prompt_log_string(
            &mut self.interaction_id,
            APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS,
        );
        truncate_prompt_log_string(
            &mut self.session_kind,
            APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS,
        );
        truncate_prompt_log_string(
            &mut self.operation,
            APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS,
        );
        truncate_prompt_log_string(&mut self.status, APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS);
        truncate_prompt_log_string(
            &mut self.workspace_dir,
            APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS,
        );
        for value in [
            &mut self.thread_id,
            &mut self.turn_id,
            &mut self.service_name,
            &mut self.model,
            &mut self.reasoning_effort,
            &mut self.error_message,
        ] {
            truncate_optional_prompt_log_string(value, APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS);
        }
        truncate_optional_prompt_log_string(
            &mut self.developer_instructions,
            APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS,
        );
        truncate_prompt_log_string(
            &mut self.started_at,
            APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS,
        );
        truncate_prompt_log_string(
            &mut self.completed_at,
            APP_SERVER_PROMPT_LOG_MAX_METADATA_CHARS,
        );
        self.input_items
            .truncate(APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION);
        self.output_items
            .truncate(APP_SERVER_PROMPT_LOG_MAX_ITEMS_PER_DIRECTION);
        self.input_items = self
            .input_items
            .into_iter()
            .map(AppServerPromptInputRecord::into_bounded)
            .collect();
        self.output_items = self
            .output_items
            .into_iter()
            .map(AppServerPromptOutputRecord::into_bounded)
            .collect();
        self
    }
}

pub(crate) fn bounded_prompt_log_string(value: &str, max_chars: usize) -> String {
    if value.chars().nth(max_chars).is_none() {
        return value.to_string();
    }

    let marker_chars = APP_SERVER_PROMPT_LOG_TRUNCATION_MARKER.chars().count();
    if marker_chars >= max_chars {
        return APP_SERVER_PROMPT_LOG_TRUNCATION_MARKER
            .chars()
            .take(max_chars)
            .collect();
    }
    let retained_chars = max_chars.saturating_sub(marker_chars);
    let mut bounded = value.chars().take(retained_chars).collect::<String>();
    bounded.push_str(APP_SERVER_PROMPT_LOG_TRUNCATION_MARKER);
    bounded
}

fn truncate_optional_prompt_log_string(value: &mut Option<String>, max_chars: usize) {
    if let Some(value) = value {
        truncate_prompt_log_string(value, max_chars);
    }
}

fn truncate_prompt_log_string(value: &mut String, max_chars: usize) {
    if value.chars().nth(max_chars).is_some() {
        *value = bounded_prompt_log_string(value, max_chars);
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
    fn is_enabled(&self) -> bool {
        false
    }

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
    fn is_enabled(&self) -> bool {
        false
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_prompt_log_disables_capture_work() {
        let port = NoopAppServerPromptLogPort;
        assert!(!port.is_enabled());
    }

    #[test]
    fn prompt_log_bounds_are_utf8_safe_before_persistence() {
        let bounded = bounded_prompt_log_string(
            &"한".repeat(APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS + 1),
            APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS,
        );

        assert_eq!(
            bounded.chars().count(),
            APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS
        );
        assert!(bounded.ends_with("retention policy]"));
    }
}
