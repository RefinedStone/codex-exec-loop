// 학습 주석: 이 frontend는 process stdout을 직접 terminal backend로 잡습니다. 그래서 std::io는
// CrosstermBackend 생성과 restore guard의 stdout command 실행 양쪽에서 필요합니다.
use std::io;
// 학습 주석: event loop는 runtime scheduler가 계산한 timeout과 fallback poll interval을 함께 사용합니다.
// Duration은 crossterm::event::poll에 넘기는 blocking window를 표현합니다.
use std::time::Duration;

// 학습 주석: frontend entrypoint는 terminal setup, draw, event read 실패를 상위 shell entrypoint로 전파해야 하므로
// anyhow::Result를 사용합니다. 여기서 error를 삼키면 raw mode 복구 외의 실패 진단이 어려워집니다.
use anyhow::Result;
// 학습 주석: frontend 종료 시 prompt가 같은 줄에 남지 않도록 한 줄 내리고 cursor를 다시 보이게 합니다.
// 이 cleanup command들은 Drop guard에서 terminal을 사람이 쓸 수 있는 상태로 되돌리는 데 쓰입니다.
use crossterm::cursor::{MoveToNextLine, Show};
// 학습 주석: crossterm event module은 keyboard/mouse/focus/resize event polling과 focus change enable/disable
// command를 모두 제공합니다. ShellRuntime은 이 raw terminal event를 받아 app event로 축소합니다.
use crossterm::event;
// 학습 주석: execute! macro는 stdout에 crossterm command를 즉시 씁니다. raw mode/focus cleanup은
// ratatui draw path가 아니라 terminal side effect라 여기서 직접 실행합니다.
use crossterm::execute;
// 학습 주석: raw mode는 Enter/Ctrl 키와 resize/focus event를 shell runtime이 직접 받기 위한 terminal mode입니다.
// guard가 enable/disable을 쌍으로 관리해 panic/error path에서도 사용자의 terminal을 복구합니다.
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
// 학습 주석: ratatui Terminal은 backend draw calls를 frame transaction으로 묶습니다. 이 frontend는 직접
// widget을 그리지 않고 InlineTerminalAdapter에게 terminal ownership을 넘깁니다.
use ratatui::Terminal;
// 학습 주석: CrosstermBackend는 ratatui drawing을 stdout/crossterm command stream으로 변환하는 concrete backend입니다.
use ratatui::backend::CrosstermBackend;

// 학습 주석: InlineTerminalAdapter는 이 앱의 "inline main buffer" rendering policy를 캡슐화합니다. frontend는
// terminal을 만들고 이벤트를 공급하며, adapter는 draw transaction과 history scrollback 전략을 담당합니다.
use super::inline_terminal_adapter::{
    InlineTerminalAdapter, InlineTerminalBackend, terminal_options_for_render_mode,
};
// 학습 주석: ShellRuntime은 app state, background message polling, draw scheduler, terminal event reducer를
// 모두 품은 실행 context입니다. frontend는 runtime 내부 규칙을 몰라도 이 메서드들만 호출하면 됩니다.
use super::shell_runtime::ShellRuntime;

// 학습 주석: run은 shell_frontend facade에서 concrete ratatui/crossterm implementation으로 들어오는 entrypoint입니다.
// runtime ownership을 받아 terminal setup을 끝낸 뒤 event loop가 종료될 때까지 ShellRuntime과 terminal adapter를 연결합니다.
pub(super) fn run(mut runtime: ShellRuntime) -> Result<()> {
    // 학습 주석: restore guard를 가장 먼저 활성화합니다. 이후 terminal 생성이나 draw loop가 실패해도 Drop이
    // raw mode와 focus subscription을 정리해 사용자의 shell을 망가진 상태로 남기지 않습니다.
    let _restore_guard = TerminalRestoreGuard::activate()?;
    // 학습 주석: stdout backend는 현재 process의 host terminal에 직접 그리는 출력 경계입니다.
    let backend = CrosstermBackend::new(io::stdout());
    // 학습 주석: render_mode는 app state가 선택한 inline history strategy입니다. terminal options를 만들기 전에
    // runtime에서 읽어 backend wrapping과 viewport behavior가 같은 mode를 보게 합니다.
    let render_mode = runtime.app_mut().inline_history_render_mode;
    // 학습 주석: build_terminal은 raw CrosstermBackend를 InlineTerminalBackend로 감싸 ratatui Terminal을 만듭니다.
    // 이 단계에서 render mode별 terminal options가 확정됩니다.
    let terminal = build_terminal(backend, render_mode)?;
    // 학습 주석: adapter가 terminal ownership을 갖고 draw_inline_transaction을 제공합니다. frontend loop는
    // draw 시점만 결정하고 실제 render state 준비와 frame drawing은 adapter/runtime 조합에 맡깁니다.
    let mut adapter = InlineTerminalAdapter::new(terminal);
    run_event_loop(&mut adapter, &mut runtime)
}

// 학습 주석: build_terminal은 concrete stdout backend를 앱 전용 inline backend와 ratatui Terminal로 조립합니다.
// 별도 함수로 둔 이유는 render mode가 terminal options에 영향을 주는 경계를 작게 보이게 하기 위해서입니다.
fn build_terminal(
    // 학습 주석: backend는 crossterm/stdout에 묶인 실제 출력 장치입니다. generic abstraction은 여기서 끝나고
    // 이후 타입은 app-specific InlineTerminalBackend로 감싸집니다.
    backend: CrosstermBackend<io::Stdout>,
    // 학습 주석: render_mode는 inline history를 host terminal scrollback에 맡길지, 어떤 viewport option을 쓸지
    // 결정하는 presentation setting입니다.
    render_mode: super::InlineHistoryRenderMode,
) -> io::Result<Terminal<InlineTerminalBackend<CrosstermBackend<io::Stdout>>>> {
    // 학습 주석: `Terminal::with_options`는 backend와 terminal options를 한 번에 묶습니다. options를 render
    // mode helper에서 받아 inline frontend 정책이 backend 생성부에 흩어지지 않게 합니다.
    Terminal::with_options(
        // 학습 주석: InlineTerminalBackend는 ratatui의 buffer flush를 host scrollback 친화적으로 조정하는 wrapper입니다.
        InlineTerminalBackend::new(backend),
        terminal_options_for_render_mode(render_mode),
    )
}

// 학습 주석: run_event_loop는 frontend의 핵심 pump입니다. 매 tick마다 background messages를 runtime에 반영하고,
// due draw가 있으면 inline transaction을 그린 뒤, crossterm event를 기다려 runtime reducer에 전달합니다.
fn run_event_loop(
    // 학습 주석: adapter는 terminal draw transaction의 소유자입니다. runtime이 draw를 요청하면 이 adapter가
    // render state preparation과 ratatui frame flush를 묶어서 수행합니다.
    adapter: &mut InlineTerminalAdapter<InlineTerminalBackend<CrosstermBackend<io::Stdout>>>,
    // 학습 주석: runtime은 quit flag, background channel polling, draw scheduler, terminal event handling을 모두
    // 제공합니다. event loop는 runtime의 scheduling 결정을 따르는 thin IO loop입니다.
    runtime: &mut ShellRuntime,
) -> Result<()> {
    // 학습 주석: loop는 runtime이 quit을 요청할 때까지 계속됩니다. Ctrl+Q, exit confirmation, fatal state는
    // 모두 runtime 내부에서 should_quit flag로 축약됩니다.
    while !runtime.should_quit() {
        // 학습 주석: background messages는 app-server streams, startup/session loads, post-turn evaluation 같은
        // non-terminal input입니다. terminal event를 기다리기 전에 먼저 소진해 UI가 stale하지 않게 합니다.
        runtime.poll_background_messages();
        // 학습 주석: scheduler가 draw due라고 판단할 때만 frame을 그립니다. 즉시/지연 draw 요청을 runtime이
        // coalescing하므로 frontend는 매 loop마다 무조건 그리지 않습니다.
        if runtime.take_due_draw_request(std::time::Instant::now()) {
            adapter.draw_inline_transaction(runtime)?;
        }

        // 학습 주석: poll_timeout은 다음 draw deadline과 기본 100ms wait cap을 조합한 값입니다. 이벤트가 없어도
        // delayed draw가 필요한 시점에는 poll이 깨어나도록 runtime이 timeout을 계산합니다.
        let poll_timeout =
            runtime.next_event_poll_timeout(std::time::Instant::now(), Duration::from_millis(100));
        // 학습 주석: terminal input이 없으면 loop를 다시 돌며 background/draw 상태를 확인합니다. 이 continue는
        // "이벤트 없음"을 정상 idle 상태로 다루는 경계입니다.
        if !event::poll(poll_timeout)? {
            continue;
        }

        // 학습 주석: crossterm event를 runtime으로 넘기면 runtime이 key, resize, focus, mouse/no-op 정책을
        // app reducer와 draw scheduler에 맞게 처리합니다. frontend는 event 의미를 직접 해석하지 않습니다.
        runtime.handle_terminal_event(event::read()?);
    }

    // 학습 주석: 정상 quit은 Ok로 상위 entrypoint에 돌아갑니다. terminal restore는 이 함수 반환 뒤 guard Drop이 처리합니다.
    Ok(())
}

// 학습 주석: TerminalRestoreGuard는 값 자체에 데이터가 없는 RAII guard입니다. 생성 성공은 raw mode/focus
// subscription이 켜졌다는 뜻이고, Drop은 반대로 terminal state를 복구합니다.
struct TerminalRestoreGuard;

// 학습 주석: activate는 frontend가 host terminal을 제어하기 시작하는 setup boundary입니다. raw mode와 focus
// change subscription 둘 다 성공해야 guard를 반환해 이후 Drop cleanup이 의미를 갖습니다.
impl TerminalRestoreGuard {
    // 학습 주석: raw mode를 먼저 켠 뒤 focus change event를 enable합니다. focus command가 실패하면 raw mode만
    // 켜진 반쪽 상태가 되므로 즉시 disable하고 error를 반환합니다.
    fn activate() -> Result<Self> {
        enable_raw_mode()?;
        // 학습 주석: focus enable command는 stdout side effect라 mutable stdout handle이 필요합니다.
        let mut stdout = io::stdout();
        // 학습 주석: focus change events는 focus lost 시 draw를 잠시 막는 scheduler 정책의 입력입니다. enable에
        // 실패하면 runtime은 focus 상태를 정확히 알 수 없으므로 frontend startup을 실패로 처리합니다.
        if let Err(error) = execute!(stdout, event::EnableFocusChange) {
            // 학습 주석: focus enable 실패 path에서도 raw mode는 이미 켜져 있으므로 수동 cleanup을 수행합니다.
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        // 학습 주석: 이 시점부터 guard Drop이 terminal cleanup을 책임집니다.
        Ok(Self)
    }
}

// 학습 주석: Drop 구현은 성공/오류/early return 모두에서 실행되는 terminal cleanup hook입니다. 각 cleanup
// command는 best-effort로 처리해, 하나가 실패해도 나머지 복구 시도를 계속합니다.
impl Drop for TerminalRestoreGuard {
    // 학습 주석: drop은 user shell을 되돌리는 마지막 방어선입니다. 여기서 error를 반환할 수 없으므로 모든
    // 결과를 무시하고 raw mode, focus subscription, cursor visibility, line position 복구를 시도합니다.
    fn drop(&mut self) {
        // 학습 주석: cleanup commands는 같은 stdout stream에 순서대로 나갑니다.
        let mut stdout = io::stdout();
        // 학습 주석: focus event subscription을 해제해 이후 shell/terminal 프로그램이 불필요한 focus events를 받지 않게 합니다.
        let _ = execute!(stdout, event::DisableFocusChange);
        // 학습 주석: raw mode를 끄면 line buffering, echo, Ctrl+C 같은 일반 terminal behavior가 복구됩니다.
        let _ = disable_raw_mode();
        // 학습 주석: inline renderer는 prompt/tail을 마지막 frame 위치에 남길 수 있으므로 한 줄 내려 shell prompt와
        // 앱 출력이 같은 줄에 겹치지 않게 합니다.
        let _ = execute!(stdout, MoveToNextLine(1));
        // 학습 주석: rendering 중 숨겨졌을 수 있는 cursor를 다시 보여 줍니다. 사용자가 앱 종료 후 입력 위치를
        // 잃지 않게 하는 최종 cleanup입니다.
        let _ = execute!(stdout, Show);
    }
}
