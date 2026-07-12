/*
 * ConversationViewModel의 message mutation 경계다. domain message log는 `messages`가
 * 보관하고, shell renderer가 바로 소비하는 줄 단위 projection은
 * `cached_conversation_lines`가 보관한다. transcript를 바꾸는 함수는 이 파일 안에서
 * cache refresh까지 끝내야 shell footer, snapshot replay, scroll 계산이 같은 원본을 본다.
 */
use crate::adapter::inbound::tui::app::shell_presentation::format_conversation_lines;
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

use super::ConversationViewModel;

const MAX_RETAINED_TRANSCRIPT_MESSAGES: usize = 2_048;
const MAX_RETAINED_TRANSCRIPT_BYTES: usize = 8 * 1024 * 1024;
const MAX_RETAINED_TRANSCRIPT_LINES: usize = 16_384;
const MAX_RETAINED_MESSAGE_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_RETAINED_MESSAGE_TEXT_LINES: usize = 8_192;
const MAX_RETAINED_MESSAGE_METADATA_BYTES: usize = 64 * 1024;
const MAX_RETAINED_MESSAGE_METADATA_LINES: usize = 128;
const MAX_BUFFERED_TOOL_MESSAGES: usize = 256;
const MAX_BUFFERED_TOOL_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_BUFFERED_TOOL_MESSAGE_LINES: usize = 2_048;
const TRANSCRIPT_RETENTION_NOTICE: &str =
    "conversation transcript was trimmed to the latest bounded TUI history";
const TOOL_RETENTION_NOTICE: &str =
    "older buffered tool notices were trimmed before transcript delivery";
const TEXT_RETENTION_MARKER: &str = "\n[truncated by Akra TUI retention policy]";

/*
 * 메시지 조작은 세 갈래를 한 impl에 묶어 둔다. app-server stream의 agent delta는
 * live buffer에 누적하고, tool/status notice는 transcript에 순서 있게 편입하며,
 * auto-follow와 planning handoff가 읽는 최신 user/agent text는 transcript 기준으로
 * 노출한다.
 */
impl ConversationViewModel {
    /*
     * Renderer는 매 frame마다 markdown/line formatting을 다시 계산하지 않고 cached
     * lines를 읽는다. transcript를 수정한 뒤 이 함수를 호출하는 규칙이 깨지면
     * message source와 vt100 화면 projection이 서로 다른 시점을 가리킨다.
     */
    pub(crate) fn refresh_conversation_lines(&mut self) {
        self.cached_conversation_lines = format_conversation_lines(&self.messages);
    }

    // 단일 append의 canonical path다. transcript와 render cache의 갱신 시점을 하나로 묶는다.
    pub(super) fn push_message(&mut self, mut message: ConversationMessage) {
        bound_conversation_message(&mut message);
        self.messages.push(message);
        self.enforce_transcript_retention();
        self.refresh_conversation_lines();
    }

    /*
     * Batch append는 refresh를 한 번으로 합친다. session load나 buffered tool flush처럼
     * 이미 순서가 정해진 message 묶음을 transcript에 붙일 때 formatting 반복을 줄이고,
     * 빈 iterator는 기존 화면 projection을 그대로 둔다.
     */
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
            self.refresh_conversation_lines();
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

    /*
     * Tool notice는 streaming agent text와 바로 섞이면 transcript ordering이 흐려진다.
     * 먼저 buffer에 모아 두고 turn boundary나 explicit flush에서 한꺼번에 옮겨 agent
     * reply와 tool activity copy의 상대 순서를 안정화한다.
     */
    pub(crate) fn buffer_tool_message(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }

        let mut message = ConversationMessage::new(ConversationMessageKind::Tool, text, None, None);
        bound_conversation_message(&mut message);
        self.buffered_tool_messages.push(message);
        let mut retained_bytes = self
            .buffered_tool_messages
            .iter()
            .map(conversation_message_retained_bytes)
            .sum::<usize>();
        let mut retained_lines = self
            .buffered_tool_messages
            .iter()
            .map(conversation_message_retained_lines)
            .sum::<usize>();
        let mut remove_count = 0;
        while remove_count < self.buffered_tool_messages.len()
            && (self
                .buffered_tool_messages
                .len()
                .saturating_sub(remove_count)
                > MAX_BUFFERED_TOOL_MESSAGES
                || retained_bytes > MAX_BUFFERED_TOOL_MESSAGE_BYTES
                || retained_lines > MAX_BUFFERED_TOOL_MESSAGE_LINES)
        {
            retained_bytes = retained_bytes.saturating_sub(conversation_message_retained_bytes(
                &self.buffered_tool_messages[remove_count],
            ));
            retained_lines = retained_lines.saturating_sub(conversation_message_retained_lines(
                &self.buffered_tool_messages[remove_count],
            ));
            remove_count += 1;
        }
        if remove_count > 0 {
            self.buffered_tool_messages.drain(0..remove_count);
            self.extend_runtime_notices([TOOL_RETENTION_NOTICE.to_string()]);
        }
    }

    /*
     * Buffered tool messages의 commit point다. `take`로 buffer를 비워 flush 재호출이
     * 같은 notice를 중복 append하지 않게 하고, cache refresh는 `push_messages`의
     * batch 규칙에 맡긴다.
     */
    pub(crate) fn flush_buffered_tool_messages(&mut self) -> bool {
        if self.buffered_tool_messages.is_empty() {
            return false;
        }

        let buffered_messages = std::mem::take(&mut self.buffered_tool_messages);
        self.push_messages(buffered_messages);
        true
    }

    /*
     * Agent delta는 codex stream item_id 단위로 이어 붙는다. 같은 item이면 live
     * message에 누적하고, 다른 item이 열리면 기존 live message를 먼저 transcript에
     * commit해 두 agent response가 하나의 message로 합쳐지지 않게 한다.
     */
    #[cfg(test)]
    pub(crate) fn push_live_agent_delta(
        &mut self,
        item_id: String,
        phase: Option<String>,
        delta: String,
    ) {
        if let Some(message) = self.live_agent_message.as_mut()
            && message.item_id.as_deref() == Some(item_id.as_str())
        {
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
            return;
        }

        self.commit_live_agent_message();
        let mut message =
            ConversationMessage::new(ConversationMessageKind::Agent, delta, phase, Some(item_id));
        bound_conversation_message(&mut message);
        self.live_agent_message = Some(message);
    }

    pub(crate) fn sync_live_agent_draft(
        &mut self,
        item_id: String,
        phase: Option<String>,
        text: String,
    ) {
        if let Some(message) = self.live_agent_message.as_mut()
            && message.item_id.as_deref() == Some(item_id.as_str())
        {
            message.text = text;
            if phase.is_some() {
                message.phase = phase;
            }
            bound_conversation_message(message);
            return;
        }

        self.commit_live_agent_message();
        let mut message =
            ConversationMessage::new(ConversationMessageKind::Agent, text, phase, Some(item_id));
        bound_conversation_message(&mut message);
        self.live_agent_message = Some(message);
    }

    /*
     * Completion event는 live delta를 authoritative final text로 닫는다. 같은
     * item_id의 live message가 있으면 그 값을 교체해 transcript에 넣고, live
     * buffer에 없으면 기존 transcript의 같은 item을 뒤에서 찾아 보정한다. 둘 다
     * 없으면 completion만으로 agent message를 생성해 late completion도 화면에 남긴다.
     */
    pub(crate) fn complete_live_agent_message(
        &mut self,
        item_id: String,
        phase: Option<String>,
        text: String,
    ) -> bool {
        if let Some(mut message) = self.live_agent_message.take() {
            if message.item_id.as_deref() == Some(item_id.as_str()) {
                message.text = text;
                message.phase = phase;
                self.push_message(message);
                return true;
            }

            // 다른 live item이 열려 있었다면 먼저 보존해 stream ordering 손실을 막는다.
            self.push_message(message);
        }

        if let Some(message) = self
            .messages
            .iter_mut()
            .rev()
            .find(|message| message.item_id.as_deref() == Some(item_id.as_str()))
        {
            message.text = text;
            message.phase = phase;
            bound_conversation_message(message);
            self.enforce_transcript_retention();
            self.refresh_conversation_lines();
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

    /*
     * Live agent buffer를 transcript로 확정한다. turn finish, 새 item delta, session
     * snapshot 전환 경로가 이 함수를 써서 streaming 중이던 답변을
     * latest-agent query와 renderer가 볼 수 있게 한다.
     */
    pub(crate) fn commit_live_agent_message(&mut self) -> bool {
        let Some(message) = self.live_agent_message.take() else {
            return false;
        };

        self.push_message(message);
        true
    }

    /*
     * Auto-follow decision은 마지막 agent reply를 planning runtime request의 근거로
     * 넘긴다. transcript에 commit된 non-empty agent message만 보므로 live buffer는
     * 먼저 commit되어야 하고, status/tool notice는 자동 후속 판단 입력에서 제외된다.
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
        conversation.push_live_agent_delta(
            "item-1".to_string(),
            None,
            "line\n".repeat(MAX_RETAINED_MESSAGE_TEXT_LINES + 10),
        );

        let live = conversation
            .live_agent_message
            .as_ref()
            .expect("live message should remain available");
        assert!(live.text.len() <= MAX_RETAINED_MESSAGE_TEXT_BYTES);
        assert!(text_line_count(&live.text) <= MAX_RETAINED_MESSAGE_TEXT_LINES);
        assert!(live.text.ends_with(TEXT_RETENTION_MARKER));
    }

    #[test]
    fn buffered_tool_retention_preserves_the_latest_notices() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        for index in 0..=MAX_BUFFERED_TOOL_MESSAGES {
            conversation.buffer_tool_message(format!("tool-{index}"));
        }

        assert_eq!(
            conversation.buffered_tool_messages.len(),
            MAX_BUFFERED_TOOL_MESSAGES
        );
        assert_eq!(conversation.buffered_tool_messages[0].text, "tool-1");
        assert_eq!(
            conversation
                .buffered_tool_messages
                .last()
                .map(|message| message.text.as_str()),
            Some("tool-256")
        );
    }
}
