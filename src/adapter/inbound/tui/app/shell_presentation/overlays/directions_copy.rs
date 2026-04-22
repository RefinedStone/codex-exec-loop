use super::super::super::{Color, DetailDocConfirmChoice, Line, Modifier, Span, Style};
use super::super::option_lines::overlay_option_line;
use super::DirectionsMaintenanceOverlayView;

fn directions_title_line(suffix: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            "Directions Maintenance",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(suffix),
    ])
}

pub(super) fn build_overview_overlay_view(
    missing_doc_count: usize,
    broken_doc_count: usize,
    total_direction_count: usize,
    queue_idle_policy: &str,
    queue_idle_prompt_status: &str,
    queue_idle_prompt: &str,
    parse_error_summary: Option<&str>,
) -> DirectionsMaintenanceOverlayView {
    DirectionsMaintenanceOverlayView {
        header_lines: vec![
            directions_title_line(" / shell inspection"),
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
            overlay_option_line(
                "Enter",
                "edit directions",
                "open directions.toml and any existing queue-idle prompt in the staged editor",
                false,
                false,
            ),
            overlay_option_line(
                "D",
                "repair detail docs",
                "choose one direction with a missing or broken doc mapping and stage a markdown file",
                false,
                parse_error_summary.is_some() || (missing_doc_count == 0 && broken_doc_count == 0),
            ),
            overlay_option_line(
                "P",
                "edit queue-idle prompt",
                "stage the queue-idle review prompt markdown and create or repair prompt_path if needed",
                false,
                parse_error_summary.is_some(),
            ),
        ],
        status_lines: vec![
            Line::from(format!(
                "directions: {total_direction_count} total / {missing_doc_count} missing docs / {broken_doc_count} broken docs"
            )),
            Line::from(format!(
                "queue idle: policy {queue_idle_policy} / prompt {queue_idle_prompt_status} / {queue_idle_prompt}"
            )),
            Line::from(match parse_error_summary {
                Some(error) => format!("directions parse error: {error}"),
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

pub(super) fn build_detail_doc_selection_overlay_view(
    option_lines: Vec<Line<'static>>,
    selected_direction_title: Option<&str>,
) -> DirectionsMaintenanceOverlayView {
    DirectionsMaintenanceOverlayView {
        header_lines: vec![
            directions_title_line(" / detail docs"),
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
            selected_direction_title.unwrap_or("none")
        ))],
        key_lines: vec![
            Line::from("Up/Down or j/k: move selection"),
            Line::from("Enter: continue    Backspace/Left: back    Esc/Ctrl+C: close"),
        ],
    }
}

pub(super) fn build_detail_doc_confirm_overlay_view(
    direction_title: &str,
    direction_id: &str,
    selected_choice: DetailDocConfirmChoice,
) -> DirectionsMaintenanceOverlayView {
    DirectionsMaintenanceOverlayView {
        header_lines: vec![
            directions_title_line(" / confirm detail doc"),
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
            overlay_option_line(
                "1",
                "yes",
                "stage a markdown file and open it with directions.toml for creation or repair",
                selected_choice == DetailDocConfirmChoice::Yes,
                false,
            ),
            overlay_option_line(
                "2",
                "no",
                "return without changing the active or staged planning files",
                selected_choice == DetailDocConfirmChoice::No,
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

pub(super) fn build_manual_editor_overlay_view() -> DirectionsMaintenanceOverlayView {
    DirectionsMaintenanceOverlayView {
        header_lines: vec![
            directions_title_line(" / staged editor"),
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
    }
}
