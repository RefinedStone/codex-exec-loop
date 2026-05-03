// Scheduler tests inject time directly so redraw deadlines stay deterministic.
// That keeps event-loop regressions visible without sleeping in the test suite.
use std::time::{Duration, Instant};

use crossterm::event::Event;

use super::{TuiFrameScheduler, make_test_runtime};

// A new runtime must request the first frame immediately; otherwise the TUI can sit blank until input or background work.
#[test]
fn runtime_starts_with_redraw_requested() {
    let mut runtime = make_test_runtime();

    assert!(runtime.take_redraw_request());
    // Draw requests are one-shot so startup paint cannot trigger duplicate terminal frames.
    assert!(!runtime.take_redraw_request());
}

// Redraw producers share one deadline; the earliest request must win so input, resize, and live pulses stay responsive.
#[test]
fn scheduler_coalesces_immediate_and_delayed_requests() {
    let now = Instant::now();
    let mut scheduler = TuiFrameScheduler {
        focused: true,
        next_deadline: None,
    };

    scheduler.request_delayed(now, Duration::from_secs(10));
    scheduler.request_delayed(now, Duration::from_secs(5));
    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_secs(30)),
        Duration::from_secs(5)
    );

    // Immediate redraw preempts the delayed queue and shortens the frontend poll timeout.
    scheduler.request_immediate(now + Duration::from_secs(1));
    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_secs(30)),
        Duration::from_secs(1)
    );
    assert!(!scheduler.take_due(now));
    assert!(scheduler.take_due(now + Duration::from_secs(1)));
    // Consuming the due deadline clears it, preserving the scheduler's one-frame-per-request contract.
    assert!(!scheduler.take_due(now + Duration::from_secs(1)));
}

// A due deadline must collapse the event poll timeout to zero so the frontend draws before waiting for input.
#[test]
fn scheduler_reports_zero_timeout_when_draw_is_due() {
    let now = Instant::now();
    let scheduler = TuiFrameScheduler::new(now);

    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_millis(100)),
        Duration::ZERO
    );
}

// Focus loss gates drawing but keeps pending layout work; focus return must redraw immediately to resync the terminal.
#[test]
fn focus_lost_blocks_draw_until_focus_returns() {
    // Clear startup paint so the focus transition is the only source of draw pressure in this scenario.
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let now = Instant::now();

    runtime.handle_terminal_event_at(Event::FocusLost, now);
    runtime.handle_terminal_event_at(Event::Resize(120, 40), now + Duration::from_millis(1));

    // Resize while unfocused should not burn CPU on frames the user cannot see.
    assert!(!runtime.take_due_draw_request(now + Duration::from_millis(1)));

    runtime.handle_terminal_event_at(Event::FocusGained, now + Duration::from_millis(2));

    // Regaining focus schedules a fresh frame at the same timestamp to repair any stale layout.
    assert!(runtime.take_due_draw_request(now + Duration::from_millis(2)));
}
