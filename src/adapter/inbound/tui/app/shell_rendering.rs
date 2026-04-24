use std::rc::Rc;

use super::shell_presentation::{build_inline_live_transcript_lines, build_inline_tail_view};
use super::*;

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
    let _ = mode;
    let directions_editor_open = app.shell_overlay == ShellOverlay::DirectionsMaintenance
        && app.directions_maintenance_overlay_ui_state.step()
            == DirectionsMaintenanceOverlayStep::ManualEditor;
    let planning_editor_open = app.shell_overlay == ShellOverlay::PlanningInit
        && app.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::ManualEditor;
    if !directions_editor_open && !planning_editor_open {
        return;
    }

    let tail_view = build_inline_tail_view(app, area.width);
    let inspection_area = build_inline_terminal_flow_layout(app, area, &tail_view.lines)[0];
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
    let _ = mode;
    let frame_area = frame.area();
    let tail_view = build_inline_tail_view(app, frame_area.width);
    let live_transcript_lines = build_inline_live_transcript_lines(app);
    let layout = build_inline_terminal_flow_layout(app, frame_area, &tail_view.lines);

    draw_inline_conversation_shell(frame, app, tail_view, live_transcript_lines, &layout);

    if app.shell_overlay != ShellOverlay::Hidden {
        draw_inline_shell_inspection(frame, app, layout[0]);
    }

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
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
        if tail_view.render_from_top {
            render_inline_body(frame, frame_area, tail_view.lines, false);
            set_cursor_if_visible(frame, frame_area, tail_view.prompt_cursor_offset);
            return;
        }

        let tail_area = inline_body_render_area(frame_area, &tail_view.lines);
        render_inline_live_transcript(frame, frame_area, tail_area, live_transcript_lines);
        render_inline_body(frame, tail_area, tail_view.lines, false);
        set_cursor_if_visible(frame, tail_area, tail_view.prompt_cursor_offset);
        return;
    }

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
    if live_transcript_lines.is_empty() || tail_area.y <= frame_area.y {
        return;
    }

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
#[path = "shell_rendering_contract_tests.rs"]
mod contract_tests;
#[cfg(test)]
#[path = "shell_rendering_tests.rs"]
mod tests;
