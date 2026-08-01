/*
 * ConversationViewModel의 message mutation 경계다. domain message log는 `messages`가
 * 보관하고, transcript를 바꾸는 함수는 이 파일 안에서 retention과 revision bookkeeping을
 * 함께 끝내 renderer, snapshot replay, scroll 계산이 같은 원본을 보게 한다.
 */
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};
use crate::domain::planning::{
    PlanningQueueMutationKind, PlanningQueueMutationReceipt, TaskStatus,
};
use crate::domain::text::compact_whitespace_detail;

use super::ConversationViewModel;

const MAX_RETAINED_TRANSCRIPT_MESSAGES: usize = 2_048;
const MAX_RETAINED_TRANSCRIPT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RETAINED_TRANSCRIPT_LINES: usize = 16_384;
const MAX_RETAINED_MESSAGE_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_RETAINED_MESSAGE_TEXT_LINES: usize = 8_192;
const MAX_RETAINED_MESSAGE_METADATA_BYTES: usize = 64 * 1024;
const MAX_RETAINED_MESSAGE_METADATA_LINES: usize = 128;
const MAX_QUEUE_RECEIPT_TITLE_CHARS: usize = 96;
const TRANSCRIPT_RETENTION_NOTICE: &str =
    "conversation transcript was trimmed to the latest bounded TUI history";
const TEXT_RETENTION_MARKER: &str = "\n[truncated by Akra TUI retention policy]";

/*
 * 메시지 조작은 한 갈래를 이 impl에 묶어 둔다. app-server stream의 agent delta는
 * item_id로 canonical transcript row를 제자리 갱신하고, tool/status notice는 즉시 순서대로 편입하며,
 * auto-follow와 planning handoff가 읽는 최신 user/agent text는 transcript 기준으로
 * 노출한다.
 */
impl ConversationViewModel {
    // 단일 append의 canonical path다. transcript와 retention 갱신 시점을 하나로 묶는다.
    pub(super) fn push_message(&mut self, mut message: ConversationMessage) {
        bound_conversation_message(&mut message);
        self.messages.push(message);
        self.enforce_transcript_retention();
        self.advance_transcript_revision();
    }

    /*
     * Batch append는 session load처럼 이미 순서가 정해진 message
     * 묶음을 transcript에 붙인다. 빈 iterator는 transcript를 그대로 둔다.
     */
    #[cfg(test)]
    pub(super) fn push_messages<I>(&mut self, messages: I)
    where
        I: IntoIterator<Item = ConversationMessage>,
    {
        let mut changed = false;
        for mut message in messages {
            bound_conversation_message(&mut message);
            self.messages.push(message);
            changed = true;
        }

        if changed {
            self.enforce_transcript_retention();
            self.advance_transcript_revision();
        }
    }

    pub(super) fn enforce_transcript_retention(&mut self) -> bool {
        for message in &mut self.messages {
            bound_conversation_message(message);
        }

        let mut retained_bytes = self
            .messages
            .iter()
            .map(conversation_message_retained_bytes)
            .sum::<usize>();
        let mut retained_lines = self
            .messages
            .iter()
            .map(conversation_message_retained_lines)
            .sum::<usize>();
        let mut remove_count = 0;
        while remove_count < self.messages.len()
            && (self.messages.len().saturating_sub(remove_count) > MAX_RETAINED_TRANSCRIPT_MESSAGES
                || retained_bytes > MAX_RETAINED_TRANSCRIPT_BYTES
                || retained_lines > MAX_RETAINED_TRANSCRIPT_LINES)
        {
            retained_bytes = retained_bytes.saturating_sub(conversation_message_retained_bytes(
                &self.messages[remove_count],
            ));
            retained_lines = retained_lines.saturating_sub(conversation_message_retained_lines(
                &self.messages[remove_count],
            ));
            remove_count += 1;
        }
        if remove_count == 0 {
            return false;
        }

        self.messages.drain(0..remove_count);
        self.extend_runtime_notices([TRANSCRIPT_RETENTION_NOTICE.to_string()]);
        true
    }

    /*
     * Status message는 transcript에 남는 operator notice다. 빈 문구와 직전 status
     * 중복을 억제해 runtime polling이나 반복 skip reason이 화면 로그를 같은 줄로
     * 계속 밀어내지 않게 한다.
     */
    pub(crate) fn append_status_message(&mut self, text: impl Into<String>) -> bool {
        let text = text.into();
        if text.trim().is_empty() {
            return false;
        }

        // 연속 status만 접어 같은 상태가 나중에 다시 나타나는 audit trail은 보존한다.
        if self.messages.last().is_some_and(|message| {
            message.kind == ConversationMessageKind::Status && message.text == text
        }) {
            return false;
        }

        self.push_message(ConversationMessage::new(
            ConversationMessageKind::Status,
            text,
            None,
            None,
        ));
        true
    }

    pub(crate) fn record_queue_mutation_receipt(
        &mut self,
        receipt: Option<PlanningQueueMutationReceipt>,
    ) {
        let Some(receipt) = receipt else {
            return;
        };
        let mut lines = vec![format!(
            "committed / revision {}",
            receipt.planning_revision
        )];
        if receipt.entries.is_empty() {
            lines.push("No queue changes committed.".to_string());
        } else {
            lines.extend(receipt.entries.iter().map(|entry| {
                let marker = match entry.mutation_kind {
                    PlanningQueueMutationKind::Created => "+",
                    PlanningQueueMutationKind::Updated => "~",
                };
                let transition = entry
                    .before_status
                    .filter(|before| *before != entry.after_status)
                    .map(|before| format!("{} -> ", receipt_status_label(before)))
                    .unwrap_or_default();
                format!(
                    "{marker} {transition}{}  {}",
                    receipt_status_label(entry.after_status),
                    compact_whitespace_detail(
                        entry.task_title.as_str(),
                        MAX_QUEUE_RECEIPT_TITLE_CHARS
                    )
                )
            }));
            let added_count = receipt.created_entries().count();
            lines.push(format!(
                "{} change{} / {added_count} newly queued",
                receipt.entries.len(),
                if receipt.entries.len() == 1 { "" } else { "s" }
            ));
        }
        self.latest_queue_mutation_receipt = Some(receipt);
        self.push_message(
            ConversationMessage::new(
                ConversationMessageKind::Status,
                lines.join("\n"),
                None,
                None,
            )
            .with_display_label("Akra Queue"),
        );
    }

    /*
     * Tool notice는 도착한 시점에 canonical transcript에 바로 들어간다. Streaming agent
     * item은 같은 Vec 안에서 item_id로 제자리 갱신되므로 tool 뒤에 늦게 도착한 agent
     * completion이 기존 행을 이동시키거나 transcript를 다시 조립하지 않는다.
     */
    #[cfg(test)]
    pub(crate) fn append_tool_message(&mut self, text: impl Into<String>) {
        self.append_tool_message_with_label(text, None);
    }

    #[cfg(test)]
    pub(crate) fn append_tool_message_with_label(
        &mut self,
        text: impl Into<String>,
        display_label: Option<String>,
    ) {
        self.append_tool_message_with_detail(text, display_label, None, None);
    }

    pub(crate) fn append_tool_message_with_detail(
        &mut self,
        text: impl Into<String>,
        display_label: Option<String>,
        item_id: Option<String>,
        detail: Option<String>,
    ) {
        let mut text = text.into();
        if text.trim().is_empty() {
            return;
        }

        if let Some(detail) = detail.as_deref().filter(|detail| !detail.trim().is_empty()) {
            let mut merged_lines = text.lines().map(str::to_string).collect::<Vec<_>>();
            for detail_line in detail.lines() {
                if !merged_lines.iter().any(|existing| existing == detail_line) {
                    merged_lines.push(detail_line.to_string());
                }
            }
            text = merged_lines.join("\n");
        }

        let mut message =
            ConversationMessage::new(ConversationMessageKind::Tool, text, None, item_id);
        if let Some(display_label) = display_label {
            message = message.with_display_label(display_label);
        }
        self.push_message(message);
    }

    /*
     * Agent delta는 codex stream item_id 단위로 canonical transcript 행을 제자리
     * 갱신한다. Tool card가 두 delta 사이에 들어와도 기존 agent 행의 index는 바뀌지 않는다.
     */
    #[cfg(test)]
    pub(crate) fn append_agent_delta(
        &mut self,
        item_id: String,
        phase: Option<String>,
        delta: String,
    ) {
        if let Some(message) = self.messages.iter_mut().rev().find(|message| {
            message.kind == ConversationMessageKind::Agent
                && message.item_id.as_deref() == Some(item_id.as_str())
        }) {
            append_bounded_text(
                &mut message.text,
                &delta,
                MAX_RETAINED_MESSAGE_TEXT_BYTES,
                MAX_RETAINED_MESSAGE_TEXT_LINES,
            );
            // Phase는 optional stream metadata라 새 값이 있을 때만 live label을 갱신한다.
            if phase.is_some() {
                message.phase = phase;
            }
            bound_conversation_message(message);
            self.enforce_transcript_retention();
            self.advance_transcript_revision();
            return;
        }

        let message =
            ConversationMessage::new(ConversationMessageKind::Agent, delta, phase, Some(item_id));
        self.push_message(message);
    }

    pub(crate) fn upsert_agent_draft(
        &mut self,
        item_id: String,
        phase: Option<String>,
        text: String,
    ) {
        if let Some(message) = self.messages.iter_mut().rev().find(|message| {
            message.kind == ConversationMessageKind::Agent
                && message.item_id.as_deref() == Some(item_id.as_str())
        }) {
            message.text = text;
            if phase.is_some() {
                message.phase = phase;
            }
            bound_conversation_message(message);
            self.enforce_transcript_retention();
            self.advance_transcript_revision();
            return;
        }

        let message =
            ConversationMessage::new(ConversationMessageKind::Agent, text, phase, Some(item_id));
        self.push_message(message);
    }

    /*
     * Completion event는 같은 item_id의 canonical 행을 authoritative final text로
     * 갱신한다. 행이 없을 때만 새 행을 추가하므로 late completion도 유실되지 않는다.
     */
    pub(crate) fn finalize_agent_message(
        &mut self,
        item_id: String,
        phase: Option<String>,
        text: String,
    ) -> bool {
        if let Some(message) = self.messages.iter_mut().rev().find(|message| {
            message.kind == ConversationMessageKind::Agent
                && message.item_id.as_deref() == Some(item_id.as_str())
        }) {
            message.text = text;
            message.phase = phase;
            bound_conversation_message(message);
            self.enforce_transcript_retention();
            self.advance_transcript_revision();
            return true;
        }

        self.push_message(ConversationMessage::new(
            ConversationMessageKind::Agent,
            text,
            phase,
            Some(item_id),
        ));
        true
    }

    pub(crate) fn visible_tool_message_has_digest(&self, digest: [u8; 32]) -> bool {
        self.messages
            .iter()
            .filter(|message| message.kind == ConversationMessageKind::Tool)
            .any(|message| {
                super::super::progressive_activity_cards::tool_message_digest(
                    message.item_id.as_deref(),
                    &message.text,
                ) == digest
            })
    }

    pub(crate) fn record_status_message(&mut self, status_text: String) {
        self.status_text = status_text;
    }

    pub(crate) fn status_text_for_viewport(&self) -> &str {
        &self.status_text
    }

    fn advance_transcript_revision(&mut self) {
        self.transcript_revision = self
            .transcript_revision
            .checked_add(1)
            .expect("conversation transcript revision exhausted");
    }

    pub(crate) fn transcript_revision(&self) -> u64 {
        self.transcript_revision
    }

    /*
     * Auto-follow decision은 마지막 agent reply를 planning runtime request의 근거로
     * 넘긴다. canonical transcript의 non-empty agent message만 읽으며,
     * status/tool notice는 자동 후속 판단 입력에서 제외된다.
     */
    pub(crate) fn latest_agent_message_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == ConversationMessageKind::Agent && !message.text.trim().is_empty()
            })
            .map(|message| message.text.as_str())
    }

    /*
     * Latest user message는 prompt assembly와 UI status가 현재 conversation의
     * operator intent를 다시 확인할 때 쓰는 query다. agent/status/tool message를
     * 건너뛰어 사람이 입력한 마지막 지시만 반환한다.
     */
    pub(crate) fn latest_user_message_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                message.kind == ConversationMessageKind::User && !message.text.trim().is_empty()
            })
            .map(|message| message.text.as_str())
    }
}

fn receipt_status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Ready => "READY",
        TaskStatus::Blocked => "BLOCKED",
        TaskStatus::InProgress => "IN PROGRESS",
        TaskStatus::Done => "DONE",
        TaskStatus::Cancelled => "CANCELLED",
        TaskStatus::AwaitingUser => "AWAITING USER",
        TaskStatus::Proposed => "PROPOSED",
    }
}

fn bound_conversation_message(message: &mut ConversationMessage) {
    truncate_text_to_limits(
        &mut message.text,
        MAX_RETAINED_MESSAGE_TEXT_BYTES,
        MAX_RETAINED_MESSAGE_TEXT_LINES,
    );
    for value in [
        &mut message.debug_detail,
        &mut message.phase,
        &mut message.item_id,
        &mut message.display_label,
    ]
    .into_iter()
    .flatten()
    {
        truncate_text_to_limits(
            value,
            MAX_RETAINED_MESSAGE_METADATA_BYTES,
            MAX_RETAINED_MESSAGE_METADATA_LINES,
        );
    }
}

fn conversation_message_retained_bytes(message: &ConversationMessage) -> usize {
    message
        .text
        .len()
        .saturating_add(message.debug_detail.as_ref().map_or(0, String::len))
        .saturating_add(message.phase.as_ref().map_or(0, String::len))
        .saturating_add(message.item_id.as_ref().map_or(0, String::len))
        .saturating_add(message.display_label.as_ref().map_or(0, String::len))
}

fn conversation_message_retained_lines(message: &ConversationMessage) -> usize {
    text_line_count(&message.text)
        .saturating_add(message.debug_detail.as_deref().map_or(0, text_line_count))
}

fn text_line_count(value: &str) -> usize {
    value
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        .saturating_add(1)
}

#[cfg(test)]
fn append_bounded_text(value: &mut String, addition: &str, max_bytes: usize, max_lines: usize) {
    if value.ends_with(TEXT_RETENTION_MARKER) {
        return;
    }
    let combined_bytes = value.len().saturating_add(addition.len());
    let combined_lines = text_line_count(value)
        .saturating_add(text_line_count(addition))
        .saturating_sub(1);
    if combined_bytes <= max_bytes && combined_lines <= max_lines {
        value.push_str(addition);
        return;
    }

    value.push_str(addition);
    truncate_text_to_limits(value, max_bytes, max_lines);
}

pub(super) fn truncate_text_to_limits(
    value: &mut String,
    max_bytes: usize,
    max_lines: usize,
) -> bool {
    if value.len() <= max_bytes && text_line_count(value) <= max_lines {
        return false;
    }

    let content_budget = max_bytes.saturating_sub(TEXT_RETENTION_MARKER.len());
    let content_line_limit = max_lines.saturating_sub(1).max(1);
    let mut retained_end = 0;
    let mut retained_lines = 1;
    for (byte_index, character) in value.char_indices() {
        let next_end = byte_index.saturating_add(character.len_utf8());
        if next_end > content_budget {
            break;
        }
        if character == '\n' {
            if retained_lines >= content_line_limit {
                break;
            }
            retained_lines += 1;
        }
        retained_end = next_end;
    }
    value.truncate(retained_end);
    value.push_str(TEXT_RETENTION_MARKER);
    true
}

#[cfg(test)]
mod retention_tests {
    use super::*;

    #[test]
    fn transcript_retention_keeps_the_newest_bounded_messages() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.push_messages((0..=MAX_RETAINED_TRANSCRIPT_MESSAGES).map(|index| {
            ConversationMessage::new(
                ConversationMessageKind::Status,
                format!("message-{index}"),
                None,
                None,
            )
        }));

        assert_eq!(
            conversation.messages.len(),
            MAX_RETAINED_TRANSCRIPT_MESSAGES
        );
        assert_eq!(conversation.messages[0].text, "message-1");
        assert_eq!(
            conversation
                .messages
                .last()
                .map(|message| message.text.as_str()),
            Some("message-2048")
        );
        assert!(
            conversation
                .runtime_notices
                .iter()
                .any(|notice| notice == TRANSCRIPT_RETENTION_NOTICE)
        );
    }

    #[test]
    fn live_delta_and_message_lines_are_bounded_before_rendering() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.append_agent_delta(
            "item-1".to_string(),
            None,
            "line\n".repeat(MAX_RETAINED_MESSAGE_TEXT_LINES + 10),
        );

        let live = conversation
            .messages
            .iter()
            .find(|message| message.item_id.as_deref() == Some("item-1"))
            .expect("streaming message should remain in the canonical transcript");
        assert!(live.text.len() <= MAX_RETAINED_MESSAGE_TEXT_BYTES);
        assert!(text_line_count(&live.text) <= MAX_RETAINED_MESSAGE_TEXT_LINES);
        assert!(live.text.ends_with(TEXT_RETENTION_MARKER));
    }

    #[test]
    fn tool_activity_is_immediately_visible_in_the_canonical_transcript() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.append_tool_message("Read src/lib.rs");

        assert_eq!(
            conversation
                .messages
                .iter()
                .map(|message| (message.kind, message.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(ConversationMessageKind::Tool, "Read src/lib.rs")]
        );
    }

    #[test]
    fn appended_tool_precedes_the_next_agent_commentary() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        assert!(conversation.finalize_agent_message(
            "agent-commentary-1".to_string(),
            Some("commentary".to_string()),
            "I will inspect the event reducer.".to_string(),
        ));
        conversation.append_tool_message("Read src/lib.rs");

        assert!(conversation.finalize_agent_message(
            "agent-commentary-2".to_string(),
            Some("commentary".to_string()),
            "The event reducer is next.".to_string(),
        ));

        let visible = conversation
            .messages
            .iter()
            .map(|message| (message.kind, message.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            visible,
            vec![
                (
                    ConversationMessageKind::Agent,
                    "I will inspect the event reducer."
                ),
                (ConversationMessageKind::Tool, "Read src/lib.rs"),
                (ConversationMessageKind::Agent, "The event reducer is next."),
            ]
        );
    }

    #[test]
    fn command_detail_merge_keeps_read_targets_once_and_appends_output() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.append_tool_message_with_detail(
            "Explored 2 targets\n1. Read src/core/app.rs\n2. Read src/domain/conversation.rs",
            Some("explore".to_string()),
            Some("command-1".to_string()),
            Some(
                "explore details\n1. Read src/core/app.rs\n2. Read src/domain/conversation.rs\n\nCommand output\nok"
                    .to_string(),
            ),
        );

        let text = &conversation.messages[0].text;
        assert_eq!(text.matches("1. Read src/core/app.rs").count(), 1);
        assert_eq!(
            text.matches("2. Read src/domain/conversation.rs").count(),
            1
        );
        assert!(text.contains("Command output\nok"));
    }

    #[test]
    fn late_agent_completion_updates_in_place_without_reassembling_history() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.upsert_agent_draft(
            "agent-1".to_string(),
            Some("commentary".to_string()),
            "I will inspect the reducer.".to_string(),
        );
        conversation.append_tool_message_with_detail(
            "Read src/lib.rs",
            Some("explore".to_string()),
            Some("tool-1".to_string()),
            Some("src/lib.rs:1-40".to_string()),
        );
        conversation.upsert_agent_draft(
            "agent-2".to_string(),
            Some("commentary".to_string()),
            "The renderer is next.".to_string(),
        );

        assert!(conversation.finalize_agent_message(
            "agent-1".to_string(),
            Some("commentary".to_string()),
            "I inspected the reducer.".to_string(),
        ));

        let rows = conversation
            .messages
            .iter()
            .map(|message| {
                (
                    message.item_id.as_deref(),
                    message.kind,
                    message.text.lines().next().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![
                (
                    Some("agent-1"),
                    ConversationMessageKind::Agent,
                    "I inspected the reducer."
                ),
                (
                    Some("tool-1"),
                    ConversationMessageKind::Tool,
                    "Read src/lib.rs"
                ),
                (
                    Some("agent-2"),
                    ConversationMessageKind::Agent,
                    "The renderer is next."
                ),
            ]
        );
    }

    #[test]
    fn queue_receipt_title_is_single_line_and_bounded() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.record_queue_mutation_receipt(Some(PlanningQueueMutationReceipt {
            completed_turn_id: "turn-1".to_string(),
            planning_revision: 7,
            entries: vec![crate::domain::planning::PlanningQueueMutationReceiptEntry {
                task_id: "task-1".to_string(),
                task_title: format!("Visible title\n{}", "forged status ".repeat(20)),
                mutation_kind: PlanningQueueMutationKind::Created,
                before_status: None,
                after_status: TaskStatus::Ready,
                after_updated_at: "2026-07-15T00:00:00Z".to_string(),
                unchanged_since_mutation: true,
            }],
        }));

        let receipt = conversation
            .messages
            .last()
            .expect("queue receipt should be appended");
        let task_line = receipt
            .text
            .lines()
            .find(|line| line.starts_with("+ READY"))
            .expect("queue receipt should contain one task line");
        assert!(task_line.contains("Visible title forged status"));
        assert!(task_line.ends_with("..."));
        assert_eq!(receipt.text.matches("+ READY").count(), 1);
    }
}
