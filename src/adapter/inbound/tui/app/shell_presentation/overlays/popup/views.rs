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
 * Supersession overlay flattens parallel-mode orchestration into independent panels.
 * Each section maps to a different operational concern: global mode, capability,
 * pool capacity, live roster, selected detail, distributor queue, and controls.
 */
pub(crate) struct SupersessionOverlayView {
    // Overlay title and current supersession mode.
    pub(crate) header_lines: Vec<Line<'static>>,
    // One-screen summary of orchestration state before detailed panels.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // Whether parallel controls are currently available and why.
    pub(crate) capability_lines: Vec<Line<'static>>,
    // Worker pool capacity and saturation summary.
    pub(crate) pool_lines: Vec<Line<'static>>,
    // Live worker/session roster rows.
    pub(crate) roster_lines: Vec<Line<'static>>,
    // Longer status, path, or reason text for the selected/focused worker.
    pub(crate) detail_lines: Vec<Line<'static>>,
    // Distributor backlog, assignment, and queue-head state.
    pub(crate) distributor_lines: Vec<Line<'static>>,
    // Available supersession controls for the current capability state.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Queue overlay projects PlanningRuntimeSnapshot into renderer sections. Accepted
 * queue rows, proposal candidates, and explanatory notes are intentionally separate
 * so the UI never blends committed work with suggested next work.
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
    // Queue overlay navigation and close hints.
    pub(crate) key_lines: Vec<Line<'static>>,
}

/*
 * Task intake overlay is the `:task` modal read model. It keeps the raw prompt,
 * generated preview, action status, and stage-specific keys separate so preview
 * confirmation can be rendered without knowing task-intake service internals.
 */
pub(crate) struct TaskIntakeOverlayView {
    // Fixed context that the operator is drafting a new ready task.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Raw operator prompt echo.
    pub(crate) prompt_lines: Vec<Line<'static>>,
    // Generated task preview before commit.
    pub(crate) preview_lines: Vec<Line<'static>>,
    // Editing, preview-ready, error, or accepted result copy.
    pub(crate) status_lines: Vec<Line<'static>>,
    // Prompt/preview-stage key hints.
    pub(crate) key_lines: Vec<Line<'static>>,
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
