use crate::application::service::planning::DirectionsMaintenanceDirectionSummary;

use super::super::super::{Color, Line, Modifier, Span, Style};

pub(super) struct DetailDocSelectionProjection {
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) selected_direction_title: Option<String>,
}

pub(super) fn build_detail_doc_selection_projection(
    actionable_directions: &[&DirectionsMaintenanceDirectionSummary],
    selected_direction_id: Option<&str>,
) -> DetailDocSelectionProjection {
    let option_lines = if actionable_directions.is_empty() {
        vec![Line::from(
            "Every direction already has a healthy detail-doc mapping.",
        )]
    } else {
        actionable_directions
            .iter()
            .map(|direction| {
                let selected =
                    selected_direction_id.is_some_and(|selected_id| selected_id == direction.id);
                let style = if selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default().fg(Color::White)
                };
                let marker = if selected { ">>" } else { "  " };
                Line::from(vec![
                    Span::styled(format!("{marker} "), style),
                    Span::styled(direction.title.clone(), style.add_modifier(Modifier::BOLD)),
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
    let selected_direction_title = actionable_directions
        .iter()
        .find(|direction| {
            selected_direction_id.is_some_and(|selected_id| selected_id == direction.id)
        })
        .map(|direction| direction.title.clone());

    DetailDocSelectionProjection {
        option_lines,
        selected_direction_title,
    }
}
