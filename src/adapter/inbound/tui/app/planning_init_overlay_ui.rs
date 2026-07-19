use crate::application::service::planning::PlanningInitStageResult;
use crate::core::app::PlanningRuntimeRefreshCorrelation;
use crate::domain::planning::PlanningValidationReport;

/*
planning init overlay state는 service state가 아니라 TUI wizard의 화면-local
state다. controller는 이 값을 보고 key routing을 결정하고, presentation router는
같은 값을 popup/inline view DTO로 투영한다. 그래서 여기에는 실제 planning 작업
수행 결과가 아니라 현재 step, 선택 cursor, staged simple draft의 review copy만 둔다.
*/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningInitOverlayStep {
    // runtime projection을 Core effect에서 읽는 동안 wizard 입력을 막는 transient 화면이다.
    Loading,
    // 첫 진입점. simple bootstrap과 detail authoring 중 무엇을 시작할지 고른다.
    ModeSelection,
    // 이미 planning workspace가 감지된 경우의 guard 화면이다. 초기화 대신 queue/directions로 보낸다.
    ExistingWorkspace,
    // detail mode 안에서 manual editor와 worker-assisted authoring backend를 고르는 중간 단계다.
    DetailSelection,
    // simple mode staging이 끝난 뒤 promote/edit/budget 조정을 고르는 confirmation surface다.
    SimpleReview,
    // shared draft editor가 planning init overlay의 inline surface를 소유하는 단계다.
    ManualEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningInitModeSelection {
    Simple,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningInitDetailSelection {
    Manual,
    WorkerAssisted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningInitRuntimeRefreshIntent {
    Inspect,
    OpenSimpleReviewWhenAbsent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPlanningInitRuntimeRefresh {
    correlation: PlanningRuntimeRefreshCorrelation,
    intent: PlanningInitRuntimeRefreshIntent,
}

// Simple review는 planning service가 staged draft를 만든 뒤에만 존재한다. 화면은
// file list 전체가 아니라 operator decision에 필요한 요약과 validation 결과만 읽는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningInitSimpleReviewState {
    draft_name: String,
    staged_file_count: usize,
    validation_report: PlanningValidationReport,
}

impl PlanningInitSimpleReviewState {
    pub fn draft_name(&self) -> &str {
        self.draft_name.as_str()
    }
    pub fn staged_file_count(&self) -> usize {
        self.staged_file_count
    }
    pub fn validation_report(&self) -> &PlanningValidationReport {
        &self.validation_report
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningInitOverlayUiState {
    // step은 controller의 keymap, renderer router, inline editor scroll sync의 공통 분기점이다.
    step: PlanningInitOverlayStep,
    // selection 값은 highlight cursor다. Enter 전에는 service 작업을 시작하지 않는다.
    mode_selection: PlanningInitModeSelection,
    detail_selection: PlanningInitDetailSelection,
    // Some이면 staged simple draft에서 detail selection으로 갔다가 돌아올 breadcrumb가 된다.
    simple_review: Option<PlanningInitSimpleReviewState>,
    // overlay를 연 refresh만 wizard branch를 바꿀 수 있게 exact generation을 보관한다.
    pending_runtime_refresh: Option<PendingPlanningInitRuntimeRefresh>,
}

impl Default for PlanningInitOverlayUiState {
    fn default() -> Self {
        Self {
            // 기본값을 simple로 두면 첫 실행에서 가장 빠른 bootstrap path가 primary action이 된다.
            step: PlanningInitOverlayStep::ModeSelection,
            mode_selection: PlanningInitModeSelection::Simple,
            // detail mode의 아직 지원되는 concrete backend는 manual editor뿐이다.
            detail_selection: PlanningInitDetailSelection::Manual,
            simple_review: None,
            pending_runtime_refresh: None,
        }
    }
}

impl PlanningInitOverlayUiState {
    pub fn reset(&mut self) {
        // overlay를 닫거나 command center에서 다시 열 때는 staged review breadcrumb까지 버린다.
        *self = Self::default();
    }

    pub fn step(&self) -> PlanningInitOverlayStep {
        self.step
    }

    pub fn selected_mode(&self) -> PlanningInitModeSelection {
        self.mode_selection
    }

    pub fn selected_detail(&self) -> PlanningInitDetailSelection {
        self.detail_selection
    }

    pub fn simple_review(&self) -> Option<&PlanningInitSimpleReviewState> {
        self.simple_review.as_ref()
    }

    pub fn begin_runtime_refresh(
        &mut self,
        correlation: PlanningRuntimeRefreshCorrelation,
        intent: PlanningInitRuntimeRefreshIntent,
    ) {
        self.step = PlanningInitOverlayStep::Loading;
        self.simple_review = None;
        self.pending_runtime_refresh = Some(PendingPlanningInitRuntimeRefresh {
            correlation,
            intent,
        });
    }

    pub fn rebind_runtime_refresh(
        &mut self,
        correlation: PlanningRuntimeRefreshCorrelation,
    ) -> bool {
        let Some(pending) = self.pending_runtime_refresh.as_mut() else {
            return false;
        };
        if self.step != PlanningInitOverlayStep::Loading
            || pending.correlation.workspace_directory != correlation.workspace_directory
        {
            return false;
        }
        pending.correlation = correlation;
        true
    }

    pub fn cancel_runtime_refresh(
        &mut self,
        correlation: &PlanningRuntimeRefreshCorrelation,
    ) -> bool {
        if self
            .pending_runtime_refresh
            .as_ref()
            .map(|pending| &pending.correlation)
            != Some(correlation)
        {
            return false;
        }
        self.reset();
        true
    }

    pub fn apply_runtime_refresh(
        &mut self,
        correlation: &PlanningRuntimeRefreshCorrelation,
        workspace_present: bool,
    ) -> Option<bool> {
        let pending = self.pending_runtime_refresh.as_ref()?;
        if &pending.correlation != correlation {
            return None;
        }
        let should_open_simple_review = !workspace_present
            && pending.intent == PlanningInitRuntimeRefreshIntent::OpenSimpleReviewWhenAbsent;
        if workspace_present {
            self.open_existing_workspace();
        } else {
            self.open_command_center_mode_selection();
        }
        Some(should_open_simple_review)
    }

    pub fn apply_runtime_refresh_error(
        &mut self,
        correlation: &PlanningRuntimeRefreshCorrelation,
    ) -> bool {
        if self
            .pending_runtime_refresh
            .as_ref()
            .map(|pending| &pending.correlation)
            != Some(correlation)
        {
            return false;
        }
        self.pending_runtime_refresh = None;
        true
    }

    pub fn open_command_center_mode_selection(&mut self) {
        // command center 진입은 새 planning init session처럼 취급해 이전 simple draft 결정을 숨긴다.
        self.step = PlanningInitOverlayStep::ModeSelection;
        self.simple_review = None;
        self.pending_runtime_refresh = None;
    }

    pub fn apply_simple_review_validation(&mut self, validation_report: PlanningValidationReport) {
        // validation은 staged draft 저장/편집 이후 갱신될 수 있으므로 review shell을 유지한 채 copy만 교체한다.
        if let Some(review) = self.simple_review.as_mut() {
            review.validation_report = validation_report;
        }
    }

    pub fn select_mode(&mut self, selection: PlanningInitModeSelection) {
        self.mode_selection = selection;
        /*
        Simple 선택은 mode screen 자체의 cursor 이동이다. Detail 화면에서 simple로 직접
        선택되면 step도 mode selection으로 되돌려 controller와 view router의 해석을 맞춘다.
        */
        if selection == PlanningInitModeSelection::Simple {
            self.step = PlanningInitOverlayStep::ModeSelection;
        }
        // 새 mode 선택은 이전 staged simple review와 같은 decision context를 공유하지 않는다.
        self.simple_review = None;
    }

    pub fn move_mode_selection(&mut self, delta: isize) {
        // 두 항목짜리 selector라 끝에서 더 이동하면 그대로 머문다. wrap은 key hint와 맞지 않는다.
        self.mode_selection = match (self.mode_selection, delta.is_negative()) {
            (PlanningInitModeSelection::Simple, false) => PlanningInitModeSelection::Detail,
            (PlanningInitModeSelection::Detail, true) => PlanningInitModeSelection::Simple,
            (selection, _) => selection,
        };
    }

    pub fn open_detail_selection(&mut self) {
        // detail step으로 들어가면 mode highlight도 detail로 고정해 breadcrumb와 header copy가 어긋나지 않게 한다.
        self.mode_selection = PlanningInitModeSelection::Detail;
        self.step = PlanningInitOverlayStep::DetailSelection;
        self.pending_runtime_refresh = None;
    }

    pub fn open_existing_workspace(&mut self) {
        // existing workspace guard는 bootstrap decision이 아니므로 staged review copy를 항상 비운다.
        self.step = PlanningInitOverlayStep::ExistingWorkspace;
        self.simple_review = None;
        self.pending_runtime_refresh = None;
    }

    pub fn open_manual_editor(&mut self) {
        // Detail/manual path는 처음부터 authoring surface로 들어가는 고급 bootstrap 흐름이다.
        self.mode_selection = PlanningInitModeSelection::Detail;
        self.detail_selection = PlanningInitDetailSelection::Manual;
        self.step = PlanningInitOverlayStep::ManualEditor;
        self.simple_review = None;
        self.pending_runtime_refresh = None;
    }

    pub fn open_simple_editor(&mut self) {
        // Simple review에서 Ctrl+E로 들어온 editor는 simple draft를 직접 고치는 흐름이라 review copy를 유지한다.
        self.mode_selection = PlanningInitModeSelection::Simple;
        self.detail_selection = PlanningInitDetailSelection::Manual;
        self.step = PlanningInitOverlayStep::ManualEditor;
        self.pending_runtime_refresh = None;
    }

    pub fn open_simple_review(&mut self, staged: PlanningInitStageResult) {
        /*
        service layer가 draft directory와 staged file 목록을 만들지만, UI state는 renderer가
        필요한 summary만 보관한다. 실제 promote/edit 작업은 controller가 service API로 다시 위임한다.
        */
        self.open_simple_review_summary(
            staged.draft_name,
            staged.staged_file_count,
            staged.validation_report,
        );
    }

    pub fn open_simple_review_summary(
        &mut self,
        draft_name: String,
        staged_file_count: usize,
        validation_report: PlanningValidationReport,
    ) {
        self.mode_selection = PlanningInitModeSelection::Simple;
        self.step = PlanningInitOverlayStep::SimpleReview;
        self.pending_runtime_refresh = None;
        self.simple_review = Some(PlanningInitSimpleReviewState {
            draft_name,
            staged_file_count,
            validation_report,
        });
    }

    pub fn return_from_detail_selection(&mut self) {
        /*
        detail selection은 두 곳에서 열린다. 첫 setup에서 왔으면 mode selection으로 돌아가고,
        staged simple review에서 'advanced path'를 눌러 왔으면 review로 돌아가야 draft decision을 잃지 않는다.
        */
        if self.simple_review.is_some() {
            self.mode_selection = PlanningInitModeSelection::Simple;
            self.step = PlanningInitOverlayStep::SimpleReview;
        } else {
            self.step = PlanningInitOverlayStep::ModeSelection;
        }
        self.pending_runtime_refresh = None;
    }

    pub fn select_detail(&mut self, selection: PlanningInitDetailSelection) {
        // 이 함수는 highlight만 바꾼다. 실제 editor open 또는 unsupported status는 Enter handler가 담당한다.
        self.detail_selection = selection;
    }

    pub fn move_detail_selection(&mut self, delta: isize) {
        // worker-assisted 항목은 아직 실행 불가지만 selector에 남겨 future backend slot과 copy contract를 고정한다.
        self.detail_selection = match (self.detail_selection, delta.is_negative()) {
            (PlanningInitDetailSelection::Manual, false) => {
                PlanningInitDetailSelection::WorkerAssisted
            }
            (PlanningInitDetailSelection::WorkerAssisted, true) => {
                PlanningInitDetailSelection::Manual
            }
            (selection, _) => selection,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningInitDetailSelection, PlanningInitModeSelection, PlanningInitOverlayStep,
        PlanningInitOverlayUiState, PlanningInitRuntimeRefreshIntent,
    };
    use crate::application::service::planning::PlanningBootstrapMode;
    use crate::application::service::planning::PlanningInitStageResult;
    use crate::core::app::PlanningRuntimeRefreshCorrelation;
    use crate::domain::planning::PlanningValidationReport;

    // 이 테스트들은 key handler가 아니라 pure UI state contract를 고정한다. controller/presentation
    // 테스트가 실패했을 때 step mutation 자체가 문제인지, key routing/rendering 문제인지 분리하기 위한 층이다.
    #[test]
    fn default_state_starts_on_simple_mode_selection() {
        let state = PlanningInitOverlayUiState::default();

        assert_eq!(state.step(), PlanningInitOverlayStep::ModeSelection);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Simple);
        assert_eq!(state.selected_detail(), PlanningInitDetailSelection::Manual);
    }

    #[test]
    fn runtime_refresh_rebinds_latest_same_workspace_generation_and_rejects_aba() {
        let mut state = PlanningInitOverlayUiState::default();
        let first = PlanningRuntimeRefreshCorrelation::new(1, "/tmp/workspace");
        let second = PlanningRuntimeRefreshCorrelation::new(2, "/tmp/workspace");
        state.begin_runtime_refresh(first.clone(), PlanningInitRuntimeRefreshIntent::Inspect);

        assert_eq!(state.step(), PlanningInitOverlayStep::Loading);
        assert!(state.rebind_runtime_refresh(second.clone()));
        assert_eq!(state.apply_runtime_refresh(&first, false), None);
        assert_eq!(state.step(), PlanningInitOverlayStep::Loading);
        assert_eq!(state.apply_runtime_refresh(&second, true), Some(false));
        assert_eq!(state.step(), PlanningInitOverlayStep::ExistingWorkspace);
        assert_eq!(state.apply_runtime_refresh(&second, false), None);
    }

    #[test]
    fn runtime_refresh_error_requires_the_exact_generation() {
        let mut state = PlanningInitOverlayUiState::default();
        let first = PlanningRuntimeRefreshCorrelation::new(1, "/tmp/root");
        let second = PlanningRuntimeRefreshCorrelation::new(2, "/tmp/root");
        state.begin_runtime_refresh(
            second.clone(),
            PlanningInitRuntimeRefreshIntent::OpenSimpleReviewWhenAbsent,
        );

        assert!(!state.apply_runtime_refresh_error(&first));
        assert_eq!(state.step(), PlanningInitOverlayStep::Loading);
        assert!(state.apply_runtime_refresh_error(&second));
        assert_eq!(state.step(), PlanningInitOverlayStep::Loading);
        assert_eq!(state.apply_runtime_refresh(&second, false), None);
    }

    #[test]
    fn runtime_refresh_rejects_workspace_drift_and_close_discards_completion() {
        let mut state = PlanningInitOverlayUiState::default();
        let original = PlanningRuntimeRefreshCorrelation::new(1, "/tmp/a");
        state.begin_runtime_refresh(original.clone(), PlanningInitRuntimeRefreshIntent::Inspect);

        assert!(!state.rebind_runtime_refresh(PlanningRuntimeRefreshCorrelation::new(2, "/tmp/b")));
        state.reset();
        assert_eq!(state.apply_runtime_refresh(&original, true), None);
        assert_eq!(state.step(), PlanningInitOverlayStep::ModeSelection);
    }

    #[test]
    fn exact_runtime_refresh_cancellation_leaves_loading_terminally() {
        let mut state = PlanningInitOverlayUiState::default();
        let correlation = PlanningRuntimeRefreshCorrelation::new(1, "/tmp/workspace");
        state.begin_runtime_refresh(
            correlation.clone(),
            PlanningInitRuntimeRefreshIntent::Inspect,
        );

        assert!(
            !state.cancel_runtime_refresh(&PlanningRuntimeRefreshCorrelation::new(
                2,
                "/tmp/workspace"
            ))
        );
        assert_eq!(state.step(), PlanningInitOverlayStep::Loading);
        assert!(state.cancel_runtime_refresh(&correlation));
        assert_eq!(state.step(), PlanningInitOverlayStep::ModeSelection);
    }

    #[test]
    fn absent_first_run_refresh_requests_simple_review_only_after_exact_completion() {
        let mut state = PlanningInitOverlayUiState::default();
        let correlation = PlanningRuntimeRefreshCorrelation::new(1, "/tmp/workspace");
        state.begin_runtime_refresh(
            correlation.clone(),
            PlanningInitRuntimeRefreshIntent::OpenSimpleReviewWhenAbsent,
        );

        assert_eq!(state.step(), PlanningInitOverlayStep::Loading);
        assert_eq!(state.apply_runtime_refresh(&correlation, false), Some(true));
        assert_eq!(state.step(), PlanningInitOverlayStep::ModeSelection);
    }

    #[test]
    fn opening_detail_selection_keeps_mode_and_step_in_sync() {
        let mut state = PlanningInitOverlayUiState::default();

        state.open_detail_selection();

        assert_eq!(state.step(), PlanningInitOverlayStep::DetailSelection);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Detail);
    }

    #[test]
    fn opening_existing_workspace_switches_overlay_step() {
        let mut state = PlanningInitOverlayUiState::default();

        state.open_existing_workspace();

        assert_eq!(state.step(), PlanningInitOverlayStep::ExistingWorkspace);
        assert!(state.simple_review().is_none());
    }

    #[test]
    fn opening_manual_editor_pins_detail_manual_selection() {
        let mut state = PlanningInitOverlayUiState::default();

        state.open_manual_editor();

        assert_eq!(state.step(), PlanningInitOverlayStep::ManualEditor);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Detail);
        assert_eq!(state.selected_detail(), PlanningInitDetailSelection::Manual);
    }

    #[test]
    fn opening_simple_review_tracks_staged_draft_metadata() {
        let mut state = PlanningInitOverlayUiState::default();

        // `PlanningInitStageResult` is intentionally reduced here: UI state keeps only the
        // review-facing metadata, while draft paths and staged files remain service concerns.
        state.open_simple_review(PlanningInitStageResult {
            mode: PlanningBootstrapMode::Simple,
            draft_name: "bootstrap-1".to_string(),
            draft_directory: "/tmp/bootstrap-1".to_string(),
            staged_files: Vec::new(),
            staged_file_count: 4,
            validation_report: PlanningValidationReport::default(),
        });

        assert_eq!(state.step(), PlanningInitOverlayStep::SimpleReview);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Simple);
        assert_eq!(
            state.simple_review().map(|review| review.draft_name()),
            Some("bootstrap-1")
        );
        assert_eq!(
            state
                .simple_review()
                .map(|review| review.staged_file_count()),
            Some(4)
        );
    }

    #[test]
    fn reset_restores_default_selections() {
        let mut state = PlanningInitOverlayUiState::default();
        state.open_simple_review(PlanningInitStageResult {
            mode: PlanningBootstrapMode::Simple,
            draft_name: "bootstrap-1".to_string(),
            draft_directory: "/tmp/bootstrap-1".to_string(),
            staged_files: Vec::new(),
            staged_file_count: 4,
            validation_report: PlanningValidationReport::default(),
        });

        state.reset();

        assert_eq!(state.step(), PlanningInitOverlayStep::ModeSelection);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Simple);
        assert_eq!(state.selected_detail(), PlanningInitDetailSelection::Manual);
        assert!(state.simple_review().is_none());
    }

    #[test]
    fn returning_from_detail_selection_restores_simple_review_when_present() {
        let mut state = PlanningInitOverlayUiState::default();
        state.open_simple_review(PlanningInitStageResult {
            mode: PlanningBootstrapMode::Simple,
            draft_name: "bootstrap-1".to_string(),
            draft_directory: "/tmp/bootstrap-1".to_string(),
            staged_files: Vec::new(),
            staged_file_count: 4,
            validation_report: PlanningValidationReport::default(),
        });
        state.open_detail_selection();

        state.return_from_detail_selection();

        assert_eq!(state.step(), PlanningInitOverlayStep::SimpleReview);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Simple);
        assert_eq!(
            state.simple_review().map(|review| review.draft_name()),
            Some("bootstrap-1")
        );
    }
}
