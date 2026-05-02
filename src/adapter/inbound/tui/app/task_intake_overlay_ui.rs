// 학습 주석: task intake overlay는 application runtime service가 만든 preview proposal과 commit result를 보관합니다.
// UI state는 domain/application 값을 직접 해석하지 않고, controller와 renderer 사이의 임시 화면 상태로만 들고 있습니다.
use crate::application::service::planning::{
    PlanningTaskIntakeCommitResult, PlanningTaskIntakeProposal,
};

// 학습 주석: 이 enum은 task intake modal의 작은 state machine입니다.
// Prompt는 사용자가 raw task prompt를 편집하는 단계이고, Preview는 runtime service가 만든 task proposal을 검토하는 단계입니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum TaskIntakeOverlayStep {
    // 학습 주석: default를 Prompt로 두면 새 overlay state가 항상 입력 화면에서 시작합니다.
    #[default]
    Prompt,
    // 학습 주석: Preview에서는 Y/E/N 키가 commit/edit/cancel 의미를 갖고, prompt editing key는 잠시 비활성화됩니다.
    Preview,
}

// 학습 주석: TaskIntakeOverlayUiState는 `:task` inline command의 modal state입니다.
// shell_controller가 이 값을 mutate하고, popup/task_intake.rs가 이 값을 읽어 header/prompt/preview/status/key lines를 만듭니다.
#[derive(Debug, Clone, Default)]
pub(super) struct TaskIntakeOverlayUiState {
    // 학습 주석: prompt_buffer는 사용자가 입력 중인 raw task request입니다. prepare_task_intake는 trim된 이 값을 사용합니다.
    prompt_buffer: String,
    // 학습 주석: proposal은 preview 단계에서 보여 줄 draft task입니다. prompt가 바뀌면 stale해지므로 즉시 None으로 지웁니다.
    proposal: Option<PlanningTaskIntakeProposal>,
    // 학습 주석: commit_result는 commit_task_intake 성공 직후 잠깐 표시할 accepted task/revision입니다.
    // 성공하면 controller가 queue overlay로 이동하기 때문에 이 값은 transient status용입니다.
    commit_result: Option<PlanningTaskIntakeCommitResult>,
    // 학습 주석: error는 prepare/commit 실패나 잘못된 조작 메시지를 status area에 보여 주기 위한 문자열입니다.
    error: Option<String>,
    // 학습 주석: step은 prompt editing과 preview confirmation의 keymap을 나누는 control state입니다.
    step: TaskIntakeOverlayStep,
}

// 학습 주석: 이 impl은 overlay state transition API입니다. shell_controller는 field를 직접 만지지 않고
// 이 methods를 호출해 stale proposal/error가 남지 않도록 합니다.
impl TaskIntakeOverlayUiState {
    // 학습 주석: open은 `:task` command가 실행될 때 overlay를 새 prompt session으로 초기화합니다.
    // command argument가 있으면 초기 prompt로 넣고, 없으면 빈 prompt editing 화면을 엽니다.
    pub(super) fn open(&mut self, prompt: Option<&str>) {
        // 학습 주석: prompt는 앞뒤 whitespace를 제거해 command buffer의 여백이 preview request에 들어가지 않게 합니다.
        self.prompt_buffer = prompt.unwrap_or_default().trim().to_string();
        // 학습 주석: 새 intake session은 이전 preview, commit 결과, 오류를 모두 버립니다.
        self.proposal = None;
        self.commit_result = None;
        self.error = None;
        // 학습 주석: argument가 있어도 처음 state는 Prompt로 열고, controller가 바로 preview_task_intake_prompt를 호출할 수 있습니다.
        self.step = TaskIntakeOverlayStep::Prompt;
    }

    // 학습 주석: reset은 overlay를 완전히 닫거나 commit 성공 후 queue overlay로 이동할 때 기본 상태로 되돌립니다.
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    // 학습 주석: step getter는 controller key handling과 renderer key-line selection이 같은 state를 보게 합니다.
    pub(super) fn step(&self) -> TaskIntakeOverlayStep {
        self.step
    }

    // 학습 주석: prompt_buffer getter는 prepare request와 prompt panel rendering에서 공유됩니다.
    pub(super) fn prompt_buffer(&self) -> &str {
        &self.prompt_buffer
    }

    // 학습 주석: proposal getter는 preview renderer와 commit handler가 같은 prepared task draft를 읽게 합니다.
    pub(super) fn proposal(&self) -> Option<&PlanningTaskIntakeProposal> {
        self.proposal.as_ref()
    }

    // 학습 주석: commit_result getter는 accepted status line을 만들 때 사용됩니다.
    pub(super) fn commit_result(&self) -> Option<&PlanningTaskIntakeCommitResult> {
        self.commit_result.as_ref()
    }

    // 학습 주석: error getter는 status panel이 owned String을 복사하지 않고 optional message를 빌려 쓰게 합니다.
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    // 학습 주석: push_character는 Prompt step의 normal character input handler입니다.
    // prompt가 바뀌면 기존 proposal/error는 더 이상 현재 입력과 맞지 않으므로 무효화합니다.
    pub(super) fn push_character(&mut self, character: char) {
        self.prompt_buffer.push(character);
        self.proposal = None;
        self.error = None;
    }

    // 학습 주석: pop_character는 Backspace handler입니다. 빈 buffer에서 호출되어도 String::pop이 안전하게 None을 반환합니다.
    // 이 경우에도 stale preview/error를 제거해 화면이 새 입력 상태와 일치하게 합니다.
    pub(super) fn pop_character(&mut self) {
        self.prompt_buffer.pop();
        self.proposal = None;
        self.error = None;
    }

    // 학습 주석: clear_prompt는 Ctrl+u handler입니다. prompt를 비우면 preview는 의미가 없으므로 같이 지웁니다.
    pub(super) fn clear_prompt(&mut self) {
        self.prompt_buffer.clear();
        self.proposal = None;
        self.error = None;
    }

    // 학습 주석: show_preview는 runtime.prepare_task_intake 성공 결과를 overlay에 반영합니다.
    // 이 순간 keymap은 Prompt에서 Preview로 바뀌어 사용자가 commit/edit/cancel 중 하나를 고르게 됩니다.
    pub(super) fn show_preview(&mut self, proposal: PlanningTaskIntakeProposal) {
        self.proposal = Some(proposal);
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Preview;
    }

    // 학습 주석: return_to_editing은 preview에서 E를 눌렀을 때 호출됩니다.
    // 기존 prompt는 유지하지만 prepared proposal은 버려 다음 Enter가 새 prompt로 다시 preview를 만들게 합니다.
    pub(super) fn return_to_editing(&mut self) {
        self.proposal = None;
        self.commit_result = None;
        self.error = None;
        self.step = TaskIntakeOverlayStep::Prompt;
    }

    // 학습 주석: show_error는 prepare/commit 실패를 status area에 표시합니다.
    // step은 바꾸지 않아 사용자가 같은 단계에서 수정하거나 다시 시도할 수 있습니다.
    pub(super) fn show_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }

    // 학습 주석: record_commit_result는 runtime.commit_task_intake 성공을 기록합니다.
    // controller는 곧 queue overlay로 이동하지만, 그 전 frame이나 테스트에서 accepted status를 확인할 수 있습니다.
    pub(super) fn record_commit_result(&mut self, result: PlanningTaskIntakeCommitResult) {
        self.commit_result = Some(result);
        self.error = None;
    }
}

// 학습 주석: 이 module의 tests는 state transition contract를 검증합니다. runtime service 없이 UI state만 고립해 봅니다.
#[cfg(test)]
mod tests {
    // 학습 주석: parent module의 state type과 step enum만 가져와 open/reset behavior를 직접 확인합니다.
    use super::{TaskIntakeOverlayStep, TaskIntakeOverlayUiState};

    // 학습 주석: open은 이전 오류/preview를 버리고 prompt editing session을 새로 시작해야 합니다.
    #[test]
    fn open_resets_preview_and_preserves_initial_prompt() {
        // 학습 주석: 이전 session의 error가 남아 있는 상태를 만들어 open이 이를 정리하는지 확인합니다.
        let mut state = TaskIntakeOverlayUiState::default();
        state.show_error("old error");

        // 학습 주석: command argument는 trim되어 prompt buffer에 들어갑니다.
        state.open(Some("  ship task intake  "));

        assert_eq!(state.step(), TaskIntakeOverlayStep::Prompt);
        assert_eq!(state.prompt_buffer(), "ship task intake");
        assert!(state.error().is_none());
        assert!(state.proposal().is_none());
    }
}
