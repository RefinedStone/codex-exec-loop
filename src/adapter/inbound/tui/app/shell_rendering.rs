use std::rc::Rc;

use super::shell_presentation::{build_inline_live_transcript_lines, build_inline_tail_view};
use super::*;

/*
 * This file is the ratatui frame boundary for the native inline shell. The
 * presentation layer builds Line-based read models, inline_layout decides how
 * the frame is split, and this module applies the layering order: base inline
 * conversation, optional inline inspection, then the exit-confirmation modal.
 */
#[path = "shell_rendering/inline_inspection.rs"]
mod inline_inspection;
#[path = "shell_rendering/inline_layout.rs"]
mod inline_layout;
#[path = "shell_rendering/popup_frame.rs"]
mod popup_frame;
#[path = "shell_rendering/popup_helpers.rs"]
mod popup_helpers;

#[cfg(test)]
use super::shell_presentation::build_planning_draft_editor_overlay_view;
use inline_inspection::draw_inline_shell_inspection;
#[cfg(test)]
use inline_layout::centered_rect;
use inline_layout::{
    build_inline_terminal_flow_layout, inline_body_render_area, render_inline_body,
    set_cursor_if_visible,
};
use popup_frame::draw_exit_confirmation;

pub(super) fn prepare_render_state(app: &mut NativeTuiApp, mode: ShellFrontendMode, area: Rect) {
    // Keep the prepare signature aligned with draw so future frontend modes can
    // specialize pre-render state without adding a second entrypoint.
    let _ = mode;
    // Manual editor overlays need a render-area-aware scroll sync. Other
    // overlays do not own a textarea cursor and should not mutate editor state
    // during a normal frame.
    let directions_editor_open = app.shell_overlay == ShellOverlay::DirectionsMaintenance
        && app.directions_maintenance_overlay_ui_state.step()
            == DirectionsMaintenanceOverlayStep::ManualEditor;
    let planning_editor_open = app.shell_overlay == ShellOverlay::PlanningInit
        && app.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::ManualEditor;
    if !directions_editor_open && !planning_editor_open {
        return;
    }
    // The editor lives inside the inspection area, whose height depends on the
    // current tail. Rebuild the same layout inputs draw will use and derive the
    // textarea viewport height from layout[0].
    let tail_view = build_inline_tail_view(app, area.width);
    let inspection_area = build_inline_terminal_flow_layout(app, area, &tail_view.lines)[0];
    // The editor chrome consumes fixed rows for title, tabs, validation/status,
    // and borders. Keep a small lower bound so tiny terminals still keep cursor
    // math well-defined.
    let editor_content_height = inspection_area
        .height
        .saturating_sub(14)
        .max(6)
        .saturating_sub(1)
        .max(1);
    app.planning_draft_editor_ui_state
        .sync_editor_scroll(editor_content_height);
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    // The current native shell has one renderer, but preserving mode here keeps
    // app runtime and shell frontend abstractions coupled through one boundary.
    let _ = mode;
    let frame_area = frame.area();
    // Tail view contains both status/prompt lines and cursor offset. The same
    // tail height also drives the inline inspection/body split.
    let tail_view = build_inline_tail_view(app, frame_area.width);
    let live_transcript_lines = build_inline_live_transcript_lines(app);
    let layout = build_inline_terminal_flow_layout(app, frame_area, &tail_view.lines);

    draw_inline_conversation_shell(frame, app, tail_view, live_transcript_lines, &layout);
    // Inline inspection is drawn after the base shell so overlays can replace
    // the top body while leaving the anchored prompt/status tail intact.
    if app.shell_overlay != ShellOverlay::Hidden {
        draw_inline_shell_inspection(frame, app, layout[0]);
    }
    // Exit confirmation is modal over every shell/overlay state and therefore
    // must be the final draw operation.
    if app.is_exit_confirmation_visible() {
        draw_exit_confirmation(frame);
    }
}

fn draw_inline_conversation_shell(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    tail_view: super::shell_presentation::InlineTailView,
    live_transcript_lines: Vec<Line<'static>>,
    layout: &Rc<[Rect]>,
) {
    // Always clear the complete frame first. Otherwise a narrower overlay or a
    // shorter tail can leave stale cells in the terminal buffer.
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    // The hidden-overlay path is the normal conversation shell. It bypasses the
    // inspection layout so the transcript can fill all space above the tail.
    if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
        // Some presentation states, such as startup banners, deliberately own
        // the full frame from the top and should not be bottom-anchored.
        if tail_view.render_from_top {
            render_inline_body(frame, frame_area, tail_view.lines, false);
            set_cursor_if_visible(frame, frame_area, tail_view.prompt_cursor_offset);
            return;
        }
        // In the standard shell, tail height is measured first, then live
        // transcript lines are clipped into the space above it.
        let tail_area = inline_body_render_area(frame_area, &tail_view.lines);
        render_inline_live_transcript(frame, frame_area, tail_area, live_transcript_lines);
        render_inline_body(frame, tail_area, tail_view.lines, false);
        set_cursor_if_visible(frame, tail_area, tail_view.prompt_cursor_offset);
        return;
    }
    // With any overlay/modal active, layout[0] is reserved for inspection and
    // layout[1] keeps the tail anchored below it. The exit modal is still drawn
    // outside this function so it can cover both regions.
    render_inline_body(
        frame,
        inline_body_render_area(layout[1], &tail_view.lines),
        tail_view.lines,
        false,
    );
}

fn render_inline_live_transcript(
    frame: &mut Frame<'_>,
    frame_area: Rect,
    tail_area: Rect,
    live_transcript_lines: Vec<Line<'static>>,
) {
    // No transcript lines, or no vertical space above the tail, means there is
    // nothing useful to render in the live region.
    if live_transcript_lines.is_empty() || tail_area.y <= frame_area.y {
        return;
    }
    // The live container is everything from frame top to the row before the
    // prompt tail. The inner render area is bottom-aligned so recent output sits
    // closest to the prompt.
    let live_container = Rect::new(
        frame_area.x,
        frame_area.y,
        frame_area.width,
        tail_area.y.saturating_sub(frame_area.y),
    );
    let live_area = inline_body_render_area(live_container, &live_transcript_lines);
    render_inline_body(frame, live_area, live_transcript_lines, false);
}

#[cfg(test)]
// Contract tests pin overlay layout, inline tail behavior, and viewport replay.
#[path = "shell_rendering_contract_tests.rs"]
mod contract_tests;
#[cfg(test)]
// Snapshot tests lock representative shell frames across runtime states.
#[path = "shell_rendering_tests.rs"]
mod tests;
