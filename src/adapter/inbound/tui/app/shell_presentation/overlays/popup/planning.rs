use super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::status_panels::plan_runtime_substate_label;
use super::super::super::{
    Color, ConversationState, FOOTER_NOTICE_DETAIL_LIMIT, Line, Modifier, NativeTuiApp,
    PlanningInitDetailSelection, PlanningInitModeSelection, PlanningInitOverlayStep,
    PlanningValidationSeverity, Span, Style, compact_inline_detail,
};
use super::super::{PlanningDraftEditorOverlayView, PlanningInitOverlayView};

pub(crate) fn build_planning_init_overlay_view(app: &NativeTuiApp) -> PlanningInitOverlayView {
    match app.planning_init_overlay_ui_state.step() {
        PlanningInitOverlayStep::ExistingWorkspace => {
            let workspace_directory = app.planning_workspace_directory();
            let snapshot = match &app.conversation_state {
                ConversationState::Ready(conversation) => {
                    conversation.planning_runtime_snapshot.clone()
                }
                ConversationState::Loading | ConversationState::Failed(_) => {
                    app.load_planning_runtime_snapshot(&workspace_directory)
                }
            };
            let plan_state_label = if snapshot.plan_enabled() {
                format!("Plan on / {}", plan_runtime_substate_label(&snapshot))
            } else {
                "Plan off".to_string()
            };
            let queue_summary = snapshot
                .queue_summary()
                .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT))
                .unwrap_or_else(|| "queue state unavailable".to_string());
            let failure_summary = snapshot
                .failure_reason()
                .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT));
            let mut status_lines = if snapshot.plan_enabled() {
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

            PlanningInitOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Planning Setup",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / existing workspace"),
                    ]),
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
                    Line::from(format!("workspace: {workspace_directory}")),
                    Line::from(format!("planning state: {plan_state_label}")),
                    Line::from(format!("queue state: {queue_summary}")),
                    Line::from(format!(
                        "queue idle policy: {}",
                        snapshot.queue_idle_policy().label()
                    )),
                ],
                status_lines: {
                    if let Some(failure_summary) = failure_summary {
                        status_lines
                            .push(Line::from(format!("planning failure: {failure_summary}")));
                    }
                    status_lines
                },
                key_lines: vec![
                    Line::from("Enter opens queue inspection or resumes Plan on."),
                    Line::from("Q opens queue inspection. D opens directions maintenance."),
                    Line::from("O toggles Plan on or off. Esc/Ctrl+C closes this surface."),
                ],
            }
        }
        PlanningInitOverlayStep::ModeSelection => PlanningInitOverlayView {
            header_lines: vec![
                Line::from(vec![
                    Span::styled(
                        "Planning Setup",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" / operator inspection"),
                ]),
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
                planning_init_option_line(
                    "A",
                    "simple mode",
                    "stage one generic direction and an empty task ledger",
                    app.planning_init_overlay_ui_state.selected_mode()
                        == PlanningInitModeSelection::Simple,
                    false,
                ),
                planning_init_option_line(
                    "B",
                    "detail mode",
                    "branch into manual or future llm-assisted authoring",
                    app.planning_init_overlay_ui_state.selected_mode()
                        == PlanningInitModeSelection::Detail,
                    false,
                ),
            ],
            status_lines: vec![
                Line::from(format!(
                    "current selection: {}",
                    match app.planning_init_overlay_ui_state.selected_mode() {
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
        },
        PlanningInitOverlayStep::DetailSelection => PlanningInitOverlayView {
            header_lines: vec![
                Line::from(vec![
                    Span::styled(
                        "Planning Setup",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" / operator inspection"),
                ]),
                Line::from("Current step: choose how detail-mode drafts should be prepared."),
            ],
            summary_lines: vec![
                Line::from("Manual opens the staged draft editor inside the shell."),
                Line::from("LLM-assisted remains visible for the target UX but is still disabled."),
            ],
            option_lines: vec![
                planning_init_option_line(
                    "A",
                    "manual",
                    "stage the detail scaffold and keep editing inside the shell",
                    app.planning_init_overlay_ui_state.selected_detail()
                        == PlanningInitDetailSelection::Manual,
                    false,
                ),
                planning_init_option_line(
                    "B",
                    "llm-assisted",
                    "future guided drafting flow (not supported yet)",
                    app.planning_init_overlay_ui_state.selected_detail()
                        == PlanningInitDetailSelection::LlmAssisted,
                    true,
                ),
            ],
            status_lines: vec![
                Line::from(format!(
                    "current selection: {}",
                    match app.planning_init_overlay_ui_state.selected_detail() {
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
        },
        PlanningInitOverlayStep::SimpleReview => {
            let simple_review = app.planning_init_overlay_ui_state.simple_review();
            let draft_name = simple_review
                .map(|review| review.draft_name().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let staged_file_count = simple_review
                .map(|review| review.staged_file_count())
                .unwrap_or_default();
            let validation_report = simple_review.map(|review| review.validation_report());
            let validation_ok = validation_report.is_none_or(|report| report.is_valid());
            let first_error = validation_report
                .and_then(|report| report.errors().into_iter().next())
                .map(|issue| {
                    compact_inline_detail(issue.message.as_str(), FOOTER_NOTICE_DETAIL_LIMIT)
                });

            PlanningInitOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Planning Setup",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / operator inspection"),
                    ]),
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
                    Line::from(
                        "No active planning files change until you explicitly promote this review.",
                    ),
                ],
                option_lines: vec![
                    Line::from(format!("staged draft: {draft_name}")),
                    Line::from(format!(
                        "reviewed artifacts: {staged_file_count} staged planning files"
                    )),
                    Line::from(
                        "promote outcome: generic direction catalog, empty task ledger, and default queue-idle review prompt",
                    ),
                    Line::from(
                        "advanced path: press D to branch into detail-mode authoring instead of promoting the simple scaffold",
                    ),
                ],
                status_lines: {
                    let mut lines = vec![
                        Line::from(format!(
                            "validation state: {}",
                            if validation_ok {
                                "ok"
                            } else {
                                "needs attention"
                            }
                        )),
                        Line::from(format!(
                            "turn budget: {}",
                            app.current_max_auto_turns_label()
                        )),
                    ];
                    if app.is_max_auto_turns_editing() {
                        lines.push(Line::from(format!(
                            "current state: editing turn budget / value: {} / controls: Enter saves, Esc/Ctrl+C cancels",
                            app.followup_overlay_ui_state.max_auto_turns_editor.buffer
                        )));
                    } else {
                        lines.push(Line::from(
                            "next action: Enter or Ctrl+P promotes the staged simple scaffold.",
                        ));
                        lines.push(Line::from(
                            "alternate action: Esc closes this review and leaves the staged draft on disk.",
                        ));
                        lines.push(Line::from(
                            "advanced action: D opens detail-mode authoring without promoting the simple scaffold.",
                        ));
                    }
                    if let Some(first_error) = first_error {
                        lines.push(Line::from(format!("first validation error: {first_error}")));
                    }
                    lines
                },
                key_lines: if app.is_max_auto_turns_editing() {
                    vec![
                        Line::from("next action: type the new turn budget directly."),
                        Line::from(
                            "controls: Enter saves  |  Esc/Ctrl+C cancels  |  Backspace deletes",
                        ),
                        Line::from(
                            "validation: use a whole number greater than 0, or type infinite.",
                        ),
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
        PlanningInitOverlayStep::ManualEditor => PlanningInitOverlayView {
            header_lines: vec![
                Line::from(vec![
                    Span::styled(
                        "Planning Draft",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" / operator inspection"),
                ]),
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
        },
    }
}

pub(crate) fn build_planning_draft_editor_overlay_view(
    app: &NativeTuiApp,
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    let buffers = app.planning_draft_editor_ui_state.buffers()?;
    let selected_index = app.planning_draft_editor_ui_state.selected_file_index()?;
    let selected_buffer = app.planning_draft_editor_ui_state.selected_buffer()?;
    let dirty_labels = app.planning_draft_editor_ui_state.dirty_file_labels();
    let validation_report = app.planning_draft_editor_ui_state.validation_report()?;
    let pending_close_risk = app.planning_draft_editor_ui_state.pending_close_risk();
    let close_risk = pending_close_risk.or_else(|| app.planning_draft_editor_ui_state.close_risk());
    let next_action = if !dirty_labels.is_empty() {
        "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
    } else if validation_report.is_valid() {
        "next action: Ctrl+P promotes this draft into active planning files"
    } else {
        "next action: fix validation errors before promoting this draft"
    };

    let file_lines = buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| {
            let selected = index == selected_index;
            let dirty_suffix = if buffer.is_dirty() { " *dirty" } else { "" };
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else if buffer.is_dirty() {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            let marker = if selected { ">>" } else { "  " };
            Line::from(vec![
                Span::styled(format!("{marker} "), style),
                Span::styled(buffer.file_label(), style.add_modifier(Modifier::BOLD)),
                Span::styled(dirty_suffix.to_string(), style),
            ])
        })
        .collect::<Vec<_>>();

    let editor_lines = selected_buffer
        .lines()
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    let editor_height = editor_height.max(1) as usize;
    let max_editor_scroll = selected_buffer
        .lines()
        .len()
        .saturating_sub(editor_height)
        .min(u16::MAX as usize) as u16;
    let editor_scroll = selected_buffer.editor_scroll().min(max_editor_scroll);
    let editor_cursor_offset = Some((
        selected_buffer.cursor_column().min(u16::MAX as usize) as u16,
        selected_buffer
            .cursor_line_index()
            .saturating_sub(editor_scroll as usize)
            .min(u16::MAX as usize) as u16,
    ));

    let mut status_lines = vec![
        Line::from(format!(
            "staged draft: {}",
            app.planning_draft_editor_ui_state
                .draft_name()
                .unwrap_or("unknown")
        )),
        Line::from(format!(
            "current file: {} ({}/{})",
            selected_buffer.active_path(),
            selected_index + 1,
            buffers.len()
        )),
        Line::from(vec![
            Span::styled("validation state: ", Style::default().fg(Color::Gray)),
            Span::styled(
                if validation_report.is_valid() {
                    "ok"
                } else {
                    "needs attention"
                },
                Style::default().fg(if validation_report.is_valid() {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]),
    ];
    if let Some(issue) = validation_report.issues.first() {
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
            Span::raw(compact_inline_detail(
                &issue.message,
                FOOTER_NOTICE_DETAIL_LIMIT,
            )),
        ]));
    } else {
        status_lines.push(Line::from(format!(
            "staged path: {}",
            compact_inline_detail(selected_buffer.staged_path(), FOOTER_NOTICE_DETAIL_LIMIT)
        )));
    }
    status_lines.push(Line::from(format!(
        "dirty: {}",
        if dirty_labels.is_empty() {
            "none".to_string()
        } else {
            compact_inline_detail(&dirty_labels.join(", "), FOOTER_NOTICE_DETAIL_LIMIT)
        }
    )));
    if !dirty_labels.is_empty() {
        status_lines.push(Line::from(
            "validation note: the status above reflects the last saved draft until Ctrl+S re-runs checks",
        ));
    }
    status_lines.push(Line::from(next_action));
    if let Some(risk) = close_risk {
        status_lines.push(Line::from(vec![
            Span::styled(
                if pending_close_risk.is_some() {
                    "close pending: "
                } else {
                    "close guard: "
                },
                Style::default().fg(if pending_close_risk.is_some() {
                    Color::Red
                } else {
                    Color::Yellow
                }),
            ),
            Span::raw(planning_draft_close_guard_detail(
                risk,
                pending_close_risk.is_some(),
            )),
        ]));
    }

    Some(PlanningDraftEditorOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Planning Draft",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / operator inspection"),
            ]),
            Line::from(format!(
                "draft dir: {}",
                app.planning_draft_editor_ui_state
                    .draft_directory()
                    .unwrap_or("unknown")
            )),
        ],
        file_lines,
        editor_title: selected_buffer.file_label(),
        editor_lines,
        editor_scroll,
        editor_cursor_offset,
        status_lines,
        key_lines: vec![
            Line::from("controls: Tab/BackTab switches files  |  arrows move the cursor"),
            Line::from(
                "controls: Enter inserts newline  |  Backspace deletes  |  Ctrl+W deletes the previous word",
            ),
            Line::from(
                "controls: Ctrl+S saves and validates  |  Ctrl+P saves and promotes active planning",
            ),
            planning_draft_editor_close_key_line(close_risk, pending_close_risk.is_some()),
        ],
    })
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

pub(crate) fn planning_init_option_line(
    shortcut: &str,
    label: &str,
    detail: &str,
    selected: bool,
    disabled: bool,
) -> Line<'static> {
    let style = if disabled {
        Style::default().fg(Color::DarkGray)
    } else if selected {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::White)
    };
    let marker = if selected { ">>" } else { "  " };

    Line::from(vec![
        Span::styled(format!("{marker} {shortcut}. "), style),
        Span::styled(label.to_string(), style.add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {detail}"), style),
    ])
}
