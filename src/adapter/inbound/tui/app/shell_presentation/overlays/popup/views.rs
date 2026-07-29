/*
 * Popup view DTOs are the presentation boundary between overlay builders and
 * ratatui renderers. Builders translate domain/runtime state into `Line` sections
 * here; renderers should only decide layout, wrapping, focus, and scrolling.
 */
use super::super::super::Line;
// Session overlay is the one popup in this file that needs selection and scrolling
// metadata in addition to preformatted lines.
use super::super::OverlayListView;

/*
 * Startup overlay is the renderer-facing snapshot of boot diagnostics. Keeping the
 * sections separate lets popup and inline inspection draw the same readiness facts
 * without reinterpreting startup service state.
 */
pub(crate) struct StartupOverlayView {
    // Stable product/startup identity, kept separate from diagnostics so the popup title does not shift with probe results.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Workspace, attachment mode, and app-server summary rows shown before individual checks.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // Individual prerequisite rows, already reduced to operator-facing success/warning/failure copy.
    pub(crate) check_lines: Vec<Line<'static>>,
    // Non-blocking constraints or remediation hints that should not be mixed into prerequisite status.
    pub(crate) warning_lines: Vec<Line<'static>>,
    // Current startup affordances, rendered like a footer instead of normal diagnostic content.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Session overlay is the catalog read model. The controller owns cursor movement
 * and selected session state; this DTO gives the renderer list metadata plus the
 * detail/warning/footer sections that must stay stable as rows change.
 */
pub(crate) struct SessionOverlayView {
    // Browser title and catalog loading status.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Session rows plus selected index and scroll window.
    pub(crate) list_view: OverlayListView,
    // Long selected-session facts that would make row scanning too noisy.
    pub(crate) detail_lines: Vec<Line<'static>>,
    // Catalog-level warnings that are independent of the current selected row.
    pub(crate) warning_lines: Vec<Line<'static>>,
    // Open/new/cancel/navigation hints expected by the session controller.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Model selection overlay mirrors the Codex-style two-step picker: choose a
 * model first, then choose the reasoning effort that should be applied with it.
 */
pub(crate) struct ModelSelectionOverlayView {
    // Picker title and current override summary.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Model rows with selection styling already applied.
    pub(crate) model_lines: Vec<Line<'static>>,
    // Reasoning-effort rows with selection/staged context already applied.
    pub(crate) effort_lines: Vec<Line<'static>>,
    // Step-specific status copy that explains what Enter will do.
    pub(crate) status_lines: Vec<Line<'static>>,
    // Navigation and close hints for the current step.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Conversation view selection controls how much implementation transcript noise
 * remains visible while preserving user, Codex, and Codex Commentary messages.
 */
pub(crate) struct ViewSelectionOverlayView {
    // Picker title and invariant visibility summary.
    pub(crate) header_lines: Vec<Line<'static>>,
    // View rows with current and selected mode styling already applied.
    pub(crate) mode_lines: Vec<Line<'static>>,
    // Current mode and Enter behavior.
    pub(crate) status_lines: Vec<Line<'static>>,
    // Navigation and close hints.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Language selection controls the local TUI copy layer. Runtime payloads and
 * user-authored text are intentionally not translated by this adapter.
 */
pub(crate) struct LanguageSelectionOverlayView {
    // Picker title and localization boundary summary.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Language rows with current and selected language styling already applied.
    pub(crate) language_lines: Vec<Line<'static>>,
    // Current language and non-translated content note.
    pub(crate) status_lines: Vec<Line<'static>>,
    // Navigation and close hints.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * The parallel operations board keeps lifecycle facts in commercial operator
 * groups: global state, accepted queue, stable slot lanes, selected delivery
 * gates, and the append-only event stream.
 */
pub(crate) struct SupersessionOverlayView {
    // Explicit :parallel inspection owns the full viewport; the passive parallel
    // home keeps the composer available below the same read-only projection.
    pub(crate) focused_full_viewport: bool,
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) overview_lines: Vec<Line<'static>>,
    pub(crate) accepted_queue_lines: Vec<Line<'static>>,
    pub(crate) timeline_lines: Vec<Line<'static>>,
    pub(crate) lane_lines: Vec<Line<'static>>,
    pub(crate) compact_lane_lines: Vec<Line<'static>>,
    pub(crate) selected_lane_lines: Vec<Line<'static>>,
    pub(crate) compact_selected_lane_lines: Vec<Line<'static>>,
    // Append-only runtime events remain a distinct stream so panel chrome never
    // enters durable host scrollback.
    pub(crate) event_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Parallel peek is a read-only drill-in over active parallel agents. It is kept
 * separate from the supervisor board because it temporarily shows a worker
 * conversation without switching the main shell thread.
 */
pub(crate) struct ParallelPeekOverlayView {
    // Picker or detail title plus current mode copy.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Active agent rows, with selection style already encoded as text.
    pub(crate) agent_lines: Vec<Line<'static>>,
    // Read-only conversation preview for the selected agent.
    pub(crate) conversation_lines: Vec<Line<'static>>,
    // Loading, empty, and snapshot status lines.
    pub(crate) status_lines: Vec<Line<'static>>,
    // Navigation and close hints for the current step.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Queue overlay projects the planning application read model into renderer sections.
 * Accepted queue rows, proposal candidates, and explanatory notes are intentionally
 * separate so the UI never blends committed work with suggested next work.
 */
pub(crate) struct QueueOverlayView {
    // Queue overlay title and conversation/runtime context.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Snapshot health, accepted revision, idle policy, and other global facts.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // Accepted queue rows in display order.
    pub(crate) queue_lines: Vec<Line<'static>>,
    // Proposed follow-up work that is not yet accepted into the queue.
    pub(crate) proposal_lines: Vec<Line<'static>>,
    // Empty/invalid/blocked explanations that tell the operator why rows may be absent.
    pub(crate) note_lines: Vec<Line<'static>>,
    // Index in the inline renderer's merged queue/proposal/note content.
    pub(crate) selected_content_line_index: Option<usize>,
    // Queue overlay navigation and close hints.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Review center overlay keeps the current thread, inbox, and recent history as
 * separate read-only panels. Each review row carries one headline plus a few
 * supporting detail lines so popup and inline renderers can reuse the same
 * compact projection without reformatting repository payloads.
 */
pub(crate) struct ReviewOverlayView {
    // One-line review headline used for fast scanning.
    pub(crate) summary_line: Line<'static>,
    // Optional timestamps, thread ids, or handoff detail shown under the headline.
    pub(crate) detail_lines: Vec<Line<'static>>,
}

pub(crate) struct ReviewsOverlayView {
    // Overlay title and workspace-level framing.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Workspace, active thread, and aggregate inbox/history facts.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // Stored review rows for the currently active thread.
    pub(crate) current_thread_reviews: Vec<ReviewOverlayView>,
    // Pending inbox rows that still need operator attention.
    pub(crate) inbox_reviews: Vec<ReviewOverlayView>,
    // Recent history rows that show the last review outcomes/events.
    pub(crate) history_reviews: Vec<ReviewOverlayView>,
    // Read-only navigation and close hints.
    pub(crate) key_lines: Vec<Line<'static>>,
}

impl ReviewsOverlayView {
    pub(in crate::adapter::inbound::tui::app) fn current_thread_section_lines(
        &self,
    ) -> Vec<Line<'static>> {
        flatten_review_lines(
            &self.current_thread_reviews,
            "No active thread review context.",
        )
    }

    pub(in crate::adapter::inbound::tui::app) fn inbox_section_lines(&self) -> Vec<Line<'static>> {
        flatten_review_lines(&self.inbox_reviews, "No pending inbox items.")
    }

    pub(in crate::adapter::inbound::tui::app) fn history_section_lines(
        &self,
    ) -> Vec<Line<'static>> {
        flatten_review_lines(&self.history_reviews, "No recent review history.")
    }
}

fn flatten_review_lines(entries: &[ReviewOverlayView], empty_message: &str) -> Vec<Line<'static>> {
    if entries.is_empty() {
        return vec![Line::from(empty_message.to_string())];
    }

    let mut lines = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        lines.push(entry.summary_line.clone());
        lines.extend(entry.detail_lines.iter().cloned());
        if index + 1 != entries.len() {
            lines.push(Line::from(""));
        }
    }
    lines
}

/*
 * Planning init overlay is the shared setup modal shape. Selection, existing
 * workspace review, manual editor entry, and simple review all collapse to this
 * DTO so the renderer can keep one section layout across setup modes.
 */
pub(crate) struct PlanningInitOverlayView {
    // Setup title and current mode.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Workspace path, existing-state summary, and high-level queue/failure context.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // Selectable options or review rows, depending on setup mode.
    pub(crate) option_lines: Vec<Line<'static>>,
    // Validation, generation, repair, or selected-option feedback.
    pub(crate) status_lines: Vec<Line<'static>>,
    // Confirm/edit/cancel hints for the current setup mode.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Planning draft editor view is shared by popup and inline inspection renderers.
 * It carries document selection, editor text, scroll/cursor coordinates, validation
 * status, and command hints as one already-projected surface.
 */
pub(crate) struct PlanningDraftEditorOverlayView {
    // Editor title, session label, and dirty/confirmation context.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Staged/detail/generated file list and current file selection copy.
    pub(crate) file_lines: Vec<Line<'static>>,
    // Active document label used by renderers as a pane title.
    pub(crate) editor_title: String,
    // Current buffer content as syntax-neutral display lines.
    pub(crate) editor_lines: Vec<Line<'static>>,
    // Vertical editor scroll offset shared by popup and inline renderers.
    pub(crate) editor_scroll: u16,
    // Visible cursor offset; None means read-only/status-only surfaces should not draw a cursor.
    pub(crate) editor_cursor_offset: Option<(u16, u16)>,
    // Validation, save, or close-confirmation feedback that should not displace editor text.
    pub(crate) status_lines: Vec<Line<'static>>,
    // Editing/review/close-confirm command hints.
    pub(crate) key_lines: Vec<Line<'static>>,
}
