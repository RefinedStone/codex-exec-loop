// 학습 주석: planning draft editor UI state는 popup 내부의 편집/닫기 확인 상태를 소유합니다. 이 파일은
// 그 UI state에서 status copy가 필요한 runtime-facing 신호만 뽑아냅니다.
use crate::adapter::inbound::tui::app::planning_draft_editor_ui::{
    PlanningDraftEditorCloseRisk, PlanningDraftEditorUiState,
};
// 학습 주석: validation report는 draft가 accepted planning state로 승격될 수 있는지 판단하는 domain 결과입니다.
// next-action copy는 dirty state와 validation validity를 함께 봐야 정확합니다.
use crate::domain::planning::PlanningValidationReport;

// 학습 주석: PlanningDraftEditorRuntimeState는 editor UI state와 validation 결과를 status renderer가 바로
// 읽을 수 있는 작은 DTO로 접은 값입니다. "runtime"이라는 이름은 app runtime이 아니라 editor interaction
// runtime에서 현재 다음 행동과 close confirmation 상태를 뜻합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlanningDraftEditorRuntimeState {
    // 학습 주석: next_action은 status panel에 표시할 operator guidance입니다. dirty files가 있으면 save/validate,
    // validation이 clean하면 promote, error가 있으면 fix 흐름을 알려 줍니다.
    pub(super) next_action: &'static str,
    // 학습 주석: close_risk는 Esc/close 시 무엇을 잃을 수 있는지 설명하는 위험 분류입니다. status copy는 이
    // 값을 사용해 unsaved edits나 unsaved valid draft 같은 손실 가능성을 경고합니다.
    pub(super) close_risk: Option<PlanningDraftEditorCloseRisk>,
    // 학습 주석: confirmation_pending은 close_risk가 단순 가능성인지, 이미 사용자가 닫기를 눌러 확인 단계에
    // 들어갔는지를 구분합니다. renderer는 이 flag로 "press Esc again"류 copy를 선택합니다.
    pub(super) confirmation_pending: bool,
}

// 학습 주석: interpret_planning_draft_editor_runtime_state는 editor surface builder의 해석 boundary입니다.
// UI state, dirty-file labels, validation report를 받아 status-copy module이 필요한 세 가지 신호만 반환합니다.
pub(super) fn interpret_planning_draft_editor_runtime_state(
    // 학습 주석: ui_state는 close confirmation과 pending close risk를 들고 있는 interactive state입니다.
    ui_state: &PlanningDraftEditorUiState,
    // 학습 주석: dirty_labels는 file projection 단계에서 이미 사람이 읽는 label로 정리된 dirty file 목록입니다.
    // 비어 있지 않으면 validation 결과와 무관하게 먼저 저장/검증 안내를 해야 합니다.
    dirty_labels: &[String],
    // 학습 주석: validation_report는 현재 draft content가 domain rule을 통과했는지 알려 줍니다. dirty file이
    // 없을 때 promote 가능 여부를 결정하는 기준입니다.
    validation_report: &PlanningValidationReport,
) -> PlanningDraftEditorRuntimeState {
    // 학습 주석: close state는 pending confirmation과 일반 risk를 함께 봐야 합니다. helper가 tuple로
    // 정리해 status-copy 생성부가 UI state 세부 메서드를 직접 알 필요 없게 합니다.
    let (close_risk, confirmation_pending) = resolve_planning_draft_editor_close_state(ui_state);

    PlanningDraftEditorRuntimeState {
        // 학습 주석: next_action은 dirty/validation 우선순위만으로 결정되는 stable static copy입니다.
        next_action: planning_draft_editor_next_action(dirty_labels, validation_report),
        close_risk,
        confirmation_pending,
    }
}

// 학습 주석: close state resolver는 "닫으면 위험할 수 있음"과 "사용자가 이미 닫기를 눌러 확인 대기 중"을
// 한 번에 계산합니다. 이 분리를 status renderer가 모르면 close confirmation copy가 엇갈릴 수 있습니다.
fn resolve_planning_draft_editor_close_state(
    // 학습 주석: ui_state는 pending_close_risk와 close_risk를 별도로 제공합니다. pending 값은 실제 확인
    // 흐름에 들어간 상태이고, 일반 close_risk는 아직 confirm을 누르기 전에도 표시할 수 있는 위험입니다.
    ui_state: &PlanningDraftEditorUiState,
) -> (Option<PlanningDraftEditorCloseRisk>, bool) {
    // 학습 주석: pending risk를 먼저 읽어 두면 Option을 두 번 계산하지 않고, 아래 tuple에서 risk와
    // confirmation flag를 같은 snapshot 기준으로 만들 수 있습니다.
    let pending_close_risk = ui_state.pending_close_risk();
    (
        // 학습 주석: pending risk가 있으면 그것이 현재 화면의 가장 구체적인 close warning입니다. 없을 때만
        // 일반 close risk를 fallback으로 사용합니다.
        pending_close_risk.or_else(|| ui_state.close_risk()),
        // 학습 주석: pending risk 존재 여부가 곧 confirmation prompt가 떠 있는지의 신호입니다.
        pending_close_risk.is_some(),
    )
}

// 학습 주석: planning_draft_editor_next_action은 status panel의 핵심 guidance priority를 고정합니다.
// dirty edits가 있으면 먼저 저장/검증, 깨끗하고 valid하면 promote, 깨끗하지만 invalid면 error fix 안내입니다.
fn planning_draft_editor_next_action(
    // 학습 주석: dirty_labels는 어떤 파일이 변경됐는지까지 담지만, 이 함수는 존재 여부만 보고 action priority를
    // 결정합니다. 구체적인 labels는 status copy의 다른 줄에서 보여 줍니다.
    dirty_labels: &[String],
    // 학습 주석: validation_report는 save 이후 현재 draft가 domain validation을 통과했는지 알려 줍니다.
    validation_report: &PlanningValidationReport,
) -> &'static str {
    // 학습 주석: dirty 상태를 validation보다 우선합니다. validation report가 이전 저장본 기준일 수 있으므로,
    // 변경 중인 파일이 있으면 promote 가능 여부보다 "저장/검증을 다시 하라"는 안내가 더 정확합니다.
    if !dirty_labels.is_empty() {
        "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
    } else if validation_report.is_valid() {
        "next action: Ctrl+P promotes this draft into accepted planning state"
    } else {
        "next action: fix validation errors before promoting this draft"
    }
}

#[cfg(test)]
// 학습 주석: 테스트는 next-action priority를 고정합니다. editor key handling은 Ctrl+S/Ctrl+P를 실제로
// 수행하고, 이 module은 그 상태에서 사용자가 무엇을 눌러야 하는지 copy를 고릅니다.
mod tests {
    // 학습 주석: private helper의 branch priority를 직접 검증하기 위해 같은 module test에서 가져옵니다.
    use super::planning_draft_editor_next_action;
    // 학습 주석: validation error test는 domain validation report에 실제 file-kind issue를 넣어 invalid 상태를 만듭니다.
    use crate::domain::planning::{PlanningFileKind, PlanningValidationReport};

    #[test]
    // 학습 주석: dirty files가 있으면 validation report가 default-valid여도 save guidance가 우선해야 합니다.
    // 이 테스트는 stale validation 결과로 바로 promote 안내를 내는 회귀를 막습니다.
    fn next_action_prefers_save_guidance_when_dirty_files_exist() {
        // 학습 주석: default report는 valid 상태입니다. dirty branch가 valid branch보다 우선하는지 보려는 setup입니다.
        let report = PlanningValidationReport::default();
        // 학습 주석: label 내용 자체는 action copy에 들어가지 않지만 non-empty 여부가 dirty branch를 선택합니다.
        let dirty_labels = vec!["result-output.md".to_string()];

        // 학습 주석: helper는 editor surface가 status DTO를 만들 때와 같은 입력 모양으로 호출합니다.
        let action = planning_draft_editor_next_action(&dirty_labels, &report);

        assert_eq!(
            action,
            "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
        );
    }

    #[test]
    // 학습 주석: dirty file이 없고 validation이 clean이면 status는 promote 가능성을 알려야 합니다. 이 copy가
    // editor의 Ctrl+P key path와 맞물립니다.
    fn next_action_promotes_when_validation_is_clean() {
        // 학습 주석: default report는 no errors/no warnings 상태로 valid branch를 대표합니다.
        let report = PlanningValidationReport::default();

        // 학습 주석: empty dirty slice는 저장할 변경이 없다는 뜻입니다.
        let action = planning_draft_editor_next_action(&[], &report);

        assert_eq!(
            action,
            "next action: Ctrl+P promotes this draft into accepted planning state"
        );
    }

    #[test]
    // 학습 주석: dirty file은 없지만 validation error가 있으면 promote 대신 fix guidance가 나와야 합니다.
    // 이 테스트는 invalid draft를 accepted planning state로 보내라고 안내하는 회귀를 막습니다.
    fn next_action_requires_fix_when_validation_has_errors() {
        // 학습 주석: report에 실제 error를 넣어 `is_valid()`가 false가 되게 합니다.
        let mut report = PlanningValidationReport::default();
        report.push_error(
            // 학습 주석: file kind는 validation issue가 directions file에서 왔다는 domain context입니다.
            PlanningFileKind::Directions,
            "missing-summary",
            "summary is required",
        );

        // 학습 주석: dirty가 없으므로 invalid branch까지 내려가 fix guidance를 선택해야 합니다.
        let action = planning_draft_editor_next_action(&[], &report);

        assert_eq!(
            action,
            "next action: fix validation errors before promoting this draft"
        );
    }
}
