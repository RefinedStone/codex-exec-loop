use super::super::{
    ConversationState, Line, NativeTuiApp, conversation_startup_screen_is_active,
    startup_ascii_art_lines,
};

/*
 * overlay/base.rs sits between shell state projection and concrete overlay rendering. Production
 * code uses the startup banner helper here; framed shell builders were removed with the legacy
 * popup renderer, so inline inspection owns the remaining overlay layout contracts.
 */
pub(crate) fn build_startup_banner_lines(
    app: &NativeTuiApp,
    parallel_mode_enabled: bool,
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    /*
     * Startup art uses the same pure predicate as ConversationScreenModel without constructing a
     * second core snapshot or render clock during history synchronization. max_height is optional
     * because renderers sometimes ask for the natural logo and sometimes need a cropped variant.
     */
    let conversation = match &app.conversation.lifecycle.conversation_state {
        ConversationState::Ready(conversation) => Some(conversation.as_ref()),
        ConversationState::Loading | ConversationState::Failed(_) => None,
    };
    if !app.shell.show_startup_ascii_art
        || !conversation_startup_screen_is_active(parallel_mode_enabled, conversation)
    {
        return None;
    }
    let max_height = match max_height {
        Some(0) => return None,
        value => value,
    };

    Some(startup_ascii_art_lines(max_height))
}
