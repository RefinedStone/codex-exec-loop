use crate::application::service::planning::{
    PlanningTaskIntakeCommitResult, PlanningTaskIntakeProposal,
};

// `:task` modal의 phase다. controller key handling과 popup key-copy projection이 같은 phase vocabulary를 공유한다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum TaskIntakeOverlayStep {
    // raw prompt editing 단계다. Enter는 planning runtime에 task proposal prepare를 요청한다.
    #[default]
    Prompt,
    // concrete proposal review 단계다. keymap은 text editing 대신 commit/edit/cancel로 바뀐다.
    Preview,
}

/*
 * `:task` overlay의 controller-owned state다.
 * planning runtime은 task proposal prepare/commit을 소유하고, 이 struct는 rendering이 읽을 modal snapshot만 보존한다.
 * prompt edit 이후 stale proposal이나 error가 살아남지 않도록 invalidation rule도 이 작은 state boundary에 모아 둔다.
 */
#[derive(Debug, Clone, Default)]
pub(super) struct TaskIntakeOverlayUiState {
    // modal에서 편집 중인 raw prompt다. controller는 runtime request를 만들 때만 trim한다.
    prompt_buffer: String,
    // preview 시점의 prompt에 묶인 runtime-generated proposal이다.
    proposal: Option<PlanningTaskIntakeProposal>,
    // transient accepted task/revision이다. 주로 test나 queue overlay가 열리기 직전 frame에서만 보인다.
    commit_result: Option<PlanningTaskIntakeCommitResult>,
    // status lane에 보여 줄 prepare/commit failure 또는 invalid action message다.
    error: Option<String>,
    // key handling과 displayed key line을 맞추는 단일 state-machine axis다.
    step: TaskIntakeOverlayStep,
}

// shell_controller가 쓰는 transition API다. field를 private으로 두어 cleanup rule이 이 파일 밖으로 새지 않게 한다.
impl TaskIntakeOverlayUiState {
    // optional inline command argument에서 새 intake session을 시작한다.
    pub(super) fn open(&mut self, prompt: Option<&str>) {
        self.prompt_buffer = prompt.unwrap_or_default().trim().to_string();
        self.proposal = None;
        self.commit_result = None;
        self.error = None;
        // argument-backed command도 먼저 Prompt에 들어간다. controller가 직후 preview를 호출할 수는 있다.
        self.step = TaskIntakeOverlayStep::Prompt;
    }

    // close 이후 또는 successful commit이 queue overlay로 제어를 넘긴 뒤 modal state를 비운다.
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    // step은 key handling과 popup projection이 함께 읽는 state-machine cursor다.
    pub(super) fn step(&self) -> TaskIntakeOverlayStep {
        self.step
    }

    // editable prompt를 노출하되 caller가 invalidation을 우회해 직접 mutation하지는 못하게 한다.
    pub(super) fn prompt_buffer(&self) -> &str {
        &self.prompt_buffer
    }

    // commit path와 render path는 같은 prepared proposal snapshot을 읽는다.
    pub(super) fn proposal(&self) -> Option<&PlanningTaskIntakeProposal> {
        self.proposal.as_ref()
    }

    // 정상 success는 곧바로 queue overlay로 전이하므로 result는 optional transient 값이다.
    pub(super) fn commit_result(&self) -> Option<&PlanningTaskIntakeCommitResult> {
        self.commit_result.as_ref()
    }

    // rendering이 service error text를 inspect하려고 clone하지 않도록 current status error를 borrow한다.
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    // prompt edit은 prepared proposal을 무효화한다. proposal은 이전 prompt text를 설명하기 때문이다.
    pub(super) fn push_character(&mut self, character: char) {
        self.prompt_buffer.push(character);
        self.proposal = None;
        self.error = None;
    }

    // backspace도 buffer가 이미 비어 있던 경우까지 같은 invalidation rule을 따른다.
    pub(super) fn pop_character(&mut self) {
        self.prompt_buffer.pop();
        self.proposal = None;
        self.error = None;
    }

    // Ctrl+u는 overlay를 proposal 없는 prompt state로 되돌린다.
    pub(super) fn clear_prompt(&mut self) {
        self.prompt_buffer.clear();
        self.proposal = None;
        self.error = None;
    }

    // runtime proposal을 저장하고 keymap을 editing에서 commit review로 전환한다.
    pub(super) fn show_preview(&mut self, proposal: PlanningTaskIntakeProposal) {
        self.proposal = Some(proposal);
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Preview;
    }

    // operator가 Preview에서 editing으로 돌아오면 prompt는 유지하되 proposal은 버린다.
    pub(super) fn return_to_editing(&mut self) {
        self.proposal = None;
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Prompt;
    }

    // error는 현재 step에 붙인다. operator가 context를 잃지 않고 retry 또는 edit을 선택할 수 있어야 한다.
    pub(super) fn show_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    // controller가 planning state를 refresh하고 queue inspection으로 이동하기 전에 commit success를 기록한다.
    pub(super) fn record_commit_result(&mut self, result: PlanningTaskIntakeCommitResult) {
        self.commit_result = Some(result);
        self.error = None;
    }
}

// test는 planning runtime service를 띄우지 않고 modal state machine만 검증한다.
#[cfg(test)]
mod tests {
    use super::{TaskIntakeOverlayStep, TaskIntakeOverlayUiState};

    // 새 command open은 이전 transient state를 지우고 initial prompt를 normalize해야 한다.
    #[test]
    fn open_resets_preview_and_preserves_initial_prompt() {
        let mut state = TaskIntakeOverlayUiState::default();
        state.show_error("old error");

        state.open(Some("  ship task intake  "));

        assert_eq!(state.step(), TaskIntakeOverlayStep::Prompt);
        assert_eq!(state.prompt_buffer(), "ship task intake");
        assert!(state.error().is_none());
        assert!(state.proposal().is_none());
    }
}
