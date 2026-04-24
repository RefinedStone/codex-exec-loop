use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::ClearType;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum HistoryInsertionMode {
    #[default]
    StandardScrollRegion,
    NewlineFallback,
}

impl HistoryInsertionMode {
    pub(super) fn from_environment() -> Self {
        Self::from_env_values(
            std::env::var(super::HISTORY_INSERT_MODE_ENV_VAR)
                .ok()
                .as_deref(),
        )
    }

    pub(super) fn from_env_values(mode_value: Option<&str>) -> Self {
        let Some(mode_value) = mode_value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
        else {
            return Self::StandardScrollRegion;
        };

        match mode_value.as_str() {
            "newline" | "newline-fallback" | "fallback" => Self::NewlineFallback,
            "standard" | "scroll-region" | "scrollregion" => Self::StandardScrollRegion,
            _ => Self::StandardScrollRegion,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct HistoryInsertionAdapter {
    mode: HistoryInsertionMode,
}

impl HistoryInsertionAdapter {
    pub(super) fn new(mode: HistoryInsertionMode) -> Self {
        Self { mode }
    }

    pub(super) fn insert<B: Backend>(
        self,
        terminal: &mut Terminal<B>,
        lines: &[Line<'static>],
    ) -> Result<(), B::Error> {
        let width = terminal.size()?.width;
        let height = count_rendered_history_rows(lines, width).min(u16::MAX as usize) as u16;
        if width == 0 || height == 0 {
            return Ok(());
        }

        let cursor = terminal.get_cursor_position()?;
        let result = match self.mode {
            HistoryInsertionMode::StandardScrollRegion => {
                insert_with_standard_scroll_region(terminal, lines, height)
            }
            HistoryInsertionMode::NewlineFallback => {
                let buffer = rendered_history_buffer(width, lines);
                insert_with_newline_fallback(terminal, &buffer)
            }
        };
        restore_cursor(terminal, cursor)?;
        result
    }
}

fn insert_with_standard_scroll_region<B: Backend>(
    terminal: &mut Terminal<B>,
    lines: &[Line<'static>],
    height: u16,
) -> Result<(), B::Error> {
    terminal.insert_before(height, |buffer| {
        history_paragraph(lines).render(buffer.area, buffer);
    })
}

fn insert_with_newline_fallback<B: Backend>(
    terminal: &mut Terminal<B>,
    buffer: &Buffer,
) -> Result<(), B::Error> {
    let size = terminal.size()?;
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }

    let bottom_y = size.height.saturating_sub(1);
    for source_y in 0..buffer.area.height {
        terminal
            .backend_mut()
            .set_cursor_position(Position { x: 0, y: bottom_y })?;
        terminal
            .backend_mut()
            .clear_region(ClearType::CurrentLine)?;
        terminal
            .backend_mut()
            .draw((0..buffer.area.width).map(|x| (x, bottom_y, &buffer[(x, source_y)])))?;
        terminal.backend_mut().set_cursor_position(Position {
            x: size.width.saturating_sub(1),
            y: bottom_y,
        })?;
        terminal.backend_mut().append_lines(1)?;
    }
    terminal.backend_mut().flush()
}

fn restore_cursor<B: Backend>(
    terminal: &mut Terminal<B>,
    cursor: Position,
) -> Result<(), B::Error> {
    let size = terminal.size()?;
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }

    terminal.set_cursor_position(Position {
        x: cursor.x.min(size.width.saturating_sub(1)),
        y: cursor.y.min(size.height.saturating_sub(1)),
    })
}

pub(super) fn count_rendered_history_rows(lines: &[Line<'static>], width: u16) -> usize {
    if width == 0 || lines.is_empty() {
        return 0;
    }

    rendered_history_height(width, lines)
}

pub(super) fn rendered_history_buffer(width: u16, lines: &[Line<'static>]) -> Buffer {
    let height = count_rendered_history_rows(lines, width).min(u16::MAX as usize) as u16;
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let mut buffer = Buffer::empty(area);
    if width == 0 || height == 0 {
        return buffer;
    }

    history_paragraph(lines).render(buffer.area, &mut buffer);
    buffer
}

fn history_paragraph(lines: &[Line<'static>]) -> Paragraph<'static> {
    Paragraph::new(lines.to_vec()).wrap(Wrap { trim: false })
}

fn rendered_history_height(width: u16, lines: &[Line<'static>]) -> usize {
    let capacity = conservative_history_row_capacity(lines).saturating_add(1);
    let probe_height = capacity.min(u16::MAX as usize) as u16;
    let mut probe_lines = lines.to_vec();
    probe_lines.push(sentinel_line());
    let area = Rect {
        x: 0,
        y: 0,
        width,
        height: probe_height,
    };
    let mut buffer = Buffer::empty(area);
    history_paragraph(&probe_lines).render(area, &mut buffer);

    for y in 0..probe_height {
        if (0..width).any(|x| {
            let cell = &buffer[(x, y)];
            cell.fg == sentinel_fg() && cell.bg == sentinel_bg()
        }) {
            return y as usize;
        }
    }

    probe_height as usize
}

fn conservative_history_row_capacity(lines: &[Line<'static>]) -> usize {
    lines.iter().map(|line| line.width().max(1)).sum::<usize>()
}

fn sentinel_line() -> Line<'static> {
    Line::from(Span::styled("X", sentinel_style()))
}

fn sentinel_style() -> Style {
    Style::default().fg(sentinel_fg()).bg(sentinel_bg())
}

fn sentinel_fg() -> Color {
    Color::Indexed(255)
}

fn sentinel_bg() -> Color {
    Color::Indexed(254)
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Position;
    use ratatui::text::Line;

    use super::{
        HistoryInsertionAdapter, HistoryInsertionMode, count_rendered_history_rows,
        rendered_history_buffer,
    };
    use crate::adapter::inbound::tui::app::InlineHistoryRenderMode;
    use crate::adapter::inbound::tui::app::tui_testkit;

    #[test]
    fn history_insertion_mode_defaults_to_standard_scroll_region() {
        assert_eq!(
            HistoryInsertionMode::from_env_values(None),
            HistoryInsertionMode::StandardScrollRegion
        );
        assert_eq!(
            HistoryInsertionMode::from_env_values(Some("")),
            HistoryInsertionMode::StandardScrollRegion
        );
        assert_eq!(
            HistoryInsertionMode::from_env_values(Some("unknown")),
            HistoryInsertionMode::StandardScrollRegion
        );
    }

    #[test]
    fn history_insertion_mode_supports_explicit_newline_fallback() {
        assert_eq!(
            HistoryInsertionMode::from_env_values(Some("newline-fallback")),
            HistoryInsertionMode::NewlineFallback
        );
        assert_eq!(
            HistoryInsertionMode::from_env_values(Some("fallback")),
            HistoryInsertionMode::NewlineFallback
        );
        assert_eq!(
            HistoryInsertionMode::from_env_values(Some("standard")),
            HistoryInsertionMode::StandardScrollRegion
        );
    }

    #[test]
    fn rendered_history_rows_wrap_url_like_lines_and_wide_chars() {
        let lines = vec![
            Line::from("https://example.test/really/long/path"),
            Line::from("wide 한글 row"),
        ];

        let buffer = rendered_history_buffer(12, &lines);
        let text = tui_testkit::buffer_text(&buffer);

        assert_eq!(count_rendered_history_rows(&lines, 12), 6);
        assert!(text.contains("https://exam"), "{text:?}");
        assert!(text.contains("ple.test/rea"), "{text:?}");
        assert!(text.contains("lly/long/pa"), "{text:?}");
        assert!(text.contains("wide"), "{text:?}");
    }

    #[test]
    fn rendered_history_rows_follow_paragraph_word_wrapping() {
        let lines = vec![Line::from("aa aa aa")];

        let buffer = rendered_history_buffer(4, &lines);
        let text = tui_testkit::buffer_text(&buffer);

        assert_eq!(count_rendered_history_rows(&lines, 4), 3);
        assert_eq!(text, "aa  \naa  \naa  ");
    }

    #[test]
    fn rendered_history_rows_are_stable_through_vt100_screen_parsing() {
        let lines = vec![
            Line::from("https://example.test/really/long/path"),
            Line::from("wide 한글 row"),
        ];
        let buffer = rendered_history_buffer(12, &lines);
        let bytes = tui_testkit::buffer_text(&buffer).replace('\n', "\r\n");
        let mut screen = tui_testkit::Vt100Screen::new(12, 8);

        screen.process(bytes.as_bytes());

        let rows = screen.rows().join("\n");
        assert!(rows.contains("https://exam"), "{rows:?}");
        assert!(rows.contains("ple.test/rea"), "{rows:?}");
        assert!(rows.contains("wide"), "{rows:?}");
    }

    #[test]
    fn rendered_history_rows_clear_full_width_continuations() {
        let lines = vec![Line::from("1234567890"), Line::from("short")];

        let buffer = rendered_history_buffer(10, &lines);
        let rows = tui_testkit::buffer_text(&buffer);

        assert_eq!(rows, "1234567890\nshort     ");
    }

    #[test]
    fn standard_scroll_region_inserts_history_before_inline_viewport() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 20, 24);
        let lines = vec![
            Line::from("first committed line"),
            Line::from("https://example.test/path"),
        ];

        HistoryInsertionAdapter::new(HistoryInsertionMode::StandardScrollRegion)
            .insert(&mut terminal, &lines)
            .unwrap();

        let rendered = tui_testkit::screen_text(&terminal);
        assert!(rendered.contains("first committed"));
        assert!(rendered.contains("https://example"));
    }

    #[test]
    fn newline_fallback_inserts_history_without_scroll_regions() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 20, 6);
        let lines = vec![
            Line::from("newline fallback one"),
            Line::from("fallback two"),
        ];

        HistoryInsertionAdapter::new(HistoryInsertionMode::NewlineFallback)
            .insert(&mut terminal, &lines)
            .unwrap();

        let rendered = tui_testkit::screen_text(&terminal);
        assert!(rendered.contains("newline fallback"));
        assert!(rendered.contains("fallback two"));
    }

    #[test]
    fn history_insertion_modes_restore_cursor_position() {
        for mode in [
            HistoryInsertionMode::StandardScrollRegion,
            HistoryInsertionMode::NewlineFallback,
        ] {
            let mut terminal = tui_testkit::inline_history_terminal(
                InlineHistoryRenderMode::HostScrollback,
                20,
                6,
            );
            terminal
                .set_cursor_position(Position { x: 3, y: 4 })
                .unwrap();

            HistoryInsertionAdapter::new(mode)
                .insert(&mut terminal, &[Line::from("cursor neutral insert")])
                .unwrap();

            assert_eq!(
                terminal.get_cursor_position().unwrap(),
                Position { x: 3, y: 4 },
                "{mode:?} should leave cursor position unchanged"
            );
        }
    }
}
