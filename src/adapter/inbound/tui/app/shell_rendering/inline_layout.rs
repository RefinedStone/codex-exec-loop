use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::super::{
    AkraTheme, MAX_INLINE_TAIL_HEIGHT, MIN_TRANSCRIPT_PANEL_HEIGHT, NativeTuiApp, ShellOverlay,
    build_conversation_scroll_offset,
};

const MAX_INLINE_INSPECTION_TAIL_HEIGHT: u16 = 6;
const MAX_INLINE_REPLAY_TAIL_HEIGHT: u16 = 12;

pub(super) fn build_inline_terminal_flow_layout(
    app: &NativeTuiApp,
    area: Rect,
    tail_lines: &[Line<'_>],
) -> Rc<[Rect]> {
    let tail_max_height =
        if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
            if app
                .inline_history_render_mode
                .mirrors_recent_transcript_in_tail()
            {
                MAX_INLINE_REPLAY_TAIL_HEIGHT
            } else {
                MAX_INLINE_TAIL_HEIGHT
            }
        } else {
            MAX_INLINE_INSPECTION_TAIL_HEIGHT
        };
    let tail_height = inline_body_height(tail_lines, area.width, tail_max_height);

    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(MIN_TRANSCRIPT_PANEL_HEIGHT.saturating_sub(2).max(6)),
            Constraint::Length(tail_height),
        ])
        .split(area)
}

pub(super) fn inline_section_height(lines: &[Line<'_>], max_height: u16) -> u16 {
    lines
        .len()
        .saturating_add(1)
        .max(2)
        .min(max_height as usize) as u16
}

fn inline_body_height(lines: &[Line<'_>], width: u16, max_height: u16) -> u16 {
    count_rendered_inline_rows(lines, width)
        .max(1)
        .min(max_height as usize) as u16
}

pub(super) fn inline_body_render_area(area: Rect, lines: &[Line<'_>]) -> Rect {
    let body_height = inline_body_height(lines, area.width, area.height);
    let y = area.y + area.height.saturating_sub(body_height);
    Rect::new(area.x, y, area.width, body_height)
}

fn count_rendered_inline_rows(lines: &[Line<'_>], width: u16) -> usize {
    if width == 0 {
        return 0;
    }

    lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1
            } else {
                line_width.div_ceil(width as usize)
            }
        })
        .sum()
}

pub(super) fn split_inline_section(area: Rect) -> Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area)
}

pub(super) fn render_inline_section(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    trim: bool,
) {
    let section_layout = split_inline_section(area);
    frame.render_widget(
        Paragraph::new(vec![title.style(AkraTheme::title())]),
        section_layout[0],
    );
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim }), section_layout[1]);
}

pub(super) fn render_inline_body(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: Vec<Line<'static>>,
    trim: bool,
) {
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim }), area);
}

pub(super) fn set_cursor_if_visible(frame: &mut Frame<'_>, area: Rect, offset: Option<(u16, u16)>) {
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

pub(super) fn render_inline_scrolled_section(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
) {
    let section_layout = split_inline_section(area);
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

pub(super) fn take_panel_body_lines(mut header_lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    if !header_lines.is_empty() {
        header_lines.remove(0);
    }
    header_lines
}

pub(super) fn clamp_scroll_offset(
    current_scroll: u16,
    lines: &[Line<'static>],
    content_width: u16,
    visible_height: u16,
) -> u16 {
    current_scroll.min(build_conversation_scroll_offset(
        lines,
        content_width,
        visible_height,
    ))
}

pub(super) fn centered_rect(horizontal_percent: u16, vertical_percent: u16, area: Rect) -> Rect {
    let horizontal_percent = horizontal_percent.min(100);
    let vertical_percent = vertical_percent.min(100);
    let vertical_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100u16.saturating_sub(vertical_percent)) / 2),
            Constraint::Percentage(vertical_percent),
            Constraint::Percentage((100u16.saturating_sub(vertical_percent)) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100u16.saturating_sub(horizontal_percent)) / 2),
            Constraint::Percentage(horizontal_percent),
            Constraint::Percentage((100u16.saturating_sub(horizontal_percent)) / 2),
        ])
        .split(vertical_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
}
