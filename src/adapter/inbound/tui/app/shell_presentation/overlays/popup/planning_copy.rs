// 학습 주석: close risk는 draft editor UI state가 계산한 "닫으면 무엇을 잃는가" 신호입니다. copy 계층은 이 UI-state enum을
// status line builder까지 운반해 renderer가 close confirmation 문구를 일관되게 만들게 합니다.
use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
// 학습 주석: AkraTheme/Line은 popup title line helper가 쓰는 공통 TUI presentation 타입이고, PlanningValidationSeverity는 draft
// editor issue copy가 validation error/warning의 강도를 보존하기 위해 사용합니다.
use super::super::super::super::{AkraTheme, Line, PlanningValidationSeverity};

// 학습 주석: PlanningExistingWorkspaceCopy는 이미 planning workspace가 있는 상태를 popup copy로 낮춘 DTO입니다. input builder가
// NativeTuiApp/conversation/runtime snapshot에서 필요한 문자열만 추출하고, view builder는 이 구조체만 읽어 line을 만듭니다.
pub(super) struct PlanningExistingWorkspaceCopy {
    // 학습 주석: workspace_directory는 사용자가 현재 어떤 planning root를 보고 있는지 확인하는 anchor입니다.
    pub(super) workspace_directory: String,
    // 학습 주석: plan_state_label은 runtime/doctor 상태를 사람이 읽을 수 있는 짧은 상태 문구로 축약한 값입니다.
    pub(super) plan_state_label: String,
    // 학습 주석: queue_summary는 current queue head나 idle 상태를 한 줄로 보여 주는 요약입니다.
    pub(super) queue_summary: String,
    // 학습 주석: queue_idle_policy는 queue가 비어 있을 때 evaluator/auto-follow가 어떤 정책으로 움직이는지 설명하는 문구입니다.
    pub(super) queue_idle_policy: String,
    // 학습 주석: failure_summary는 workspace 로드나 runtime snapshot 읽기 실패를 overlay에 부드럽게 노출하기 위한 선택 필드입니다.
    pub(super) failure_summary: Option<String>,
}

// 학습 주석: PlanningSimpleReviewCopy는 simple planning init review 화면의 raw presentation input입니다. app state에서 staged draft,
// validation, turn budget editing 값을 한 번만 추출하고, deep review view pipeline은 이 DTO를 section/contract/view로 계속 넘깁니다.
pub(super) struct PlanningSimpleReviewCopy {
    // 학습 주석: draft_name은 promote 대상이 되는 staged draft session의 이름입니다.
    pub(super) draft_name: String,
    // 학습 주석: staged_file_count는 simple review가 실제로 생성한 draft artifacts 규모를 보여 줍니다.
    pub(super) staged_file_count: usize,
    // 학습 주석: validation_ok는 Enter/Ctrl+P promote 가능 여부를 status copy와 key guidance가 함께 판단하는 boolean gate입니다.
    pub(super) validation_ok: bool,
    // 학습 주석: first_error는 validation 실패 시 사용자가 제일 먼저 봐야 할 오류를 한 줄로 전달합니다.
    pub(super) first_error: Option<String>,
    // 학습 주석: max_auto_turns_label은 auto-follow turn budget을 화면에 표시할 ready-made label입니다.
    pub(super) max_auto_turns_label: String,
    // 학습 주석: is_turn_budget_editing은 budget 값이 확정된 domain state가 아니라 입력 buffer를 보여 줘야 하는지를 나타냅니다.
    pub(super) is_turn_budget_editing: bool,
    // 학습 주석: turn_budget_buffer는 편집 중인 raw input입니다. parsing 전 문자열을 그대로 보존해 사용자가 입력 상태를 확인할 수 있습니다.
    pub(super) turn_budget_buffer: String,
}

// 학습 주석: PlanningDraftEditorIssueCopy는 validation report의 첫 issue를 editor status line에 싣기 위한 작은 DTO입니다. 원본 report
// 전체를 renderer까지 끌고 가지 않고 severity/detail만 남깁니다.
pub(super) struct PlanningDraftEditorIssueCopy {
    // 학습 주석: severity는 error/warning에 따라 status line 스타일이나 copy 강도를 바꾸기 위한 값입니다.
    pub(super) severity: PlanningValidationSeverity,
    // 학습 주석: detail은 사용자가 바로 수정 행동으로 옮길 수 있게 하는 issue 설명입니다.
    pub(super) detail: String,
}

// 학습 주석: PlanningDraftEditorStatusCopy는 manual draft editor status/header/key line builder가 함께 쓰는 copy snapshot입니다.
// lifetime을 빌리는 필드는 session view에서 온 문자열을 복사하지 않고 읽기만 하며, 계산된 summary/issue는 owned 값으로 둡니다.
pub(super) struct PlanningDraftEditorStatusCopy<'a> {
    // 학습 주석: draft_name은 현재 편집 중인 staged draft session을 식별합니다.
    pub(super) draft_name: &'a str,
    // 학습 주석: active_path는 선택된 파일이 promote 후 어느 active planning path로 들어갈지 보여 주는 mapping입니다.
    pub(super) active_path: &'a str,
    // 학습 주석: selected_file_position은 "2/5" 같은 위치 표시를 위한 1-based index 성격의 값입니다.
    pub(super) selected_file_position: usize,
    // 학습 주석: file_count는 editor가 열고 있는 draft file 총개수입니다.
    pub(super) file_count: usize,
    // 학습 주석: validation_ok는 promote 가능 여부와 "fix errors" next action을 가르는 핵심 gate입니다.
    pub(super) validation_ok: bool,
    // 학습 주석: first_issue는 validation 실패/경고 중 status line에 대표로 노출할 첫 번째 issue입니다.
    pub(super) first_issue: Option<PlanningDraftEditorIssueCopy>,
    // 학습 주석: staged_path_summary는 draft file path와 active target path의 관계를 짧게 설명합니다.
    pub(super) staged_path_summary: String,
    // 학습 주석: dirty_label_summary는 저장되지 않은 파일 label들을 한 줄로 압축한 값입니다.
    pub(super) dirty_label_summary: String,
    // 학습 주석: has_dirty_labels는 close/promote 안내가 저장되지 않은 변경을 강조해야 하는지 빠르게 판단하게 합니다.
    pub(super) has_dirty_labels: bool,
    // 학습 주석: next_action은 validation/dirty 상태를 해석한 뒤 사용자에게 제시할 다음 명령 문구입니다.
    pub(super) next_action: &'static str,
    // 학습 주석: close_risk는 editor를 닫을 때 잃을 수 있는 unsaved/unpromoted 상태를 status copy에 싣는 필드입니다.
    pub(super) close_risk: Option<PlanningDraftEditorCloseRisk>,
    // 학습 주석: confirmation_pending은 이미 close confirmation prompt가 열린 상태인지 알려 key guidance copy를 바꿉니다.
    pub(super) confirmation_pending: bool,
}

// 학습 주석: planning_setup_title_line은 planning init/setup 계열 overlay가 같은 title chrome을 쓰게 하는 helper입니다. suffix만 바꿔
// selection, existing workspace, review 같은 하위 화면의 위치를 나타냅니다.
pub(super) fn planning_setup_title_line(suffix: &'static str) -> Line<'static> {
    AkraTheme::title_line("Planning Setup", suffix)
}

// 학습 주석: planning_draft_title_line은 staged draft/manual editor 계열 overlay title을 통일합니다. setup title과 분리해 사용자가
// "초기 설정 단계"와 "draft 편집 단계"를 시각적으로 구분하게 합니다.
pub(super) fn planning_draft_title_line(suffix: &'static str) -> Line<'static> {
    AkraTheme::title_line("Planning Draft", suffix)
}
