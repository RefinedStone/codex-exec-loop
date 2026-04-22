use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::super::{
    Color, Line, Modifier, PlanningInitDetailSelection, PlanningInitModeSelection,
    PlanningValidationSeverity, Span, Style,
};
use super::super::super::option_lines::overlay_option_line;
use super::PlanningInitOverlayView;

pub(super) struct PlanningExistingWorkspaceCopy<'a> {
    pub(super) workspace_directory: &'a str,
    pub(super) plan_state_label: &'a str,
    pub(super) queue_summary: &'a str,
    pub(super) queue_idle_policy: &'a str,
    pub(super) failure_summary: Option<&'a str>,
    pub(super) plan_enabled: bool,
}

pub(super) struct PlanningSimpleReviewCopy<'a> {
    pub(super) draft_name: &'a str,
    pub(super) staged_file_count: usize,
    pub(super) validation_ok: bool,
    pub(super) first_error: Option<&'a str>,
    pub(super) max_auto_turns_label: &'a str,
    pub(super) is_turn_budget_editing: bool,
    pub(super) turn_budget_buffer: &'a str,
}

pub(super) struct PlanningDraftEditorIssueCopy {
    pub(super) severity: PlanningValidationSeverity,
    pub(super) detail: String,
}

pub(super) struct PlanningDraftEditorStatusCopy<'a> {
    pub(super) draft_name: &'a str,
    pub(super) active_path: &'a str,
    pub(super) selected_file_position: usize,
    pub(super) file_count: usize,
    pub(super) validation_ok: bool,
    pub(super) first_issue: Option<PlanningDraftEditorIssueCopy>,
    pub(super) staged_path_summary: String,
    pub(super) dirty_label_summary: String,
    pub(super) has_dirty_labels: bool,
    pub(super) next_action: &'static str,
    pub(super) close_risk: Option<PlanningDraftEditorCloseRisk>,
    pub(super) confirmation_pending: bool,
}

fn planning_setup_title_line(suffix: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "Planning Setup",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(suffix),
    ])
}

fn planning_draft_title_line(suffix: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "Planning Draft",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(suffix),
    ])
}

pub(super) fn build_existing_workspace_overlay_view(
    copy: PlanningExistingWorkspaceCopy<'_>,
) -> PlanningInitOverlayView {
    let mut status_lines = if copy.plan_enabled {
        vec![
            Line::from("Enter opens queue inspection for the existing planning workspace."),
            Line::from("Press D to maintain directions, or O to turn Plan off."),
        ]
    } else {
        vec![
            Line::from("Enter turns Plan on and resumes the existing planning workspace."),
            Line::from("Directions maintenance stays blocked while Plan off."),
        ]
    };
    if let Some(failure_summary) = copy.failure_summary {
        status_lines.push(Line::from(format!("planning failure: {failure_summary}")));
    }

    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / existing workspace"),
            Line::from(
                "This workspace already has active planning files. Manage the current runtime instead of restaging a bootstrap scaffold.",
            ),
        ],
        summary_lines: vec![
            Line::from(
                "Use :directions only after Plan on. Hidden planner sessions still update task-ledger.json only.",
            ),
            Line::from(
                "Turning Plan off keeps the workspace files on disk and blocks directions maintenance until planning resumes.",
            ),
        ],
        option_lines: vec![
            Line::from(format!("workspace: {}", copy.workspace_directory)),
            Line::from(format!("planning state: {}", copy.plan_state_label)),
            Line::from(format!("queue state: {}", copy.queue_summary)),
            Line::from(format!("queue idle policy: {}", copy.queue_idle_policy)),
        ],
        status_lines,
        key_lines: vec![
            Line::from("Enter opens queue inspection or resumes Plan on."),
            Line::from("Q opens queue inspection. D opens directions maintenance."),
            Line::from("O toggles Plan on or off. Esc/Ctrl+C closes this surface."),
        ],
    }
}

pub(super) fn build_mode_selection_overlay_view(
    selected_mode: PlanningInitModeSelection,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Pick the planning entry path before any files are staged."),
        ],
        summary_lines: vec![
            Line::from(
                "Every guided path stages a promotable draft before active planning changes.",
            ),
            Line::from(
                "Simple mode keeps one generic active direction; detail mode prepares richer direction authoring.",
            ),
        ],
        option_lines: vec![
            overlay_option_line(
                "A",
                "simple mode",
                "stage one generic direction and an empty task ledger",
                selected_mode == PlanningInitModeSelection::Simple,
                false,
            ),
            overlay_option_line(
                "B",
                "detail mode",
                "branch into manual or future llm-assisted authoring",
                selected_mode == PlanningInitModeSelection::Detail,
                false,
            ),
        ],
        status_lines: vec![
            Line::from(format!(
                "current selection: {}",
                match selected_mode {
                    PlanningInitModeSelection::Simple => "simple mode",
                    PlanningInitModeSelection::Detail => "detail mode",
                }
            )),
            Line::from("simple mode is the low-ceremony path for planning-aware execution."),
        ],
        key_lines: vec![
            Line::from("A/B or arrows move selection."),
            Line::from("Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}

pub(super) fn build_detail_selection_overlay_view(
    selected_detail: PlanningInitDetailSelection,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Current step: choose how detail-mode drafts should be prepared."),
        ],
        summary_lines: vec![
            Line::from("Manual opens the staged draft editor inside the shell."),
            Line::from("LLM-assisted remains visible for the target UX but is still disabled."),
        ],
        option_lines: vec![
            overlay_option_line(
                "A",
                "manual",
                "stage the detail scaffold and keep editing inside the shell",
                selected_detail == PlanningInitDetailSelection::Manual,
                false,
            ),
            overlay_option_line(
                "B",
                "llm-assisted",
                "future guided drafting flow (not supported yet)",
                selected_detail == PlanningInitDetailSelection::LlmAssisted,
                true,
            ),
        ],
        status_lines: vec![
            Line::from(format!(
                "current selection: {}",
                match selected_detail {
                    PlanningInitDetailSelection::Manual => "manual",
                    PlanningInitDetailSelection::LlmAssisted => "llm-assisted (disabled)",
                }
            )),
            Line::from("Enter on manual opens the embedded draft editor."),
        ],
        key_lines: vec![
            Line::from("A/B or arrows move selection."),
            Line::from("Backspace/Left goes back. Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy<'_>,
) -> PlanningInitOverlayView {
    let mut status_lines = vec![
        Line::from(format!(
            "validation state: {}",
            if copy.validation_ok {
                "ok"
            } else {
                "needs attention"
            }
        )),
        Line::from(format!("turn budget: {}", copy.max_auto_turns_label)),
    ];
    if copy.is_turn_budget_editing {
        status_lines.push(Line::from(format!(
            "current state: editing turn budget / value: {} / controls: Enter saves, Esc/Ctrl+C cancels",
            copy.turn_budget_buffer
        )));
    } else {
        status_lines.push(Line::from(
            "next action: Enter or Ctrl+P promotes the staged simple scaffold.",
        ));
        status_lines.push(Line::from(
            "alternate action: Esc closes this review and leaves the staged draft on disk.",
        ));
        status_lines.push(Line::from(
            "advanced action: D opens detail-mode authoring without promoting the simple scaffold.",
        ));
    }
    if let Some(first_error) = copy.first_error {
        status_lines.push(Line::from(format!("first validation error: {first_error}")));
    }

    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from(
                "Simple mode review: promote the lightest planning baseline before you invest in richer authoring.",
            ),
        ],
        summary_lines: vec![
            Line::from(
                "After promote, planning starts with one generic direction and no active queue task yet.",
            ),
            Line::from(
                "The default queue-idle review prompt is already staged so the first reply can justify follow-up work when needed.",
            ),
            Line::from("No active planning files change until you explicitly promote this review."),
        ],
        option_lines: vec![
            Line::from(format!("staged draft: {}", copy.draft_name)),
            Line::from(format!(
                "reviewed artifacts: {} staged planning files",
                copy.staged_file_count
            )),
            Line::from(
                "promote outcome: generic direction catalog, empty task ledger, and default queue-idle review prompt",
            ),
            Line::from(
                "advanced path: press D to branch into detail-mode authoring instead of promoting the simple scaffold",
            ),
        ],
        status_lines,
        key_lines: if copy.is_turn_budget_editing {
            vec![
                Line::from("next action: type the new turn budget directly."),
                Line::from("controls: Enter saves  |  Esc/Ctrl+C cancels  |  Backspace deletes"),
                Line::from("validation: use a whole number greater than 0, or type infinite."),
            ]
        } else {
            vec![
                Line::from("Enter or Ctrl+P promotes the staged scaffold."),
                Line::from(
                    "D opens detail-mode authoring. Ctrl+L edits turn budget. Ctrl+E inspects or edits the draft.",
                ),
                Line::from("Esc/Ctrl+C closes this review."),
            ]
        },
    }
}

pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_draft_title_line(" / operator inspection"),
            Line::from("Edit the staged planning draft and save to re-run validation."),
        ],
        summary_lines: vec![Line::from(
            "This state renders through the dedicated planning draft editor view.",
        )],
        option_lines: vec![Line::from(
            "next action: Tab switches files. Ctrl+S saves and re-runs validation.",
        )],
        status_lines: vec![Line::from(
            "current state: editing the staged planning draft",
        )],
        key_lines: vec![Line::from("Esc/Ctrl+C closes this surface.")],
    }
}

pub(super) fn build_planning_draft_editor_header_lines(
    draft_directory: &str,
) -> Vec<Line<'static>> {
    vec![
        planning_draft_title_line(" / operator inspection"),
        Line::from(format!("draft dir: {draft_directory}")),
    ]
}

pub(super) fn build_planning_draft_editor_status_lines(
    copy: PlanningDraftEditorStatusCopy<'_>,
) -> Vec<Line<'static>> {
    let mut status_lines = vec![
        Line::from(format!("staged draft: {}", copy.draft_name)),
        Line::from(format!(
            "current file: {} ({}/{})",
            copy.active_path, copy.selected_file_position, copy.file_count
        )),
        Line::from(vec![
            Span::styled("validation state: ", Style::default().fg(Color::Gray)),
            Span::styled(
                if copy.validation_ok {
                    "ok"
                } else {
                    "needs attention"
                },
                Style::default().fg(if copy.validation_ok {
                    Color::Green
                } else {
                    Color::Yellow
                }),
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
                Style::default().fg(match issue.severity {
                    PlanningValidationSeverity::Error => Color::Red,
                    PlanningValidationSeverity::Warning => Color::Yellow,
                }),
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
                Style::default().fg(if copy.confirmation_pending {
                    Color::Red
                } else {
                    Color::Yellow
                }),
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
) -> String {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
        confirmation_pending,
    ) {
        (true, true, true) => {
            "discard unsaved edits or keep editing; the invalid staged draft will remain on disk"
                .to_string()
        }
        (true, false, true) => "discard unsaved edits or press n to keep editing".to_string(),
        (false, true, true) => {
            "close now or press n to keep editing; the invalid staged draft will remain on disk"
                .to_string()
        }
        (true, true, false) => {
            "unsaved edits and an invalid staged draft require confirmation before close"
                .to_string()
        }
        (true, false, false) => "unsaved edits require confirmation before close".to_string(),
        (false, true, false) => {
            "an invalid staged draft requires confirmation before close".to_string()
        }
        (false, false, _) => "close is available immediately".to_string(),
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
