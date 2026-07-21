use ratatui::Terminal;
use ratatui::TerminalOptions;
use ratatui::Viewport;
use ratatui::backend::{Backend, ClearType};
use ratatui::layout::{Position, Rect, Size};
use ratatui::text::Line;

use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;

use super::history_insertion::HistoryInsertionMode;
use super::shell_presentation::{
    ConversationProjectionSample, build_startup_banner_lines,
    format_conversation_scrollback_lines_with_expand,
};
use super::shell_rendering::{
    InlineConversationFrameProjection, draw_projected, inline_parallel_event_stream_visible_rows,
    prepare_projected_render_state,
};
use super::shell_runtime::ShellRuntime;
use super::{
    ConversationState, INLINE_VIEWPORT_HEIGHT, InlineHistoryRenderMode, NativeTuiApp,
    ShellFrontendMode,
};
#[path = "inline_terminal_adapter/backend.rs"]
pub(super) mod backend;
#[path = "inline_terminal_adapter/history_flush.rs"]
mod history_flush;

use self::backend::InlineResizeSnapshot;
pub(super) use self::backend::{InlineResizeBackend, InlineTerminalBackend};
use self::history_flush::HistoryFlushState;

/* Inline mode uses ratatui's inline viewport while also writing durable history
 * into the host scrollback. This adapter keeps those two surfaces synchronized:
 * history may append above the viewport, while the tail frame is redrawn only when
 * its signature or the terminal geometry has changed.
 */
pub(super) fn terminal_options_for_render_mode(
    render_mode: InlineHistoryRenderMode,
) -> TerminalOptions {
    /*
     * Both inline history modes keep the live tail in ratatui's inline viewport.
     * The difference is whether historical rows are also emitted to the host
     * scrollback, so viewport height stays fixed across modes.
     */
    let viewport = match render_mode {
        InlineHistoryRenderMode::HostScrollback => Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
        InlineHistoryRenderMode::ViewportReplay => Viewport::Inline(INLINE_VIEWPORT_HEIGHT),
    };
    TerminalOptions { viewport }
}

#[derive(Debug, Clone, Copy)]
struct InlineTerminalSyncPolicy {
    render_mode: InlineHistoryRenderMode,
    insert_mode: HistoryInsertionMode,
    parallel_mode_enabled: bool,
}

enum InlineViewportSync {
    Deferred,
    Stable {
        redraw_required: bool,
        frame_projection: InlineConversationFrameProjection,
        redraw_after_successful_frame: bool,
        resize_snapshot: InlineResizeSnapshot,
    },
}

impl InlineTerminalSyncPolicy {
    fn from_sample(sample: &ConversationProjectionSample) -> Self {
        Self {
            render_mode: sample.inline_history_render_mode(),
            insert_mode: sample.history_insert_mode(),
            parallel_mode_enabled: sample.parallel_mode_enabled(),
        }
    }

    fn host_insert_mode(self) -> Option<HistoryInsertionMode> {
        self.render_mode
            .writes_host_scrollback()
            .then_some(self.insert_mode.resolve(self.parallel_mode_enabled))
    }
}

pub(super) struct InlineTerminalAdapter<B: InlineResizeBackend> {
    terminal: Terminal<B>,
    state: InlineTerminalState,
}

impl<B: InlineResizeBackend> InlineTerminalAdapter<B> {
    pub(super) fn new(terminal: Terminal<B>) -> Self {
        Self {
            terminal,
            state: InlineTerminalState::default(),
        }
    }
    pub(super) fn draw_inline_transaction(
        &mut self,
        runtime: &mut ShellRuntime,
    ) -> Result<bool, B::Error> {
        draw_inline_transaction(&mut self.terminal, runtime, &mut self.state)
    }
}

pub(super) fn draw_inline_transaction<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> Result<bool, B::Error> {
    /*
     * A transaction first reconciles durable history and geometry, then redraws
     * only if the visible tail may differ. This keeps every terminal tick cheap
     * when no stream text, history insertion, overlay, or resize changed state.
     */
    let InlineViewportSync::Stable {
        redraw_required,
        frame_projection,
        redraw_after_successful_frame,
        resize_snapshot,
    } = sync_inline_viewport_transaction(terminal, runtime, inline_terminal)?
    else {
        return Ok(false);
    };
    if redraw_required
        && !draw_inline_frame(
            terminal,
            runtime,
            inline_terminal,
            frame_projection,
            redraw_after_successful_frame,
            resize_snapshot,
        )?
    {
        return Ok(false);
    }
    Ok(true)
}

fn draw_inline_frame<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
    frame_projection: InlineConversationFrameProjection,
    redraw_after_successful_frame: bool,
    resize_snapshot: InlineResizeSnapshot,
) -> Result<bool, B::Error> {
    let acknowledge_viewport_handoff_after_draw =
        frame_projection.renders_viewport_transcript_handoff;
    if !inline_terminal.viewport.back_buffer_trustworthy {
        /*
         * Inline viewport content is not a full-screen alternate buffer. Once
         * scrollback insertion or resize may have shifted visible rows, clearing
         * before draw prevents stale glyphs from surviving under shorter frames.
         */
        clear_inline_viewport(terminal)?;
    }

    // ratatui resize can append lines while drawing; suppressing backend append
    // noise keeps the host scrollback from gaining duplicate tail frames.
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(true);
    let mut drawn_viewport_area = Rect::default();
    let mut frame_projection = Some(frame_projection);
    let result = terminal
        .draw(|frame| {
            let frame_area = frame.area();
            drawn_viewport_area = frame_area;
            let app = runtime.app_mut();
            let frame_projection = frame_projection
                .take()
                .expect("inline frame projection is consumed by one draw");
            prepare_projected_render_state(
                app,
                ShellFrontendMode::InlineMainBuffer,
                frame_area,
                &frame_projection,
            );
            draw_projected(
                frame,
                app,
                ShellFrontendMode::InlineMainBuffer,
                frame_projection,
            );
        })
        .map(|completed_frame| completed_frame.area.as_size());
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(false);
    let drawn_screen_size = result?;
    let cursor_position = terminal.get_cursor_position()?;
    let terminal_size = terminal.size()?;
    if inline_terminal.screen_size_changed(drawn_screen_size)
        || drawn_screen_size != terminal_size
        || !terminal
            .backend()
            .matches_resize_snapshot(resize_snapshot)?
    {
        /*
         * A resize can land after sync or after Ratatui flushes this frame. Keep the previous
         * screen-size observation so the next transaction reconciles physical scrollback movement
         * instead of treating it as an application-driven history fit.
         */
        defer_resize_redraw(runtime, inline_terminal);
        return Ok(false);
    }
    /*
     * ratatui reports the actual frame area used for this draw. Recording that
     * area with the post-draw cursor position is what makes the next transaction
     * able to decide whether the back buffer is still trustworthy.
     */
    inline_terminal.mark_frame_drawn(terminal_size, drawn_viewport_area, cursor_position);
    let viewport_handoff_acknowledged = acknowledge_viewport_handoff_after_draw
        && acknowledge_transcript_handoff_after_delivery(runtime, true);
    if viewport_handoff_acknowledged || redraw_after_successful_frame {
        // ACK changes the semantic viewport from the held transcript to the
        // ordinary shell/parallel frame. Make that state change its own draw.
        inline_terminal.invalidate_back_buffer();
        runtime.request_delivery_redraw();
    }
    runtime.record_successful_frame_delivery();
    Ok(true)
}

fn clear_inline_viewport<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
    terminal.clear()
}

fn clear_visible_inline_rows<B: Backend>(terminal: &mut Terminal<B>) -> Result<(), B::Error> {
    let area = current_viewport_area(terminal);
    for y in area.y..area.bottom() {
        terminal
            .backend_mut()
            .set_cursor_position(Position { x: area.x, y })?;
        terminal
            .backend_mut()
            .clear_region(ClearType::CurrentLine)?;
    }
    terminal.backend_mut().flush()
}

#[cfg(test)]
fn sync_inline_viewport<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> Result<bool, B::Error> {
    match sync_inline_viewport_transaction(terminal, runtime, inline_terminal)? {
        InlineViewportSync::Deferred => Ok(false),
        InlineViewportSync::Stable {
            redraw_required, ..
        } => Ok(redraw_required),
    }
}

fn sync_inline_viewport_transaction<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) -> Result<InlineViewportSync, B::Error> {
    inline_terminal.observe_focus_reacquire(runtime.terminal_focus_reacquire_epoch());
    // Capture render settings before mutating terminal state so one transaction uses
    // a stable compatibility policy snapshot instead of ad hoc env-owned fields.
    let projection_sample = ConversationProjectionSample::capture(runtime.app_mut());
    inline_terminal.observe_conversation_history_identity_revision(
        projection_sample.conversation_history_identity_revision(),
    );
    let policy = InlineTerminalSyncPolicy::from_sample(&projection_sample);
    /*
     * Autoresize can itself move the inline viewport. It happens before history
     * flush so the flush logic knows how many visible rows fit in the current
     * terminal, not the previous frame's dimensions.
     */
    let resize_event_epoch = runtime.terminal_resize_epoch();
    terminal
        .backend_mut()
        .observe_resize_epoch(resize_event_epoch);
    let Some(resize_snapshot) = autoresize_inline_viewport(terminal)? else {
        defer_resize_redraw(runtime, inline_terminal);
        return Ok(InlineViewportSync::Deferred);
    };
    let terminal_size = resize_snapshot.size;
    let physical_terminal_resized = inline_terminal.physical_terminal_resized(resize_snapshot);
    let viewport_area = current_viewport_area(terminal);
    let sampled_parallel_frame_projection = policy.parallel_mode_enabled.then(|| {
        InlineConversationFrameProjection::from_app_with_sample(
            runtime.app_mut(),
            viewport_area.width,
            &projection_sample,
        )
    });
    let parallel_handoff_conversation_lines =
        if policy.parallel_mode_enabled && policy.host_insert_mode().is_some() {
            parallel_conversation_handoff_projection(runtime.app_mut())
        } else {
            None
        };
    let current_history_projection = current_inline_history_lines_for_viewport(
        runtime.app_mut(),
        viewport_area,
        &projection_sample,
        sampled_parallel_frame_projection.as_ref(),
    );
    let preserves_conversation_baseline = current_history_projection.is_none();
    let current_lines = current_history_projection.unwrap_or_default();
    let parallel_handoff_pending_lines = parallel_handoff_conversation_lines
        .as_deref()
        .map(|conversation_lines| {
            inline_terminal
                .history_flush
                .pending_lines(conversation_lines)
        })
        .unwrap_or_default();
    if !terminal
        .backend()
        .matches_resize_snapshot(resize_snapshot)?
    {
        defer_resize_redraw(runtime, inline_terminal);
        return Ok(InlineViewportSync::Deferred);
    }
    let Some(insert_mode) = policy.host_insert_mode() else {
        /*
         * ViewportReplay keeps transcript rows inside ratatui rendering and must not
         * mutate host scrollback just because stale host-only row accounting survived
         * from an earlier host-scrollback transaction.
         */
        let cursor_position = terminal.get_cursor_position()?;
        if !terminal
            .backend()
            .matches_resize_snapshot(resize_snapshot)?
        {
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        }
        if !preserves_conversation_baseline {
            if policy.parallel_mode_enabled {
                inline_terminal
                    .history_flush
                    .remember_parallel_without_flush(&current_lines);
            } else {
                inline_terminal
                    .history_flush
                    .remember_without_flush(&current_lines);
            }
        }
        if physical_terminal_resized {
            inline_terminal.invalidate_back_buffer();
        }
        inline_terminal.record_terminal_viewport(terminal_size, viewport_area, cursor_position);
        inline_terminal.mark_resize_reconciled(resize_snapshot);
        let frame_projection = sampled_parallel_frame_projection.unwrap_or_else(|| {
            InlineConversationFrameProjection::from_app_with_sample(
                runtime.app_mut(),
                viewport_area.width,
                &projection_sample,
            )
        });
        let tail_frame_changed = inline_terminal.should_draw_inline_frame(
            &frame_projection,
            viewport_area.width,
            viewport_area.height,
        );
        return Ok(InlineViewportSync::Stable {
            redraw_required: tail_frame_changed,
            frame_projection,
            redraw_after_successful_frame: false,
            resize_snapshot,
        });
    };
    let parallel_history_pending = policy.parallel_mode_enabled
        && (inline_terminal
            .history_flush
            .has_pending_parallel_lines(&current_lines)
            || !parallel_handoff_pending_lines.is_empty());
    let parallel_history_fit_would_scroll = policy.parallel_mode_enabled
        && inline_terminal.history_flush.visible_history_rows > viewport_area.top();
    let visible_history_rows_before = inline_terminal.history_flush.visible_history_rows;
    if parallel_history_pending || parallel_history_fit_would_scroll {
        if !terminal
            .backend()
            .matches_resize_snapshot(resize_snapshot)?
        {
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        }
        /*
         * Parallel mode streams event rows into host scrollback while redrawing
         * the board as a live inline panel. Any scrollback movement before the
         * next draw can otherwise push panel chrome into terminal history. Blank
         * the old live frame first, then let history fitting/insertion move only
         * empty rows plus the event rows it explicitly writes.
         */
        clear_visible_inline_rows(terminal)?;
        inline_terminal.invalidate_back_buffer();
        if !terminal
            .backend()
            .matches_resize_snapshot(resize_snapshot)?
        {
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        }
    }
    let visible_history_adjusted = if physical_terminal_resized {
        if !terminal
            .backend()
            .matches_resize_snapshot(resize_snapshot)?
        {
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        }
        inline_terminal
            .history_flush
            .reconcile_physical_resize(viewport_area)
    } else {
        let Some(adjusted) = inline_terminal.history_flush.fit_visible_rows_to_viewport(
            terminal,
            resize_snapshot,
            viewport_area,
        )?
        else {
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        };
        adjusted
    };
    if visible_history_adjusted {
        /*
         * Fitting history to a smaller viewport may insert or remove visible
         * scrollback rows above the live tail. Even if the tail signature is the
         * same, the existing back buffer no longer proves what is on screen.
         */
        inline_terminal.invalidate_back_buffer();
    }

    /*
     * HostScrollback mode writes only the history delta. The tail frame stays
     * in the inline viewport so the operator can scroll back through durable
     * transcript rows without duplicating the live status panel.
     */
    let history_sync_result = if preserves_conversation_baseline {
        Ok(self::history_flush::HistoryFlushResult::preserved_baseline())
    } else if policy.parallel_mode_enabled {
        inline_terminal.history_flush.sync_parallel(
            terminal,
            &current_lines,
            resize_snapshot,
            insert_mode,
        )
    } else {
        inline_terminal
            .history_flush
            .sync(terminal, &current_lines, resize_snapshot, insert_mode)
    };
    let history_sync = match history_sync_result {
        Ok(history_sync) => history_sync,
        Err(error) => {
            inline_terminal.history_flush.visible_history_rows = visible_history_rows_before;
            return Err(error);
        }
    };
    let handoff_acknowledged = acknowledge_transcript_handoff_after_delivery(
        runtime,
        history_sync.history_committed() && !policy.parallel_mode_enabled,
    );
    if handoff_acknowledged {
        inline_terminal.invalidate_back_buffer();
    }
    if !history_sync.stable_geometry() {
        if history_sync.history_committed() {
            inline_terminal
                .history_flush
                .mark_visible_history_rows_dirty();
        } else {
            inline_terminal.history_flush.visible_history_rows = visible_history_rows_before;
        }
        defer_resize_redraw(runtime, inline_terminal);
        return Ok(InlineViewportSync::Deferred);
    }
    let mut redraw_after_successful_frame = false;
    let parallel_handoff_sync =
        if let Some(conversation_lines) = parallel_handoff_conversation_lines.as_deref() {
            let visible_history_rows_before_handoff =
                inline_terminal.history_flush.visible_history_rows;
            let handoff_sync = match inline_terminal
                .history_flush
                .append_durable_lines_preserving_baseline(
                    terminal,
                    &parallel_handoff_pending_lines,
                    resize_snapshot,
                    insert_mode,
                ) {
                Ok(handoff_sync) => handoff_sync,
                Err(error) => {
                    inline_terminal.history_flush.visible_history_rows =
                        visible_history_rows_before_handoff;
                    return Err(error);
                }
            };
            if handoff_sync.history_committed() {
                inline_terminal
                    .history_flush
                    .remember_conversation_projection(conversation_lines);
            }
            let parallel_handoff_acknowledged = acknowledge_transcript_handoff_after_delivery(
                runtime,
                handoff_sync.history_committed(),
            );
            if parallel_handoff_acknowledged {
                inline_terminal.invalidate_back_buffer();
                redraw_after_successful_frame = true;
            }
            if !handoff_sync.stable_geometry() {
                if !handoff_sync.history_committed() {
                    inline_terminal.history_flush.visible_history_rows =
                        visible_history_rows_before_handoff;
                }
                defer_resize_redraw(runtime, inline_terminal);
                return Ok(InlineViewportSync::Deferred);
            }
            Some(handoff_sync)
        } else {
            None
        };
    let history_inserted = history_sync.inserted()
        || parallel_handoff_sync.is_some_and(|handoff_sync| handoff_sync.inserted());
    if history_inserted {
        inline_terminal.invalidate_back_buffer();
    }

    // History flushing can move the inline viewport. Re-read frame geometry before
    // comparing the tail-frame signature.
    let viewport_area = current_viewport_area(terminal);
    let cursor_position = terminal.get_cursor_position()?;
    if !terminal
        .backend()
        .matches_resize_snapshot(resize_snapshot)?
    {
        inline_terminal
            .history_flush
            .mark_visible_history_rows_dirty();
        defer_resize_redraw(runtime, inline_terminal);
        return Ok(InlineViewportSync::Deferred);
    }
    if physical_terminal_resized {
        inline_terminal.invalidate_back_buffer();
    }
    inline_terminal.viewport.insert_mode = insert_mode;
    inline_terminal.record_terminal_viewport(terminal_size, viewport_area, cursor_position);
    inline_terminal.mark_resize_reconciled(resize_snapshot);
    let frame_projection = sampled_parallel_frame_projection.unwrap_or_else(|| {
        InlineConversationFrameProjection::from_app_with_sample(
            runtime.app_mut(),
            viewport_area.width,
            &projection_sample,
        )
    });
    let tail_frame_changed = inline_terminal.should_draw_inline_frame(
        &frame_projection,
        viewport_area.width,
        viewport_area.height,
    );
    Ok(InlineViewportSync::Stable {
        redraw_required: visible_history_adjusted || history_inserted || tail_frame_changed,
        frame_projection,
        redraw_after_successful_frame,
        resize_snapshot,
    })
}

fn acknowledge_transcript_handoff_after_delivery(
    runtime: &mut ShellRuntime,
    delivery_committed: bool,
) -> bool {
    if !delivery_committed {
        return false;
    }
    let ConversationState::Ready(conversation) = &mut runtime.app_mut().conversation_state else {
        return false;
    };
    conversation.acknowledge_viewport_transcript_handoff_flush()
}

fn parallel_conversation_handoff_projection(app: &NativeTuiApp) -> Option<Vec<Line<'static>>> {
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        return None;
    };
    conversation.viewport_transcript_handoff_release_messages()?;
    Some(format_conversation_scrollback_lines_with_expand(
        conversation.host_scrollback_messages(),
        app.conversation_view_mode,
        app.conversation_view_mode.shows_debug_details()
            || app.planning_worker_shows_debug_details(),
        Some(app.progressive_activity_overlay_ui_state.expand_state()),
    ))
}

fn defer_resize_redraw(runtime: &mut ShellRuntime, inline_terminal: &mut InlineTerminalState) {
    inline_terminal.invalidate_back_buffer();
    runtime.request_resize_redraw_retry();
}

fn current_viewport_area<B: Backend>(terminal: &mut Terminal<B>) -> Rect {
    terminal.get_frame().area()
}

fn autoresize_inline_viewport<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
) -> Result<Option<InlineResizeSnapshot>, B::Error> {
    const MAX_STABILITY_ATTEMPTS: usize = 3;

    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(true);
    let result = (|| {
        let mut observed_size = terminal.size()?;
        for _ in 0..MAX_STABILITY_ATTEMPTS {
            let observation_epoch_before = terminal.backend().resize_observation_epoch();
            terminal.autoresize()?;
            let terminal_size = terminal.size()?;
            let observation_epoch = terminal.backend().resize_observation_epoch();
            if terminal_size == observed_size && observation_epoch == observation_epoch_before {
                return Ok(Some(InlineResizeSnapshot {
                    size: terminal_size,
                    event_epoch: terminal.backend().resize_event_epoch(),
                    observation_epoch,
                }));
            }
            observed_size = terminal_size;
        }
        Ok(None)
    })();
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(false);
    result
}

#[cfg(test)]
fn current_inline_history_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    let sample = ConversationProjectionSample::capture(app);
    let parallel_frame_projection = sample
        .parallel_mode_enabled()
        .then(|| InlineConversationFrameProjection::from_app_with_sample(app, 80, &sample));
    current_inline_history_lines_for_viewport(
        app,
        Rect::new(0, 0, 80, INLINE_VIEWPORT_HEIGHT),
        &sample,
        parallel_frame_projection.as_ref(),
    )
    .unwrap_or_default()
}

fn current_inline_history_lines_for_viewport(
    app: &NativeTuiApp,
    viewport_area: Rect,
    sample: &ConversationProjectionSample,
    parallel_frame_projection: Option<&InlineConversationFrameProjection>,
) -> Option<Vec<Line<'static>>> {
    if sample.parallel_mode_enabled() {
        /*
         * Parallel mode owns the main inline body with the supervisor board. The
         * durable host scrollback should receive only append-only event rows so
         * operators can scroll back through past activity without replaying the
         * live panel title or footer chrome.
         */
        return Some(
            parallel_frame_projection.map_or_else(Vec::new, |projection| {
                current_inline_parallel_history_lines(viewport_area, sample, projection)
            }),
        );
    }
    if let Some(startup_banner_lines) =
        build_startup_banner_lines(app, sample.parallel_mode_enabled(), None)
    {
        /*
         * Startup banner wins over conversation history because before the first
         * ready conversation the scrollback should explain boot diagnostics, not
         * show an empty transcript placeholder.
         */
        return Some(startup_banner_lines);
    }
    match &app.conversation_state {
        ConversationState::Ready(conversation) => {
            /*
             * Host scrollback is the durable transcript surface, so it must not
             * share the live screen's capped projection. Reformat from committed
             * messages only: live agent deltas and prompt text stay in the tail.
             */
            let messages = conversation.host_scrollback_messages();
            if messages.is_empty()
                && conversation
                    .viewport_transcript_handoff_messages()
                    .is_some()
            {
                return Some(Vec::new());
            }
            Some(format_conversation_scrollback_lines_with_expand(
                messages,
                app.conversation_view_mode,
                app.conversation_view_mode.shows_debug_details()
                    || app.planning_worker_shows_debug_details(),
                Some(app.progressive_activity_overlay_ui_state.expand_state()),
            ))
        }
        // Loading and failure are not authoritative empty conversations. Keep
        // the prior diff baseline until a semantic identity revision or a Ready
        // projection decides what should be delivered next.
        ConversationState::Loading | ConversationState::Failed(_) => None,
    }
}

fn current_inline_parallel_history_lines(
    viewport_area: Rect,
    sample: &ConversationProjectionSample,
    frame_projection: &InlineConversationFrameProjection,
) -> Vec<Line<'static>> {
    let live_tail_lines =
        inline_parallel_event_stream_visible_rows(frame_projection, viewport_area);
    sample.parallel_supervisor_event_scrollback_lines_before_live_tail(
        live_tail_lines,
        viewport_area.width,
    )
}

#[derive(Default)]
pub(super) struct InlineTerminalState {
    viewport: TerminalViewportState,
    history_flush: HistoryFlushState,
    frame_cache: FrameCacheState,
    last_conversation_history_identity_revision: u64,
}

impl InlineTerminalState {
    fn observe_conversation_history_identity_revision(&mut self, revision: u64) {
        if self.last_conversation_history_identity_revision == revision {
            return;
        }
        self.last_conversation_history_identity_revision = revision;
        self.history_flush.reset_conversation_projection();
        self.invalidate_back_buffer();
    }

    fn screen_size_changed(&self, terminal_size: Size) -> bool {
        self.viewport
            .last_known_screen_size
            .is_some_and(|last_known_screen_size| last_known_screen_size != terminal_size)
    }

    fn physical_terminal_resized(&self, snapshot: InlineResizeSnapshot) -> bool {
        self.screen_size_changed(snapshot.size)
            || self.viewport.last_reconciled_resize_event_epoch != snapshot.event_epoch
            || self.viewport.last_reconciled_resize_observation_epoch != snapshot.observation_epoch
    }

    fn observe_focus_reacquire(&mut self, focus_reacquire_epoch: u64) {
        if self.viewport.last_observed_focus_reacquire_epoch == focus_reacquire_epoch {
            return;
        }
        self.viewport.last_observed_focus_reacquire_epoch = focus_reacquire_epoch;
        self.invalidate_back_buffer();
    }

    fn mark_resize_reconciled(&mut self, snapshot: InlineResizeSnapshot) {
        self.viewport.last_reconciled_resize_event_epoch = snapshot.event_epoch;
        self.viewport.last_reconciled_resize_observation_epoch = snapshot.observation_epoch;
    }

    fn record_terminal_viewport(
        &mut self,
        terminal_size: Size,
        viewport_area: Rect,
        cursor_position: Position,
    ) {
        /*
         * Screen size changes invalidate the viewport even if ratatui reports
         * the same inline area for one tick. Terminal emulators can keep cursor
         * position stable while wrapping rows differently after a resize.
         */
        let terminal_resized = self.screen_size_changed(terminal_size);
        self.viewport
            .record_terminal_viewport(terminal_size, viewport_area, cursor_position);
        if terminal_resized {
            self.invalidate_back_buffer();
        }
    }

    fn invalidate_back_buffer(&mut self) {
        self.viewport.invalidate_back_buffer();
    }

    fn mark_frame_drawn(
        &mut self,
        terminal_size: Size,
        viewport_area: Rect,
        cursor_position: Position,
    ) {
        self.viewport
            .mark_frame_drawn(terminal_size, viewport_area, cursor_position);
    }

    fn should_draw_inline_frame(
        &mut self,
        frame_projection: &InlineConversationFrameProjection,
        terminal_width: u16,
        terminal_height: u16,
    ) -> bool {
        self.frame_cache.should_draw_inline_frame(
            frame_projection,
            &self.viewport,
            terminal_width,
            terminal_height,
        )
    }
    #[cfg(test)]
    fn last_known_screen_size(&self) -> Option<Size> {
        self.viewport.last_known_screen_size
    }
    #[cfg(test)]
    fn last_known_cursor_pos(&self) -> Option<Position> {
        self.viewport.last_known_cursor_pos
    }
    #[cfg(test)]
    fn viewport_area(&self) -> Option<Rect> {
        self.viewport.viewport_area
    }
    #[cfg(test)]
    fn back_buffer_trustworthy(&self) -> bool {
        self.viewport.back_buffer_trustworthy
    }
    #[cfg(test)]
    fn insert_mode(&self) -> HistoryInsertionMode {
        self.viewport.insert_mode
    }
}

// A trustworthy back buffer means the visible inline tail exactly matches the
// last frame we drew. Resize, focus reacquisition, scrollback insertion, and
// history-fit changes invalidate that trust and force a clear before the next draw.
struct TerminalViewportState {
    viewport_area: Option<Rect>,
    last_known_screen_size: Option<Size>,
    last_known_cursor_pos: Option<Position>,
    last_reconciled_resize_event_epoch: u64,
    last_reconciled_resize_observation_epoch: u64,
    last_observed_focus_reacquire_epoch: u64,
    back_buffer_trustworthy: bool,
    insert_mode: HistoryInsertionMode,
}

impl Default for TerminalViewportState {
    fn default() -> Self {
        Self {
            viewport_area: None,
            last_known_screen_size: None,
            last_known_cursor_pos: None,
            last_reconciled_resize_event_epoch: 0,
            last_reconciled_resize_observation_epoch: 0,
            last_observed_focus_reacquire_epoch: 0,
            back_buffer_trustworthy: true,
            insert_mode: HistoryInsertionMode::default(),
        }
    }
}

impl TerminalViewportState {
    fn record_terminal_viewport(
        &mut self,
        terminal_size: Size,
        viewport_area: Rect,
        cursor_position: Position,
    ) {
        self.viewport_area = Some(viewport_area);
        self.last_known_screen_size = Some(terminal_size);
        self.last_known_cursor_pos = Some(cursor_position);
    }

    fn invalidate_back_buffer(&mut self) {
        self.back_buffer_trustworthy = false;
    }

    fn mark_frame_drawn(
        &mut self,
        terminal_size: Size,
        viewport_area: Rect,
        cursor_position: Position,
    ) {
        /*
         * Marking a frame drawn is the only path that restores trust. Recording
         * viewport geometry without a draw only observes the terminal; it does
         * not prove the visible cells match our tail-frame signature.
         */
        self.record_terminal_viewport(terminal_size, viewport_area, cursor_position);
        self.back_buffer_trustworthy = true;
    }
}

#[derive(Default)]
struct FrameCacheState {
    last_tail_frame: Option<InlineTailFrameSignature>,
}

impl FrameCacheState {
    fn should_draw_inline_frame(
        &mut self,
        frame_projection: &InlineConversationFrameProjection,
        viewport: &TerminalViewportState,
        terminal_width: u16,
        terminal_height: u16,
    ) -> bool {
        if frame_projection.shell_overlay != ShellOverlay::Hidden
            || frame_projection.exit_confirmation_visible
            || frame_projection.turn_steer_confirmation.is_some()
        {
            // Overlay frames are modal and can overwrite the tail; drop the cache so
            // returning to the main shell redraws from a fresh signature.
            self.last_tail_frame = None;
            return true;
        }

        /*
         * The signature stores rendered lines, not source messages. That makes
         * cache invalidation follow the exact text ratatui will paint after
         * wrapping, planning status projection, and terminal-width decisions.
         */
        let next_signature = InlineTailFrameSignature {
            core_revision: frame_projection.core_revision,
            terminal_width,
            terminal_height,
            lines: frame_projection.tail_view.lines.clone(),
            prompt_cursor_offset: frame_projection.tail_view.prompt_cursor_offset,
            live_transcript_lines: frame_projection.live_transcript_lines.clone(),
            parallel_supervisor_events: frame_projection.parallel_supervisor_event_lines.clone(),
            renders_viewport_transcript_handoff: frame_projection
                .renders_viewport_transcript_handoff,
        };
        let should_draw = !viewport.back_buffer_trustworthy
            || self.last_tail_frame.as_ref() != Some(&next_signature);
        self.last_tail_frame = Some(next_signature);
        should_draw
    }
}

#[derive(Clone, PartialEq, Eq)]
struct InlineTailFrameSignature {
    core_revision: u64,
    terminal_width: u16,
    terminal_height: u16,
    lines: Vec<Line<'static>>,
    prompt_cursor_offset: Option<(u16, u16)>,
    live_transcript_lines: Vec<Line<'static>>,
    parallel_supervisor_events: Vec<Line<'static>>,
    renders_viewport_transcript_handoff: bool,
}

#[cfg(test)]
#[path = "inline_terminal_adapter/tests.rs"]
mod tests;
