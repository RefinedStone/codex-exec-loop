use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::Print;
use ratatui::layout::Rect;

#[cfg(test)]
use crate::core::app::CoreInput;
use crate::domain::operator_alert::OperatorAlert;

use super::app_runtime::TUI_BACKGROUND_CHANNEL_CAPACITY;
use super::fullscreen_frame_model::{
    ConversationTranscriptProjectionCache, FullscreenConversationFrameProjection,
    FullscreenFrameRenderReceipt, FullscreenShellFrameModel, apply_fullscreen_frame_render_receipt,
    capture_fullscreen_shell_frame_model,
};
use super::shell_presentation::{ConversationProjectionFrameInput, ConversationProjectionSample};
use super::{
    BackgroundMessage, InputCursorMovement, NativeTuiApp, ShellChromeEvent, ShellFrontendMode,
    TerminalUiEffect,
};

const BACKGROUND_MESSAGE_DRAIN_BUDGET: usize = 128;
const TERMINAL_RESIZE_RETRY_DELAY: Duration = Duration::from_millis(16);
const MIN_FULLSCREEN_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const _: () = assert!(TUI_BACKGROUND_CHANNEL_CAPACITY > BACKGROUND_MESSAGE_DRAIN_BUDGET);

/* ShellRuntime is the thin event-loop owner around NativeTuiApp. It drains
 * background work, applies terminal input in priority order, and only exposes
 * redraw timing through TuiFrameScheduler so rendering remains pull-driven.
 */
pub(super) struct ShellRuntime {
    app: NativeTuiApp,
    should_quit: bool,
    quit_after_redraw: bool,
    frame_scheduler: TuiFrameScheduler,
    terminal_resize_epoch: u64,
    first_frame_delivered: bool,
    last_live_activity_pulse: Option<u64>,
    background_drain_limited: bool,
    transcript_projection_cache: ConversationTranscriptProjectionCache,
}

impl ShellRuntime {
    pub(super) fn new(app: NativeTuiApp) -> Self {
        let now = Instant::now();
        Self {
            app,
            should_quit: false,
            quit_after_redraw: false,
            frame_scheduler: TuiFrameScheduler::new(now),
            terminal_resize_epoch: 0,
            first_frame_delivered: false,
            last_live_activity_pulse: None,
            background_drain_limited: false,
            transcript_projection_cache: ConversationTranscriptProjectionCache::default(),
        }
    }
    #[cfg(test)]
    pub(super) fn app(&self) -> &NativeTuiApp {
        &self.app
    }
    #[cfg(test)]
    pub(super) fn app_mut(&mut self) -> &mut NativeTuiApp {
        &mut self.app
    }
    pub(super) fn capture_fullscreen_projection_sample(&self) -> ConversationProjectionSample {
        ConversationProjectionSample::from_frame_input(ConversationProjectionFrameInput {
            planning_parallel: self
                .app
                .runtime
                .client_runtime
                .revisioned_planning_parallel_projection(),
            parallel_control_plane: self
                .app
                .runtime
                .client_runtime
                .parallel_control_plane_projection(),
            parallel_supervisor_events: self.app.shell.parallel_event_stream.snapshot(),
        })
    }
    pub(super) fn capture_fullscreen_conversation_frame_projection(
        &mut self,
        terminal_width: u16,
        sample: &ConversationProjectionSample,
    ) -> FullscreenConversationFrameProjection {
        FullscreenConversationFrameProjection::from_app_with_sample_and_cache(
            &self.app,
            terminal_width,
            sample,
            &mut self.transcript_projection_cache,
        )
    }

    #[cfg(test)]
    pub(super) const fn transcript_projection_rebuild_count(&self) -> usize {
        self.transcript_projection_cache.rebuild_count()
    }
    pub(super) fn capture_fullscreen_shell_frame_model(
        &self,
        mode: ShellFrontendMode,
        area: Rect,
        projection: FullscreenConversationFrameProjection,
    ) -> FullscreenShellFrameModel {
        capture_fullscreen_shell_frame_model(&self.app, mode, area, projection)
    }
    pub(super) fn commit_fullscreen_frame_render_receipt(
        &mut self,
        receipt: FullscreenFrameRenderReceipt,
    ) -> bool {
        apply_fullscreen_frame_render_receipt(&mut self.app, receipt)
    }
    pub(super) fn clear_queue_receipt_undo_hit_area(&mut self) {
        self.app.clear_queue_receipt_undo_hit_area();
    }
    pub(super) fn clear_transcript_card_hit_areas(&mut self) {
        self.app.clear_transcript_card_hit_areas();
    }
    pub(super) fn take_terminal_ui_effects(&mut self) -> Vec<TerminalUiEffect> {
        self.app.take_terminal_ui_effects()
    }
    pub(super) fn should_quit(&self) -> bool {
        self.should_quit
    }
    pub(super) fn terminal_resize_epoch(&self) -> u64 {
        self.terminal_resize_epoch
    }
    #[cfg(test)]
    pub(super) fn take_redraw_request(&mut self) -> bool {
        self.frame_scheduler.take_pending_for_test()
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
    pub(super) fn request_delivery_redraw(&mut self) {
        self.request_redraw_at(Instant::now());
    }
    pub(super) fn request_resize_redraw_retry(&mut self) {
        self.request_resize_redraw_retry_at(Instant::now());
    }
    fn request_resize_redraw_retry_at(&mut self, now: Instant) {
        // A short delay prevents an unstable terminal from spinning while still
        // repairing a deferred frame without waiting for another input event.
        self.frame_scheduler
            .request_delayed(now, TERMINAL_RESIZE_RETRY_DELAY);
    }
    pub(super) fn finish_pending_quit_after_transaction(&mut self, transaction_completed: bool) {
        if transaction_completed && self.quit_after_redraw {
            self.quit_after_redraw = false;
            self.should_quit = true;
        }
    }

    pub(super) fn record_successful_frame_delivery(&mut self) {
        if self.first_frame_delivered {
            return;
        }
        self.first_frame_delivered = true;
        let workspace_directory = self.app.planning_workspace_directory();
        if self
            .app
            .maybe_start_github_review_polling_setup(&workspace_directory)
        {
            self.request_redraw_at(Instant::now());
        }
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
            let Ok(message) = self.app.runtime.rx.try_recv() else {
                break;
            };
            drained_background_messages += 1;
            redraw_requested = true;
            match message {
                #[cfg(test)]
                BackgroundMessage::ConversationRuntimeNotice(notice) => {
                    self.app
                        .dispatch_client_event(CoreInput::ConversationRuntimeNotice(notice));
                }
                BackgroundMessage::OperatorAlert(alert) => {
                    self.emit_operator_alert(&alert);
                }
                BackgroundMessage::ClipboardImageProbed(result) => {
                    self.app.apply_clipboard_image_probe(result);
                }
            }
        }
        self.background_drain_limited =
            drained_background_messages == BACKGROUND_MESSAGE_DRAIN_BUDGET;

        redraw_requested |= self
            .app
            .poll_client_runtime_events(BACKGROUND_MESSAGE_DRAIN_BUDGET);
        if self.first_frame_delivered {
            let workspace_directory = self.app.planning_workspace_directory();
            redraw_requested |= self
                .app
                .maybe_start_github_review_polling_setup(&workspace_directory);
        }
        redraw_requested |= self.app.maybe_start_github_review_poll(now);
        let parallel_presentation_sample = self.app.parallel_panel_projection_sample();
        let live_activity_pulse = self
            .app
            .live_activity_pulse_with_sample(now, &parallel_presentation_sample);
        if live_activity_pulse != self.last_live_activity_pulse {
            redraw_requested = true;
        }
        self.last_live_activity_pulse = live_activity_pulse;
        redraw_requested |= self
            .app
            .tick_parallel_mode_control_plane(now, &parallel_presentation_sample);
        redraw_requested |= self.app.reconcile_directions_maintenance_context();
        redraw_requested |= self.app.reconcile_reviews_overlay_authority_context();
        redraw_requested |= self.app.reconcile_queue_overlay_authority_context();
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
        if self.quit_after_redraw {
            return;
        }
        match event {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return;
                }

                self.handle_key_press(key, now);
            }
            Event::Mouse(mouse) => {
                if self.app.handle_transcript_mouse_event(mouse)
                    || self.app.handle_progressive_activity_mouse_event(mouse)
                    || self.app.handle_queue_receipt_mouse_event(mouse)
                {
                    self.request_redraw_at(now);
                }
            }
            Event::Paste(text) => self.handle_paste_text(text, now),
            Event::Resize(_, _) => {
                /*
                 * Several resize events can be drained into one redraw. Preserve
                 * that fact even when the final dimensions equal the previous
                 * frame, because the intermediate resize invalidates wrapping,
                 * transcript offsets, and all frame-local mouse hit areas.
                 */
                self.terminal_resize_epoch = self.terminal_resize_epoch.saturating_add(1);
                self.app.clear_queue_receipt_undo_hit_area();
                self.app.clear_progressive_activity_card_hit_areas();
                self.app.clear_transcript_card_hit_areas();
                self.request_redraw_at(now);
            }
            Event::FocusGained => {
                self.frame_scheduler.set_focused(true, now);
            }
            Event::FocusLost => {
                self.frame_scheduler.set_focused(false, now);
            }
        }
    }

    fn handle_paste_text(&mut self, text: String, now: Instant) {
        if self.app.approval_overlay_active() || self.app.is_turn_steer_confirmation_visible() {
            self.request_redraw_at(now);
            return;
        }
        if self.app.handle_session_rename_paste(&text) {
            self.request_redraw_at(now);
            return;
        }
        /*
         * Terminals cannot transport image bytes through bracketed paste. An
         * exactly-empty paste therefore means "the clipboard holds an image"
         * (iTerm2, Ghostty, and Windows Terminal all behave this way), so it
         * becomes a clipboard probe. A non-empty paste has image file paths
         * consumed into attachment chips (drag-and-drop); only the remaining
         * text lands in the composer so an attached path is never re-typed.
         */
        if text.is_empty() {
            if self.app.request_clipboard_image_probe() {
                self.request_redraw_at(now);
            }
            return;
        }
        let remaining_text = self.app.consume_pasted_image_paths(&text);
        let text_inserted = if remaining_text.is_empty() {
            false
        } else {
            self.app.insert_input_text(remaining_text)
        };
        if text_inserted || self.app.has_image_attachments() {
            self.request_redraw_at(now);
        }
    }

    fn handle_key_press(&mut self, key: KeyEvent, now: Instant) {
        // Transcript selection owns the conventional copy chord before Ctrl+C
        // is overloaded as turn interruption, navigation, or exit intent. This
        // is especially important on Windows Terminal + WSL, where the same
        // chord otherwise appears to terminate the selected session.
        if key.modifiers == KeyModifiers::CONTROL
            && key.code == KeyCode::Char('c')
            && self.app.copy_active_transcript_selection()
        {
            self.request_redraw_at(now);
            return;
        }

        // Exit confirmation owns the first key pass so Escape/Enter cannot leak into
        // overlays or prompt editing while the quit dialog is active.
        if let Some(confirmed_exit) = self.app.handle_exit_confirmation_key(key) {
            self.request_redraw_at(now);
            if confirmed_exit {
                self.quit_after_redraw = true;
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

        if self.app.handle_turn_steer_confirmation_key(key) {
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
                KeyCode::Tab
                    if key.modifiers.is_empty()
                        && self.app.move_inline_command_palette_selection(1) =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                KeyCode::BackTab
                    if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
                        && self.app.move_inline_command_palette_selection(-1) =>
                {
                    self.request_redraw_at(now);
                    return;
                }
                KeyCode::Enter if key.modifiers.is_empty() => {
                    self.app.accept_inline_command_palette_selection();
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
            /*
             * Ctrl+V (and Alt+V for terminals that swallow Ctrl+V) probes the OS
             * clipboard for image data. Terminals that intercept the chord send
             * an empty bracketed paste instead, which routes to the same probe.
             */
            KeyCode::Char('v')
                if key.modifiers == KeyModifiers::CONTROL || key.modifiers == KeyModifiers::ALT =>
            {
                if !self.app.request_clipboard_image_probe() {
                    return;
                }
            }
            KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => {
                if !self.app.toggle_latest_transcript_tool_card() {
                    return;
                }
            }
            KeyCode::PageUp if key.modifiers.is_empty() => {
                if !self.app.scroll_transcript_page_up() {
                    return;
                }
            }
            KeyCode::PageDown if key.modifiers.is_empty() => {
                if !self.app.scroll_transcript_page_down() {
                    return;
                }
            }
            KeyCode::Left if key.modifiers.is_empty() => self
                .app
                .move_input_cursor(InputCursorMovement::PreviousCharacter),
            KeyCode::Right if key.modifiers.is_empty() => self
                .app
                .move_input_cursor(InputCursorMovement::NextCharacter),
            KeyCode::Up if key.modifiers.is_empty() => self
                .app
                .move_input_cursor(InputCursorMovement::PreviousLine),
            KeyCode::Down if key.modifiers.is_empty() => {
                self.app.move_input_cursor(InputCursorMovement::NextLine)
            }
            KeyCode::Left if key.modifiers == KeyModifiers::ALT => self
                .app
                .move_input_cursor(InputCursorMovement::PreviousWord),
            KeyCode::Right if key.modifiers == KeyModifiers::ALT => {
                self.app.move_input_cursor(InputCursorMovement::NextWord)
            }
            KeyCode::Left if key.modifiers == KeyModifiers::CONTROL => self
                .app
                .move_input_cursor(InputCursorMovement::PreviousWord),
            KeyCode::Right if key.modifiers == KeyModifiers::CONTROL => {
                self.app.move_input_cursor(InputCursorMovement::NextWord)
            }
            KeyCode::Left if key.modifiers == KeyModifiers::SUPER => {
                self.app.move_input_cursor(InputCursorMovement::LineStart)
            }
            KeyCode::Right if key.modifiers == KeyModifiers::SUPER => {
                self.app.move_input_cursor(InputCursorMovement::LineEnd)
            }
            KeyCode::Up if key.modifiers == KeyModifiers::SUPER => {
                self.app.move_input_cursor(InputCursorMovement::BufferStart)
            }
            KeyCode::Down if key.modifiers == KeyModifiers::SUPER => {
                self.app.move_input_cursor(InputCursorMovement::BufferEnd)
            }
            KeyCode::Left if key.modifiers == KeyModifiers::META => {
                self.app.move_input_cursor(InputCursorMovement::LineStart)
            }
            KeyCode::Right if key.modifiers == KeyModifiers::META => {
                self.app.move_input_cursor(InputCursorMovement::LineEnd)
            }
            KeyCode::Up if key.modifiers == KeyModifiers::META => {
                self.app.move_input_cursor(InputCursorMovement::BufferStart)
            }
            KeyCode::Down if key.modifiers == KeyModifiers::META => {
                self.app.move_input_cursor(InputCursorMovement::BufferEnd)
            }
            KeyCode::Home if key.modifiers.is_empty() => {
                self.app.move_input_cursor(InputCursorMovement::LineStart)
            }
            KeyCode::End if key.modifiers.is_empty() => {
                self.app.move_input_cursor(InputCursorMovement::LineEnd)
            }
            KeyCode::Home if key.modifiers == KeyModifiers::CONTROL => {
                if !self.app.jump_transcript_to_top() {
                    return;
                }
            }
            KeyCode::End if key.modifiers == KeyModifiers::CONTROL => {
                if !self.app.follow_latest_transcript() {
                    return;
                }
            }
            KeyCode::Backspace => self.app.pop_input_character(),
            KeyCode::Delete => self.app.delete_next_input_character(),
            KeyCode::Tab if key.modifiers.is_empty() => {
                if !self.app.show_turn_steer_confirmation() {
                    return;
                }
            }
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
    last_frame_admitted_at: Option<Instant>,
}

impl TuiFrameScheduler {
    fn new(now: Instant) -> Self {
        let mut scheduler = Self {
            focused: true,
            next_deadline: None,
            last_frame_admitted_at: None,
        };
        scheduler.request_immediate(now);
        scheduler
    }
    fn request_immediate(&mut self, now: Instant) {
        self.coalesce_deadline(self.next_admissible_deadline(now));
    }
    fn request_delayed(&mut self, now: Instant, delay: Duration) {
        self.coalesce_deadline(self.next_admissible_deadline(now + delay));
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
        self.last_frame_admitted_at = Some(now);
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
    fn set_focused(&mut self, focused: bool, now: Instant) -> bool {
        if self.focused == focused {
            return false;
        }
        self.focused = focused;
        if focused {
            self.request_immediate(now);
        }
        true
    }
    fn coalesce_deadline(&mut self, deadline: Instant) {
        if self
            .next_deadline
            .is_none_or(|existing_deadline| deadline < existing_deadline)
        {
            self.next_deadline = Some(deadline);
        }
    }

    fn next_admissible_deadline(&self, requested: Instant) -> Instant {
        self.last_frame_admitted_at
            .map(|last_frame| requested.max(last_frame + MIN_FULLSCREEN_FRAME_INTERVAL))
            .unwrap_or(requested)
    }

    #[cfg(test)]
    fn take_pending_for_test(&mut self) -> bool {
        if !self.focused || self.next_deadline.is_none() {
            return false;
        }
        self.next_deadline = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::layout::Rect;

    use super::{MIN_FULLSCREEN_FRAME_INTERVAL, ShellRuntime, TuiFrameScheduler};
    use crate::adapter::inbound::tui::app::{
        ConversationState, TerminalUiEffect, TranscriptCardHitArea, TranscriptRenderedRow,
        TranscriptViewportFrame, test_helpers::test_native_tui_app,
    };

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn mouse(kind: MouseEventKind, column: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn bind_selectable_row(runtime: &mut ShellRuntime, text: &str) {
        runtime
            .app_mut()
            .shell
            .transcript_viewport_ui_state
            .bind_frame(
                Vec::new(),
                Vec::new(),
                Some(TranscriptViewportFrame {
                    area: Rect::new(0, 0, text.chars().count() as u16, 1),
                    rows: vec![TranscriptRenderedRow {
                        absolute_row: 0,
                        logical_line_index: 0,
                        soft_wrap_separator: String::new(),
                        selection_range_id: Some(1),
                        selectable_from_column: 0,
                        selection_excluded_columns: Vec::new(),
                        cells: text
                            .chars()
                            .map(|character| character.to_string())
                            .collect(),
                    }],
                }),
            );
    }

    #[test]
    fn frame_scheduler_coalesces_bursts_at_a_sixty_hertz_boundary() {
        let started_at = Instant::now();
        let mut scheduler = TuiFrameScheduler::new(started_at);
        assert!(scheduler.take_due(started_at));

        scheduler.request_immediate(started_at + Duration::from_millis(1));
        scheduler.request_immediate(started_at + Duration::from_millis(4));
        scheduler.request_immediate(started_at + Duration::from_millis(9));

        assert!(!scheduler.take_due(started_at + Duration::from_millis(15)));
        assert!(scheduler.take_due(started_at + MIN_FULLSCREEN_FRAME_INTERVAL));
        assert!(!scheduler.take_due(started_at + MIN_FULLSCREEN_FRAME_INTERVAL));
    }

    #[test]
    fn focus_pause_retains_one_dirty_frame_for_resume() {
        let started_at = Instant::now();
        let mut scheduler = TuiFrameScheduler::new(started_at);
        assert!(scheduler.take_due(started_at));
        assert!(scheduler.set_focused(false, started_at));
        scheduler.request_immediate(started_at + Duration::from_millis(1));

        assert!(!scheduler.take_due(started_at + Duration::from_secs(1)));
        assert!(scheduler.set_focused(true, started_at + Duration::from_secs(1)));
        assert!(scheduler.take_due(started_at + Duration::from_secs(1)));
    }

    #[test]
    fn page_keys_and_ctrl_end_drive_the_app_owned_transcript_viewport() {
        let mut runtime = ShellRuntime::new(test_native_tui_app());
        assert!(runtime.take_redraw_request());
        runtime
            .app_mut()
            .shell
            .transcript_viewport_ui_state
            .bind_document(Some("thread:test".to_string()));
        runtime
            .app_mut()
            .shell
            .transcript_viewport_ui_state
            .resolve_frame(120, 10, 1);

        runtime.handle_terminal_event(key(KeyCode::PageUp, KeyModifiers::NONE));
        let viewport = &runtime.app().shell.transcript_viewport_ui_state;
        assert_eq!(viewport.top_row(), 102);
        assert!(!viewport.follow_tail());
        assert!(runtime.take_redraw_request());

        runtime.handle_terminal_event(key(KeyCode::End, KeyModifiers::CONTROL));
        let viewport = &runtime.app().shell.transcript_viewport_ui_state;
        assert_eq!(viewport.top_row(), 110);
        assert!(viewport.follow_tail());
        assert!(runtime.take_redraw_request());
    }

    #[test]
    fn resize_invalidates_frame_local_geometry_and_requests_one_redraw() {
        let mut runtime = ShellRuntime::new(test_native_tui_app());
        assert!(runtime.take_redraw_request());
        runtime
            .app_mut()
            .shell
            .transcript_viewport_ui_state
            .bind_frame(
                vec![[7; 32]],
                vec![TranscriptCardHitArea {
                    digest: [7; 32],
                    area: Rect::new(2, 3, 20, 1),
                }],
                None,
            );
        assert_eq!(
            runtime
                .app()
                .shell
                .transcript_viewport_ui_state
                .card_hit_areas()
                .len(),
            1
        );

        runtime.handle_terminal_event(Event::Resize(80, 24));

        assert_eq!(runtime.terminal_resize_epoch(), 1);
        assert!(
            runtime
                .app()
                .shell
                .transcript_viewport_ui_state
                .card_hit_areas()
                .is_empty()
        );
        assert!(runtime.take_redraw_request());
        assert!(!runtime.take_redraw_request());
    }

    #[test]
    fn ctrl_c_copies_an_active_transcript_selection_without_leaving_the_session() {
        let mut runtime = ShellRuntime::new(test_native_tui_app());
        bind_selectable_row(&mut runtime, "selectable");

        runtime.handle_terminal_event(mouse(MouseEventKind::Down(MouseButton::Left), 0));
        runtime.handle_terminal_event(mouse(MouseEventKind::Drag(MouseButton::Left), 9));
        runtime.handle_terminal_event(mouse(MouseEventKind::Up(MouseButton::Left), 9));
        assert_eq!(
            runtime.take_terminal_ui_effects(),
            vec![TerminalUiEffect::CopyToClipboard("selectable".to_string())]
        );

        runtime.handle_terminal_event(key(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert_eq!(
            runtime.take_terminal_ui_effects(),
            vec![TerminalUiEffect::CopyToClipboard("selectable".to_string())]
        );
        assert!(!runtime.should_quit());
        assert!(matches!(
            runtime.app().conversation.lifecycle.conversation_state,
            ConversationState::Ready(_)
        ));
    }
}
