use crate::application::service::planning::DirectionsMaintenanceDirectionSummary;

use super::super::super::{AkraTheme, Line, Modifier, Span, Style};

// Detail-doc selection projection is the adapter boundary between service-computed maintenance facts
// and the overlay list rows that directions copy/rendering can display.
pub(super) struct DetailDocSelectionProjection {
    // Rows for the selectable direction list, or a single healthy-state message when no action is needed.
    pub(super) option_lines: Vec<Line<'static>>,
    // Status copy only needs the selected title, not the full service summary.
    pub(super) selected_direction_title: Option<String>,
}

// Service decides which directions are actionable; UI state decides which one is focused.
// This function joins those inputs into visual rows without re-evaluating detail-doc health.
pub(super) fn build_detail_doc_selection_projection(
    // Already-filtered subset with missing or broken detail-doc mappings.
    actionable_directions: &[&DirectionsMaintenanceDirectionSummary],
    // Focused item from UI state; may be None when the actionable list is empty or still being aligned.
    selected_direction: Option<&DirectionsMaintenanceDirectionSummary>,
) -> DetailDocSelectionProjection {
    // Empty state still returns option_lines so downstream copy/rendering can keep the same section layout.
    let option_lines = if actionable_directions.is_empty() {
        vec![Line::from(
            "Every direction already has a healthy detail-doc mapping.",
        )]
    } else {
        actionable_directions
            .iter()
            .map(|direction| {
                // Compare by stable direction id rather than borrowed pointer identity; summaries may be reborrowed.
                let selected =
                    selected_direction.is_some_and(|candidate| candidate.id == direction.id);

                // Selected style mirrors other overlay lists so Enter target and keyboard focus stay visually consistent.
                let style = if selected {
                    AkraTheme::selected()
                } else {
                    Style::default()
                };

                // Marker column keeps alignment stable and gives a non-color focus cue.
                let marker = if selected {
                    AkraTheme::selected_marker()
                } else {
                    AkraTheme::idle_marker()
                };

                // Title is the choice label; id/status/path stay in the same row as diagnostic metadata for repair context.
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

    // Clone only the title needed by status copy so the projection can outlive the borrowed summary slice.
    let selected_direction_title = selected_direction.map(|direction| direction.title.clone());

    // Returning rows and selected title together keeps list and status areas tied to the same selection snapshot.
    DetailDocSelectionProjection {
        option_lines,
        selected_direction_title,
    }
}
