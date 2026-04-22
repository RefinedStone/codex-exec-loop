use super::super::prompt_composer::{build_prompt_cursor_offset, wrapped_row_count};
use super::super::{
    INLINE_TAIL_NOTICE_DETAIL_LIMIT, Line, NativeTuiApp, ShellConversationState,
    ShellCorePresentationContext,
};
use super::tail_copy::{
    build_inline_tail_lines_with_context, build_inline_tail_prompt_lines_with_context,
};

#[derive(Clone)]
pub(crate) struct InlineTailView {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) prompt_cursor_offset: Option<(u16, u16)>,
    pub(crate) render_from_top: bool,
}

pub(crate) fn build_inline_tail_view(app: &NativeTuiApp, content_width: u16) -> InlineTailView {
    let context = ShellCorePresentationContext::from_app(app);
    let lines = build_inline_tail_lines_with_context(
        app,
        &context,
        app.github_review_recent_changes_summary(INLINE_TAIL_NOTICE_DETAIL_LIMIT),
    );
    let prompt_cursor_offset =
        build_inline_prompt_cursor_offset_for_lines(app, &context, content_width, &lines);

    InlineTailView {
        lines,
        prompt_cursor_offset,
        render_from_top: context.startup_screen_is_active(),
    }
}

fn build_inline_prompt_cursor_offset_for_lines(
    app: &NativeTuiApp,
    context: &ShellCorePresentationContext<'_>,
    content_width: u16,
    tail_lines: &[Line<'static>],
) -> Option<(u16, u16)> {
    let ShellConversationState::Ready(conversation) = context.conversation_state else {
        return None;
    };
    let prompt_lines =
        build_inline_tail_prompt_lines_with_context(context, app.shell_action_availability());
    let prompt_start_index = tail_lines.len().saturating_sub(prompt_lines.len());
    let prompt_start_row = tail_lines[..prompt_start_index]
        .iter()
        .map(|line| wrapped_row_count(line.width(), content_width))
        .sum::<usize>()
        .try_into()
        .unwrap_or(u16::MAX);
    let (cursor_x, cursor_y) = build_prompt_cursor_offset(conversation, content_width)?;

    Some((cursor_x, prompt_start_row.saturating_add(cursor_y)))
}
