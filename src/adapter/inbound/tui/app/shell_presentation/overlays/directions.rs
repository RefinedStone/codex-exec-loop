use super::super::{
    Color, DetailDocConfirmChoice, DirectionsMaintenanceOverlayStep, Line, Modifier, NativeTuiApp,
    QUEUE_INSPECTION_NOTE_DETAIL_LIMIT, Span, Style, compact_whitespace_detail,
};
use super::DirectionsMaintenanceOverlayView;
use super::planning::planning_init_option_line;

pub(crate) fn build_directions_maintenance_overlay_view(
    app: &NativeTuiApp,
) -> DirectionsMaintenanceOverlayView {
    match app.directions_maintenance_overlay_ui_state.step() {
        DirectionsMaintenanceOverlayStep::Overview => {
            let summary = app.directions_maintenance_overlay_ui_state.summary();
            let parse_error = summary.and_then(|summary| summary.parse_error.as_deref());
            let missing_doc_count = summary
                .map(|summary| summary.missing_detail_doc_count)
                .unwrap_or_default();
            let broken_doc_count = summary
                .map(|summary| summary.broken_detail_doc_count)
                .unwrap_or_default();
            let total_direction_count =
                summary.map(|summary| summary.directions.len()).unwrap_or(0);
            let queue_idle_policy = summary
                .map(|summary| summary.queue_idle_policy.label().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let queue_idle_prompt = summary
                .and_then(|summary| summary.queue_idle_prompt_path.as_deref())
                .map(|path| compact_whitespace_detail(path, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT))
                .unwrap_or_else(|| "<none>".to_string());
            let queue_idle_prompt_status = summary
                .map(|summary| summary.queue_idle_prompt_status.label())
                .unwrap_or("unknown");

            DirectionsMaintenanceOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Directions Maintenance",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / shell inspection"),
                    ]),
                    Line::from(
                        "Review operator-owned planning directions and queue-idle policy without editing raw files first.",
                    ),
                ],
                summary_lines: vec![
                    Line::from(
                        "Use Enter to open the staged editor, `d` to create a detail-doc mapping, or `p` to create/edit the queue-idle prompt.",
                    ),
                    Line::from(
                        "The active planning files do not change until you promote the staged draft.",
                    ),
                ],
                option_lines: vec![
                    planning_init_option_line(
                        "Enter",
                        "edit directions",
                        "open directions.toml and any existing queue-idle prompt in the staged editor",
                        false,
                        false,
                    ),
                    planning_init_option_line(
                        "D",
                        "repair detail docs",
                        "choose one direction with a missing or broken doc mapping and stage a markdown file",
                        false,
                        parse_error.is_some() || (missing_doc_count == 0 && broken_doc_count == 0),
                    ),
                    planning_init_option_line(
                        "P",
                        "edit queue-idle prompt",
                        "stage the queue-idle review prompt markdown and create or repair prompt_path if needed",
                        false,
                        parse_error.is_some(),
                    ),
                ],
                status_lines: vec![
                    Line::from(format!(
                        "directions: {total_direction_count} total / {missing_doc_count} missing docs / {broken_doc_count} broken docs"
                    )),
                    Line::from(format!(
                        "queue idle: policy {queue_idle_policy} / prompt {queue_idle_prompt_status} / {queue_idle_prompt}"
                    )),
                    Line::from(match parse_error {
                        Some(error) => format!(
                            "directions parse error: {}",
                            compact_whitespace_detail(error, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                        ),
                        None => "directions parsing: ok".to_string(),
                    }),
                ],
                key_lines: vec![
                    Line::from(
                        "Enter: edit directions    d: create or repair detail doc    p: edit queue-idle prompt",
                    ),
                    Line::from("r: reload summary    Esc/Ctrl+C: close"),
                ],
            }
        }
        DirectionsMaintenanceOverlayStep::DetailDocSelection => {
            let actionable_directions = app
                .directions_maintenance_overlay_ui_state
                .actionable_detail_doc_directions();
            let selected_direction = app
                .directions_maintenance_overlay_ui_state
                .selected_actionable_detail_doc_direction();
            let option_lines = if actionable_directions.is_empty() {
                vec![Line::from(
                    "Every direction already has a healthy detail-doc mapping.",
                )]
            } else {
                actionable_directions
                    .iter()
                    .map(|direction| {
                        let selected = selected_direction.is_some_and(|selected_direction| {
                            selected_direction.id == direction.id
                        });
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        let marker = if selected { ">>" } else { "  " };
                        Line::from(vec![
                            Span::styled(format!("{marker} "), style),
                            Span::styled(
                                direction.title.clone(),
                                style.add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!(
                                    "  id={} / status={} / path={}",
                                    direction.id,
                                    direction.detail_doc_status.label(),
                                    direction.detail_doc_path.as_deref().unwrap_or("<unset>")
                                ),
                                style,
                            ),
                        ])
                    })
                    .collect()
            };

            DirectionsMaintenanceOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Directions Maintenance",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / detail docs"),
                    ]),
                    Line::from(
                        "Choose a direction whose detail-doc mapping should be created or repaired.",
                    ),
                ],
                summary_lines: vec![
                    Line::from(
                        "Generated docs follow `.codex-exec-loop/planning/directions/<direction-id>.md`.",
                    ),
                    Line::from(
                        "The file and `detail_doc_path` mapping are staged first and only become active after promote.",
                    ),
                ],
                option_lines,
                status_lines: vec![Line::from(format!(
                    "selected: {}",
                    selected_direction
                        .map(|direction| direction.title.as_str())
                        .unwrap_or("none")
                ))],
                key_lines: vec![
                    Line::from("Up/Down or j/k: move selection"),
                    Line::from("Enter: continue    Backspace/Left: back    Esc/Ctrl+C: close"),
                ],
            }
        }
        DirectionsMaintenanceOverlayStep::DetailDocConfirm => {
            let pending = app
                .directions_maintenance_overlay_ui_state
                .pending_detail_doc_creation();
            let direction_id = pending
                .map(|pending| pending.direction_id())
                .unwrap_or("unknown");
            let direction_title = pending
                .map(|pending| pending.direction_title())
                .unwrap_or("unknown");

            DirectionsMaintenanceOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Directions Maintenance",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / confirm detail doc"),
                    ]),
                    Line::from(
                        "Open a staged detail document for the selected direction and repair the mapping if needed?",
                    ),
                ],
                summary_lines: vec![
                    Line::from(format!("direction: {direction_title}")),
                    Line::from(format!(
                        "default repair path: .codex-exec-loop/planning/directions/{direction_id}.md"
                    )),
                ],
                option_lines: vec![
                    planning_init_option_line(
                        "1",
                        "yes",
                        "stage a markdown file and open it with directions.toml for creation or repair",
                        app.directions_maintenance_overlay_ui_state
                            .detail_doc_confirm_choice()
                            == DetailDocConfirmChoice::Yes,
                        false,
                    ),
                    planning_init_option_line(
                        "2",
                        "no",
                        "return without changing the active or staged planning files",
                        app.directions_maintenance_overlay_ui_state
                            .detail_doc_confirm_choice()
                            == DetailDocConfirmChoice::No,
                        false,
                    ),
                ],
                status_lines: vec![Line::from("confirmation: generate a staged doc file now")],
                key_lines: vec![
                    Line::from("Up/Down or j/k: change selection"),
                    Line::from("Enter: act    Backspace/Left: back    Esc/Ctrl+C: close"),
                ],
            }
        }
        DirectionsMaintenanceOverlayStep::ManualEditor => DirectionsMaintenanceOverlayView {
            header_lines: vec![
                Line::from(vec![
                    Span::styled(
                        "Directions Maintenance",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" / staged editor"),
                ]),
                Line::from("Edit the staged directions draft and save to re-run validation."),
            ],
            summary_lines: vec![Line::from(
                "This state renders through the dedicated draft editor view.",
            )],
            option_lines: vec![Line::from(
                "Use Tab to switch files and Ctrl+S to save + validate.",
            )],
            status_lines: vec![Line::from("editor ready")],
            key_lines: vec![Line::from("Esc/Ctrl+C: close")],
        },
    }
}
