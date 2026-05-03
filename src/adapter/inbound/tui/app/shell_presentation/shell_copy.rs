/*
 * Shell copy is the final presentation mapping before ratatui widgets receive
 * Lines. The helpers here consume ShellCorePresentationContext instead of
 * NativeTuiApp so copy decisions stay tied to the shell projection boundary:
 * conversation lifecycle, input readiness, and startup action availability.
 */
use super::capability_copy::thread_history_loading_header_line;
use super::*;

pub(super) fn build_shell_header_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    /*
     * The header is the first lifecycle summary in the chrome. Loading and
     * failed states cannot safely expose ready conversation metadata, while
     * Ready can combine thread identity, composer state, and startup gates.
     */
    match context.conversation_state {
        ShellConversationState::Loading => vec![
            // Keep the shell brand visible while making the missing thread metadata explicit.
            Line::from(vec![
                Span::styled("Conversation Shell", AkraTheme::title()),
                Span::raw(" / loading thread"),
            ]),
            // Use the shared capability copy so header/footer loading language cannot drift.
            Line::from(thread_history_loading_header_line()),
        ],
        ShellConversationState::Ready(conversation) => vec![
            // Ready title binds the visible shell to the selected conversation/session title.
            Line::from(vec![
                Span::styled("Conversation Shell", AkraTheme::title()),
                Span::raw(" / "),
                Span::raw(conversation.title.clone()),
            ]),
            /*
             * The second ready line is a compact operational tuple:
             * thread identity, input-state affordance, and startup gate. This
             * gives the operator the same readiness model used by the input
             * title without requiring them to scan the footer.
             */
            Line::from(vec![
                Span::raw(format!(
                    "thread: {}  |  input: ",
                    if conversation.has_active_thread() {
                        conversation.thread_id.as_str()
                    } else {
                        /*
                         * Draft conversations are ready enough for startup
                         * actions, but may not have an app-server thread id
                         * until the first submit. The placeholder makes that
                         * state explicit instead of rendering an empty slot.
                         */
                        "not started yet"
                    }
                )),
                // Input state carries both copy and emphasis so armed/running/ready are distinguishable at a glance.
                Span::styled(
                    conversation.input_state.label(),
                    input_state_style(conversation.input_state),
                ),
                Span::raw("  |  startup: "),
                // Startup availability is a gate result, so the header uses success/warning/danger color semantics.
                Span::styled(
                    context.shell_action_availability.status_text(),
                    startup_state_style_for_availability(context.shell_action_availability),
                ),
            ]),
        ],
        ShellConversationState::Failed(message) => vec![
            // A failed conversation is the shell's primary state, so the brand line itself switches to danger styling.
            Line::from(vec![
                Span::styled("Conversation Shell", AkraTheme::danger()),
                Span::raw(" / failed"),
            ]),
            // Surface the load error in the header because transcript/footer content may be absent or off-screen.
            Line::from(message.to_string()),
        ],
    }
}

pub(super) fn build_shell_title() -> Line<'static> {
    /*
     * The outer frame title is global chrome. It stays free of thread/runtime
     * state so overlay and inline shell layouts share a stable navigation
     * affordance while stateful copy lives in header/footer/input titles.
     */
    Line::from("Shell / Ctrl+t new draft / Ctrl+C back / Ctrl+q quit")
}

pub(super) fn build_transcript_title_with_context(
    _context: &ShellCorePresentationContext<'_>,
) -> Line<'static> {
    /*
     * The current transcript title is state-invariant, but the context-shaped
     * signature preserves the presentation boundary for future inspection or
     * startup-specific titles without changing all renderer call sites.
     */
    Line::from("Transcript / live scrollback")
}

pub(in super::super) fn build_status_title() -> Line<'static> {
    /*
     * The footer panel mixes shortcut guidance with live runtime status. This
     * title names that combined role so contract tests can distinguish the
     * full shell footer from inline overlay renderings that intentionally
     * suppress panel chrome.
     */
    Line::from("Controls / shell shortcuts and live status")
}

pub(super) fn build_input_title_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Line<'static> {
    /*
     * The prompt title is the submit affordance. Loading/failed states keep
     * the composer region visually stable but mark it unavailable; Ready is
     * the only state where Enter and newline guidance can be truthful.
     */
    match context.conversation_state {
        ShellConversationState::Loading => {
            // Metadata is still resolving, so the prompt frame remains but does not advertise a submit target.
            Line::from(vec![Span::raw("Prompt"), Span::raw(" / loading")])
        }
        ShellConversationState::Failed(_) => {
            // A failed conversation has no valid runtime target for prompt submission.
            Line::from(vec![Span::raw("Prompt"), Span::raw(" / unavailable")])
        }
        ShellConversationState::Ready(conversation) => {
            /*
             * Submit copy is derived from the same action/runtime context as
             * the header, then paired with the input-state label so the
             * operator can see both "what the composer is" and "what Enter
             * will do" in one title.
             */
            let submit_hint = build_primary_submit_hint_with_context(context);
            Line::from(vec![
                Span::raw("Prompt"),
                Span::raw(" / "),
                Span::styled(
                    conversation.input_state.label(),
                    input_state_style(conversation.input_state),
                ),
                Span::raw(" / "),
                Span::raw(submit_hint),
                // Newline guidance stays adjacent to Enter guidance because both are composer keystroke contracts.
                Span::raw(" / Ctrl+j newline"),
            ])
        }
    }
}

pub(super) fn build_frontend_summary_line() -> Line<'static> {
    /*
     * This line is an operator-facing rendering-mode checksum. It documents
     * that the primary shell uses the inline main buffer, host terminal
     * scrollback, and a prompt-anchored tail; snapshot tests also rely on it
     * to catch accidental frontend-mode regressions.
     */
    Line::from(
        "frontend: inline main buffer  |  history: host terminal scrollback  |  tail: prompt anchored",
    )
}

fn build_primary_submit_hint_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> &'static str {
    /*
     * Submit hints are ordered by operator consequence. An already armed
     * startup submit is more specific than running-turn backpressure, which
     * is more specific than generic startup gate readiness.
     */
    match context.conversation_state {
        ShellConversationState::Ready(conversation) if conversation.startup_submit_armed => {
            "queued until ready"
        }
        // A running app-server turn means Enter cannot submit immediately even if the input buffer has text.
        ShellConversationState::Ready(conversation) if conversation.has_running_turn() => {
            "Enter send when idle"
        }
        // Startup/capability gates can block submission after the conversation model itself is ready.
        ShellConversationState::Ready(_) if !context.shell_action_availability.allows_actions() => {
            "Enter send when ready"
        }
        // Ready conversation, no running turn, and open action gates make Enter an immediate submit.
        ShellConversationState::Ready(_) => "Enter send",
        // Loading/failed callers omit this segment; returning empty copy keeps the helper total.
        _ => "",
    }
}

fn startup_state_style_for_availability(
    shell_action_availability: ShellActionAvailability,
) -> Style {
    /*
     * Header startup styling compresses action availability into the same
     * semantic colors used elsewhere in shell chrome: green for actionable,
     * yellow for waiting, and red for operator-blocking failures.
     */
    match shell_action_availability {
        ShellActionAvailability::Ready => AkraTheme::success(),
        ShellActionAvailability::Pending => AkraTheme::warning(),
        ShellActionAvailability::Blocked => AkraTheme::danger(),
    }
}
