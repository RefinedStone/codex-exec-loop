use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::super::{AkraTheme, MAX_SHELL_TAIL_HEIGHT, MIN_TRANSCRIPT_PANEL_HEIGHT, ShellOverlay};
use super::FullscreenConversationFrameProjection;
use crate::adapter::inbound::tui::app::shell_presentation::ShellTailView;

// Fullscreen geometry has two persistent bands: the app-owned transcript (or a
// focused view) and a bottom-anchored status/composer surface. This module owns
// wrapping, height budgets, scrolling, and cursor placement for those bands.
const MAX_FOCUSED_VIEW_TAIL_HEIGHT: u16 = 6;

pub(super) fn build_fullscreen_flow_layout(
    projection: &FullscreenConversationFrameProjection,
    area: Rect,
    tail_lines: &[Line<'_>],
) -> Rc<[Rect]> {
    // Conversation mode gives the composer its normal budget. A focused view
    // keeps only a compact action tail so its content remains useful on small
    // terminals.
    let tail_max_height = if projection.shell_overlay == ShellOverlay::Hidden {
        MAX_SHELL_TAIL_HEIGHT
    } else {
        MAX_FOCUSED_VIEW_TAIL_HEIGHT
    };
    let _ = tail_lines;
    let tail_height = projection
        .tail_view
        .rendered_height(area.width, tail_max_height);
    let inspection_constraint = if projection.shell_overlay == ShellOverlay::Hidden {
        // The prompt tail owns short viewports; transcript receives every remaining row.
        Constraint::Min(0)
    } else if projection.shell_overlay == ShellOverlay::Activity && area.width <= 48 {
        Constraint::Length(area.height.saturating_sub(tail_height))
    } else {
        Constraint::Min(MIN_TRANSCRIPT_PANEL_HEIGHT.saturating_sub(2).max(6))
    };
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([inspection_constraint, Constraint::Length(tail_height)])
        .split(area)
}

pub(in crate::adapter::inbound::tui::app) fn fullscreen_section_height(
    lines: &[Line<'_>],
    max_height: u16,
) -> u16 {
    // fullscreen inspection panel은 title row 하나와 최소 body row 하나를 예약한 뒤 caller가 준 상한으로 자른다.
    lines
        .len()
        .saturating_add(1)
        .max(2)
        .min(max_height as usize) as u16
}

pub(super) fn fullscreen_tail_render_area(area: Rect, tail_view: &ShellTailView) -> Rect {
    let body_height = tail_view.rendered_height(area.width, area.height);
    let y = area.y + area.height.saturating_sub(body_height);
    Rect::new(area.x, y, area.width, body_height)
}

pub(in crate::adapter::inbound::tui::app) fn count_wrapped_rows(
    lines: &[Line<'_>],
    width: u16,
) -> usize {
    if width == 0 {
        return 0;
    }

    Paragraph::new(lines.to_vec())
        .wrap(Wrap { trim: false })
        .line_count(width)
}

pub(super) fn render_body_suffix(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: Vec<Line<'static>>,
    focus_row: Option<u16>,
) -> u16 {
    if area.width == 0 || area.height == 0 {
        return 0;
    }

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let rendered_rows = paragraph.line_count(area.width);
    let bottom_scroll = rendered_rows.saturating_sub(usize::from(area.height));
    let dropped_rows = focus_row
        .map(usize::from)
        .filter(|focus_row| *focus_row < bottom_scroll)
        .unwrap_or(bottom_scroll);
    let scroll_offset = dropped_rows.min(usize::from(u16::MAX)) as u16;
    frame.render_widget(paragraph.scroll((scroll_offset, 0)), area);
    scroll_offset
}

pub(super) fn split_fullscreen_section(area: Rect) -> Rc<[Rect]> {
    // fullscreen overlay가 공유하는 title/body split으로 모든 panel이 같은 visual rhythm을 유지한다.
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area)
}

pub(super) struct FullscreenTitledPanel {
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    trim: bool,
}

impl FullscreenTitledPanel {
    pub(super) fn new(title: Line<'static>, lines: Vec<Line<'static>>, trim: bool) -> Self {
        Self { title, lines, trim }
    }

    pub(super) fn render(self, frame: &mut Frame<'_>, area: Rect) {
        render_fullscreen_section(frame, area, self.title, self.lines, self.trim);
    }
}

pub(super) struct FullscreenScrolledPanel {
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
}

impl FullscreenScrolledPanel {
    pub(super) fn new(title: Line<'static>, lines: Vec<Line<'static>>, scroll_offset: u16) -> Self {
        Self {
            title,
            lines,
            scroll_offset,
        }
    }

    pub(super) fn render(self, frame: &mut Frame<'_>, area: Rect) {
        render_fullscreen_scrolled_section(frame, area, self.title, self.lines, self.scroll_offset);
    }
}

pub(super) enum FullscreenAppendOnlyStreamTitle {
    Visible(Line<'static>),
    Hidden,
}

pub(super) struct FullscreenAppendOnlyStream {
    title: FullscreenAppendOnlyStreamTitle,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
}

impl FullscreenAppendOnlyStream {
    pub(super) fn new(
        title: FullscreenAppendOnlyStreamTitle,
        lines: Vec<Line<'static>>,
        scroll_offset: u16,
    ) -> Self {
        Self {
            title,
            lines,
            scroll_offset,
        }
    }

    pub(super) fn render(self, frame: &mut Frame<'_>, area: Rect) {
        match self.title {
            FullscreenAppendOnlyStreamTitle::Visible(title) => {
                render_fullscreen_scrolled_section(
                    frame,
                    area,
                    title,
                    self.lines,
                    self.scroll_offset,
                );
            }
            FullscreenAppendOnlyStreamTitle::Hidden => {
                render_fullscreen_scrolled_body(frame, area, self.lines, self.scroll_offset);
            }
        }
    }
}

fn render_fullscreen_section(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    trim: bool,
) {
    // fullscreen inspection은 whitespace와 title을 chrome으로 쓰므로 border 없는 titled panel을 render한다.
    let section_layout = split_fullscreen_section(area);
    frame.render_widget(
        Paragraph::new(vec![title.style(AkraTheme::title())]),
        section_layout[0],
    );
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim }), section_layout[1]);
}

pub(super) fn set_cursor_if_visible(frame: &mut Frame<'_>, area: Rect, offset: Option<(u16, u16)>) {
    /*
     * cursor offset은 textarea/body area 기준 local 좌표지만 ratatui는 absolute frame 좌표를 요구한다.
     * 먼저 local area 안에서 clamp하고, overlay 계산 결과가 zero row로 잘릴 수 있으므로 실제 terminal frame 밖 좌표는 버린다.
     */
    let Some((cursor_x, cursor_y)) = offset else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let frame_area = frame.area();
    let clamped_x = cursor_x.min(area.width.saturating_sub(1));
    let clamped_y = cursor_y.min(area.height.saturating_sub(1));
    let absolute_x = area.x.saturating_add(clamped_x);
    let absolute_y = area.y.saturating_add(clamped_y);
    if absolute_x < frame_area.x
        || absolute_y < frame_area.y
        || absolute_x >= frame_area.x.saturating_add(frame_area.width)
        || absolute_y >= frame_area.y.saturating_add(frame_area.height)
    {
        return;
    }

    frame.set_cursor_position(Position::new(absolute_x, absolute_y));
}

fn render_fullscreen_scrolled_section(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
) {
    // scrolled section은 leading whitespace 보존이 중요한 editor-style panel에서 사용한다.
    let section_layout = split_fullscreen_section(area);
    frame.render_widget(
        Paragraph::new(vec![title.style(AkraTheme::title())]),
        section_layout[0],
    );
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll_offset, 0))
            .wrap(Wrap { trim: false }),
        section_layout[1],
    );
}

fn render_fullscreen_scrolled_body(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
) {
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll_offset, 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub(super) fn take_panel_body_lines(mut header_lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    // 일부 presentation builder는 title과 body를 함께 반환하므로 layout caller는 body row만 꺼내 쓴다.
    if !header_lines.is_empty() {
        header_lines.remove(0);
    }
    header_lines
}

pub(super) fn centered_fixed_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x.saturating_add(area.width.saturating_sub(width) / 2),
        area.y
            .saturating_add(area.height.saturating_sub(height) / 2),
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;

    use super::*;

    #[test]
    fn set_cursor_if_visible_ignores_area_outside_frame() {
        let backend = TestBackend::new(80, 8);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");

        terminal
            .draw(|frame| {
                set_cursor_if_visible(frame, Rect::new(0, 8, 80, 1), Some((0, 0)));
            })
            .expect("cursor outside frame should be ignored");
    }

    #[test]
    fn render_body_suffix_scrolls_to_the_last_rendered_rows() {
        let backend = TestBackend::new(5, 2);
        let mut terminal = Terminal::new(backend).expect("terminal should initialize");

        terminal
            .draw(|frame| {
                let dropped_rows = render_body_suffix(
                    frame,
                    frame.area(),
                    vec![
                        Line::from("alpha"),
                        Line::from("bravo"),
                        Line::from("charl"),
                    ],
                    None,
                );
                assert_eq!(dropped_rows, 1);
            })
            .expect("suffix should render");

        terminal.backend().assert_buffer_lines(["bravo", "charl"]);
    }

    #[test]
    fn focused_parallel_operations_reserves_the_task_composer_tail() {
        let mut app = test_native_tui_app();
        app.show_supersession_overlay();
        let projection = FullscreenConversationFrameProjection::from_app(&app, 120);
        let layout =
            build_fullscreen_flow_layout(&projection, Rect::new(0, 0, 120, 32), &Vec::new());

        assert!(
            layout[1].height > 0,
            "focused parallel operations should retain the task composer tail"
        );
    }
}
