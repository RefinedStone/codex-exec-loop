use crate::adapter::inbound::tui::app::Line;

pub(in super::super) struct PlanningSimpleReviewAssemblyContract {
    pub(in super::super) header_lines: Vec<Line<'static>>,
    pub(in super::super) summary_lines: Vec<Line<'static>>,
    pub(in super::super) option_lines: Vec<Line<'static>>,
    pub(in super::super) status_lines: Vec<Line<'static>>,
    pub(in super::super) key_lines: Vec<Line<'static>>,
}
