use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::cursor::{MoveToNextLine, Show};
use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use super::inline_terminal_adapter::{
    InlineTerminalAdapter, InlineTerminalBackend, terminal_options_for_render_mode,
};
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
    let mut restore_guard = TerminalRestoreGuard::activate()?;
    let backend = CrosstermBackend::new(io::stdout());
    /*
     * inline history mode는 ratatui TerminalOptions와 adapter의 history 정책이 같은 전제를
     * 보게 해야 한다. 그래서 Terminal을 만들기 전에 runtime에서 현재 presentation setting을
     * 읽고, 그 값으로 backend wrapper의 viewport 옵션을 확정한다.
     */
    let render_mode = runtime.inline_history_render_mode();
    let terminal = build_terminal(backend, render_mode)?;
    /*
     * Terminal ownership은 InlineTerminalAdapter가 갖는다. frontend loop는 "언제 그릴지"만
     * 결정하고, frame 준비와 host scrollback 보정은 adapter/runtime 조합에 맡긴다.
     */
    let mut adapter = InlineTerminalAdapter::new(terminal);
    run_event_loop(&mut adapter, &mut runtime, shutdown, &mut restore_guard)
}

/*
 * CrosstermBackend는 process stdout이라는 실제 출력 장치에 묶여 있고,
 * InlineTerminalBackend는 그 앞에서 inline viewport와 host scrollback 간의 보정을 제공한다.
 * 이 작은 조립 함수가 concrete terminal stack의 타입 경계를 한곳에 모아 둔다.
 */
fn build_terminal(
    backend: CrosstermBackend<io::Stdout>,
    render_mode: super::InlineHistoryRenderMode,
) -> io::Result<Terminal<InlineTerminalBackend<CrosstermBackend<io::Stdout>>>> {
    Terminal::with_options(
        InlineTerminalBackend::new(backend),
        terminal_options_for_render_mode(render_mode),
    )
}

/*
 * Event loop는 deliberately thin하다. 매 반복에서 background channel을 비우고, scheduler가
 * due라고 판단한 경우에만 draw transaction을 실행한 뒤, crossterm event를 읽어 runtime
 * reducer에 넘긴다. key binding, resize, focus lost 같은 의미 해석은 이 층에 두지 않는다.
 */
fn run_event_loop(
    adapter: &mut InlineTerminalAdapter<InlineTerminalBackend<CrosstermBackend<io::Stdout>>>,
    runtime: &mut ShellRuntime,
    shutdown: &crate::shutdown::GracefulShutdown,
    restore_guard: &mut TerminalRestoreGuard,
) -> Result<()> {
    while !runtime.should_quit() && !shutdown.is_requested() {
        /*
         * app-server stream, startup/session load, post-turn evaluation은 terminal input과 별개로
         * 들어온다. 이벤트 poll 전에 먼저 반영해야 사용자가 입력하지 않아도 화면이 stale하지 않다.
         * 이미 준비된 terminal event도 draw deadline을 소비하기 전에 반영해야 같은 크기로 돌아온
         * resize ABA가 cursor와 history 보정에서 누락되지 않는다.
         */
        let draw_due = prepare_runtime_for_due_draw(runtime, read_ready_terminal_event)?;
        if runtime.should_quit() {
            break;
        }
        if draw_due {
            let transaction_completed = adapter.draw_inline_transaction(runtime)?;
            runtime.finish_pending_quit_after_transaction(transaction_completed);
        }
        // Preserve normal terminal selection and wheel scrolling unless the current frame owns a clickable action.
        restore_guard.sync_mouse_capture(runtime.queue_receipt_undo_mouse_capture_requested())?;
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
    }

    Ok(())
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

/*
 * RAII guard의 activate 성공은 raw mode, focus-change event 구독, paste event 구독을
 * frontend가 소유한다는 뜻이고, Drop은 정상 종료, 오류 반환, early return 모두에서 복구를 시도한다.
 */
struct TerminalRestoreGuard {
    bracketed_paste_enabled: bool,
    mouse_capture_enabled: bool,
}

impl TerminalRestoreGuard {
    fn activate() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        /*
         * focus events는 focus lost 중 draw를 늦추는 runtime scheduler 정책의 입력이다. enable이
         * 실패하면 raw mode만 켜진 반쪽 상태가 되므로 즉시 되돌리고 startup 실패로 전파한다.
         */
        if let Err(error) = execute!(stdout, event::EnableFocusChange) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        let bracketed_paste_enabled = execute!(stdout, event::EnableBracketedPaste).is_ok();
        Ok(Self {
            bracketed_paste_enabled,
            mouse_capture_enabled: false,
        })
    }

    fn sync_mouse_capture(&mut self, requested: bool) -> Result<()> {
        if requested == self.mouse_capture_enabled {
            return Ok(());
        }
        let mut stdout = io::stdout();
        if requested {
            execute!(stdout, event::EnableMouseCapture)?;
        } else {
            execute!(stdout, event::DisableMouseCapture)?;
        }
        self.mouse_capture_enabled = requested;
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
        if self.mouse_capture_enabled {
            let _ = execute!(stdout, event::DisableMouseCapture);
        }
        if self.bracketed_paste_enabled {
            let _ = execute!(stdout, event::DisableBracketedPaste);
        }
        let _ = execute!(stdout, event::DisableFocusChange);
        let _ = disable_raw_mode();
        /*
         * inline renderer는 마지막 frame의 prompt/tail을 현재 줄에 남길 수 있다. 한 줄 내리고
         * cursor를 다시 보이게 해서 앱 종료 뒤 shell prompt가 앱 출력과 겹치지 않게 한다.
         */
        let _ = execute!(stdout, MoveToNextLine(1));
        let _ = execute!(stdout, Show);
    }
}
