use std::sync::mpsc::Sender;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::canonical_active_planning_file_path;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessage,
    ConversationMessageKind, ConversationToolActivity, ConversationToolActivityKind,
};

#[derive(Debug, Clone, PartialEq)]
pub(in crate::adapter::outbound::app_server) struct AppServerNotification {
    method: String,
    params: Value,
}

impl AppServerNotification {
    pub(in crate::adapter::outbound::app_server) fn from_value(value: Value) -> Option<Self> {
        let method = value.get("method").and_then(Value::as_str)?.to_string();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        Some(Self { method, params })
    }

    pub(in crate::adapter::outbound::app_server) fn method(&self) -> &str {
        &self.method
    }

    pub(in crate::adapter::outbound::app_server) fn params(&self) -> &Value {
        &self.params
    }

    pub(in crate::adapter::outbound::app_server) fn should_defer_to_turn_stream(&self) -> bool {
        self.method == "error"
            || self.method == "thread/status/changed"
            || self.method.starts_with("turn/")
            || self.method.starts_with("item/")
    }

    pub(in crate::adapter::outbound::app_server) fn warning_text(&self, context: &str) -> String {
        match self.method.as_str() {
            "configWarning" => self
                .params
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("app-server sent a config warning {context}")),
            "error" => format!(
                "app-server reported an error {context}: {}",
                self.params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
            ),
            _ => format!("app-server sent notification `{}` {context}", self.method),
        }
    }
}

pub(in crate::adapter::outbound::app_server) enum TurnNotificationHandling {
    Consumed,
    Completed,
    Dropped(String),
}

pub(in crate::adapter::outbound::app_server) fn handle_turn_notification(
    notification: &AppServerNotification,
    thread_id: &str,
    turn_id: &str,
    changed_planning_file_paths: &mut Vec<String>,
    event_sender: &Sender<ConversationStreamEvent>,
) -> Result<TurnNotificationHandling> {
    let params = notification.params();

    match notification.method() {
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed" => {
            if !matches_active_turn(params, thread_id, turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            if let Some(event) = parse_approval_review_event(notification, thread_id, turn_id) {
                let _ = event_sender.send(event);
                return Ok(TurnNotificationHandling::Consumed);
            }

            Ok(TurnNotificationHandling::Dropped(
                notification.warning_text(
                    "with an approval review payload the adapter could not translate",
                ),
            ))
        }
        "thread/status/changed" => {
            if params.get("threadId").and_then(Value::as_str) != Some(thread_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let status = params
                .get("status")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let _ = event_sender.send(ConversationStreamEvent::StatusUpdated {
                text: format!("thread status: {status}"),
            });
            Ok(TurnNotificationHandling::Consumed)
        }
        "turn/started" => {
            if params.get("threadId").and_then(Value::as_str) != Some(thread_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let started_turn_id = params
                .get("turn")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str)
                .unwrap_or(turn_id);
            let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
                turn_id: started_turn_id.to_string(),
            });
            Ok(TurnNotificationHandling::Consumed)
        }
        "item/agentMessage/delta" => {
            if params.get("turnId").and_then(Value::as_str) != Some(turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let item_id = params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let delta = params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let _ = event_sender.send(ConversationStreamEvent::AgentMessageDelta {
                item_id,
                phase: params
                    .get("phase")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                delta,
            });
            Ok(TurnNotificationHandling::Consumed)
        }
        "item/completed" => {
            if params.get("turnId").and_then(Value::as_str) != Some(turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            record_changed_planning_file_paths(params.get("item"), changed_planning_file_paths);
            handle_completed_item(params.get("item"), event_sender);
            Ok(TurnNotificationHandling::Consumed)
        }
        "error" => {
            let message = params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("app-server reported an error");
            bail!(message.to_string());
        }
        "turn/completed" => {
            let completed_thread_id = params.get("threadId").and_then(Value::as_str);
            let completed_turn_id = params
                .get("turn")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str);

            if completed_thread_id != Some(thread_id) || completed_turn_id != Some(turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let _ = event_sender.send(ConversationStreamEvent::TurnCompleted {
                turn_id: turn_id.to_string(),
                changed_planning_file_paths: changed_planning_file_paths.clone(),
            });
            Ok(TurnNotificationHandling::Completed)
        }
        _ => Ok(TurnNotificationHandling::Dropped(
            notification.warning_text("that has no adapter translation for the active turn stream"),
        )),
    }
}

pub(super) fn to_conversation_message(item: Value) -> Option<ConversationMessage> {
    let item_type = item.get("type")?.as_str()?;
    match item_type {
        "userMessage" => {
            let text = item
                .get("content")
                .and_then(Value::as_array)
                .map(|content| extract_user_input_text(content.as_slice()))
                .filter(|value| !value.trim().is_empty())?;

            Some(ConversationMessage::new(
                ConversationMessageKind::User,
                text,
                None,
                item.get("id").and_then(Value::as_str).map(str::to_string),
            ))
        }
        "agentMessage" => Some(ConversationMessage::new(
            ConversationMessageKind::Agent,
            item.get("text").and_then(Value::as_str).unwrap_or_default(),
            item.get("phase")
                .and_then(Value::as_str)
                .map(str::to_string),
            item.get("id").and_then(Value::as_str).map(str::to_string),
        )),
        "fileChange" => Some(ConversationMessage::new(
            ConversationMessageKind::Tool,
            format_file_change_summary(&item),
            None,
            item.get("id").and_then(Value::as_str).map(str::to_string),
        )),
        "commandExecution" => Some(ConversationMessage::new(
            ConversationMessageKind::Tool,
            format_command_execution_summary(&item),
            None,
            item.get("id").and_then(Value::as_str).map(str::to_string),
        )),
        _ => None,
    }
}

fn changed_planning_file_paths(item: &Value) -> Vec<String> {
    let mut paths = Vec::new();

    for path in changed_file_paths(item) {
        if let Some(canonical_path) = canonical_active_planning_file_path(&path) {
            push_unique_path(&mut paths, canonical_path.to_string());
        }
    }

    paths
}

fn format_file_change_summary(item: &Value) -> String {
    let changes = item
        .get("changes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if changes.is_empty() {
        return "file change completed".to_string();
    }

    let entries = changes
        .iter()
        .map(|change| {
            let path = change
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("unknown-path");
            let kind = change
                .get("kind")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("update");
            format!("{kind} {path}")
        })
        .collect::<Vec<_>>();

    format!("file change: {}", entries.join(", "))
}

fn count_file_changes(item: &Value) -> usize {
    item.get("changes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default()
}

fn format_command_execution_summary(item: &Value) -> String {
    let command = item
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("command");
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    format!("command: {command} [{status}]")
}

fn extract_user_input_text(items: &[Value]) -> String {
    items
        .iter()
        .filter_map(|content| {
            if content.get("type").and_then(Value::as_str) == Some("text") {
                content.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn changed_file_paths(item: &Value) -> Vec<String> {
    let mut paths = Vec::new();

    for change in item
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(path) = change.get("path").and_then(Value::as_str) {
            paths.push(path.to_string());
        }
        if let Some(move_path) = change
            .get("kind")
            .and_then(|kind| kind.get("move_path"))
            .and_then(Value::as_str)
        {
            paths.push(move_path.to_string());
        }
    }

    paths
}

fn push_unique_path(target: &mut Vec<String>, path: String) {
    if !target.iter().any(|existing| existing == &path) {
        target.push(path);
    }
}

fn parse_approval_review_event(
    notification: &AppServerNotification,
    thread_id: &str,
    turn_id: &str,
) -> Option<ConversationStreamEvent> {
    if !matches!(
        notification.method(),
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed"
    ) {
        return None;
    }

    let params = notification.params();
    if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
        || params.get("turnId").and_then(Value::as_str) != Some(turn_id)
    {
        return None;
    }

    let review = params.get("review")?;
    let status = review
        .get("status")
        .and_then(Value::as_str)
        .map(parse_approval_review_status)?;
    let target_item_id = params.get("targetItemId").and_then(Value::as_str)?;

    Some(ConversationStreamEvent::ApprovalReviewUpdated {
        review: ConversationApprovalReview {
            target_item_id: target_item_id.to_string(),
            status,
            risk_level: review
                .get("riskLevel")
                .and_then(Value::as_str)
                .map(str::to_string),
            rationale: review
                .get("rationale")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
    })
}

fn parse_approval_review_status(value: &str) -> ConversationApprovalReviewStatus {
    match value {
        "inProgress" => ConversationApprovalReviewStatus::InProgress,
        "approved" => ConversationApprovalReviewStatus::Approved,
        "denied" => ConversationApprovalReviewStatus::Denied,
        "aborted" => ConversationApprovalReviewStatus::Aborted,
        other => ConversationApprovalReviewStatus::Unknown(other.to_string()),
    }
}

fn handle_completed_item(item: Option<&Value>, event_sender: &Sender<ConversationStreamEvent>) {
    let Some(item) = item else {
        return;
    };

    let item_type = item.get("type").and_then(Value::as_str);
    match item_type {
        Some("agentMessage") => {
            let item_id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let phase = item
                .get("phase")
                .and_then(Value::as_str)
                .map(str::to_string);
            let _ = event_sender.send(ConversationStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
            });
        }
        Some("fileChange") => {
            let _ = event_sender.send(ConversationStreamEvent::ToolActivity {
                activity: ConversationToolActivity {
                    kind: ConversationToolActivityKind::FileChange,
                    text: format_file_change_summary(item),
                    file_change_count: count_file_changes(item),
                },
            });
        }
        Some("commandExecution") => {
            let _ = event_sender.send(ConversationStreamEvent::ToolActivity {
                activity: ConversationToolActivity {
                    kind: ConversationToolActivityKind::CommandExecution,
                    text: format_command_execution_summary(item),
                    file_change_count: 0,
                },
            });
        }
        _ => {}
    }
}

fn record_changed_planning_file_paths(
    item: Option<&Value>,
    changed_file_paths_for_turn: &mut Vec<String>,
) {
    let Some(item) = item else {
        return;
    };
    if item.get("type").and_then(Value::as_str) != Some("fileChange") {
        return;
    }

    for path in changed_planning_file_paths(item) {
        push_unique_path(changed_file_paths_for_turn, path);
    }
}

fn matches_active_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    params.get("threadId").and_then(Value::as_str) == Some(thread_id)
        && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
}
