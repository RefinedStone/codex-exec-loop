/*
 * Shell core owns the DTO boundary between NativeTuiApp and shell presentation.
 * Production renderers often draw directly from the same projections, while
 * contract tests materialize these structs to assert copy, layout rectangles,
 * scroll offsets, and startup-state decisions without parsing terminal output.
 */
#[cfg(test)]
use super::Line;
#[cfg(test)]
use super::Rect;

// Recent-session capability is collapsed before entering shell copy so downstream helpers do not reread app state.
use super::capability_projection::recent_session_status_label;
use super::{
    ConversationState, ConversationViewModel, NativeTuiApp, ShellActionAvailability, StartupState,
};

#[cfg(test)]
/*
 * ConversationShellView captures "what the shell intends to draw" before
 * layout. Tests that only care about chrome text, transcript rows, footer
 * notices, or composer copy can use this lighter DTO without coupling to
 * terminal dimensions.
 */
pub(in super::super) struct ConversationShellView {
    // Outer frame chrome; intentionally stable across conversation lifecycle states.
    pub(in super::super) shell_title: Line<'static>,
    // Header lines expose lifecycle, thread identity, input state, and startup gate copy.
    pub(in super::super) header_lines: Vec<Line<'static>>,
    // Transcript/startup-inspection body lines after conversation projection but before viewport clipping.
    pub(in super::super) conversation_lines: Vec<Line<'static>>,
    // Footer panel title used by full shell renderings.
    pub(in super::super) status_title: Line<'static>,
    // Runtime notices, approvals, warnings, planning summaries, and live-agent tail copy in draw order.
    pub(in super::super) footer_lines: Vec<Line<'static>>,
    // Composer block title, including input state and submit affordance.
    pub(in super::super) input_title: Line<'static>,
    // Composer body lines: draft text, command palette prompt, attachment mode, or placeholder copy.
    pub(in super::super) input_lines: Vec<Line<'static>>,
}

#[cfg(test)]
/*
 * ConversationShellFrameView extends the copy DTO with concrete panel
 * rectangles. Viewport-sensitive contract tests use it to verify that header,
 * transcript, footer, and composer areas remain distinct as content heights
 * change.
 */
#[allow(dead_code)]
pub(in super::super) struct ConversationShellFrameView {
    // Full shell frame title paired with the frame-level block.
    pub(in super::super) shell_title: Line<'static>,
    // Header copy paired with header_area.
    pub(in super::super) header_lines: Vec<Line<'static>>,
    // Terminal area allocated to the header panel.
    pub(in super::super) header_area: Rect,
    // Transcript copy plus scroll decision for the inner transcript panel.
    pub(in super::super) transcript_view: TranscriptPanelView,
    // Terminal area allocated to transcript chrome and content.
    pub(in super::super) transcript_area: Rect,
    // Footer/status panel title.
    pub(in super::super) status_title: Line<'static>,
    // Footer/status lines in final presentation order.
    pub(in super::super) footer_lines: Vec<Line<'static>>,
    // Terminal area allocated to footer/status chrome and content.
    pub(in super::super) footer_area: Rect,
    // Composer panel title.
    pub(in super::super) input_title: Line<'static>,
    // Composer body lines used for height and cursor tests.
    pub(in super::super) input_lines: Vec<Line<'static>>,
    // Terminal area allocated to composer chrome and content.
    pub(in super::super) input_area: Rect,
}

#[cfg(test)]
/*
 * TranscriptPanelView keeps the rendered transcript rows and the scroll offset
 * together. A line snapshot alone cannot prove whether the renderer followed
 * the tail, clipped from the right inner width, or preserved a startup
 * inspection replacement.
 */
pub(in super::super) struct TranscriptPanelView {
    // Transcript title after shell-copy projection.
    pub(in super::super) title: Line<'static>,
    // Logical lines passed to the transcript paragraph.
    pub(in super::super) lines: Vec<Line<'static>>,
    // Vertical scroll offset handed to ratatui.
    pub(in super::super) scroll_offset: u16,
}

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
 * shell_copy, prompt_composer, startup_banner, transcript_copy, and status
 * panels. It narrows NativeTuiApp to fields that affect visible shell chrome
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
    /*
     * Debug-detail visibility is test-only because production drawing can ask
     * the app directly before rendering, while contract DTOs need the value
     * inside the frozen context they inspect.
     */
    #[cfg(test)]
    pub(super) planner_shows_debug_details: bool,
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
            recent_session_status_label: recent_session_status_label(app),
            // GitHub polling adapters remain outside presentation; this context carries their display label.
            github_review_polling_status_label: app.github_review_polling_status_label(),
            // Snapshot helpers need the same debug-detail decision as the renderer that formats transcript lines.
            #[cfg(test)]
            planner_shows_debug_details: app.planner_shows_debug_details(),
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
