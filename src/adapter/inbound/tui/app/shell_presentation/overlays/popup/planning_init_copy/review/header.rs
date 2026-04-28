use super::super::super::super::super::super::Line;
use super::super::super::copy::planning_setup_title_line;

pub(super) fn build_simple_review_header_lines() -> Vec<Line<'static>> {
    vec![
        planning_setup_title_line(" / operator inspection"),
        Line::from(
            "Simple mode review: promote the lightest planning baseline before you invest in richer authoring.",
        ),
    ]
}

pub(super) fn build_simple_review_summary_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(
            "After promote, planning starts with one generic direction and no active queue task yet.",
        ),
        Line::from(
            "The default queue-idle review prompt is already staged so the first reply can justify follow-up work when needed.",
        ),
        Line::from("No accepted planning state changes until you explicitly promote this review."),
    ]
}
