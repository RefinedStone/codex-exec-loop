/*
 * 학습 주석: 이 파일은 ConversationViewModel의 message mutation boundary다. domain message log는
 * `messages`가 보관하고, shell renderer가 바로 쓸 줄 단위 캐시는 `cached_conversation_lines`가 보관한다.
 * 따라서 메시지를 추가/수정하는 함수는 반드시 cache refresh 규칙까지 함께 책임진다.
 */
use crate::adapter::inbound::tui::app::shell_presentation::format_conversation_lines;
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

use super::ConversationViewModel;

/*
 * 학습 주석: 메시지 관련 함수는 세 흐름을 묶는다. streaming 중인 agent delta를 live buffer에 누적하고,
 * tool/status message를 transcript에 안전하게 추가하며, auto-follow와 planning handoff가 읽는 최신
 * user/agent text를 제공한다.
 */
impl ConversationViewModel {
    /*
     * 학습 주석: renderer는 매 frame마다 markdown/line formatting을 다시 계산하지 않고 cached lines를 읽는다.
     * transcript message를 건드린 뒤 이 함수를 호출해야 scroll calculation, snapshot tests, vt100 rendering이
     * 같은 message source에서 파생된 화면을 보게 된다.
     */
    pub(crate) fn refresh_conversation_lines(&mut self) {
        self.cached_conversation_lines = format_conversation_lines(&self.messages);
    }

    // 학습 주석: 단일 message append의 canonical path로, transcript와 render cache를 같은 시점에 갱신한다.
    pub(super) fn push_message(&mut self, message: ConversationMessage) {
        self.messages.push(message);
        self.refresh_conversation_lines();
    }

    /*
     * 학습 주석: 여러 message를 한꺼번에 넣을 때 cache refresh를 한 번으로 합친다. session load나
     * buffered tool flush처럼 batch append가 많은 경로에서 불필요한 formatting 반복을 피하면서, 빈 iterator는
     * 화면 cache를 건드리지 않는다.
     */
    pub(super) fn push_messages<I>(&mut self, messages: I)
    where
        I: IntoIterator<Item = ConversationMessage>,
    {
        let mut changed = false;
        for message in messages {
            self.messages.push(message);
            changed = true;
        }

        if changed {
            self.refresh_conversation_lines();
        }
    }

    /*
     * 학습 주석: status message는 transcript에 남는 operator notice다. 빈 문구와 직전 message 중복을
     * 억제해 runtime status polling이나 반복 skip reason이 화면을 같은 줄로 계속 오염시키지 않게 한다.
     */
    pub(crate) fn append_status_message(&mut self, text: impl Into<String>) -> bool {
        let text = text.into();
        if text.trim().is_empty() {
            return false;
        }

        // 학습 주석: 중복 억제는 연속 status에만 적용해 같은 상태가 나중에 다시 나타나는 audit trail은 보존한다.
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
     * 학습 주석: tool message는 streaming 중 agent text와 섞이면 transcript ordering이 흐려질 수 있다.
     * 먼저 buffer에 모아 두고 turn boundary나 explicit flush에서 한꺼번에 transcript로 이동해 agent reply와
     * tool activity copy의 상대 순서를 안정화한다.
     */
    pub(crate) fn buffer_tool_message(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }

        self.buffered_tool_messages.push(ConversationMessage::new(
            ConversationMessageKind::Tool,
            text,
            None,
            None,
        ));
    }

    /*
     * 학습 주석: buffered tool messages를 transcript에 편입하는 commit point다. `take`로 buffer를 비워
     * flush 재호출이 같은 tool notice를 중복 append하지 않게 하고, push_messages가 cache refresh를 한 번만 한다.
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
     * 학습 주석: agent delta는 codex stream item_id 단위로 이어 붙는다. 같은 item_id면 live message에
     * 누적하고, 다른 item_id가 오면 기존 live message를 먼저 transcript에 commit해 두 agent response가
     * 하나의 message로 합쳐지지 않게 한다.
     */
    pub(crate) fn push_live_agent_delta(
        &mut self,
        item_id: String,
        phase: Option<String>,
        delta: String,
    ) {
        if let Some(message) = self.live_agent_message.as_mut()
            && message.item_id.as_deref() == Some(item_id.as_str())
        {
            message.text.push_str(&delta);
            // 학습 주석: phase는 optional stream metadata라 새 값이 있을 때만 live message의 phase label을 갱신한다.
            if phase.is_some() {
                message.phase = phase;
            }
            return;
        }

        self.commit_live_agent_message();
        self.live_agent_message = Some(ConversationMessage::new(
            ConversationMessageKind::Agent,
            delta,
            phase,
            Some(item_id),
        ));
    }

    /*
     * 학습 주석: completion event는 live delta를 authoritative final text로 닫는다. 같은 item_id의 live
     * message가 있으면 그 값을 교체해 transcript에 넣고, live buffer에 없으면 기존 transcript의 같은 item을
     * 뒤에서 찾아 보정한다. 둘 다 없으면 completion만으로 agent message를 생성한다.
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

            // 학습 주석: 다른 live item이 열려 있었다면 먼저 보존해 stream ordering 손실을 막는다.
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
     * 학습 주석: live agent buffer를 transcript로 확정한다. turn finish, 새 item delta, session snapshot 전환
     * 경로가 이 함수를 써서 streaming 중이던 답변을 latest_agent_message_text와 renderer가 볼 수 있게 한다.
     */
    pub(crate) fn commit_live_agent_message(&mut self) -> bool {
        let Some(message) = self.live_agent_message.take() else {
            return false;
        };

        self.push_message(message);
        true
    }

    /*
     * 학습 주석: auto-follow decision은 "마지막 agent reply"를 planning runtime request의 근거로 넘긴다.
     * transcript에 commit된 non-empty agent message만 보므로, live buffer는 먼저 commit되어야 하고 status/tool
     * message는 자동 후속 판단의 입력에서 제외된다.
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
     * 학습 주석: latest user message는 prompt assembly와 UI status가 현재 conversation의 operator intent를
     * 다시 확인할 때 쓰는 query다. agent/status/tool message를 건너뛰어 사람이 입력한 마지막 지시만 반환한다.
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
