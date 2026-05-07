use super::super::{Line, NativeTuiApp, ShellCorePresentationContext, startup_ascii_art_lines};

/*
 * overlay/base.rs sits between shell state projection and concrete overlay rendering. Production
 * code uses the startup banner helper here; framed shell builders were removed with the legacy
 * popup renderer, so inline inspection owns the remaining overlay layout contracts.
 */
pub(crate) fn build_startup_banner_lines(
    app: &NativeTuiApp,
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    /*
     * Startup art is gated by ShellCorePresentationContext rather than raw app flags so inline
     * terminal, popup overlays, and startup inspection all agree on when the banner is transiently
     * visible. max_height is optional because renderers sometimes ask for the natural logo and
     * sometimes need a cropped variant for narrow overlay areas.
     */
    let context = ShellCorePresentationContext::from_app(app);
    if !context.startup_banner_is_active() {
        return None;
    }
    let max_height = match max_height {
        Some(0) => return None,
        value => value,
    };

    Some(startup_ascii_art_lines(max_height))
}
