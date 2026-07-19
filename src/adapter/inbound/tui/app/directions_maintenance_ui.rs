use crate::core::app::{
    DirectionsMaintenanceDirectionSnapshot, DirectionsMaintenanceLoadCorrelation,
    DirectionsMaintenanceSummarySnapshot, DirectionsSupportingFileStatus,
};

use super::{NativeTuiApp, ShellOverlay};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum DirectionsMaintenanceOverlayStep {
    #[default]
    Overview,
    DetailDocSelection,
    DetailDocConfirm,
    ManualEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum DetailDocConfirmChoice {
    #[default]
    Yes,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingDetailDocCreation {
    direction_id: String,
    direction_title: String,
}

impl PendingDetailDocCreation {
    pub fn direction_id(&self) -> &str {
        self.direction_id.as_str()
    }

    pub fn direction_title(&self) -> &str {
        self.direction_title.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum DirectionsMaintenanceProjectionState {
    #[default]
    Idle,
    Loading(DirectionsMaintenanceLoadCorrelation),
    Ready {
        correlation: DirectionsMaintenanceLoadCorrelation,
        summary: DirectionsMaintenanceSummarySnapshot,
    },
    Failed {
        correlation: DirectionsMaintenanceLoadCorrelation,
        error: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectionsMaintenanceProjectionKind {
    Idle,
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DirectionsMaintenanceScreenModel<'a> {
    Idle,
    Loading {
        workspace_directory: &'a str,
    },
    Ready {
        summary: &'a DirectionsMaintenanceSummarySnapshot,
    },
    Failed {
        workspace_directory: &'a str,
        error: &'a str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct DirectionsMaintenanceOverlayUiState {
    projection: DirectionsMaintenanceProjectionState,
    step: DirectionsMaintenanceOverlayStep,
    selected_missing_detail_doc_index: usize,
    pending_detail_doc_creation: Option<PendingDetailDocCreation>,
    detail_doc_confirm_choice: DetailDocConfirmChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectionsMaintenanceLoadCompletion {
    Applied,
    Ignored,
    ReloadRequired,
}

impl DirectionsMaintenanceOverlayUiState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn begin_load(&mut self, correlation: DirectionsMaintenanceLoadCorrelation) {
        self.projection = DirectionsMaintenanceProjectionState::Loading(correlation);
        self.reset_interaction();
    }

    pub(super) fn loading_request(
        &self,
        correlation: &DirectionsMaintenanceLoadCorrelation,
    ) -> Option<&DirectionsMaintenanceLoadCorrelation> {
        match &self.projection {
            DirectionsMaintenanceProjectionState::Loading(pending) if *pending == *correlation => {
                Some(pending)
            }
            DirectionsMaintenanceProjectionState::Idle
            | DirectionsMaintenanceProjectionState::Loading(_)
            | DirectionsMaintenanceProjectionState::Ready { .. }
            | DirectionsMaintenanceProjectionState::Failed { .. } => None,
        }
    }

    pub(super) fn apply_loaded(
        &mut self,
        correlation: DirectionsMaintenanceLoadCorrelation,
        result: Result<Box<DirectionsMaintenanceSummarySnapshot>, String>,
    ) -> bool {
        if self.loading_request(&correlation).is_none() {
            return false;
        }
        self.projection = match result {
            Ok(summary) => DirectionsMaintenanceProjectionState::Ready {
                correlation,
                summary: *summary,
            },
            Err(error) => DirectionsMaintenanceProjectionState::Failed { correlation, error },
        };
        self.reset_interaction();
        true
    }

    pub(super) fn projection_kind(&self) -> DirectionsMaintenanceProjectionKind {
        match &self.projection {
            DirectionsMaintenanceProjectionState::Idle => DirectionsMaintenanceProjectionKind::Idle,
            DirectionsMaintenanceProjectionState::Loading(_) => {
                DirectionsMaintenanceProjectionKind::Loading
            }
            DirectionsMaintenanceProjectionState::Ready { .. } => {
                DirectionsMaintenanceProjectionKind::Ready
            }
            DirectionsMaintenanceProjectionState::Failed { .. } => {
                DirectionsMaintenanceProjectionKind::Failed
            }
        }
    }

    pub(super) fn requires_load_for_workspace(&self, workspace_directory: &str) -> bool {
        if self.step == DirectionsMaintenanceOverlayStep::ManualEditor {
            return false;
        }
        match &self.projection {
            DirectionsMaintenanceProjectionState::Idle => true,
            DirectionsMaintenanceProjectionState::Loading(correlation)
            | DirectionsMaintenanceProjectionState::Ready { correlation, .. }
            | DirectionsMaintenanceProjectionState::Failed { correlation, .. } => {
                correlation.workspace_directory != workspace_directory
            }
        }
    }

    pub(super) fn authority_workspace_directory(&self) -> Option<&str> {
        match &self.projection {
            DirectionsMaintenanceProjectionState::Idle => None,
            DirectionsMaintenanceProjectionState::Loading(correlation)
            | DirectionsMaintenanceProjectionState::Ready { correlation, .. }
            | DirectionsMaintenanceProjectionState::Failed { correlation, .. } => {
                Some(correlation.workspace_directory.as_str())
            }
        }
    }

    pub(super) fn screen_model(&self) -> DirectionsMaintenanceScreenModel<'_> {
        match &self.projection {
            DirectionsMaintenanceProjectionState::Idle => DirectionsMaintenanceScreenModel::Idle,
            DirectionsMaintenanceProjectionState::Loading(correlation) => {
                DirectionsMaintenanceScreenModel::Loading {
                    workspace_directory: correlation.workspace_directory.as_str(),
                }
            }
            DirectionsMaintenanceProjectionState::Ready { summary, .. } => {
                DirectionsMaintenanceScreenModel::Ready { summary }
            }
            DirectionsMaintenanceProjectionState::Failed { correlation, error } => {
                DirectionsMaintenanceScreenModel::Failed {
                    workspace_directory: correlation.workspace_directory.as_str(),
                    error,
                }
            }
        }
    }

    pub fn step(&self) -> DirectionsMaintenanceOverlayStep {
        self.step
    }

    pub fn summary(&self) -> Option<&DirectionsMaintenanceSummarySnapshot> {
        match &self.projection {
            DirectionsMaintenanceProjectionState::Ready { summary, .. } => Some(summary),
            DirectionsMaintenanceProjectionState::Idle
            | DirectionsMaintenanceProjectionState::Loading(_)
            | DirectionsMaintenanceProjectionState::Failed { .. } => None,
        }
    }

    pub fn actionable_detail_doc_directions(&self) -> Vec<&DirectionsMaintenanceDirectionSnapshot> {
        self.summary()
            .map(|summary| {
                summary
                    .directions
                    .iter()
                    .filter(|direction| {
                        direction.detail_doc_status != DirectionsSupportingFileStatus::Ready
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn selected_actionable_detail_doc_direction(
        &self,
    ) -> Option<&DirectionsMaintenanceDirectionSnapshot> {
        let directions = self.actionable_detail_doc_directions();
        directions
            .get(
                self.selected_missing_detail_doc_index
                    .min(directions.len().saturating_sub(1)),
            )
            .copied()
    }

    pub fn open_detail_doc_selection(&mut self) {
        self.step = DirectionsMaintenanceOverlayStep::DetailDocSelection;
        self.selected_missing_detail_doc_index = 0;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    pub fn return_to_overview(&mut self) {
        self.step = DirectionsMaintenanceOverlayStep::Overview;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    pub fn move_missing_detail_doc_selection(&mut self, delta: isize) {
        let directions = self.actionable_detail_doc_directions();
        if directions.is_empty() {
            self.selected_missing_detail_doc_index = 0;
            return;
        }
        let max_index = directions.len().saturating_sub(1) as isize;
        self.selected_missing_detail_doc_index =
            (self.selected_missing_detail_doc_index as isize + delta).clamp(0, max_index) as usize;
    }

    pub fn open_detail_doc_confirm(&mut self) {
        let Some(direction) = self.selected_actionable_detail_doc_direction() else {
            return;
        };
        self.pending_detail_doc_creation = Some(PendingDetailDocCreation {
            direction_id: direction.id.clone(),
            direction_title: direction.title.clone(),
        });
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
        self.step = DirectionsMaintenanceOverlayStep::DetailDocConfirm;
    }

    pub fn pending_detail_doc_creation(&self) -> Option<&PendingDetailDocCreation> {
        self.pending_detail_doc_creation.as_ref()
    }

    pub fn detail_doc_confirm_choice(&self) -> DetailDocConfirmChoice {
        self.detail_doc_confirm_choice
    }

    pub fn move_detail_doc_confirm_choice(&mut self, delta: isize) {
        self.detail_doc_confirm_choice = match (self.detail_doc_confirm_choice, delta.is_negative())
        {
            (DetailDocConfirmChoice::Yes, false) => DetailDocConfirmChoice::No,
            (DetailDocConfirmChoice::No, true) => DetailDocConfirmChoice::Yes,
            (choice, _) => choice,
        };
    }

    pub fn open_manual_editor(&mut self) {
        self.step = DirectionsMaintenanceOverlayStep::ManualEditor;
    }

    fn reset_interaction(&mut self) {
        self.step = DirectionsMaintenanceOverlayStep::Overview;
        self.selected_missing_detail_doc_index = 0;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    #[cfg(test)]
    pub(super) fn open_summary(&mut self, summary: DirectionsMaintenanceSummarySnapshot) {
        let correlation = DirectionsMaintenanceLoadCorrelation::new(0, "/test");
        self.begin_load(correlation.clone());
        assert!(self.apply_loaded(correlation, Ok(Box::new(summary))));
    }
}

impl NativeTuiApp {
    pub(super) fn directions_editor_workspace_is_current(&self) -> bool {
        self.directions_maintenance_overlay_ui_state
            .authority_workspace_directory()
            .is_some_and(|workspace| workspace == self.planning_workspace_directory())
    }

    pub(super) fn directions_maintenance_load_required(&self) -> bool {
        self.shell_overlay == ShellOverlay::DirectionsMaintenance
            && self
                .directions_maintenance_overlay_ui_state
                .requires_load_for_workspace(self.planning_workspace_directory().as_str())
    }

    pub(super) fn apply_directions_maintenance_loaded(
        &mut self,
        correlation: DirectionsMaintenanceLoadCorrelation,
        result: Result<Box<DirectionsMaintenanceSummarySnapshot>, String>,
    ) -> DirectionsMaintenanceLoadCompletion {
        let directions_is_visible = self.shell_overlay == ShellOverlay::DirectionsMaintenance;
        let directions_is_suspended_for_approval = self.shell_overlay == ShellOverlay::Approval
            && self.approval_return_overlay == Some(ShellOverlay::DirectionsMaintenance);
        if !directions_is_visible && !directions_is_suspended_for_approval {
            return DirectionsMaintenanceLoadCompletion::Ignored;
        }
        if self
            .directions_maintenance_overlay_ui_state
            .loading_request(&correlation)
            .is_none()
        {
            return DirectionsMaintenanceLoadCompletion::Ignored;
        }
        if correlation.workspace_directory != self.planning_workspace_directory() {
            return DirectionsMaintenanceLoadCompletion::ReloadRequired;
        }
        let applied = self
            .directions_maintenance_overlay_ui_state
            .apply_loaded(correlation, result);
        debug_assert!(applied);
        DirectionsMaintenanceLoadCompletion::Applied
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::test_native_tui_app;
    use super::super::{ConversationState, ShellChromeEvent};
    use super::*;
    use crate::core::app::AppEvent;
    use crate::domain::planning::QueueIdlePolicy;

    fn summary() -> DirectionsMaintenanceSummarySnapshot {
        DirectionsMaintenanceSummarySnapshot {
            directions: Vec::new(),
            missing_detail_doc_count: 0,
            broken_detail_doc_count: 0,
            queue_idle_policy: QueueIdlePolicy::Stop,
            queue_idle_prompt_path: None,
            queue_idle_prompt_status: DirectionsSupportingFileStatus::MissingMapping,
            parse_error: None,
        }
    }

    fn sync_draft_workspace(app: &mut NativeTuiApp, workspace_directory: &str) {
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should have a ready conversation");
        };
        assert!(conversation.sync_draft_workspace(workspace_directory.to_string()));
    }

    #[test]
    fn loading_accepts_only_the_exact_pending_completion_once() {
        let mut state = DirectionsMaintenanceOverlayUiState::default();
        let pending = DirectionsMaintenanceLoadCorrelation::new(2, "/workspace");
        state.begin_load(pending.clone());

        for stale in [
            DirectionsMaintenanceLoadCorrelation::new(1, "/workspace"),
            DirectionsMaintenanceLoadCorrelation::new(2, "/other"),
        ] {
            assert!(!state.apply_loaded(stale, Ok(Box::new(summary()))));
        }
        assert_eq!(
            state.projection_kind(),
            DirectionsMaintenanceProjectionKind::Loading
        );

        assert!(state.apply_loaded(pending.clone(), Ok(Box::new(summary()))));
        assert_eq!(
            state.projection_kind(),
            DirectionsMaintenanceProjectionKind::Ready
        );
        assert!(!state.apply_loaded(pending, Ok(Box::new(summary()))));
    }

    #[test]
    fn failed_load_retries_only_after_the_workspace_changes() {
        let mut app = test_native_tui_app();
        app.dispatch_shell_chrome(ShellChromeEvent::DirectionsMaintenanceOverlayShown);
        let workspace_directory = app.planning_workspace_directory();
        let failed = DirectionsMaintenanceLoadCorrelation::new(1, workspace_directory.clone());
        app.directions_maintenance_overlay_ui_state
            .begin_load(failed.clone());

        assert_eq!(
            app.apply_directions_maintenance_loaded(
                failed,
                Err("authority unavailable".to_string()),
            ),
            DirectionsMaintenanceLoadCompletion::Applied
        );
        assert!(matches!(
            app.directions_maintenance_overlay_ui_state.screen_model(),
            DirectionsMaintenanceScreenModel::Failed {
                error: "authority unavailable",
                ..
            }
        ));
        assert!(!app.directions_maintenance_load_required());

        sync_draft_workspace(&mut app, "/different-workspace");

        assert!(app.directions_maintenance_load_required());
    }

    #[test]
    fn closing_the_overlay_invalidates_a_late_completion() {
        let mut app = test_native_tui_app();
        app.dispatch_shell_chrome(ShellChromeEvent::DirectionsMaintenanceOverlayShown);
        let correlation =
            DirectionsMaintenanceLoadCorrelation::new(1, app.planning_workspace_directory());
        app.directions_maintenance_overlay_ui_state
            .begin_load(correlation.clone());

        app.close_shell_overlay();

        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            app.apply_directions_maintenance_loaded(correlation, Ok(Box::new(summary()))),
            DirectionsMaintenanceLoadCompletion::Ignored
        );
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Idle
        );
    }

    #[test]
    fn approval_suspension_accepts_the_exact_loading_completion_before_restore() {
        let mut app = test_native_tui_app();
        app.dispatch_shell_chrome(ShellChromeEvent::DirectionsMaintenanceOverlayShown);
        let correlation =
            DirectionsMaintenanceLoadCorrelation::new(1, app.planning_workspace_directory());
        app.directions_maintenance_overlay_ui_state
            .begin_load(correlation.clone());

        app.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown);
        app.apply_core_event(AppEvent::DirectionsMaintenanceLoaded {
            correlation,
            result: Ok(Box::new(summary())),
        });

        assert_eq!(app.shell_overlay, ShellOverlay::Approval);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Ready
        );

        app.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayClosed);

        assert_eq!(app.shell_overlay, ShellOverlay::DirectionsMaintenance);
        assert_eq!(
            app.directions_maintenance_overlay_ui_state
                .projection_kind(),
            DirectionsMaintenanceProjectionKind::Ready
        );
    }

    #[test]
    fn exact_completion_for_a_drifted_workspace_requires_reload() {
        let mut app = test_native_tui_app();
        app.dispatch_shell_chrome(ShellChromeEvent::DirectionsMaintenanceOverlayShown);
        let correlation =
            DirectionsMaintenanceLoadCorrelation::new(1, app.planning_workspace_directory());
        app.directions_maintenance_overlay_ui_state
            .begin_load(correlation.clone());
        sync_draft_workspace(&mut app, "/different-workspace");

        assert_eq!(
            app.apply_directions_maintenance_loaded(correlation.clone(), Ok(Box::new(summary())),),
            DirectionsMaintenanceLoadCompletion::ReloadRequired
        );
        assert!(
            app.directions_maintenance_overlay_ui_state
                .loading_request(&correlation)
                .is_some()
        );
    }

    #[test]
    fn manual_editor_suppresses_automatic_workspace_reload() {
        let mut app = test_native_tui_app();
        app.dispatch_shell_chrome(ShellChromeEvent::DirectionsMaintenanceOverlayShown);
        let correlation =
            DirectionsMaintenanceLoadCorrelation::new(1, app.planning_workspace_directory());
        app.directions_maintenance_overlay_ui_state
            .begin_load(correlation.clone());
        assert!(
            app.directions_maintenance_overlay_ui_state
                .apply_loaded(correlation, Ok(Box::new(summary())))
        );
        app.directions_maintenance_overlay_ui_state
            .open_manual_editor();
        sync_draft_workspace(&mut app, "/different-workspace");

        assert!(!app.directions_maintenance_load_required());
    }
}
