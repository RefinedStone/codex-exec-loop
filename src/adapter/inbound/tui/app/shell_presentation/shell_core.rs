/*
 * Shell core owns the DTO boundary between NativeTuiApp and shell presentation.
 * Production renderers draw from the same immutable projection so startup,
 * prompt, transcript, and status helpers do not reread app state independently.
 */

// Recent-session capability is collapsed before entering shell copy so downstream helpers do not reread app state.
use super::capability_projection::recent_session_status_label;
use super::{
    ConversationState, ConversationViewModel, NativeTuiApp, ShellActionAvailability, StartupState,
    TuiLanguage,
};

#[derive(Clone, Copy)]
/*
 * Shell copy and status panels need only the conversation lifecycle shape, not
 * the whole ConversationState enum. This reference projection keeps loading
 * and failed paths cheap while allowing Ready renderers to borrow the
 * ConversationViewModel without cloning transcript/cache data.
 */
pub(super) enum ShellConversationState<'a> {
    Loading,
    Failed(&'a str),
    Ready(&'a ConversationViewModel),
}

/*
 * ShellCorePresentationContext is the shared read-only projection used by
 * prompt_composer, startup_banner, transcript_copy, and status panels. It narrows NativeTuiApp to fields that affect visible shell chrome
 * so those modules stay presentation-focused rather than reaching back into
 * runtime ownership and mutation APIs.
 */
pub(super) struct ShellCorePresentationContext<'a> {
    // Feature/runtime flag deciding whether startup mode may render the ASCII banner.
    pub(super) show_startup_ascii_art: bool,
    // Startup overlay/source state used by banner, prompt, and recovery-anchor copy.
    pub(super) startup_state: &'a StartupState,
    // Current action gate collapsed into the shell-level availability enum.
    pub(super) shell_action_availability: ShellActionAvailability,
    // Session-loading capability already projected to a short label for header/footer reuse.
    pub(super) recent_session_status_label: String,
    // GitHub review polling state projected to display copy before entering footer builders.
    pub(super) github_review_polling_status_label: String,
    // Current TUI copy language, projected once so startup chrome does not reread app state.
    pub(super) tui_language: TuiLanguage,
    // Parallel mode replaces the empty draft startup screen with the supervisor board.
    pub(super) parallel_mode_enabled: bool,
    // Loading/failed/ready conversation projection used consistently by shell copy and transcript/status helpers.
    pub(super) conversation_state: ShellConversationState<'a>,
}

impl<'a> ShellCorePresentationContext<'a> {
    pub(super) fn from_app(app: &'a NativeTuiApp) -> Self {
        /*
         * NativeTuiApp owns runtime services, overlay state, input buffers,
         * planning controllers, and session catalogs. This constructor is the
         * intentional choke point that extracts only the immutable shell
         * presentation facts needed for one render/test snapshot.
         */
        Self {
            // Startup banner eligibility begins with the app-level feature flag.
            show_startup_ascii_art: app.show_startup_ascii_art,
            // Startup state is shared by prompt composer, startup inspection, and attachment-mode copy.
            startup_state: &app.startup_state,
            // Availability calculation stays encapsulated on NativeTuiApp; presentation sees only the result.
            shell_action_availability: app.shell_action_availability(),
            // Capability projection hides session-loader internals from shell copy/status panels.
            recent_session_status_label: recent_session_status_label(app, app.tui_language),
            // GitHub polling adapters remain outside presentation; this context carries their display label.
            github_review_polling_status_label: app.github_review_polling_status_label(),
            tui_language: app.tui_language,
            parallel_mode_enabled: app.parallel_mode_enabled(),
            /*
             * Ready borrows the view model because downstream presentation
             * code reads cached lines, input state, live activity, and planning
             * snapshots without mutating conversation state.
             */
            conversation_state: match &app.conversation_state {
                ConversationState::Loading => ShellConversationState::Loading,
                ConversationState::Failed(message) => ShellConversationState::Failed(message),
                ConversationState::Ready(conversation) => {
                    ShellConversationState::Ready(conversation)
                }
            },
        }
    }

    pub(super) fn ready_conversation(&self) -> Option<&'a ConversationViewModel> {
        /*
         * Ready-only copy builders use this helper to opt into conversation
         * details while keeping their loading/failed behavior explicit through
         * Option rather than panicking or synthesizing placeholder models.
         */
        match self.conversation_state {
            ShellConversationState::Ready(conversation) => Some(conversation),
            _ => None,
        }
    }

    pub(super) fn startup_screen_is_active(&self) -> bool {
        /*
         * Startup screen is a Ready conversation with no durable app-server
         * thread, no history, and no live turn output. Loading/failed states
         * have their own placeholders and must not be treated as startup just
         * because they also lack transcript lines.
         */
        if self.parallel_mode_enabled {
            return false;
        }
        let Some(conversation) = self.ready_conversation() else {
            return false;
        };

        /*
         * Each guard removes one reason to keep normal transcript rendering:
         * an active thread id, stored history, a running turn, or a live agent
         * message means the shell has real conversation content to preserve.
         */
        !conversation.has_active_thread()
            && conversation.messages.is_empty()
            && conversation.active_turn_id.is_none()
            && conversation.live_agent_message.is_none()
    }

    pub(super) fn startup_banner_is_active(&self) -> bool {
        /*
         * Banner visibility is stricter than startup-screen visibility. The
         * app may keep startup inspection active while suppressing ASCII art,
         * so renderers check this combined predicate only for the banner.
         */
        self.show_startup_ascii_art && self.startup_screen_is_active()
    }
}
