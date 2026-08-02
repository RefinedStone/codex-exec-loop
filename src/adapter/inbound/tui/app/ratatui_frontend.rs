use std::io;
use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use crossterm::cursor::Show;
use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use super::TerminalUiEffect;
use super::fullscreen_terminal_adapter::FullscreenTerminalAdapter;
use super::shell_runtime::ShellRuntime;

const READY_EVENT_DRAIN_LIMIT: usize = 64;

/*
 * 이 모듈은 TUI의 concrete terminal boundary다. ShellRuntime은 app state, background
 * message reduction, draw scheduling, key/focus/resize semantics를 소유하고, 이 파일은
 * stdout/crossterm/raw-mode/ratatui Terminal을 연결하는 IO pump만 맡는다.
 */
pub(super) fn run(
    mut runtime: ShellRuntime,
    shutdown: &crate::shutdown::GracefulShutdown,
) -> Result<()> {
    if shutdown.is_requested() {
        return Ok(());
    }
    /*
     * raw mode와 focus subscription은 host terminal에 남는 side effect라 terminal 생성보다
     * 먼저 guard로 감싼다. 이후 backend 생성, draw, event read 중 어디서 실패해도 Drop이
     * 사용자 shell을 복구하는 단일 경로가 된다.
     */
    let mut restore_guard =
        TerminalRestoreGuard::activate(runtime.terminal_mouse_capture_enabled())?;
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = build_terminal(backend)?;
    let mut adapter = FullscreenTerminalAdapter::new(terminal);
    run_event_loop(&mut adapter, &mut runtime, shutdown, &mut restore_guard)
}

// Fullscreen Ratatui owns the alternate-screen surface directly; no second
// terminal backend or host-history synchronization layer participates.
fn build_terminal(
    backend: CrosstermBackend<io::Stdout>,
) -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    Terminal::new(backend)
}

/*
 * Event loop는 deliberately thin하다. 매 반복에서 background channel을 비우고, scheduler가
 * due라고 판단한 경우에만 draw transaction을 실행한 뒤, crossterm event를 읽어 runtime
 * reducer에 넘긴다. key binding, resize, focus lost 같은 의미 해석은 이 층에 두지 않는다.
 */
fn run_event_loop(
    adapter: &mut FullscreenTerminalAdapter<CrosstermBackend<io::Stdout>>,
    runtime: &mut ShellRuntime,
    shutdown: &crate::shutdown::GracefulShutdown,
    terminal_session: &mut TerminalRestoreGuard,
) -> Result<()> {
    match run_event_loop_until_exit(adapter, runtime, shutdown, terminal_session) {
        /*
         * Closing a Unix PTY can make the terminal descriptor report EIO before the process-level
         * SIGHUP flag becomes visible. Broken pipes and EOF are equivalent output/input closure
         * signals on other terminal backends. They are a normal lifecycle boundary, not an
         * application failure; the outer stack will drop the runtime and its app-server children.
         */
        Err(error) if terminal_disconnected(&error) => Ok(()),
        result => result,
    }
}

fn run_event_loop_until_exit(
    adapter: &mut FullscreenTerminalAdapter<CrosstermBackend<io::Stdout>>,
    runtime: &mut ShellRuntime,
    shutdown: &crate::shutdown::GracefulShutdown,
    terminal_session: &mut TerminalRestoreGuard,
) -> Result<()> {
    while !runtime.should_quit() && !shutdown.is_requested() {
        /*
         * app-server stream, startup/session load, post-turn evaluation은 terminal input과 별개로
         * 들어온다. 이벤트 poll 전에 먼저 반영해야 사용자가 입력하지 않아도 화면이 stale하지 않다.
         * 이미 준비된 terminal event도 draw deadline을 소비하기 전에 반영해야 같은 크기로 돌아온
         * resize ABA가 cursor와 history 보정에서 누락되지 않는다.
         */
        let draw_due = prepare_runtime_for_due_draw(runtime, read_ready_terminal_event)?;
        apply_pending_terminal_ui_effects(runtime, terminal_session)?;
        if runtime.should_quit() {
            break;
        }
        if draw_due {
            let transaction_completed = adapter.draw_fullscreen_transaction(runtime)?;
            runtime.finish_pending_quit_after_transaction(transaction_completed);
        }
        /*
         * poll timeout은 기본 idle wait와 다음 scheduled draw deadline의 교집합이다. 입력이 없어도
         * delayed draw 시점에는 poll이 깨어나 frame coalescing이 실제 화면에 반영된다.
         */
        let poll_timeout =
            runtime.next_event_poll_timeout(std::time::Instant::now(), Duration::from_millis(100));
        if !event::poll(poll_timeout)? {
            continue;
        }

        /*
         * crossterm event는 여기서 해석하지 않는다. frontend가 raw event를 그대로 넘겨야 runtime의
         * reducer, draw scheduler, overlay state가 한곳에서 동일한 정책으로 반응할 수 있다.
         */
        runtime.handle_terminal_event(event::read()?);
        drain_ready_terminal_events(runtime)?;
        apply_pending_terminal_ui_effects(runtime, terminal_session)?;
    }

    Ok(())
}

fn terminal_disconnected(error: &anyhow::Error) -> bool {
    error.chain().any(|source| {
        source
            .downcast_ref::<io::Error>()
            .is_some_and(terminal_io_disconnected)
    })
}

fn terminal_io_disconnected(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof
    ) {
        return true;
    }
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::EIO)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

pub(super) fn prepare_runtime_for_due_draw(
    runtime: &mut ShellRuntime,
    read_ready_event: impl FnMut() -> Result<Option<event::Event>>,
) -> Result<bool> {
    runtime.poll_background_messages();
    drain_ready_terminal_events_with(runtime, read_ready_event)?;
    if runtime.should_quit() {
        return Ok(false);
    }
    Ok(runtime.take_due_draw_request(Instant::now()))
}

fn drain_ready_terminal_events(runtime: &mut ShellRuntime) -> Result<()> {
    drain_ready_terminal_events_with(runtime, read_ready_terminal_event)
}

fn read_ready_terminal_event() -> Result<Option<event::Event>> {
    if !event::poll(Duration::ZERO)? {
        return Ok(None);
    }
    Ok(Some(event::read()?))
}

fn drain_ready_terminal_events_with(
    runtime: &mut ShellRuntime,
    mut read_ready_event: impl FnMut() -> Result<Option<event::Event>>,
) -> Result<()> {
    /*
     * Terminal emulators can queue several input or resize events while a frame
     * is being rendered. Draining the ready batch lets the scheduler coalesce
     * them without losing intermediate resize epochs.
     */
    for _ in 0..READY_EVENT_DRAIN_LIMIT {
        if runtime.should_quit() {
            break;
        }
        let Some(event) = read_ready_event()? else {
            break;
        };
        runtime.handle_terminal_event(event);
    }
    Ok(())
}

fn apply_pending_terminal_ui_effects(
    runtime: &mut ShellRuntime,
    terminal_session: &mut TerminalRestoreGuard,
) -> Result<()> {
    for effect in runtime.take_terminal_ui_effects() {
        match effect {
            TerminalUiEffect::CopyToClipboard(text) => write_terminal_clipboard(&text)?,
            TerminalUiEffect::SetMouseCapture(enabled) => {
                terminal_session.set_mouse_capture(enabled)?;
            }
        }
    }
    Ok(())
}

fn write_terminal_clipboard(text: &str) -> io::Result<()> {
    let tmux_passthrough = std::env::var_os("TMUX").is_some();
    let sequence = terminal_clipboard_sequence(text, tmux_passthrough);
    let mut stdout = io::stdout();
    stdout.write_all(sequence.as_bytes())?;
    stdout.flush()
}

fn terminal_clipboard_sequence(text: &str, tmux_passthrough: bool) -> String {
    let encoded = BASE64_STANDARD.encode(text.as_bytes());
    let osc52 = format!("\u{1b}]52;c;{encoded}\u{7}");
    if !tmux_passthrough {
        return osc52;
    }
    let escaped = osc52.replace('\u{1b}', "\u{1b}\u{1b}");
    format!("\u{1b}Ptmux;{escaped}\u{1b}\\")
}

/*
 * RAII guard의 activate 성공은 raw mode, focus-change event 구독, paste event 구독을
 * frontend가 소유한다는 뜻이고, Drop은 정상 종료, 오류 반환, early return 모두에서 복구를 시도한다.
 */
struct TerminalRestoreGuard {
    bracketed_paste_enabled: bool,
    mouse_capture_enabled: bool,
}

impl TerminalRestoreGuard {
    fn activate(mouse_capture_enabled: bool) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        /*
         * focus events는 focus lost 중 draw를 늦추는 runtime scheduler 정책의 입력이다. enable이
         * 실패하면 raw mode만 켜진 반쪽 상태가 되므로 즉시 되돌리고 startup 실패로 전파한다.
         */
        if let Err(error) = enter_fullscreen_terminal_session(&mut stdout, mouse_capture_enabled) {
            let _ = leave_fullscreen_terminal_session(&mut stdout, false, mouse_capture_enabled);
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        let bracketed_paste_enabled = enable_bracketed_paste(&mut stdout).is_ok();
        Ok(Self {
            bracketed_paste_enabled,
            mouse_capture_enabled,
        })
    }

    fn set_mouse_capture(&mut self, enabled: bool) -> io::Result<()> {
        if self.mouse_capture_enabled == enabled {
            return Ok(());
        }
        let mut stdout = io::stdout();
        if enabled {
            execute!(stdout, event::EnableMouseCapture)?;
        } else {
            execute!(stdout, event::DisableMouseCapture)?;
        }
        self.mouse_capture_enabled = enabled;
        Ok(())
    }
}

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        /*
         * Drop에서는 error를 반환할 수 없으므로 cleanup은 모두 best-effort다. 어느 한 command가
         * 실패해도 raw mode 해제, focus 구독 해제, cursor 복구를 계속 시도하는 편이 낫다.
         */
        let mut stdout = io::stdout();
        let _ = leave_fullscreen_terminal_session(
            &mut stdout,
            self.bracketed_paste_enabled,
            self.mouse_capture_enabled,
        );
        let _ = disable_raw_mode();
    }
}

fn enter_fullscreen_terminal_session(
    writer: &mut impl Write,
    mouse_capture_enabled: bool,
) -> io::Result<()> {
    execute!(writer, EnterAlternateScreen, event::EnableFocusChange)?;
    if mouse_capture_enabled {
        execute!(writer, event::EnableMouseCapture)?;
    }
    Ok(())
}

fn enable_bracketed_paste(writer: &mut impl Write) -> io::Result<()> {
    execute!(writer, event::EnableBracketedPaste)
}

fn leave_fullscreen_terminal_session(
    writer: &mut impl Write,
    bracketed_paste_enabled: bool,
    mouse_capture_enabled: bool,
) -> io::Result<()> {
    let mut first_error = None;
    if mouse_capture_enabled {
        remember_terminal_restore_error(
            &mut first_error,
            execute!(writer, event::DisableMouseCapture),
        );
    }
    if bracketed_paste_enabled {
        remember_terminal_restore_error(
            &mut first_error,
            execute!(writer, event::DisableBracketedPaste),
        );
    }
    remember_terminal_restore_error(
        &mut first_error,
        execute!(writer, event::DisableFocusChange),
    );
    remember_terminal_restore_error(&mut first_error, execute!(writer, LeaveAlternateScreen));
    remember_terminal_restore_error(&mut first_error, execute!(writer, Show));
    first_error.map_or(Ok(()), Err)
}

fn remember_terminal_restore_error(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result
        && first_error.is_none()
    {
        *first_error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    #[cfg(unix)]
    use super::terminal_disconnected;
    use super::{
        enable_bracketed_paste, enter_fullscreen_terminal_session,
        leave_fullscreen_terminal_session, terminal_clipboard_sequence, terminal_io_disconnected,
    };

    #[test]
    fn fullscreen_terminal_session_enters_and_restores_every_owned_mode() {
        let mut entered = Vec::new();
        enter_fullscreen_terminal_session(&mut entered, true).expect("enter commands");
        enable_bracketed_paste(&mut entered).expect("paste command");
        let entered = String::from_utf8(entered).expect("terminal commands are utf-8 escape bytes");
        assert!(entered.contains("\u{1b}[?1049h"));
        assert!(entered.contains("\u{1b}[?1004h"));
        assert!(entered.contains("\u{1b}[?2004h"));

        let mut restored = Vec::new();
        leave_fullscreen_terminal_session(&mut restored, true, true).expect("restore commands");
        let restored =
            String::from_utf8(restored).expect("terminal commands are utf-8 escape bytes");
        assert!(restored.contains("\u{1b}[?2004l"));
        assert!(restored.contains("\u{1b}[?1004l"));
        assert!(restored.contains("\u{1b}[?1049l"));
        assert!(restored.contains("\u{1b}[?25h"));
    }

    #[test]
    fn native_selection_mode_skips_mouse_reporting_lifecycle() {
        let mut entered = Vec::new();
        enter_fullscreen_terminal_session(&mut entered, false).expect("enter commands");
        let entered = String::from_utf8(entered).expect("terminal commands are utf-8 escape bytes");
        assert!(entered.contains("\u{1b}[?1049h"));
        assert!(!entered.contains("\u{1b}[?1000h"));

        let mut restored = Vec::new();
        leave_fullscreen_terminal_session(&mut restored, false, false).expect("restore commands");
        let restored =
            String::from_utf8(restored).expect("terminal commands are utf-8 escape bytes");
        assert!(!restored.contains("\u{1b}[?1000l"));
        assert!(restored.contains("\u{1b}[?1049l"));
    }

    #[test]
    fn clipboard_sequence_supports_direct_and_tmux_osc52() {
        let direct = terminal_clipboard_sequence("hello", false);
        assert_eq!(direct, "\u{1b}]52;c;aGVsbG8=\u{7}");

        let tmux = terminal_clipboard_sequence("hello", true);
        assert!(tmux.starts_with("\u{1b}Ptmux;\u{1b}\u{1b}]52;c;aGVsbG8="));
        assert!(tmux.ends_with("\u{7}\u{1b}\\"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_pty_eio_is_a_terminal_disconnect() {
        let error = io::Error::from_raw_os_error(libc::EIO);

        assert!(terminal_io_disconnected(&error));
        assert!(terminal_disconnected(&anyhow::Error::new(error)));
    }

    #[test]
    fn broken_pipe_is_a_terminal_disconnect() {
        assert!(terminal_io_disconnected(&io::Error::new(
            io::ErrorKind::BrokenPipe,
            "terminal output closed",
        )));
    }

    #[test]
    fn unrelated_terminal_io_errors_remain_failures() {
        assert!(!terminal_io_disconnected(&io::Error::new(
            io::ErrorKind::PermissionDenied,
            "terminal permissions changed",
        )));
    }
}
