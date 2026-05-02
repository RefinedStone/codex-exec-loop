// 학습 주석: scheduler tests는 실제 wall clock을 기다리지 않고 Instant/Duration 값을 직접 주입합니다.
// 그래서 redraw deadline 계산을 deterministic하게 검증할 수 있습니다.
use std::time::{Duration, Instant};

// 학습 주석: crossterm Event는 ShellRuntime이 production에서 받는 terminal input과 같은 타입입니다.
// FocusLost/FocusGained/Resize를 그대로 넣어 scheduler와 runtime boundary를 함께 검증합니다.
use crossterm::event::Event;

// 학습 주석: make_test_runtime은 NativeTuiApp 의존성을 fake로 조립하고,
// TuiFrameScheduler는 이 파일에서 deadline coalescing 규칙을 직접 검사하기 위해 가져옵니다.
use super::{TuiFrameScheduler, make_test_runtime};

// 학습 주석: 새 runtime은 첫 frame을 그릴 수 있어야 합니다. 초기 draw가 없으면 TUI가 입력이나 background message 전까지
// blank 상태로 머물 수 있으므로 constructor contract를 테스트로 고정합니다.
#[test]
fn runtime_starts_with_redraw_requested() {
    // 학습 주석: test runtime 생성은 ShellRuntime::new를 지나며 TuiFrameScheduler::new가 immediate deadline을 심습니다.
    let mut runtime = make_test_runtime();

    // 학습 주석: 첫 호출은 pending draw를 소비하므로 true입니다.
    assert!(runtime.take_redraw_request());
    // 학습 주석: take_due는 one-shot 소비 모델입니다. 같은 request가 두 번 그려지면 불필요한 terminal redraw가 생깁니다.
    assert!(!runtime.take_redraw_request());
}

// 학습 주석: scheduler는 여러 redraw request가 들어와도 가장 이른 deadline 하나로 합칩니다.
// background pulse, resize, key input이 겹쳐도 event loop timeout이 불필요하게 길어지지 않는다는 계약입니다.
#[test]
fn scheduler_coalesces_immediate_and_delayed_requests() {
    // 학습 주석: now를 고정해 5초/10초/1초 deadline의 상대 순서를 명확히 비교합니다.
    let now = Instant::now();
    // 학습 주석: scheduler 내부 상태를 직접 구성해 "이미 focused이고 pending deadline 없음" 출발점을 만듭니다.
    let mut scheduler = TuiFrameScheduler {
        // 학습 주석: focused=true여야 next_poll_timeout과 take_due가 deadline을 실제로 반영합니다.
        focused: true,
        // 학습 주석: None은 아직 redraw가 예약되지 않았다는 뜻입니다.
        next_deadline: None,
    };

    // 학습 주석: 먼저 10초 뒤 draw를 예약하고, 더 이른 5초 draw를 다시 요청합니다.
    scheduler.request_delayed(now, Duration::from_secs(10));
    scheduler.request_delayed(now, Duration::from_secs(5));
    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_secs(30)),
        // 학습 주석: coalescing 결과는 10초가 아니라 더 빠른 5초 timeout이어야 합니다.
        Duration::from_secs(5)
    );

    // 학습 주석: immediate request는 "그 시각에 바로 draw 가능" deadline이므로 기존 5초 예약보다 앞섭니다.
    scheduler.request_immediate(now + Duration::from_secs(1));
    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_secs(30)),
        // 학습 주석: event loop는 now 기준 1초 뒤에 다시 poll을 깨워야 합니다.
        Duration::from_secs(1)
    );
    // 학습 주석: 아직 1초 deadline 전이므로 due draw가 아닙니다.
    assert!(!scheduler.take_due(now));
    // 학습 주석: deadline 시각에 도달하면 draw request가 true로 소비됩니다.
    assert!(scheduler.take_due(now + Duration::from_secs(1)));
    // 학습 주석: 소비 뒤에는 next_deadline이 None이 되어 같은 frame을 중복 draw하지 않습니다.
    assert!(!scheduler.take_due(now + Duration::from_secs(1)));
}

// 학습 주석: poll timeout이 0이면 event loop는 입력 대기 대신 즉시 draw path를 탈 수 있습니다.
// 초기 scheduler가 즉시 draw를 요청한다는 ShellRuntime startup behavior를 낮은 수준에서 확인합니다.
#[test]
fn scheduler_reports_zero_timeout_when_draw_is_due() {
    // 학습 주석: TuiFrameScheduler::new(now)는 내부에서 request_immediate(now)를 호출합니다.
    let now = Instant::now();
    // 학습 주석: 따라서 next_deadline은 현재 시각과 같아 due 상태입니다.
    let scheduler = TuiFrameScheduler::new(now);

    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_millis(100)),
        // 학습 주석: default timeout보다 due deadline이 우선하므로 즉시 반환값은 0입니다.
        Duration::ZERO
    );
}

// 학습 주석: terminal focus를 잃었을 때는 resize 같은 redraw request가 들어와도 실제 draw를 보류합니다.
// focus가 돌아오는 순간 즉시 redraw를 예약해 화면 상태를 다시 동기화하는 contract를 검증합니다.
#[test]
fn focus_lost_blocks_draw_until_focus_returns() {
    // 학습 주석: runtime startup draw는 이 테스트의 관심사가 아니므로 먼저 소비해 scheduler를 비웁니다.
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    // 학습 주석: 수동 now 값으로 FocusLost, Resize, FocusGained 순서를 밀리초 단위로 표현합니다.
    let now = Instant::now();

    // 학습 주석: focus lost는 scheduler.focused=false로 바꾸어 due draw 소비를 막습니다.
    runtime.handle_terminal_event_at(Event::FocusLost, now);
    // 학습 주석: resize는 normally redraw request지만, focus가 없는 동안에는 take_due가 true를 내면 안 됩니다.
    runtime.handle_terminal_event_at(Event::Resize(120, 40), now + Duration::from_millis(1));

    // 학습 주석: pending deadline이 있어도 focused=false이므로 draw request는 보류됩니다.
    assert!(!runtime.take_due_draw_request(now + Duration::from_millis(1)));

    // 학습 주석: focus gained는 focused=true로 되돌리면서 immediate redraw를 예약합니다.
    runtime.handle_terminal_event_at(Event::FocusGained, now + Duration::from_millis(2));

    // 학습 주석: focus가 돌아온 같은 시각에 due draw가 true가 되어 terminal을 최신 layout으로 복구합니다.
    assert!(runtime.take_due_draw_request(now + Duration::from_millis(2)));
}
