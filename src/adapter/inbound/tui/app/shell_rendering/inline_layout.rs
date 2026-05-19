use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use super::super::{
    AkraTheme, MAX_INLINE_TAIL_HEIGHT, MIN_TRANSCRIPT_PANEL_HEIGHT, NativeTuiApp, ShellOverlay,
};

/*
 * inline_layout.rs는 inline shell mode와 popup overlay가 공유하는 low-level geometry layer다.
 * 상위 module이 어떤 presentation line을 보여 줄지 정하고, 이 파일은 그 line이 차지할 terminal row 수,
 * bottom-anchored tail 위치, textarea cursor를 현재 frame 안에 둘 수 있는지를 결정한다.
 */
const MAX_INLINE_INSPECTION_TAIL_HEIGHT: u16 = 6;
// replay mode는 tail에 최근 transcript를 mirror하므로 일반 prompt tail보다 더 많은 row가 필요하다.
const MAX_INLINE_REPLAY_TAIL_HEIGHT: u16 = 12;

pub(super) fn build_inline_terminal_flow_layout(
    app: &NativeTuiApp,
    area: Rect,
    tail_lines: &[Line<'_>],
) -> Rc<[Rect]> {
    /*
     * inline shell은 위쪽 transcript/live content와 아래쪽 prompt/status tail로 나뉜 two-band frame이다.
     * hidden-overlay mode에서는 tail이 primary interaction surface라 더 많은 공간을 준다.
     * inspection/confirmation mode에서는 작은 terminal에서도 overlay content가 밀려나지 않도록 tail을 작게 제한한다.
     */
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
    // inline inspection panel은 title row 하나와 최소 body row 하나를 예약한 뒤 caller가 준 상한으로 자른다.
    lines
        .len()
        .saturating_add(1)
        .max(2)
        .min(max_height as usize) as u16
}

fn inline_body_height(lines: &[Line<'_>], width: u16, max_height: u16) -> u16 {
    // body height는 logical line이 아니라 rendered row 기준이라 wrap된 text도 필요한 공간을 확보한다.
    count_rendered_inline_rows(lines, width)
        .max(1)
        .min(max_height as usize) as u16
}

pub(super) fn inline_body_render_area(area: Rect, lines: &[Line<'_>]) -> Rect {
    /*
     * tail body는 bottom-anchored다.
     * prompt/status text가 가용 영역보다 짧으면 위쪽 row를 blank padding으로 쓰지 않고 transcript replay에 남겨 둔다.
     */
    let body_height = inline_body_height(lines, area.width, area.height);
    let y = area.y + area.height.saturating_sub(body_height);
    Rect::new(area.x, y, area.width, body_height)
}

pub(super) fn count_rendered_inline_rows(lines: &[Line<'_>], width: u16) -> usize {
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
    // inline overlay가 공유하는 title/body split으로 모든 panel이 같은 visual rhythm을 유지한다.
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
    // inline inspection은 whitespace와 title을 chrome으로 쓰므로 border 없는 titled panel을 render한다.
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

pub(super) fn render_inline_scrolled_section(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
) {
    // scrolled section은 leading whitespace 보존이 중요한 editor-style panel에서 사용한다.
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

pub(super) fn render_inline_scrolled_body(
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

pub(super) fn centered_rect(horizontal_percent: u16, vertical_percent: u16, area: Rect) -> Rect {
    /*
     * popup overlay는 percent 기반 영역을 요청하지만 design 조정 중 caller가 100을 넘는 값을 줄 수 있다.
     * split 전에 clamp해 ratatui가 invalid percentage constraint를 받지 않게 한다.
     */
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
