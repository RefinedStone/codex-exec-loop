/*
 * Planning draft editor session collection is the read-only bridge from the
 * mutable editor UI state into popup presentation. It does not re-run planning
 * validation or reinterpret editor buffers; it freezes the state already owned
 * by PlanningDraftEditorUiState so runtime guidance, file projection, and
 * status copy all read the same session snapshot.
 */
use crate::domain::planning::PlanningValidationReport;

use super::super::super::super::super::planning_draft_editor_ui::{
    PlanningDraftEditorBufferState, PlanningDraftEditorUiState,
};

/*
 * PlanningDraftEditorSessionView is the complete popup-facing slice of an open
 * manual planning draft editor session. planning_editor_surface fans these
 * fields out to the runtime next-action builder, the left file list, the right
 * editor panel, and the status/key copy builders.
 */
pub(super) struct PlanningDraftEditorSessionView<'a> {
    // Session identity shown in header/status copy; borrowed so the collector does not duplicate app state.
    pub(super) draft_name: &'a str,
    // Draft working directory, distinct from any individual active/staged file path.
    pub(super) draft_directory: &'a str,
    /*
     * All editable buffers in service-provided order. The selected index and
     * selected_buffer below must be read from this same UI-state snapshot so
     * list highlighting and editor body never describe different files.
     */
    pub(super) buffers: &'a [PlanningDraftEditorBufferState],
    // Zero-based file-list cursor used by projection; display copy can add one only at the final formatting layer.
    pub(super) selected_index: usize,
    // The buffer currently drawn by the editor panel, including path labels, body, cursor, and scroll.
    pub(super) selected_buffer: &'a PlanningDraftEditorBufferState,
    /*
     * Validation report comes from the session state after stage/save flows.
     * Keeping a borrowed report here ensures runtime guidance and status copy
     * agree about whether the staged planning draft is valid.
     */
    pub(super) validation_report: &'a PlanningValidationReport,
    /*
     * Dirty labels are derived once at the collector boundary. Runtime logic
     * only needs to know whether unsaved buffers exist, while status copy needs
     * the human labels; computing them here keeps both consumers aligned.
     */
    pub(super) dirty_labels: Vec<String>,
}

pub(super) fn collect_planning_draft_editor_session_view(
    ui_state: &PlanningDraftEditorUiState,
) -> Option<PlanningDraftEditorSessionView<'_>> {
    /*
     * The editor UI state is partial before a manual draft session is opened.
     * Treat every required accessor as part of one all-or-nothing presentation
     * contract: if any field is absent, the popup should not synthesize a
     * half-valid view.
     */
    Some(PlanningDraftEditorSessionView {
        // Missing draft name means there is no open editor session to identify.
        draft_name: ui_state.draft_name()?,
        // Directory is session-level metadata; without it the header would imply a false workspace context.
        draft_directory: ui_state.draft_directory()?,
        // Buffers feed both the file list and the editor panel, so an absent buffer slice aborts the whole view.
        buffers: ui_state.buffers()?,
        // Selection must come from the same state read as buffers; callers should not recompute it independently.
        selected_index: ui_state.selected_file_index()?,
        // The surface layer receives the selected buffer directly to avoid another index lookup after collection.
        selected_buffer: ui_state.selected_buffer()?,
        // Validation is session data, not a presentation default; absent validation means no reliable save guidance.
        validation_report: ui_state.validation_report()?,
        // Dirty labels are computed after required fields are present, making them part of the same display snapshot.
        dirty_labels: ui_state.dirty_file_labels(),
    })
}

#[cfg(test)]
mod tests {
    /*
     * These tests pin the collector contract before renderer involvement:
     * no session yields None, and an open session yields one coherent snapshot
     * that includes metadata, buffers, selected buffer, validation, and dirty
     * labels.
     */
    use super::collect_planning_draft_editor_session_view;
    use crate::adapter::inbound::tui::app::planning_draft_editor_ui::PlanningDraftEditorUiState;
    use crate::application::service::planning::{
        PlanningDraftEditorFile, PlanningDraftEditorSession,
    };
    use crate::domain::planning::PlanningValidationReport;

    #[test]
    fn collect_returns_none_when_editor_session_is_missing() {
        // Default UI state has no service-provided session payload, so popup surface creation must stop.
        let ui_state = PlanningDraftEditorUiState::default();

        assert!(collect_planning_draft_editor_session_view(&ui_state).is_none());
    }

    #[test]
    fn collect_returns_current_session_snapshot() {
        /*
         * Opening a session converts the application-service payload into UI
         * buffer state. The collector then borrows that converted state rather
         * than reconstructing files from the original service DTO.
         */
        let mut ui_state = PlanningDraftEditorUiState::default();
        ui_state.open_session(sample_session());

        let session = collect_planning_draft_editor_session_view(&ui_state)
            .expect("session view should exist");

        assert_eq!(session.draft_name, "draft-123");
        assert_eq!(session.draft_directory, "/tmp/planning-draft");
        assert_eq!(session.buffers.len(), 2);
        assert_eq!(session.selected_index, 0);
        assert_eq!(
            session.selected_buffer.active_path(),
            "planning/result-output.md"
        );
        assert!(session.validation_report.is_valid());
        assert!(session.dirty_labels.is_empty());
    }

    #[test]
    fn collect_includes_dirty_labels_after_edits() {
        /*
         * Dirty status is produced by real editor mutation, not by test-only
         * field injection. This verifies the path used by close guards,
         * runtime next-action text, and status dirty summaries.
         */
        let mut ui_state = PlanningDraftEditorUiState::default();
        ui_state.open_session(sample_session());
        ui_state.insert_character('#');

        let session = collect_planning_draft_editor_session_view(&ui_state)
            .expect("session view should exist");

        assert_eq!(session.dirty_labels, vec!["result-output.md".to_string()]);
    }

    fn sample_session() -> PlanningDraftEditorSession {
        /*
         * The fixture uses the real application-service session payload so the
         * collector test covers the same field mapping used when the TUI opens
         * a manual draft editor from planning controller results.
         */
        PlanningDraftEditorSession {
            draft_name: "draft-123".to_string(),
            draft_directory: "/tmp/planning-draft".to_string(),
            editable_files: vec![
                PlanningDraftEditorFile {
                    /*
                     * The first file is selected by default and anchors both
                     * selected_buffer assertions and dirty-label expectations.
                     */
                    active_path: "planning/result-output.md".to_string(),
                    staged_path: ".draft/planning/result-output.md".to_string(),
                    body: "title = \"Directions\"\n".to_string(),
                },
                PlanningDraftEditorFile {
                    /*
                     * The second file proves the collector carries the full
                     * buffer list, not just the currently selected editor body.
                     */
                    active_path: "planning/result-output.md".to_string(),
                    staged_path: ".draft/planning/result-output.md".to_string(),
                    body: "{}\n".to_string(),
                },
            ],
            validation_report: PlanningValidationReport::default(),
        }
    }
}
