use std::sync::mpsc::Sender;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::canonical_active_planning_file_path;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessage,
    ConversationMessageKind, ConversationToolActivity, ConversationToolActivityKind,
};

/*
 * turn_notifications.rs owns the app-server notification stream translation. connection.rs only reads JSON-RPC
 * messages, while mod.rs drives the active turn loop; this module decides which notifications belong to that
 * active thread/turn and how raw app-server item payloads become ConversationStreamEvent or snapshot messages.
 */
#[derive(Debug, Clone, PartialEq)]
pub(in crate::adapter::outbound::app_server) struct AppServerNotification {
    // method is the JSON-RPC notification method, for example `item/completed` or `turn/completed`.
    method: String,
    // params stays as raw JSON because item schemas evolve faster than the domain events we expose.
    params: Value,
}

impl AppServerNotification {
    pub(in crate::adapter::outbound::app_server) fn from_value(value: Value) -> Option<Self> {
        // Non-notification JSON-RPC messages have no method and are handled by request/response paths instead.
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
        /*
         * These notification families are owned by the active turn reducer even when
         * they arrive while a JSON-RPC request is waiting for its response. Deferring
         * them keeps `turn/start` races from turning valid early deltas into generic
         * connection warnings.
         */
        self.method == "error"
            || self.method == "thread/status/changed"
            || self.method.starts_with("turn/")
            || self.method.starts_with("item/")
    }

    pub(in crate::adapter::outbound::app_server) fn warning_text(&self, context: &str) -> String {
        /*
         * Dropped notifications are not silent because app-server schemas are still a
         * moving boundary. The warning copy keeps method identity plus call-site
         * context so diagnostics can distinguish stale-turn noise from actual schema
         * drift.
         */
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
    // Consumed means the notification updated stream state but the turn should keep reading.
    Consumed,
    // Completed is only emitted after the active `turn/completed` notification has been translated.
    Completed,
    // Dropped keeps the loop alive while preserving a warning for out-of-scope or unknown notifications.
    Dropped(String),
}

pub(in crate::adapter::outbound::app_server) fn handle_turn_notification(
    notification: &AppServerNotification,
    thread_id: &str,
    turn_id: &str,
    changed_planning_file_paths: &mut Vec<String>,
    event_sender: &Sender<ConversationStreamEvent>,
) -> Result<TurnNotificationHandling> {
    /*
     * This function is the live stream reducer. Every branch first verifies thread/turn identity before emitting
     * domain events so a shared connection cannot leak notifications from a stale or concurrent turn into the
     * TUI transcript. `changed_planning_file_paths` is intentionally accumulated outside the function because
     * the final TurnCompleted event needs the whole turn summary.
     */
    let params = notification.params();

    match notification.method() {
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed" => {
            // Approval review events are attached to tool items, so malformed payloads are warnings rather than fatal errors.
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
            // Status updates are coarse UI copy; missing status type is tolerated so the stream keeps moving.
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
            // turn/started establishes the stream turn id shown in the transcript even if app-server omits nested turn.id.
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
            /*
             * Delta items update the live transcript only. The completed agent message
             * is emitted by `item/completed`, so replay and final transcript state do
             * not depend on reconstructing text from a possibly missing delta stream.
             */
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
            /*
             * Completed items are the live stream's finalization point for transcript
             * records and tool summaries. Planning changed-file tracking is recorded
             * before UI fan-out so the later `turn/completed` event can carry the full
             * turn-level planning refresh summary.
             */
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
            // app-server error notifications terminate the active stream; callers attach request context above this layer.
            let message = params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("app-server reported an error");
            bail!(message.to_string());
        }
        "turn/completed" => {
            /*
             * `turn/completed` is the only notification that may stop the read loop.
             * It also transfers the side-band planning-file summary accumulated from
             * earlier fileChange items, letting post-turn planning refresh run after
             * the transcript has seen the complete app-server turn.
             */
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
    /*
     * Snapshot reads replay historical turn items rather than live notifications. This parser intentionally mirrors
     * handle_completed_item so resumed sessions and live streams render agent messages, tool summaries, and user
     * messages with the same ConversationMessage vocabulary.
     */
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
    /*
     * Only canonical planning runtime files should trigger planning refresh/repair
     * follow-up after a turn completes. The app-server may report repo-relative,
     * absolute, or legacy-looking paths; canonical_active_planning_file_path is the
     * single gate that keeps arbitrary file edits from waking planning automation.
     */
    let mut paths = Vec::new();

    for path in changed_file_paths(item) {
        if let Some(canonical_path) = canonical_active_planning_file_path(&path) {
            push_unique_path(&mut paths, canonical_path.to_string());
        }
    }

    paths
}

fn format_file_change_summary(item: &Value) -> String {
    /*
     * Tool activity copy stays compact because it appears inline in the live TUI
     * activity area. The detailed diff remains in the underlying app-server item;
     * this summary is only a scan-friendly progress signal.
     */
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
    // The count is separate from text so UI components can choose badge/count rendering without reparsing the summary.
    item.get("changes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default()
}

fn format_command_execution_summary(item: &Value) -> String {
    // Command execution payloads are reduced to command plus status; stdout/stderr detail is left to app-server transcript items.
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
    // userMessage content is an array; only text fragments become transcript text and multiple fragments keep line breaks.
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
    /*
     * fileChange items can report both the original path and a move destination.
     * Planning tracking has to inspect both because moving an active planning doc out
     * of or into the canonical location is just as relevant as editing it in place.
     */
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
    // app-server may report repeated changes in one item; downstream planning refresh only needs each canonical path once.
    if !target.iter().any(|existing| existing == &path) {
        target.push(path);
    }
}

fn parse_approval_review_event(
    notification: &AppServerNotification,
    thread_id: &str,
    turn_id: &str,
) -> Option<ConversationStreamEvent> {
    /*
     * Approval review parsing is intentionally optional. A malformed review payload
     * should surface as a dropped-notification warning, while the surrounding tool
     * execution and message stream continue reducing normally.
     */
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
    // Unknown statuses are preserved verbatim so UI/debug output can reveal new app-server vocabulary.
    match value {
        "inProgress" => ConversationApprovalReviewStatus::InProgress,
        "approved" => ConversationApprovalReviewStatus::Approved,
        "denied" => ConversationApprovalReviewStatus::Denied,
        "aborted" => ConversationApprovalReviewStatus::Aborted,
        other => ConversationApprovalReviewStatus::Unknown(other.to_string()),
    }
}

fn handle_completed_item(item: Option<&Value>, event_sender: &Sender<ConversationStreamEvent>) {
    // Live item/completed notifications fan out to either final message events or tool activity events.
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
    /*
     * Planning file tracking is side-band state because fileChange tool activity is
     * emitted before the turn is known to be complete. Holding only canonical paths
     * here lets TurnCompleted remain the single post-turn trigger for planning
     * refreshes.
     */
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
    // Most item notifications include both identifiers, which is the strongest guard against stale shared-connection events.
    params.get("threadId").and_then(Value::as_str) == Some(thread_id)
        && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
}
