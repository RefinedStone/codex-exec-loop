use super::super::{
    ConversationViewModel, Line, conversation_startup_screen_is_active,
    startup_operator_ledger_lines,
};

/*
 * overlay/base.rs sits between shell state projection and concrete overlay rendering. Production
 * code uses the startup banner helper here; framed shell builders were removed with the legacy
 * popup renderer, so fullscreen inspection owns the remaining overlay layout contracts.
 */
#[derive(Clone, Copy)]
pub(crate) struct StartupBannerFrameInput<'a> {
    pub(crate) show_startup_visual: bool,
    pub(crate) parallel_mode_enabled: bool,
    pub(crate) conversation: Option<&'a ConversationViewModel>,
}

pub(crate) fn build_startup_banner_lines(
    input: StartupBannerFrameInput<'_>,
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    /*
     * The startup ledger uses the same pure predicate as ConversationScreenModel without
     * constructing a second core snapshot or render clock during history synchronization.
     * max_height is optional because short inspection callers may request a compact projection.
     */
    if !input.show_startup_visual
        || !conversation_startup_screen_is_active(input.parallel_mode_enabled, input.conversation)
    {
        return None;
    }
    let max_height = match max_height {
        Some(0) => return None,
        value => value,
    };

    Some(startup_operator_ledger_lines(max_height))
}
