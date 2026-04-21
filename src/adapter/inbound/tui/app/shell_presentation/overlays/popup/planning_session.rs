use crate::domain::planning::PlanningValidationReport;

use super::super::super::super::super::planning_draft_editor_ui::{
    PlanningDraftEditorBufferState, PlanningDraftEditorUiState,
};

pub(super) struct PlanningDraftEditorSessionView<'a> {
    pub(super) draft_name: &'a str,
    pub(super) draft_directory: &'a str,
    pub(super) buffers: &'a [PlanningDraftEditorBufferState],
    pub(super) selected_index: usize,
    pub(super) selected_buffer: &'a PlanningDraftEditorBufferState,
    pub(super) validation_report: &'a PlanningValidationReport,
    pub(super) dirty_labels: Vec<String>,
}

pub(super) fn collect_planning_draft_editor_session_view(
    ui_state: &PlanningDraftEditorUiState,
) -> Option<PlanningDraftEditorSessionView<'_>> {
    Some(PlanningDraftEditorSessionView {
        draft_name: ui_state.draft_name().unwrap_or("unknown"),
        draft_directory: ui_state.draft_directory().unwrap_or("unknown"),
        buffers: ui_state.buffers()?,
        selected_index: ui_state.selected_file_index()?,
        selected_buffer: ui_state.selected_buffer()?,
        validation_report: ui_state.validation_report()?,
        dirty_labels: ui_state.dirty_file_labels(),
    })
}

#[cfg(test)]
mod tests {
    use super::collect_planning_draft_editor_session_view;
    use crate::adapter::inbound::tui::app::planning_draft_editor_ui::PlanningDraftEditorUiState;
    use crate::application::service::planning::{
        PlanningDraftEditorFile, PlanningDraftEditorSession,
    };
    use crate::domain::planning::PlanningValidationReport;

    #[test]
    fn collect_returns_none_when_editor_session_is_missing() {
        let ui_state = PlanningDraftEditorUiState::default();

        assert!(collect_planning_draft_editor_session_view(&ui_state).is_none());
    }

    #[test]
    fn collect_returns_current_session_snapshot() {
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
            "planning/directions.toml"
        );
        assert!(session.validation_report.is_valid());
        assert!(session.dirty_labels.is_empty());
    }

    #[test]
    fn collect_includes_dirty_labels_after_edits() {
        let mut ui_state = PlanningDraftEditorUiState::default();
        ui_state.open_session(sample_session());
        ui_state.insert_character('#');

        let session = collect_planning_draft_editor_session_view(&ui_state)
            .expect("session view should exist");

        assert_eq!(session.dirty_labels, vec!["directions.toml".to_string()]);
    }

    fn sample_session() -> PlanningDraftEditorSession {
        PlanningDraftEditorSession {
            draft_name: "draft-123".to_string(),
            draft_directory: "/tmp/planning-draft".to_string(),
            editable_files: vec![
                PlanningDraftEditorFile {
                    active_path: "planning/directions.toml".to_string(),
                    staged_path: ".draft/planning/directions.toml".to_string(),
                    body: "title = \"Directions\"\n".to_string(),
                },
                PlanningDraftEditorFile {
                    active_path: "planning/task-ledger.json".to_string(),
                    staged_path: ".draft/planning/task-ledger.json".to_string(),
                    body: "{}\n".to_string(),
                },
            ],
            validation_report: PlanningValidationReport::default(),
        }
    }
}
