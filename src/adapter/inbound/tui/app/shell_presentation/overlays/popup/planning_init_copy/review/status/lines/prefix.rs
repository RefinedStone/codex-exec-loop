use super::super::super::super::super::super::super::super::Line;

pub(super) fn build_simple_review_status_prefix_lines(
    validation_ok: bool,
    max_auto_turns_label: &str,
) -> Vec<Line<'static>> {
    vec![
        Line::from(format!(
            "validation state: {}",
            if validation_ok {
                "ok"
            } else {
                "needs attention"
            }
        )),
        Line::from(format!("turn budget: {max_auto_turns_label}")),
    ]
}
