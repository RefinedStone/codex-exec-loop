use ratatui::layout::Rect;
use ratatui::text::Line;

use super::parallel_supervisor_events::{
    ParallelEventStreamSnapshot, ProjectedParallelEvent, rendered_parallel_event_line_rows,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParallelLiveLayout {
    Pending,
    Planned {
        title_visible: bool,
        scroll_offset: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelLiveStreamModel {
    events: Vec<ProjectedParallelEvent>,
    status_lines: Vec<Line<'static>>,
    layout: ParallelLiveLayout,
}

pub(super) struct ParallelLiveRenderParts {
    pub(super) title_visible: bool,
    pub(super) scroll_offset: u16,
    pub(super) lines: Vec<Line<'static>>,
}

impl ParallelLiveStreamModel {
    pub(super) fn pending_viewport(
        snapshot: &ParallelEventStreamSnapshot,
        status_lines: Vec<Line<'static>>,
    ) -> Self {
        Self {
            events: snapshot.events().to_vec(),
            status_lines,
            layout: ParallelLiveLayout::Pending,
        }
    }

    pub(super) fn finalize_pending_geometry(&mut self, event_area: Rect) {
        if !matches!(self.layout, ParallelLiveLayout::Pending) {
            return;
        }
        let lines = self.render_lines();
        let titled_rows = usize::from(event_area.height.saturating_sub(1));
        let title_visible = rendered_rows(&lines, event_area.width) <= titled_rows;
        let visible_rows = usize::from(if title_visible {
            event_area.height.saturating_sub(1)
        } else {
            event_area.height
        });
        self.layout = ParallelLiveLayout::Planned {
            title_visible,
            scroll_offset: rendered_rows(&lines, event_area.width)
                .saturating_sub(visible_rows)
                .min(usize::from(u16::MAX)) as u16,
        };
    }

    pub(super) fn into_render_parts(self) -> ParallelLiveRenderParts {
        let (title_visible, scroll_offset) = match self.layout {
            ParallelLiveLayout::Planned {
                title_visible,
                scroll_offset,
            } => (title_visible, scroll_offset),
            ParallelLiveLayout::Pending => (true, 0),
        };
        let mut lines = self.status_lines;
        lines.extend(self.events.into_iter().map(|event| event.line().clone()));
        ParallelLiveRenderParts {
            title_visible,
            scroll_offset,
            lines,
        }
    }

    pub(super) fn render_lines(&self) -> Vec<Line<'static>> {
        self.status_lines
            .iter()
            .cloned()
            .chain(self.events.iter().map(|event| event.line().clone()))
            .collect()
    }
}

fn rendered_rows(lines: &[Line<'static>], width: u16) -> usize {
    lines
        .iter()
        .map(|line| rendered_parallel_event_line_rows(line, width))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_parallel_stream_keeps_its_title() {
        let mut stream = ParallelLiveStreamModel::pending_viewport(
            &ParallelEventStreamSnapshot::default(),
            vec![Line::from("READY · slots 1/3")],
        );

        stream.finalize_pending_geometry(Rect::new(0, 0, 80, 4));
        let rendered = stream.into_render_parts();

        assert!(rendered.title_visible);
        assert_eq!(rendered.scroll_offset, 0);
        assert_eq!(rendered.lines.len(), 1);
    }

    #[test]
    fn dense_parallel_stream_hides_only_the_title_and_keeps_rows() {
        let status_lines = (0..6)
            .map(|index| Line::from(format!("parallel event {index}")))
            .collect();
        let mut stream = ParallelLiveStreamModel::pending_viewport(
            &ParallelEventStreamSnapshot::default(),
            status_lines,
        );

        stream.finalize_pending_geometry(Rect::new(0, 0, 24, 3));
        let rendered = stream.into_render_parts();

        assert!(!rendered.title_visible);
        assert!(rendered.scroll_offset > 0);
        assert_eq!(rendered.lines.len(), 6);
    }

    #[test]
    fn pending_stream_falls_back_to_a_safe_unscrolled_view() {
        let stream = ParallelLiveStreamModel::pending_viewport(
            &ParallelEventStreamSnapshot::default(),
            vec![Line::from("READY · slots 0/3")],
        );

        let rendered = stream.into_render_parts();

        assert!(rendered.title_visible);
        assert_eq!(rendered.scroll_offset, 0);
        assert_eq!(rendered.lines.len(), 1);
    }
}
