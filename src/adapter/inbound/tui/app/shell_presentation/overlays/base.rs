use super::super::{
    ConversationViewModel, Line, conversation_startup_screen_is_active, startup_ascii_art_lines,
};

/*
 * overlay/base.rs sits between shell state projection and concrete overlay rendering. Production
 * code uses the startup banner helper here; framed shell builders were removed with the legacy
 * popup renderer, so fullscreen inspection owns the remaining overlay layout contracts.
 */
#[derive(Clone, Copy)]
pub(crate) struct StartupBannerFrameInput<'a> {
    pub(crate) show_startup_ascii_art: bool,
    pub(crate) parallel_mode_enabled: bool,
    pub(crate) conversation: Option<&'a ConversationViewModel>,
}

pub(crate) fn build_startup_banner_lines(
    input: StartupBannerFrameInput<'_>,
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    /*
     * Startup art uses the same pure predicate as ConversationScreenModel without constructing a
     * second core snapshot or render clock during history synchronization. max_height is optional
     * because renderers sometimes ask for the natural logo and sometimes need a cropped variant.
     */
    if !input.show_startup_ascii_art
        || !conversation_startup_screen_is_active(input.parallel_mode_enabled, input.conversation)
    {
        return None;
    }
    let max_height = match max_height {
        Some(0) => return None,
        value => value,
    };

    Some(startup_ascii_art_lines(max_height))
}
