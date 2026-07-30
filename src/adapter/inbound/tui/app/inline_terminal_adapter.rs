use ratatui::Terminal;
use ratatui::TerminalOptions;
use ratatui::Viewport;
use ratatui::backend::{Backend, ClearType};
use ratatui::layout::{Position, Rect, Size};
use ratatui::text::Line;

use crate::adapter::inbound::tui::shell_chrome::ShellOverlay;

#[cfg(test)]
use super::NativeTuiApp;
use super::history_insertion::{
    HistoryInsertionAdapter, HistoryInsertionMode, ParallelHistoryInsertionOutcome,
    count_rendered_history_rows,
};
use super::inline_frame_model::InlineTerminalSyncProjection;
#[cfg(test)]
use super::inline_frame_model::{
    apply_inline_frame_render_receipt, capture_inline_shell_frame_model,
};
#[cfg(test)]
use super::inline_frame_model::{
    capture_inline_terminal_sync_projection, capture_parallel_conversation_handoff_projection,
};
use super::parallel_terminal_delivery::{
    ParallelHostReceiptSettlement, ParallelHostWriteStartError, ParallelHostWriteTransition,
    ParallelStreamDeliveryPlan, ParallelTerminalDeliveryState, TerminalSurfaceTransition,
};
use super::shell_presentation::{ConversationProjectionSample, TranscriptHandoffDeliveryToken};
use super::shell_rendering::{
    InlineConversationFrameProjection, InlineFrameRenderReceipt, draw_projected,
    inline_parallel_event_stream_area,
};
use super::shell_runtime::ShellRuntime;
use super::{
    INLINE_HOST_SCROLLBACK_REFLOW_GUARD_ROWS, INLINE_VIEWPORT_HEIGHT, InlineHistoryRenderMode,
    ShellFrontendMode, TuiLanguage,
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
        frame_projection: Box<InlineConversationFrameProjection>,
        redraw_after_successful_frame: bool,
        resize_snapshot: InlineResizeSnapshot,
    },
}

enum ParallelHostSync {
    Deferred,
    Stable {
        inserted: bool,
        stable_geometry: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FrameRenderAttempt(u64);

struct PendingInlineFrameRenderReceipt {
    attempt: FrameRenderAttempt,
    receipt: InlineFrameRenderReceipt,
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
            .then_some(self.insert_mode.resolve())
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
            *frame_projection,
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
    let viewport_handoff_delivery_token =
        frame_projection.transcript_handoff_delivery_token.clone();
    let park_hidden_cursor_at_terminal_anchor =
        frame_projection.shell_overlay == ShellOverlay::Supersession;
    if !inline_terminal.viewport.back_buffer_trustworthy {
        /*
         * Inline viewport content is not a full-screen alternate buffer. Once
         * scrollback insertion or resize may have shifted visible rows, clearing
         * before draw prevents stale glyphs from surviving under shorter frames.
         */
        if let Err(error) = clear_inline_viewport(
            terminal,
            inline_terminal.viewport.last_drawn_viewport_area,
            inline_terminal.viewport.resize_reflow_rows_to_clear,
        ) {
            fail_closed_frame_delivery(runtime, inline_terminal);
            return Err(error);
        }
    }

    let frame_model = runtime.capture_inline_shell_frame_model(
        ShellFrontendMode::InlineMainBuffer,
        current_viewport_area(terminal),
        frame_projection,
    );
    let render_attempt = inline_terminal.begin_frame_render_attempt();

    // ratatui resize can append lines while drawing; suppressing backend append
    // noise keeps the host scrollback from gaining duplicate tail frames.
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(true);
    let mut drawn_viewport_area = Rect::default();
    let mut frame_model = Some(frame_model);
    let mut pending_render_receipt = None;
    let result = terminal
        .draw(|frame| {
            drawn_viewport_area = frame.area();
            let frame_model = frame_model
                .take()
                .expect("inline frame model is consumed by one draw");
            pending_render_receipt = Some(PendingInlineFrameRenderReceipt {
                attempt: render_attempt,
                receipt: draw_projected(frame, ShellFrontendMode::InlineMainBuffer, frame_model),
            });
        })
        .map(|completed_frame| completed_frame.area.as_size());
    terminal
        .backend_mut()
        .set_resize_append_lines_suppressed(false);
    let drawn_screen_size = match result {
        Ok(drawn_screen_size) => drawn_screen_size,
        Err(error) => {
            // The frame signature was staged before terminal I/O. A failed
            // flush cannot prove any visible cell, so the next transaction
            // must redraw even when the semantic projection is unchanged.
            fail_closed_frame_delivery(runtime, inline_terminal);
            return Err(error);
        }
    };
    let terminal_size = match terminal.size() {
        Ok(terminal_size) => terminal_size,
        Err(error) => {
            fail_closed_frame_delivery(runtime, inline_terminal);
            return Err(error);
        }
    };
    if park_hidden_cursor_at_terminal_anchor && terminal_size.height > 0 {
        /*
         * Focused parallel operations has no interactive cursor. Anchor its
         * still-hidden physical cursor at the live viewport origin so tmux
         * reflow keeps the host/live boundary stable instead of treating the
         * last changed event row as terminal history. The canonical stream
         * redraws clipped live rows after the resize.
         */
        if let Err(error) = terminal.backend_mut().set_cursor_position(Position::new(
            drawn_viewport_area.x,
            terminal_size
                .height
                .saturating_sub(drawn_viewport_area.height),
        )) {
            fail_closed_frame_delivery(runtime, inline_terminal);
            return Err(error);
        }
    }
    let cursor_position = match terminal.get_cursor_position() {
        Ok(cursor_position) => cursor_position,
        Err(error) => {
            fail_closed_frame_delivery(runtime, inline_terminal);
            return Err(error);
        }
    };
    let matches_resize_snapshot = match terminal.backend().matches_resize_snapshot(resize_snapshot)
    {
        Ok(matches_resize_snapshot) => matches_resize_snapshot,
        Err(error) => {
            fail_closed_frame_delivery(runtime, inline_terminal);
            return Err(error);
        }
    };
    if inline_terminal.screen_size_changed(drawn_screen_size)
        || drawn_screen_size != terminal_size
        || !matches_resize_snapshot
    {
        /*
         * A resize can land after sync or after Ratatui flushes this frame. Keep the previous
         * screen-size observation so the next transaction reconciles physical scrollback movement
         * instead of treating it as an application-driven history fit.
         */
        defer_resize_redraw(runtime, inline_terminal);
        return Ok(false);
    }
    let pending_render_receipt =
        pending_render_receipt.expect("successful terminal draw must produce one render receipt");
    if !inline_terminal.commit_frame_render_receipt(pending_render_receipt, |receipt| {
        runtime.commit_inline_frame_render_receipt(receipt)
    }) {
        fail_closed_frame_delivery(runtime, inline_terminal);
        runtime.request_delivery_redraw();
        return Ok(false);
    }
    /*
     * ratatui reports the actual frame area used for this draw. Recording that
     * area with the post-draw cursor position is what makes the next transaction
     * able to decide whether the back buffer is still trustworthy.
     */
    inline_terminal.mark_frame_drawn(terminal_size, drawn_viewport_area, cursor_position);
    let viewport_handoff_acknowledged = acknowledge_transcript_handoff_after_delivery(
        runtime,
        viewport_handoff_delivery_token.as_deref(),
    );
    if viewport_handoff_acknowledged || redraw_after_successful_frame {
        // ACK changes the semantic viewport from the held transcript to the
        // ordinary shell/parallel frame. Make that state change its own draw.
        inline_terminal.invalidate_back_buffer();
        runtime.request_delivery_redraw();
    }
    runtime.record_successful_frame_delivery();
    Ok(true)
}

fn clear_inline_viewport<B: Backend>(
    terminal: &mut Terminal<B>,
    last_drawn_viewport_area: Option<Rect>,
    resize_reflow_rows_to_clear: u16,
) -> Result<(), B::Error> {
    let current_area = current_viewport_area(terminal);
    let terminal_size = terminal.size()?;
    /*
     * Some main-buffer terminals reflow a previously drawn tail before Ratatui
     * observes the new width. The extra tail row lands immediately above the
     * newly anchored viewport and is outside both Ratatui buffers. Host
     * scrollback always ends with reserved blank guard rows, so the guarded
     * cleanup is safe even when the emulator keeps the cursor fixed while
     * reflowing.
     * Additional rows require both an observed cursor shift and a prior-tail
     * wrap budget; durable transcript rows above that bound remain untouched.
     */
    let reflow_start = current_area.y.saturating_sub(resize_reflow_rows_to_clear);
    for y in reflow_start..current_area.y {
        terminal.backend_mut().set_cursor_position(Position {
            x: current_area.x,
            y,
        })?;
        terminal
            .backend_mut()
            .clear_region(ClearType::CurrentLine)?;
    }
    if let Some(previous_area) = last_drawn_viewport_area {
        // A resize can move an inline viewport without moving every old cell
        // into the new area. Clear only the previous rows outside the current
        // viewport before Terminal::clear resets the current viewport buffer.
        for y in previous_area.y..previous_area.bottom().min(terminal_size.height) {
            if y >= current_area.y && y < current_area.bottom() {
                continue;
            }
            terminal.backend_mut().set_cursor_position(Position {
                x: previous_area.x.min(terminal_size.width.saturating_sub(1)),
                y,
            })?;
            terminal
                .backend_mut()
                .clear_region(ClearType::CurrentLine)?;
        }
    }
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
    let projection_sample = runtime.capture_inline_terminal_projection_sample();
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
    let InlineTerminalSyncProjection {
        mut sampled_parallel_frame_projection,
        parallel_handoff_conversation_lines,
        current_history_projection,
        parallel_event_stream_snapshot,
    } = runtime.capture_inline_terminal_sync_projection(viewport_area, &projection_sample);
    let preserves_conversation_baseline = current_history_projection.is_none();
    let current_lines = current_history_projection.unwrap_or_default();
    let conversation_handoff_delivery_token = (!policy.parallel_mode_enabled)
        .then(|| TranscriptHandoffDeliveryToken::from_sample(&projection_sample))
        .flatten();
    let parallel_handoff_pending_lines = parallel_handoff_conversation_lines
        .as_ref()
        .map(|handoff| inline_terminal.history_flush.pending_lines(&handoff.lines))
        .unwrap_or_default();
    if !terminal
        .backend()
        .matches_resize_snapshot(resize_snapshot)?
    {
        defer_resize_redraw(runtime, inline_terminal);
        return Ok(InlineViewportSync::Deferred);
    }
    if physical_terminal_resized {
        let resize_cursor_position = terminal.get_cursor_position()?;
        let has_host_scrollback_guard = policy.render_mode.writes_host_scrollback()
            && inline_terminal
                .history_flush
                .has_trailing_reflow_guard_rows();
        if !terminal
            .backend()
            .matches_resize_snapshot(resize_snapshot)?
        {
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        }
        inline_terminal.observe_physical_resize_reflow(
            terminal_size,
            resize_cursor_position,
            has_host_scrollback_guard,
        );
    }
    let mut parallel_plan = match (
        sampled_parallel_frame_projection.as_ref(),
        parallel_event_stream_snapshot.as_ref(),
    ) {
        (Some(frame_projection), Some(snapshot)) => {
            inline_terminal
                .parallel_delivery
                .transition_terminal_surface(
                    TerminalSurfaceTransition::PreserveHostScrollback,
                    snapshot.generation(),
                );
            let event_area = inline_parallel_event_stream_area(frame_projection, viewport_area);
            let fallback_status_lines = frame_projection
                .parallel_live_stream()
                .map_or_else(Vec::new, |stream| stream.fallback_status_lines());
            Some(inline_terminal.parallel_delivery.prepare_plan(
                snapshot,
                policy.render_mode,
                event_area,
                frame_projection.tui_language,
                &fallback_status_lines,
            ))
        }
        (None, None) => None,
        _ => unreachable!("parallel frame and event snapshot must be sampled together"),
    };
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
            inline_terminal
                .history_flush
                .remember_without_flush(&current_lines);
        }
        if physical_terminal_resized {
            inline_terminal.invalidate_back_buffer();
        }
        inline_terminal.record_terminal_viewport(terminal_size, viewport_area, cursor_position);
        inline_terminal.mark_resize_reconciled(resize_snapshot);
        let mut frame_projection = sampled_parallel_frame_projection.unwrap_or_else(|| {
            runtime.capture_inline_conversation_frame_projection(
                viewport_area.width,
                &projection_sample,
            )
        });
        if let Some(plan) = parallel_plan.take() {
            let (_, live_stream) = plan.into_parts();
            frame_projection.install_parallel_live_stream(live_stream);
        }
        let tail_frame_changed = inline_terminal.should_draw_inline_frame(
            &frame_projection,
            viewport_area.width,
            viewport_area.height,
        );
        return Ok(InlineViewportSync::Stable {
            redraw_required: tail_frame_changed,
            frame_projection: Box::new(frame_projection),
            redraw_after_successful_frame: false,
            resize_snapshot,
        });
    };
    let parallel_history_pending = policy.parallel_mode_enabled
        && (parallel_plan
            .as_ref()
            .is_some_and(|plan| plan.host_batch().is_some())
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

    let parallel_host_inserted = if policy.parallel_mode_enabled {
        let plan = parallel_plan
            .take()
            .expect("parallel host mode must own one delivery plan");
        let language = sampled_parallel_frame_projection
            .as_ref()
            .expect("parallel plan must own a frame projection")
            .tui_language;
        let host_sync = sync_parallel_host_delivery(
            terminal,
            inline_terminal,
            &plan,
            resize_snapshot,
            insert_mode,
            language,
        )?;
        let ParallelHostSync::Stable {
            inserted,
            stable_geometry,
        } = host_sync
        else {
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        };
        if !stable_geometry {
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        }

        let frame_projection = sampled_parallel_frame_projection
            .as_mut()
            .expect("parallel plan must retain its sampled frame");
        let snapshot = parallel_event_stream_snapshot
            .as_ref()
            .expect("parallel plan must retain its sampled event window");
        let event_area = inline_parallel_event_stream_area(frame_projection, viewport_area);
        let fallback_status_lines = frame_projection
            .parallel_live_stream()
            .map_or_else(Vec::new, |stream| stream.fallback_status_lines());
        let post_write_plan = inline_terminal.parallel_delivery.prepare_plan(
            snapshot,
            policy.render_mode,
            event_area,
            frame_projection.tui_language,
            &fallback_status_lines,
        );
        if post_write_plan.host_batch().is_some() {
            // A second durable prefix can be handled by the next bounded
            // transaction, but must not be mixed into this frame.
            defer_resize_redraw(runtime, inline_terminal);
            return Ok(InlineViewportSync::Deferred);
        }
        let (_, live_stream) = post_write_plan.into_parts();
        frame_projection.install_parallel_live_stream(live_stream);
        inserted
    } else {
        false
    };

    /*
     * HostScrollback mode writes only the transcript delta. Parallel events use
     * the typed host receipt above and never enter this rendered-line baseline.
     */
    let history_sync_result = if preserves_conversation_baseline {
        Ok(self::history_flush::HistoryFlushResult::preserved_baseline())
    } else {
        inline_terminal
            .history_flush
            .sync(terminal, &current_lines, resize_snapshot, insert_mode)
    };
    let history_sync = match history_sync_result {
        Ok(history_sync) => history_sync.with_handoff(conversation_handoff_delivery_token),
        Err(error) => {
            inline_terminal.history_flush.visible_history_rows = visible_history_rows_before;
            return Err(error);
        }
    };
    let handoff_acknowledged =
        acknowledge_transcript_handoff_after_delivery(runtime, history_sync.committed_handoff());
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
    let parallel_handoff_sync = if let Some(handoff) = parallel_handoff_conversation_lines.as_ref()
    {
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
            Ok(handoff_sync) => handoff_sync.with_handoff(Some(handoff.delivery_token.clone())),
            Err(error) => {
                inline_terminal.history_flush.visible_history_rows =
                    visible_history_rows_before_handoff;
                return Err(error);
            }
        };
        if handoff_sync.history_committed() {
            inline_terminal
                .history_flush
                .remember_conversation_projection(&handoff.lines);
        }
        let parallel_handoff_acknowledged = acknowledge_transcript_handoff_after_delivery(
            runtime,
            handoff_sync.committed_handoff(),
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
    let history_inserted = parallel_host_inserted
        || history_sync.inserted()
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
        runtime
            .capture_inline_conversation_frame_projection(viewport_area.width, &projection_sample)
    });
    let tail_frame_changed = inline_terminal.should_draw_inline_frame(
        &frame_projection,
        viewport_area.width,
        viewport_area.height,
    );
    Ok(InlineViewportSync::Stable {
        redraw_required: visible_history_adjusted || history_inserted || tail_frame_changed,
        frame_projection: Box::new(frame_projection),
        redraw_after_successful_frame,
        resize_snapshot,
    })
}

fn sync_parallel_host_delivery<B: InlineResizeBackend>(
    terminal: &mut Terminal<B>,
    inline_terminal: &mut InlineTerminalState,
    plan: &ParallelStreamDeliveryPlan,
    resize_snapshot: InlineResizeSnapshot,
    insert_mode: HistoryInsertionMode,
    language: TuiLanguage,
) -> Result<ParallelHostSync, B::Error> {
    let Some(batch) = plan.host_batch() else {
        return Ok(ParallelHostSync::Stable {
            inserted: false,
            stable_geometry: true,
        });
    };
    let lines = inline_terminal
        .history_flush
        .lines_with_reflow_guards(&batch.lines(language));
    let inserted_rows = count_rendered_history_rows(&lines, resize_snapshot.size.width)
        .min(usize::from(u16::MAX)) as u16;
    if inserted_rows == 0 {
        return Ok(ParallelHostSync::Deferred);
    }
    let token = match inline_terminal.parallel_delivery.begin_host_write(plan) {
        Ok(token) => token,
        Err(
            ParallelHostWriteStartError::DeliveryBlocked
            | ParallelHostWriteStartError::StalePlan
            | ParallelHostWriteStartError::StaleSurface,
        ) => return Ok(ParallelHostSync::Deferred),
    };
    let insertion = HistoryInsertionAdapter::new(insert_mode).attempt_parallel_insert_at_snapshot(
        terminal,
        &lines,
        inserted_rows,
        resize_snapshot,
    );
    match insertion {
        ParallelHistoryInsertionOutcome::AbortedBeforeWrite => {
            let transition = inline_terminal.parallel_delivery.abort_before_write(&token);
            debug_assert_eq!(transition, ParallelHostWriteTransition::Applied);
            Ok(ParallelHostSync::Deferred)
        }
        ParallelHistoryInsertionOutcome::FailedBeforeWrite(error) => {
            let transition = inline_terminal.parallel_delivery.abort_before_write(&token);
            debug_assert_eq!(transition, ParallelHostWriteTransition::Applied);
            Err(error)
        }
        ParallelHistoryInsertionOutcome::Committed { stable_geometry } => {
            let settlement = inline_terminal
                .parallel_delivery
                .commit_host_receipt(token.receipt());
            match settlement {
                ParallelHostReceiptSettlement::Applied => {
                    inline_terminal.history_flush.commit_parallel_insertion(
                        inserted_rows,
                        terminal.get_frame().area().top(),
                        stable_geometry,
                    );
                    Ok(ParallelHostSync::Stable {
                        inserted: true,
                        stable_geometry,
                    })
                }
                ParallelHostReceiptSettlement::Duplicate
                | ParallelHostReceiptSettlement::Rejected => {
                    let _ = inline_terminal.parallel_delivery.mark_uncertain(&token);
                    inline_terminal
                        .history_flush
                        .mark_visible_history_rows_dirty();
                    Ok(ParallelHostSync::Deferred)
                }
            }
        }
        ParallelHistoryInsertionOutcome::CommittedWithError(error) => {
            let settlement = inline_terminal
                .parallel_delivery
                .commit_host_receipt(token.receipt());
            if settlement == ParallelHostReceiptSettlement::Applied {
                inline_terminal.history_flush.commit_parallel_insertion(
                    inserted_rows,
                    terminal.get_frame().area().top(),
                    false,
                );
            } else {
                let _ = inline_terminal.parallel_delivery.mark_uncertain(&token);
                inline_terminal
                    .history_flush
                    .mark_visible_history_rows_dirty();
            }
            Err(error)
        }
        ParallelHistoryInsertionOutcome::Uncertain(error) => {
            let transition = inline_terminal.parallel_delivery.mark_uncertain(&token);
            debug_assert_eq!(transition, ParallelHostWriteTransition::Applied);
            inline_terminal
                .history_flush
                .mark_visible_history_rows_dirty();
            Err(error)
        }
    }
}

fn acknowledge_transcript_handoff_after_delivery(
    runtime: &mut ShellRuntime,
    delivery_token: Option<&TranscriptHandoffDeliveryToken>,
) -> bool {
    let Some(delivery_token) = delivery_token else {
        return false;
    };
    runtime.acknowledge_transcript_handoff_after_delivery(delivery_token)
}

#[cfg(test)]
fn parallel_conversation_handoff_projection(
    app: &NativeTuiApp,
    sample: &ConversationProjectionSample,
) -> Option<super::inline_frame_model::ParallelConversationHandoffProjection> {
    capture_parallel_conversation_handoff_projection(app, sample)
}

fn defer_resize_redraw(runtime: &mut ShellRuntime, inline_terminal: &mut InlineTerminalState) {
    fail_closed_frame_delivery(runtime, inline_terminal);
    runtime.request_resize_redraw_retry();
}

fn fail_closed_frame_delivery(
    runtime: &mut ShellRuntime,
    inline_terminal: &mut InlineTerminalState,
) {
    inline_terminal.invalidate_back_buffer();
    runtime.clear_queue_receipt_undo_hit_area();
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
    capture_inline_terminal_sync_projection(
        app,
        Rect::new(0, 0, 80, INLINE_VIEWPORT_HEIGHT),
        &sample,
    )
    .current_history_projection
    .unwrap_or_default()
}

#[derive(Default)]
pub(super) struct InlineTerminalState {
    viewport: TerminalViewportState,
    history_flush: HistoryFlushState,
    parallel_delivery: ParallelTerminalDeliveryState,
    frame_cache: FrameCacheState,
    last_conversation_history_identity_revision: u64,
    latest_frame_render_attempt: u64,
    last_committed_frame_render_attempt: Option<FrameRenderAttempt>,
}

impl InlineTerminalState {
    fn begin_frame_render_attempt(&mut self) -> FrameRenderAttempt {
        self.latest_frame_render_attempt = self
            .latest_frame_render_attempt
            .checked_add(1)
            .expect("frame render attempt generation exhausted");
        FrameRenderAttempt(self.latest_frame_render_attempt)
    }

    fn commit_frame_render_receipt(
        &mut self,
        pending: PendingInlineFrameRenderReceipt,
        commit_receipt: impl FnOnce(InlineFrameRenderReceipt) -> bool,
    ) -> bool {
        let current_attempt = FrameRenderAttempt(self.latest_frame_render_attempt);
        if pending.attempt != current_attempt
            || self
                .last_committed_frame_render_attempt
                .is_some_and(|last_committed| pending.attempt <= last_committed)
        {
            return false;
        }
        if !commit_receipt(pending.receipt) {
            return false;
        }
        self.last_committed_frame_render_attempt = Some(pending.attempt);
        true
    }

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

    fn observe_physical_resize_reflow(
        &mut self,
        terminal_size: Size,
        cursor_position: Position,
        has_host_scrollback_guard: bool,
    ) {
        if !has_host_scrollback_guard {
            self.viewport.resize_reflow_rows_to_clear = 0;
            return;
        }
        let observed_cursor_shift = self
            .viewport
            .last_known_cursor_pos
            .map_or(0, |previous| cursor_position.y.saturating_sub(previous.y));
        let cleanup_budget = self
            .frame_cache
            .resize_reflow_cleanup_budget(terminal_size.width);
        self.viewport.resize_reflow_rows_to_clear =
            cleanup_budget.min(observed_cursor_shift.max(INLINE_HOST_SCROLLBACK_REFLOW_GUARD_ROWS));
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
    fn resize_reflow_rows_to_clear(&self) -> u16 {
        self.viewport.resize_reflow_rows_to_clear
    }
    #[cfg(test)]
    fn insert_mode(&self) -> HistoryInsertionMode {
        self.viewport.insert_mode
    }
    #[cfg(test)]
    fn latest_frame_render_attempt(&self) -> u64 {
        self.latest_frame_render_attempt
    }
    #[cfg(test)]
    fn last_committed_frame_render_attempt(&self) -> Option<u64> {
        self.last_committed_frame_render_attempt
            .map(|attempt| attempt.0)
    }
}

// A trustworthy back buffer means the visible inline tail exactly matches the
// last frame we drew. Resize, focus reacquisition, scrollback insertion, and
// history-fit changes invalidate that trust and force a clear before the next draw.
struct TerminalViewportState {
    viewport_area: Option<Rect>,
    last_drawn_viewport_area: Option<Rect>,
    last_known_screen_size: Option<Size>,
    last_known_cursor_pos: Option<Position>,
    last_reconciled_resize_event_epoch: u64,
    last_reconciled_resize_observation_epoch: u64,
    last_observed_focus_reacquire_epoch: u64,
    resize_reflow_rows_to_clear: u16,
    back_buffer_trustworthy: bool,
    insert_mode: HistoryInsertionMode,
}

impl Default for TerminalViewportState {
    fn default() -> Self {
        Self {
            viewport_area: None,
            last_drawn_viewport_area: None,
            last_known_screen_size: None,
            last_known_cursor_pos: None,
            last_reconciled_resize_event_epoch: 0,
            last_reconciled_resize_observation_epoch: 0,
            last_observed_focus_reacquire_epoch: 0,
            resize_reflow_rows_to_clear: 0,
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
        self.last_drawn_viewport_area = Some(viewport_area);
        self.resize_reflow_rows_to_clear = 0;
        self.back_buffer_trustworthy = true;
    }
}

#[derive(Default)]
struct FrameCacheState {
    last_tail_frame: Option<InlineTailFrameSignature>,
}

impl FrameCacheState {
    fn resize_reflow_cleanup_budget(&self, next_terminal_width: u16) -> u16 {
        let Some(previous) = self.last_tail_frame.as_ref() else {
            return 0;
        };
        if next_terminal_width == 0 || next_terminal_width == previous.terminal_width {
            return 0;
        }
        if next_terminal_width > previous.terminal_width {
            // Expanding can unwrap one former soft row above the newly anchored
            // viewport even though no new wrapping is introduced.
            return INLINE_HOST_SCROLLBACK_REFLOW_GUARD_ROWS;
        }
        previous
            .lines
            .iter()
            .chain(previous.live_transcript_lines.iter())
            .chain(previous.parallel_live_stream_lines.iter())
            .fold(0u16, |total, line| {
                let line_width = line.width();
                let previous_rows = wrapped_terminal_rows(line_width, previous.terminal_width);
                let next_rows = wrapped_terminal_rows(line_width, next_terminal_width);
                total.saturating_add(next_rows.saturating_sub(previous_rows))
            })
            .max(INLINE_HOST_SCROLLBACK_REFLOW_GUARD_ROWS)
    }

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
            parallel_live_stream_lines: frame_projection
                .parallel_live_stream()
                .map_or_else(Vec::new, |stream| stream.render_lines()),
            renders_viewport_transcript_handoff: frame_projection
                .renders_viewport_transcript_handoff,
        };
        let should_draw = !viewport.back_buffer_trustworthy
            || self.last_tail_frame.as_ref() != Some(&next_signature);
        self.last_tail_frame = Some(next_signature);
        should_draw
    }
}

fn wrapped_terminal_rows(line_width: usize, terminal_width: u16) -> u16 {
    if line_width == 0 || terminal_width == 0 {
        return 1;
    }
    u16::try_from(line_width.div_ceil(usize::from(terminal_width))).unwrap_or(u16::MAX)
}

#[derive(Clone, PartialEq, Eq)]
struct InlineTailFrameSignature {
    core_revision: u64,
    terminal_width: u16,
    terminal_height: u16,
    lines: Vec<Line<'static>>,
    prompt_cursor_offset: Option<(u16, u16)>,
    live_transcript_lines: Vec<Line<'static>>,
    parallel_live_stream_lines: Vec<Line<'static>>,
    renders_viewport_transcript_handoff: bool,
}

#[cfg(test)]
#[path = "inline_terminal_adapter/tests.rs"]
mod tests;
