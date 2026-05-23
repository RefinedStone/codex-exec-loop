use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::ClearType;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};

/*
 * Inline history insertion moves completed transcript rows into the host
 * scrollback while the live shell viewport stays on screen. The normal path uses
 * ratatui's scroll-region primitive; the newline fallback reconstructs the same
 * effect for terminals that mis-handle scroll regions, notably Windows Terminal.
 */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum HistoryInsertionMode {
    #[default]
    StandardScrollRegion,
    NewlineFallback,
}
impl HistoryInsertionMode {
    /*
     * Runtime selection is intentionally adapter-local. The application state
     * only decides that committed history should move to host scrollback; this
     * module decides which terminal escape strategy is safe for the current
     * environment.
     */
    pub(super) fn from_environment() -> Self {
        Self::from_env_and_terminal_values(
            std::env::var(super::HISTORY_INSERT_MODE_ENV_VAR)
                .ok()
                .as_deref(),
            std::env::var("WT_SESSION").ok().as_deref(),
        )
    }
    #[cfg(test)]
    pub(super) fn from_env_values(mode_value: Option<&str>) -> Self {
        Self::from_env_and_terminal_values(mode_value, None)
    }
    fn from_env_and_terminal_values(mode_value: Option<&str>, wt_session: Option<&str>) -> Self {
        let Some(mode_value) = mode_value
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
        else {
            /*
             * WT_SESSION is a conservative default to avoid scroll-region corruption
             * on Windows. An explicit env override still wins so manual debugging
             * can compare both strategies on the same terminal.
             */
            return if wt_session
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
            {
                Self::NewlineFallback
            } else {
                Self::StandardScrollRegion
            };
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

    /*
     * Tests use the high-level insert path to exercise row counting and escape
     * emission together. Production callers can pass precomputed rendered_rows
     * from history flush state, avoiding a second paragraph render when the
     * pending history window already knows its row cost.
     */
    #[cfg(test)]
    pub(super) fn insert<B: Backend>(
        self,
        terminal: &mut Terminal<B>,
        lines: &[Line<'static>],
    ) -> Result<(), B::Error> {
        let width = terminal.size()?.width;
        let rendered_rows = count_rendered_history_rows(lines, width).min(u16::MAX as usize) as u16;
        self.insert_with_rendered_rows(terminal, lines, rendered_rows)
    }
    pub(super) fn insert_with_rendered_rows<B: Backend>(
        self,
        terminal: &mut Terminal<B>,
        lines: &[Line<'static>],
        rendered_rows: u16,
    ) -> Result<(), B::Error> {
        let width = terminal.size()?.width;
        if width == 0 || rendered_rows == 0 {
            return Ok(());
        }
        let cursor = terminal.get_cursor_position()?;
        /*
         * History insertion is a side effect on host scrollback, not a shell
         * cursor move. Both strategies temporarily move the backend cursor, so
         * the public contract restores the shell-owned cursor afterward.
         */
        let result = match self.mode {
            HistoryInsertionMode::StandardScrollRegion => {
                insert_with_standard_scroll_region(terminal, lines, rendered_rows)
            }
            HistoryInsertionMode::NewlineFallback => {
                let viewport_top = terminal.get_frame().area().top();
                let buffer =
                    rendered_history_buffer_with_height(width, rendered_rows, lines.to_vec());
                insert_with_newline_fallback(terminal, &buffer, viewport_top)
            }
        };
        restore_cursor(terminal, cursor)?;
        result
    }
}

/*
 * The standard path delegates the hard part to ratatui's insert-before support.
 * That keeps wrapping, wide-cell handling, and scroll-region escape generation
 * inside the backend abstraction when the terminal supports it correctly.
 */
fn insert_with_standard_scroll_region<B: Backend>(
    terminal: &mut Terminal<B>,
    lines: &[Line<'static>],
    height: u16,
) -> Result<(), B::Error> {
    terminal.insert_before(height, |buffer| {
        history_paragraph(lines.to_vec()).render(buffer.area, buffer);
    })
}

/*
 * The fallback cannot rely on scroll regions, so it renders history into an
 * off-screen buffer and appends lines from the bottom of the terminal. Rows that
 * exceed the visible viewport are staged in chunks above the shell area before
 * the suffix rows are drawn into the newly-created gap.
 */
fn insert_with_newline_fallback<B: Backend>(
    terminal: &mut Terminal<B>,
    buffer: &Buffer,
    viewport_top: u16,
) -> Result<(), B::Error> {
    let size = terminal.size()?;
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    let pending_rows = buffer.area.height;
    let overflow_pending_rows = pending_rows.saturating_sub(viewport_top);
    let staging_rows = viewport_top.max(1);
    let mut source_y = 0;
    /*
     * Overflow rows are the prefix that cannot fit between terminal top and the
     * inline viewport. They must be appended first from the bottom so existing
     * shell rows stay below the staged history instead of being overwritten.
     */
    while source_y < overflow_pending_rows {
        let rows_this_chunk = (overflow_pending_rows - source_y).min(staging_rows);
        let destination_y = viewport_top.saturating_sub(rows_this_chunk);
        scroll_terminal_from_bottom(terminal, size.height, rows_this_chunk)?;
        draw_buffer_rows_at(terminal, buffer, source_y, rows_this_chunk, destination_y)?;
        source_y += rows_this_chunk;
    }
    let suffix_rows = pending_rows.saturating_sub(overflow_pending_rows);
    if suffix_rows > 0 {
        /*
         * Suffix rows are the portion that fits directly above the inline viewport.
         * They are drawn last because all overflow chunks have already expanded the
         * host scrollback and moved older shell rows out of the way.
         */
        scroll_terminal_from_bottom(terminal, size.height, suffix_rows)?;
        let destination_y = viewport_top.saturating_sub(suffix_rows);
        draw_buffer_rows_at(terminal, buffer, source_y, suffix_rows, destination_y)?;
    }
    terminal.backend_mut().flush()
}

/*
 * append_lines scrolls from the current cursor row. Parking the cursor at the
 * terminal bottom turns append_lines into "create host-scrollback rows" instead
 * of "insert rows inside the live inline viewport".
 */
fn scroll_terminal_from_bottom<B: Backend>(
    terminal: &mut Terminal<B>,
    terminal_height: u16,
    row_count: u16,
) -> Result<(), B::Error> {
    if row_count == 0 || terminal_height == 0 {
        return Ok(());
    }

    terminal.backend_mut().set_cursor_position(Position {
        x: 0,
        y: terminal_height - 1,
    })?;
    terminal.backend_mut().append_lines(row_count)
}

/*
 * Draw one staged buffer row at a time. The source buffer is produced by
 * ratatui Paragraph, so copying cells through a one-row diff preserves hidden
 * wide-character continuation cells instead of turning them into visible spaces.
 */
fn draw_buffer_rows_at<B: Backend>(
    terminal: &mut Terminal<B>,
    buffer: &Buffer,
    source_y: u16,
    row_count: u16,
    destination_y: u16,
) -> Result<(), B::Error> {
    for row_offset in 0..row_count {
        let y = source_y + row_offset;
        let destination_row = destination_y + row_offset;
        terminal.backend_mut().set_cursor_position(Position {
            x: 0,
            y: destination_row,
        })?;
        terminal
            .backend_mut()
            .clear_region(ClearType::CurrentLine)?;
        let row_area = Rect {
            x: 0,
            y: destination_row,
            width: buffer.area.width,
            height: 1,
        };
        let mut rendered_row = Buffer::empty(row_area);
        for x in 0..buffer.area.width {
            rendered_row[(x, destination_row)] = buffer[(x, y)].clone();
        }
        let blank_row = Buffer::empty(row_area);
        terminal
            .backend_mut()
            .draw(blank_row.diff(&rendered_row).into_iter())?;
    }
    terminal.backend_mut().flush()
}

fn restore_cursor<B: Backend>(
    terminal: &mut Terminal<B>,
    cursor: Position,
) -> Result<(), B::Error> {
    /*
     * Terminal resize can race with history insertion in tests and real TUI
     * redraws. Clamp instead of trusting the saved cursor so restoring after a
     * shrink does not ask the backend to move outside the current frame.
     */
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
    /*
     * The row count must match the buffer that will later be inserted. Counting
     * plain lines is not enough because prompt history includes long URLs, tabs,
     * CJK width, and ratatui wrapping behavior.
     */
    if width == 0 || lines.is_empty() {
        return 0;
    }

    rendered_history_height(width, lines)
}

#[cfg(test)]
pub(super) fn rendered_history_buffer(width: u16, lines: &[Line<'static>]) -> Buffer {
    let height = count_rendered_history_rows(lines, width).min(u16::MAX as usize) as u16;
    rendered_history_buffer_with_height(width, height, lines.to_vec())
}

fn rendered_history_buffer_with_height<'a>(
    width: u16,
    height: u16,
    text: impl Into<Text<'a>>,
) -> Buffer {
    /*
     * This buffer is the single source for fallback insertion. The area starts at
     * y=0 even though rows will later be drawn at viewport-relative positions;
     * draw_buffer_rows_at performs that coordinate translation explicitly.
     */
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

    history_paragraph(text).render(buffer.area, &mut buffer);
    buffer
}

fn history_paragraph<'a>(text: impl Into<Text<'a>>) -> Paragraph<'a> {
    /*
     * trim=false is part of the transcript contract: command output, Markdown
     * code blocks, and indentation-sensitive snippets must not lose trailing
     * spaces merely because they are moving into host scrollback.
     */
    Paragraph::new(text).wrap(Wrap { trim: false })
}

/*
 * Ratatui paragraph wrapping is the source of truth for row count. A sentinel
 * line lets us render once into a bounded probe buffer and locate the first row
 * after real history without reimplementing text-width and wrapping rules.
 */
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
    history_paragraph(probe_lines).render(area, &mut buffer);
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
    /*
     * line.width() is an upper bound for wrapped rows when width is at least one.
     * It can over-allocate, but the sentinel probe below trims the actual height
     * without risking truncation of wide or wrapped content.
     */
    lines.iter().map(|line| line.width().max(1)).sum::<usize>()
}
fn sentinel_line() -> Line<'static> {
    Line::from(Span::styled("X", sentinel_style()))
}
fn sentinel_style() -> Style {
    /*
     * The sentinel uses an unlikely fg/bg pair rather than text comparison. Real
     * history can contain any glyph, but it should not carry this synthetic style.
     */
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
    use super::{
        HistoryInsertionAdapter, HistoryInsertionMode, count_rendered_history_rows,
        rendered_history_buffer,
    };
    use crate::adapter::inbound::tui::app::InlineHistoryRenderMode;
    use crate::adapter::inbound::tui::app::tui_testkit;
    use ratatui::layout::Position;
    use ratatui::text::{Line, Span};
    use std::io::Write;
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
    fn history_insertion_mode_uses_newline_fallback_for_windows_terminal() {
        assert_eq!(
            HistoryInsertionMode::from_env_and_terminal_values(None, Some("wt-session-id")),
            HistoryInsertionMode::NewlineFallback
        );
        assert_eq!(
            HistoryInsertionMode::from_env_and_terminal_values(Some("standard"), Some("wt")),
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
    fn rendered_history_rows_preserve_wrapped_parallel_notice_suffix() {
        let lines = vec![Line::from(vec![
            Span::raw("[--:--:--] "),
            Span::raw("Supervisor: "),
            Span::raw(
                "parallel board refreshed. control tower is live in read-only supervisor mode",
            ),
        ])];
        let buffer = rendered_history_buffer(80, &lines);
        let text = tui_testkit::buffer_text(&buffer);

        assert_eq!(count_rendered_history_rows(&lines, 80), 2);
        assert!(
            text.contains("read-only supervisor mode"),
            "wrapped notice suffix should remain in rendered history: {text:?}"
        );
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
    fn standard_scroll_region_preserves_wrapped_parallel_notice_suffix() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let lines = vec![Line::from(vec![
            Span::raw("[--:--:--] "),
            Span::raw("Supervisor: "),
            Span::raw(
                "parallel board refreshed. control tower is live in read-only supervisor mode",
            ),
        ])];

        HistoryInsertionAdapter::new(HistoryInsertionMode::StandardScrollRegion)
            .insert(&mut terminal, &lines)
            .unwrap();
        let terminal_history = tui_testkit::inline_terminal_history_text(&terminal);

        assert!(
            terminal_history.contains("read-only supervisor mode"),
            "standard insertion should preserve wrapped suffix rows: {terminal_history:?}"
        );
    }
    #[test]
    fn standard_scroll_region_preserves_wrapped_notice_before_ledger_rows() {
        let mut terminal =
            tui_testkit::inline_history_terminal(InlineHistoryRenderMode::HostScrollback, 80, 24);
        let lines = vec![
            Line::from(vec![
                Span::raw("[--:--:--] "),
                Span::raw("Supervisor: "),
                Span::raw(
                    "parallel board refreshed. control tower is live in read-only supervisor mode",
                ),
            ]),
            Line::from("[--:--:--] Ledger: reported stage record: no agent results reported yet"),
            Line::from(
                "[--:--:--] Ledger: ledger refreshing stage record: no official refresh workers are active",
            ),
            Line::from("[--:--:--] Ledger: official stage record: nothing is queued for merge"),
            Line::from(
                "[--:--:--] Ledger: merge queued stage record: no distributor queue items are waiting",
            ),
            Line::from(
                "[--:--:--] Ledger: merged stage record: nothing has been integrated into prerelease yet",
            ),
        ];

        HistoryInsertionAdapter::new(HistoryInsertionMode::StandardScrollRegion)
            .insert(&mut terminal, &lines)
            .unwrap();
        let terminal_history = tui_testkit::inline_terminal_history_text(&terminal);

        assert!(
            terminal_history.contains("read-only supervisor mode"),
            "standard insertion should not blank the wrapped notice before ledger rows: {terminal_history:?}"
        );
        assert!(
            terminal_history.contains("no agent results reported yet"),
            "ledger rows should still follow the wrapped notice: {terminal_history:?}"
        );
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
        let rendered = tui_testkit::inline_terminal_history_text(&terminal);
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
    #[test]
    fn newline_fallback_preserves_shell_rows_by_scrolling_before_insert() {
        /*
         * The fallback must behave like scrollback append, not like repainting the
         * live shell viewport. Seeding visible shell rows before insertion proves
         * history creation does not erase prompt-owned content.
         */
        let mut terminal = tui_testkit::inline_history_vt100_terminal(
            InlineHistoryRenderMode::HostScrollback,
            30,
            20,
        );
        terminal
            .backend_mut()
            .inner_mut()
            .write_all(b"SHELL_ONE\r\nSHELL_TWO\r\nSHELL_THREE\r\nSHELL_FOUR")
            .unwrap();
        let lines = vec![
            Line::from("history one"),
            Line::from("history two"),
            Line::from("history three"),
            Line::from("history four"),
            Line::from("history five"),
            Line::from("history six"),
        ];
        HistoryInsertionAdapter::new(HistoryInsertionMode::NewlineFallback)
            .insert(&mut terminal, &lines)
            .unwrap();
        let terminal_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
        for marker in [
            "SHELL_ONE",
            "SHELL_TWO",
            "SHELL_THREE",
            "SHELL_FOUR",
            "history one",
            "history two",
            "history three",
            "history four",
            "history five",
            "history six",
        ] {
            assert!(
                terminal_history.contains(marker),
                "newline fallback should preserve {marker}: {terminal_history:?}"
            );
        }
    }
    #[test]
    fn newline_fallback_keeps_hangul_graphemes_compact() {
        /*
         * Wide cells are the easiest place for a buffer-copy fallback to leak
         * hidden continuation cells as spaces. Hangul text keeps that regression
         * visible without depending on emoji rendering differences.
         */
        let mut terminal = tui_testkit::inline_history_vt100_terminal(
            InlineHistoryRenderMode::HostScrollback,
            40,
            16,
        );
        let lines = vec![
            Line::from("동화 설명해 주세요"),
            Line::from("한글 간격이 벌어지면 안 됩니다"),
        ];
        HistoryInsertionAdapter::new(HistoryInsertionMode::NewlineFallback)
            .insert(&mut terminal, &lines)
            .unwrap();
        let terminal_history = tui_testkit::inline_vt100_scrollback_text(&mut terminal);
        assert!(
            terminal_history.contains("동화 설명해 주세요"),
            "newline fallback should keep Hangul contiguous: {terminal_history:?}"
        );
        assert!(
            terminal_history.contains("한글 간격이 벌어지면 안 됩니다"),
            "newline fallback should keep wrapped Hangul contiguous: {terminal_history:?}"
        );
        assert!(
            !terminal_history.contains("동 화"),
            "newline fallback should not expose hidden Hangul cells as spaces: {terminal_history:?}"
        );
    }
}
