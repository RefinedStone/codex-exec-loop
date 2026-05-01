use std::time::{Duration, Instant};

use crossterm::event::Event;

use super::{TuiFrameScheduler, make_test_runtime};

#[test]
fn runtime_starts_with_redraw_requested() {
    let mut runtime = make_test_runtime();

    assert!(runtime.take_redraw_request());
    assert!(!runtime.take_redraw_request());
}

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

    scheduler.request_immediate(now + Duration::from_secs(1));
    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_secs(30)),
        Duration::from_secs(1)
    );
    assert!(!scheduler.take_due(now));
    assert!(scheduler.take_due(now + Duration::from_secs(1)));
    assert!(!scheduler.take_due(now + Duration::from_secs(1)));
}

#[test]
fn scheduler_reports_zero_timeout_when_draw_is_due() {
    let now = Instant::now();
    let scheduler = TuiFrameScheduler::new(now);

    assert_eq!(
        scheduler.next_poll_timeout(now, Duration::from_millis(100)),
        Duration::ZERO
    );
}

#[test]
fn focus_lost_blocks_draw_until_focus_returns() {
    let mut runtime = make_test_runtime();
    runtime.take_redraw_request();
    let now = Instant::now();

    runtime.handle_terminal_event_at(Event::FocusLost, now);
    runtime.handle_terminal_event_at(Event::Resize(120, 40), now + Duration::from_millis(1));

    assert!(!runtime.take_due_draw_request(now + Duration::from_millis(1)));

    runtime.handle_terminal_event_at(Event::FocusGained, now + Duration::from_millis(2));

    assert!(runtime.take_due_draw_request(now + Duration::from_millis(2)));
}
