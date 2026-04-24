use crate::application::service::planning::DirectionsMaintenanceDirectionSummary;

use super::super::super::{AkraTheme, Line, Modifier, Span, Style};

pub(super) struct DetailDocSelectionProjection {
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) selected_direction_title: Option<String>,
}

pub(super) fn build_detail_doc_selection_projection(
    actionable_directions: &[&DirectionsMaintenanceDirectionSummary],
    selected_direction: Option<&DirectionsMaintenanceDirectionSummary>,
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
                    selected_direction.is_some_and(|candidate| candidate.id == direction.id);
                let style = if selected {
                    AkraTheme::selected()
                } else {
                    Style::default()
                };
                let marker = if selected {
                    AkraTheme::selected_marker()
                } else {
                    AkraTheme::idle_marker()
                };
                Line::from(vec![
                    Span::styled(marker, style),
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
    let selected_direction_title = selected_direction.map(|direction| direction.title.clone());

    DetailDocSelectionProjection {
        option_lines,
        selected_direction_title,
    }
}
