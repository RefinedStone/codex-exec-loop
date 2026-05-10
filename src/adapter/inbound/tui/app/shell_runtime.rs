use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::Print;

use crate::core::app::CoreInput;
use crate::domain::operator_alert::OperatorAlert;

use super::post_turn_automation::PostTurnAutomationBackgroundResult;
use super::{BackgroundMessage, ConversationRuntimeEvent, NativeTuiApp, ShellChromeEvent};

const BACKGROUND_MESSAGE_DRAIN_BUDGET: usize = 128;

/* ShellRuntime is the thin event-loop owner around NativeTuiApp. It drains
 * background work, applies terminal input in priority order, and only exposes
 * redraw timing through TuiFrameScheduler so rendering remains pull-driven.
 */
pub(super) struct ShellRuntime {
    app: NativeTuiApp,
    should_quit: bool,
    frame_scheduler: TuiFrameScheduler,
    last_live_activity_pulse: Option<u64>,
    background_drain_limited: bool,
}

impl ShellRuntime {
    pub(super) fn new(app: NativeTuiApp) -> Self {
        let now = Instant::now();
        Self {
            app,
            should_quit: false,
            frame_scheduler: TuiFrameScheduler::new(now),
            last_live_activity_pulse: None,
            background_drain_limited: false,
        }
    }
    pub(super) fn app_mut(&mut self) -> &mut NativeTuiApp {
        &mut self.app
    }
    #[cfg(test)]
    pub(super) fn app(&self) -> &NativeTuiApp {
        &self.app
    }
    pub(super) fn should_quit(&self) -> bool {
        self.should_quit
    }
    #[cfg(test)]
    pub(super) fn take_redraw_request(&mut self) -> bool {
        self.take_due_draw_request(Instant::now())
    }
    pub(super) fn take_due_draw_request(&mut self, now: Instant) -> bool {
        self.frame_scheduler.take_due(now)
    }
    pub(super) fn next_event_poll_timeout(
        &self,
        now: Instant,
        default_timeout: Duration,
    ) -> Duration {
        if self.frame_scheduler.focused && self.background_drain_limited {
            return Duration::ZERO;
        }
        self.frame_scheduler.next_poll_timeout(now, default_timeout)
    }
    fn request_redraw_at(&mut self, now: Instant) {
        self.frame_scheduler.request_immediate(now);
    }
    pub(super) fn poll_background_messages(&mut self) {
        self.poll_background_messages_at(Instant::now());
    }

    fn poll_background_messages_at(&mut self, now: Instant) {
        let mut redraw_requested = false;
        let mut drained_background_messages = 0usize;
        self.background_drain_limited = false;

        // Process a bounded background batch before drawing. Streaming providers can
        // enqueue faster than the terminal paints, so this must yield often enough
        // for already-buffered keyboard input to update the prompt without waiting
        // behind the whole stream backlog.
        while drained_background_messages < BACKGROUND_MESSAGE_DRAIN_BUDGET {
            let Ok(message) = self.app.rx.try_recv() else {
                break;
            };
            drained_background_messages += 1;
            redraw_requested = true;
            match message {
                #[cfg(test)]
                BackgroundMessage::StartupLoaded(result) => {
                    let workspace_directory = match &result {
                        Ok(ready) => Some(ready.workspace_path.clone()),
                        Err(_) => None,
                    };
                    self.app
                        .dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                            result,
                            session_page_size: super::SESSION_PAGE_SIZE,
                        });
                    if let Some(workspace_directory) = workspace_directory {
                        self.app.sync_draft_shell_workspace(&workspace_directory);
                    }
                    self.app.resolve_startup_submit_queue();
                }
                #[cfg(test)]
                BackgroundMessage::ConversationLoaded(result) => {
                    self.app.apply_loaded_conversation_result(result);
                }
                BackgroundMessage::ConversationStream(event) => {
                    self.app
                        .dispatch_core_input(CoreInput::ConversationStreamUpdated(event));
                }
                BackgroundMessage::ConversationTurnCompleted {
                    turn_id,
                    changed_planning_file_paths,
                    execution_snapshot_capture,
                } => {
                    self.app.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::StreamTurnCompleted {
                            turn_id,
                            changed_planning_file_paths,
                            execution_snapshot_capture: Some(execution_snapshot_capture),
                        },
                    );
                }
                BackgroundMessage::ConversationRuntimeNotice(notice) => {
                    self.app.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::StreamExecutionObserved { notice },
                    );
                }
                BackgroundMessage::OperatorAlert(alert) => {
                    self.emit_operator_alert(&alert);
                }
                BackgroundMessage::InvalidateParallelModeSupervisorSnapshot => {
                    self.app.invalidate_parallel_mode_supervisor_snapshot();
                }
                BackgroundMessage::ParallelModeControlPlaneEvent(event) => {
                    self.app
                        .apply_parallel_mode_control_plane_background_event(event);
                }
                BackgroundMessage::PostTurnEvaluated {
                    thread_id,
                    completed_turn_id,
                    evaluation,
                    planning_worker_panel_state,
                } => {
                    self.app.route_post_turn_automation_result(
                        PostTurnAutomationBackgroundResult {
                            thread_id,
                            completed_turn_id,
                            evaluation,
                            planning_worker_panel_state,
                        },
                    );
                }
                BackgroundMessage::GithubReviewPollLoaded(result) => {
                    self.app.record_github_review_poll_result(now, result)
                }
            }
        }
        self.background_drain_limited =
            drained_background_messages == BACKGROUND_MESSAGE_DRAIN_BUDGET;

        redraw_requested |= self
            .app
            .poll_core_runtime_inputs(BACKGROUND_MESSAGE_DRAIN_BUDGET);
        redraw_requested |= self.app.maybe_start_github_review_poll(now);
        let live_activity_pulse = self.app.live_activity_pulse(now);
        if live_activity_pulse != self.last_live_activity_pulse {
            redraw_requested = true;
        }
        self.last_live_activity_pulse = live_activity_pulse;
        redraw_requested |= self.app.tick_parallel_mode_control_plane(now);
        if redraw_requested {
            self.request_redraw_at(now);
        } else if live_activity_pulse.is_some() {
            // Live indicators need periodic frames even when no background message arrives.
            self.frame_scheduler
                .request_delayed(now, Duration::from_millis(250));
        }
    }

    fn emit_operator_alert(&self, alert: &OperatorAlert) {
        if !alert.audible {
            return;
        }
        let mut stdout = std::io::stdout();
        let _ = execute!(stdout, Print("\x07"));
    }

    pub(super) fn handle_terminal_event(&mut self, event: Event) {
        self.handle_terminal_event_at(event, Instant::now());
    }

    fn handle_terminal_event_at(&mut self, event: Event, now: Instant) {
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return;
                }

                self.handle_key_press(key, now);
            }
            Event::Resize(_, _) => self.request_redraw_at(now),
            Event::FocusGained => self.frame_scheduler.set_focused(true, now),
            Event::FocusLost => self.frame_scheduler.set_focused(false, now),
            _ => {}
        }
    }

    fn handle_key_press(&mut self, key: KeyEvent, now: Instant) {
        // Exit confirmation owns the first key pass so Escape/Enter cannot leak into
        // overlays or prompt editing while the quit dialog is active.
        if let Some(confirmed_exit) = self.app.handle_exit_confirmation_key(key) {
            if !confirmed_exit {
                self.request_redraw_at(now);
            }
            if confirmed_exit {
                self.should_quit = true;
            }
            return;
        }
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('q') {
            self.should_quit = true;
            return;
        }

        // Shell overlays are modal at the runtime boundary; prompt editing only runs
        // after they decline the key.
        if self.app.handle_shell_overlay_key(key) {
            self.request_redraw_at(now);
            return;
        }

        if self.app.is_inline_command_palette_active() {
            match key.code {
                KeyCode::Esc
                    if key.modifiers.is_empty() && self.app.dismiss_inline_command_palette() =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                KeyCode::Up
                    if key.modifiers.is_empty()
                        && self.app.move_inline_command_palette_selection(-1) =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                KeyCode::Down
                    if key.modifiers.is_empty()
                        && self.app.move_inline_command_palette_selection(1) =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                KeyCode::Enter
                    if key.modifiers.is_empty()
                        && self.app.accept_inline_command_palette_selection() =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                _ => {}
            }
        }

        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            self.app.handle_ctrl_c();
            self.request_redraw_at(now);
            return;
        }
        match key.code {
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_startup_overlay()
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_session_overlay()
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => self
                .app
                .dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested),
            KeyCode::Char('t') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.open_new_conversation_shell()
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.insert_input_newline()
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.clear_prompt_input()
            }
            KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.delete_previous_input_word()
            }
            KeyCode::Backspace => self.app.pop_input_character(),
            KeyCode::Enter => self.app.start_turn_submission(),
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.app.push_input_character(character)
            }
            _ => return,
        }

        self.request_redraw_at(now);
    }
}

// The scheduler coalesces redraw requests and suppresses drawing while the terminal
// reports focus loss. Tests drive it with explicit Instants to avoid wall-clock flakes.
#[derive(Debug, Clone)]
struct TuiFrameScheduler {
    focused: bool,
    next_deadline: Option<Instant>,
}

impl TuiFrameScheduler {
    fn new(now: Instant) -> Self {
        let mut scheduler = Self {
            focused: true,
            next_deadline: None,
        };
        scheduler.request_immediate(now);
        scheduler
    }
    fn request_immediate(&mut self, now: Instant) {
        self.coalesce_deadline(now);
    }
    fn request_delayed(&mut self, now: Instant, delay: Duration) {
        self.coalesce_deadline(now + delay);
    }
    fn take_due(&mut self, now: Instant) -> bool {
        if !self.focused {
            return false;
        }
        let Some(deadline) = self.next_deadline else {
            return false;
        };
        if deadline > now {
            return false;
        }

        self.next_deadline = None;
        true
    }
    fn next_poll_timeout(&self, now: Instant, default_timeout: Duration) -> Duration {
        if !self.focused {
            return default_timeout;
        }
        let Some(deadline) = self.next_deadline else {
            return default_timeout;
        };
        default_timeout.min(deadline.saturating_duration_since(now))
    }
    fn set_focused(&mut self, focused: bool, now: Instant) {
        self.focused = focused;
        if focused {
            self.request_immediate(now);
        }
    }
    fn coalesce_deadline(&mut self, deadline: Instant) {
        if self
            .next_deadline
            .is_none_or(|existing_deadline| deadline < existing_deadline)
        {
            self.next_deadline = Some(deadline);
        }
    }
}

#[cfg(test)]
#[path = "shell_runtime/tests.rs"]
mod tests;
