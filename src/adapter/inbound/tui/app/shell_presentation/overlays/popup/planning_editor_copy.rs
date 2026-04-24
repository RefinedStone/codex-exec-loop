use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::super::{AkraTheme, Line, PlanningValidationSeverity, Span};
use super::copy::{PlanningDraftEditorStatusCopy, planning_draft_title_line};

pub(super) fn build_planning_draft_editor_header_lines(
    draft_directory: &str,
) -> Vec<Line<'static>> {
    vec![
        planning_draft_title_line(" / operator inspection"),
        Line::from(format!("draft dir: {draft_directory}")),
    ]
}

pub(super) fn build_planning_draft_editor_status_lines(
    copy: PlanningDraftEditorStatusCopy,
) -> Vec<Line<'static>> {
    let mut status_lines = vec![
        Line::from(format!("staged draft: {}", copy.draft_name)),
        Line::from(format!(
            "current file: {} ({}/{})",
            copy.active_path, copy.selected_file_position, copy.file_count
        )),
        Line::from(vec![
            Span::styled("validation state: ", AkraTheme::muted()),
            Span::styled(
                if copy.validation_ok {
                    "ok"
                } else {
                    "needs attention"
                },
                if copy.validation_ok {
                    AkraTheme::success()
                } else {
                    AkraTheme::warning()
                },
            ),
        ]),
    ];
    if let Some(issue) = copy.first_issue {
        status_lines.push(Line::from(vec![
            Span::styled(
                match issue.severity {
                    PlanningValidationSeverity::Error => "error: ",
                    PlanningValidationSeverity::Warning => "warning: ",
                },
                match issue.severity {
                    PlanningValidationSeverity::Error => AkraTheme::danger(),
                    PlanningValidationSeverity::Warning => AkraTheme::warning(),
                },
            ),
            Span::raw(issue.detail),
        ]));
    } else {
        status_lines.push(Line::from(format!(
            "staged path: {}",
            copy.staged_path_summary
        )));
    }
    status_lines.push(Line::from(format!("dirty: {}", copy.dirty_label_summary)));
    if copy.has_dirty_labels {
        status_lines.push(Line::from(
            "validation note: the status above reflects the last saved draft until Ctrl+S re-runs checks",
        ));
    }
    status_lines.push(Line::from(copy.next_action));
    if let Some(risk) = copy.close_risk {
        status_lines.push(Line::from(vec![
            Span::styled(
                if copy.confirmation_pending {
                    "close pending: "
                } else {
                    "close guard: "
                },
                if copy.confirmation_pending {
                    AkraTheme::danger()
                } else {
                    AkraTheme::warning()
                },
            ),
            Span::raw(planning_draft_close_guard_detail(
                risk,
                copy.confirmation_pending,
            )),
        ]));
    }
    status_lines
}

pub(super) fn build_planning_draft_editor_key_lines(
    close_risk: Option<PlanningDraftEditorCloseRisk>,
    confirmation_pending: bool,
) -> Vec<Line<'static>> {
    vec![
        Line::from("controls: Tab/BackTab switches files  |  arrows move the cursor"),
        Line::from(
            "controls: Enter inserts newline  |  Backspace deletes  |  Ctrl+W deletes the previous word",
        ),
        Line::from(
            "controls: Ctrl+S saves and validates  |  Ctrl+P saves and promotes active planning",
        ),
        planning_draft_editor_close_key_line(close_risk, confirmation_pending),
    ]
}

fn planning_draft_close_guard_detail(
    risk: PlanningDraftEditorCloseRisk,
    confirmation_pending: bool,
) -> &'static str {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
        confirmation_pending,
    ) {
        (true, true, true) => {
            "discard unsaved edits or keep editing; the invalid staged draft will remain on disk"
        }
        (true, false, true) => "discard unsaved edits or press n to keep editing",
        (false, true, true) => {
            "close now or press n to keep editing; the invalid staged draft will remain on disk"
        }
        (true, true, false) => {
            "unsaved edits and an invalid staged draft require confirmation before close"
        }
        (true, false, false) => "unsaved edits require confirmation before close",
        (false, true, false) => "an invalid staged draft requires confirmation before close",
        (false, false, _) => "close is available immediately",
    }
}

fn planning_draft_editor_close_key_line(
    close_risk: Option<PlanningDraftEditorCloseRisk>,
    confirmation_pending: bool,
) -> Line<'static> {
    if confirmation_pending {
        return Line::from("controls: Enter, Esc, or Ctrl+C confirms close  |  n keeps editing");
    }

    if close_risk.is_some() {
        return Line::from("controls: Esc/Ctrl+C reviews close");
    }

    Line::from("controls: Esc/Ctrl+C closes this surface")
}
