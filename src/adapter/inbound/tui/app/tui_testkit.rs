use super::inline_terminal_adapter::{InlineTerminalBackend, terminal_options_for_render_mode};
use super::shell_rendering::draw;
use super::{
    ConversationInputState, ConversationMessage, ConversationMessageKind, ConversationState,
    INLINE_VIEWPORT_HEIGHT, InlineHistoryRenderMode, NativeTuiApp, ShellFrontendMode,
};
use ratatui::backend::{Backend, ClearType, CrosstermBackend, TestBackend, WindowSize};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Size};
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Write};

// VT100-backed helpers keep a larger scrollback than the visible viewport so
// inline rendering tests can assert both host scrollback and current-screen text.
const DEFAULT_VT100_SCROLLBACK_ROWS: usize = 256;

// Test terminals mirror the production frontend modes: plain TestBackend for
// cell-level assertions and InlineTerminalBackend when host scrollback behavior
// is part of the contract.
pub(super) fn inline_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::with_options(
        TestBackend::new(width, height),
        TerminalOptions {
            viewport: Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
        },
    )
    .expect("inline test terminal")
}
pub(super) fn shell_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).expect("test terminal")
}
pub(super) fn inline_history_terminal(
    render_mode: InlineHistoryRenderMode,
    width: u16,
    height: u16,
) -> Terminal<InlineTerminalBackend<TestBackend>> {
    Terminal::with_options(
        InlineTerminalBackend::new(TestBackend::new(width, height)),
        terminal_options_for_render_mode(render_mode),
    )
    .expect("inline history test terminal")
}
pub(super) fn inline_history_vt100_terminal(
    render_mode: InlineHistoryRenderMode,
    width: u16,
    height: u16,
) -> Terminal<InlineTerminalBackend<Vt100Backend>> {
    Terminal::with_options(
        InlineTerminalBackend::new(Vt100Backend::new(width, height)),
        terminal_options_for_render_mode(render_mode),
    )
    .expect("inline vt100 history test terminal")
}
pub(super) fn resize_terminal(terminal: &mut Terminal<TestBackend>, width: u16, height: u16) {
    terminal.backend_mut().resize(width, height);
}
pub(super) fn resize_inline_history_terminal(
    terminal: &mut Terminal<InlineTerminalBackend<TestBackend>>,
    width: u16,
    height: u16,
) {
    terminal.backend_mut().inner_mut().resize(width, height);
}
pub(super) fn resize_inline_history_vt100_terminal(
    terminal: &mut Terminal<InlineTerminalBackend<Vt100Backend>>,
    width: u16,
    height: u16,
) {
    terminal.backend_mut().inner_mut().resize(width, height);
}
pub(super) fn render_inline_snapshot(app: &mut NativeTuiApp, width: u16, height: u16) -> String {
    // Snapshot helpers render through the real shell draw path so tests exercise
    // the same presentation composition as the TUI runtime.
    let mut terminal = inline_terminal(width, height);
    terminal
        .draw(|frame| draw(frame, app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    screen_text(&terminal)
}
pub(super) fn render_inline_vt100_snapshot(
    app: &mut NativeTuiApp,
    width: u16,
    height: u16,
) -> String {
    let mut terminal = inline_terminal(width, height);
    terminal
        .draw(|frame| draw(frame, app, ShellFrontendMode::InlineMainBuffer))
        .expect("inline render succeeds");
    vt100_contents_from_buffer(terminal.backend().buffer())
}
pub(super) fn render_shell_snapshot(app: &mut NativeTuiApp, width: u16, height: u16) -> String {
    let mut terminal = shell_terminal(width, height);
    terminal
        .draw(|frame| draw(frame, app, ShellFrontendMode::InlineMainBuffer))
        .expect("shell render succeeds");
    screen_text(&terminal)
}
pub(super) fn render_shell_vt100_snapshot(
    app: &mut NativeTuiApp,
    width: u16,
    height: u16,
) -> String {
    let mut terminal = shell_terminal(width, height);
    terminal
        .draw(|frame| draw(frame, app, ShellFrontendMode::InlineMainBuffer))
        .expect("shell render succeeds");
    vt100_contents_from_buffer(terminal.backend().buffer())
}
pub(super) fn screen_text<B>(terminal: &Terminal<B>) -> String
where
    B: Backend + std::fmt::Display,
{
    format!("{}", terminal.backend())
}
pub(super) fn inline_scrollback_text(
    terminal: &Terminal<InlineTerminalBackend<TestBackend>>,
) -> String {
    buffer_text(terminal.backend().inner().scrollback())
}
pub(super) fn inline_terminal_history_text(
    terminal: &Terminal<InlineTerminalBackend<TestBackend>>,
) -> String {
    // Inline history assertions need the persisted scrollback followed by the
    // active viewport, matching what a user sees in their terminal history.
    let scrollback = inline_scrollback_text(terminal);
    let screen = screen_text(terminal);
    if scrollback.trim().is_empty() {
        screen
    } else {
        format!("{scrollback}\n{screen}")
    }
}
pub(super) fn inline_vt100_scrollback_text(
    terminal: &mut Terminal<InlineTerminalBackend<Vt100Backend>>,
) -> String {
    terminal
        .backend_mut()
        .inner_mut()
        .scrollback_rows()
        .into_iter()
        .map(|row| row.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
pub(super) fn inline_vt100_host_scrollback_text(
    terminal: &mut Terminal<InlineTerminalBackend<Vt100Backend>>,
) -> String {
    terminal
        .backend_mut()
        .inner_mut()
        .host_scrollback_rows()
        .into_iter()
        .map(|row| row.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}
pub(super) fn buffer_text(buffer: &Buffer) -> String {
    if buffer.area.width == 0 {
        return String::new();
    }

    buffer
        .content
        .chunks(buffer.area.width as usize)
        .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn vt100_contents_from_buffer(buffer: &Buffer) -> String {
    if buffer.area.width == 0 {
        return String::new();
    }
    // Ratatui buffers are cell grids; feed their symbols through vt100 so tests
    // observe terminal wrapping and line endings instead of raw cell padding.
    let mut screen = Vt100Screen::new(buffer.area.width, buffer.area.height);
    for (index, row) in buffer
        .content
        .chunks(buffer.area.width as usize)
        .enumerate()
    {
        for cell in row {
            screen.process(cell.symbol().as_bytes());
        }
        if index + 1 < buffer.area.height as usize {
            screen.process(b"\r\n");
        }
    }
    trim_line_end_padding(&screen.contents())
}
fn trim_line_end_padding(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}
pub(super) fn append_agent_history_message(app: &mut NativeTuiApp, text: &str) {
    // History injection keeps fixtures small while still rebuilding the cached
    // formatted transcript lines expected by renderers.
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        text.to_string(),
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.refresh_conversation_lines();
}
pub(super) fn set_live_agent_message(app: &mut NativeTuiApp, text: &str) {
    // Live-message injection sets the same running-turn markers used by runtime
    // background updates so inline tail tests cover the streaming path.
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start in a ready conversation state");
    };
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-1".to_string());
    conversation.active_turn_started_at = Some(std::time::Instant::now());
    conversation.live_agent_message = Some(ConversationMessage::new(
        ConversationMessageKind::Agent,
        text.to_string(),
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
}
pub(super) struct Vt100Screen {
    parser: vt100::Parser,
    width: u16,
}

impl Vt100Screen {
    pub(super) fn new(width: u16, height: u16) -> Self {
        Self {
            parser: vt100::Parser::new(height, width, DEFAULT_VT100_SCROLLBACK_ROWS),
            width,
        }
    }
    pub(super) fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }
    pub(super) fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.parser.screen_mut().set_size(height, width);
    }
    pub(super) fn contents(&self) -> String {
        self.parser.screen().contents()
    }
    pub(super) fn rows(&self) -> Vec<String> {
        self.parser.screen().rows(0, self.width).collect()
    }
    pub(super) fn scrollback_rows(&mut self) -> Vec<String> {
        // vt100 exposes scrollback by moving the viewport; restore the original
        // offset after collecting rows so callers can continue using the screen.
        let normal_offset = self.parser.screen().scrollback();
        self.parser.screen_mut().set_scrollback(0);
        let mut rows = VecDeque::from(self.rows());
        for offset in 1.. {
            self.parser.screen_mut().set_scrollback(offset);
            if self.parser.screen().scrollback() != offset {
                break;
            }
            if let Some(top_row) = self.parser.screen().rows(0, self.width).next() {
                rows.push_front(top_row);
            }
        }

        self.parser.screen_mut().set_scrollback(normal_offset);
        rows.into_iter().collect()
    }
}
pub(super) fn vt100_contents(width: u16, height: u16, bytes: &[u8]) -> String {
    let mut screen = Vt100Screen::new(width, height);
    screen.process(bytes);
    screen.contents()
}
pub(super) struct Vt100Backend {
    backend: CrosstermBackend<vt100::Parser>,
    width: u16,
    height: u16,
}

impl Vt100Backend {
    pub(super) fn new(width: u16, height: u16) -> Self {
        // Crossterm normally detects color support from the process environment;
        // tests force color so style-sensitive rendering stays deterministic.
        crossterm::style::force_color_output(true);
        Self {
            backend: CrosstermBackend::new(vt100::Parser::new(
                height,
                width,
                DEFAULT_VT100_SCROLLBACK_ROWS,
            )),
            width,
            height,
        }
    }
    pub(super) fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.backend
            .writer_mut()
            .screen_mut()
            .set_size(height, width);
    }
    fn parser(&self) -> &vt100::Parser {
        self.backend.writer()
    }
    fn parser_mut(&mut self) -> &mut vt100::Parser {
        self.backend.writer_mut()
    }
    pub(super) fn scrollback_rows(&mut self) -> Vec<String> {
        let normal_offset = self.parser().screen().scrollback();
        self.parser_mut().screen_mut().set_scrollback(0);
        let mut rows = VecDeque::from(self.rows());
        for offset in 1.. {
            self.parser_mut().screen_mut().set_scrollback(offset);
            if self.parser().screen().scrollback() != offset {
                break;
            }
            if let Some(top_row) = self.parser().screen().rows(0, self.width).next() {
                rows.push_front(top_row);
            }
        }

        self.parser_mut().screen_mut().set_scrollback(normal_offset);
        rows.into_iter().collect()
    }
    pub(super) fn host_scrollback_rows(&mut self) -> Vec<String> {
        // Host scrollback excludes the currently visible rows, which lets tests
        // distinguish append_lines behavior from the active screen contents.
        let normal_offset = self.parser().screen().scrollback();
        let mut rows = VecDeque::new();
        for offset in 1.. {
            self.parser_mut().screen_mut().set_scrollback(offset);
            if self.parser().screen().scrollback() != offset {
                break;
            }
            if let Some(top_row) = self.parser().screen().rows(0, self.width).next() {
                rows.push_front(top_row);
            }
        }

        self.parser_mut().screen_mut().set_scrollback(normal_offset);
        rows.into_iter().collect()
    }
    fn rows(&self) -> Vec<String> {
        self.parser().screen().rows(0, self.width).collect()
    }
}
impl Write for Vt100Backend {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.backend.writer_mut().write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.backend.writer_mut().flush()
    }
}
impl fmt::Display for Vt100Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.parser().screen().contents())
    }
}
impl Backend for Vt100Backend {
    type Error = io::Error;
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
    {
        self.backend.draw(content)
    }
    fn hide_cursor(&mut self) -> io::Result<()> {
        self.backend.hide_cursor()
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        self.backend.show_cursor()
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.parser().screen().cursor_position().into())
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.backend.set_cursor_position(position)
    }
    fn clear(&mut self) -> io::Result<()> {
        self.backend.clear()
    }
    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.backend.clear_region(clear_type)
    }
    fn append_lines(&mut self, line_count: u16) -> io::Result<()> {
        self.backend.append_lines(line_count)
    }
    fn size(&self) -> io::Result<Size> {
        Ok(Size::new(self.width, self.height))
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: Size::new(self.width, self.height),
            pixels: Size::new(self.width.saturating_mul(8), self.height.saturating_mul(16)),
        })
    }
    fn flush(&mut self) -> io::Result<()> {
        self.backend.writer_mut().flush()
    }
    fn scroll_region_up(
        &mut self,
        region: std::ops::Range<u16>,
        line_count: u16,
    ) -> io::Result<()> {
        self.backend.scroll_region_up(region, line_count)
    }
    fn scroll_region_down(
        &mut self,
        region: std::ops::Range<u16>,
        line_count: u16,
    ) -> io::Result<()> {
        self.backend.scroll_region_down(region, line_count)
    }
}
#[cfg(test)]
mod tests {
    use super::{Vt100Backend, Vt100Screen, resize_terminal, shell_terminal, vt100_contents};
    use ratatui::backend::Backend;
    use std::io::Write;
    #[test]
    fn vt100_contents_tracks_visible_screen_text() {
        assert_eq!(vt100_contents(12, 3, b"ready\nshell"), "ready\n     shell");
    }
    #[test]
    fn vt100_screen_can_resize_between_frames() {
        let mut screen = Vt100Screen::new(12, 3);
        screen.process(b"first line");
        screen.resize(20, 3);
        screen.process(b"\r\nsecond line");

        assert!(screen.contents().contains("second line"));
        assert_eq!(screen.rows().len(), 3);
    }
    #[test]
    fn vt100_screen_exposes_scrollback_view() {
        let mut screen = Vt100Screen::new(10, 2);
        screen.process(b"one\r\ntwo\r\nthree");

        assert!(
            screen
                .scrollback_rows()
                .iter()
                .any(|row| row.contains("one"))
        );
    }
    #[test]
    fn ratatui_test_backend_resize_is_shared() {
        let mut terminal = shell_terminal(10, 2);

        resize_terminal(&mut terminal, 12, 4);
        let size = terminal.size().expect("test terminal size");
        assert_eq!(size.width, 12);
        assert_eq!(size.height, 4);
    }
    #[test]
    fn vt100_backend_tracks_cursor_scrollback_and_resize() {
        let mut backend = Vt100Backend::new(10, 2);

        backend.write_all(b"one\r\ntwo\r\nthree").unwrap();
        assert!(backend.to_string().contains("three"));
        assert!(
            backend
                .scrollback_rows()
                .iter()
                .any(|row| row.contains("one"))
        );

        backend.resize(12, 3);
        let size = backend.size().expect("vt100 backend size");
        assert_eq!(size.width, 12);
        assert_eq!(size.height, 3);
    }
    #[test]
    fn vt100_backend_append_lines_scrolls_top_row_into_host_scrollback() {
        let mut backend = Vt100Backend::new(10, 2);
        let buffer = ratatui::buffer::Buffer::with_lines(["history"]);

        backend
            .draw((0..7).map(|x| (x, 0, &buffer[(x, 0)])))
            .expect("draw top row");
        backend
            .set_cursor_position(ratatui::layout::Position { x: 0, y: 1 })
            .expect("move cursor");
        backend.append_lines(1).expect("append line");
        let scrollback = backend.host_scrollback_rows().join("\n");
        assert!(
            scrollback.contains("history"),
            "expected drawn row in host scrollback: {scrollback:?}"
        );
    }
}
