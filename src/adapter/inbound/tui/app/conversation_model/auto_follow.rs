use std::time::Instant;

use super::{
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD,
    INFINITE_AUTO_FOLLOW_MAX_TURNS, INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN,
};

const AUTO_FOLLOW_MODE_LABEL: &str = "planning queue";

#[path = "auto_follow_decision.rs"]
mod decision;
pub(crate) use decision::{AutoFollowupDecision, AutoFollowupSkipReason};

/*
 * AutoFollowState is the conversation-local control plane for internal
 * continuation. Post-turn execution asks it whether another planning-queue turn
 * can be queued, conversation_runtime mutates its phase as the prompt is queued
 * and streamed, and tail/footer copy reads the same state for operator feedback.
 */
#[derive(Debug, Clone)]
pub(crate) struct AutoFollowState {
    /*
     * Operator pause is separate from runtime_phase. A pause should stop the next
     * continuation decision but must not erase an already-submitted turn or the
     * completed-turn budget counter.
     */
    post_turn_continuation_paused: bool,
    // Number of auto-follow turns that reached stream completion in this chain.
    pub(crate) completed_auto_turns: usize,
    // Budget for this automatic chain; usize::MAX is displayed as the infinite token.
    pub(crate) max_auto_turns: usize,
    // Live lifecycle phase used by runtime status/tail copy.
    pub(crate) runtime_phase: AutoFollowRuntimePhase,
    // User-visible guards that can stop auto-follow before queuing another turn.
    pub(crate) stop_rules: AutoFollowStopRules,
}

/*
 * Phase tracks work after a normal turn completes. Evaluating is post-turn
 * planning inspection, Queued is a prompt accepted for submission, Submitting is
 * the conversation runtime handing it to the stream worker, and Running means the
 * app-server stream has actually started.
 */
#[derive(Debug, Clone)]
pub(crate) enum AutoFollowRuntimePhase {
    Idle,
    Evaluating {
        started_at: Instant,
    },
    Queued {
        started_at: Instant,
        turn_index: usize,
    },
    Submitting {
        started_at: Instant,
        turn_index: usize,
    },
    Running {
        started_at: Instant,
        turn_index: usize,
    },
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AutoFollowStopRules {
    // Keyword rule scans the last assistant text before post-turn code queues more work.
    pub(crate) stop_keyword: StopKeywordRule,
    // File-change rule lets planning queue mode stop when a turn did not mutate planning files.
    pub(crate) stop_on_no_file_changes: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StopKeywordRule {
    pub(crate) enabled: bool,
    // Stored normalized keyword; matching is token-based and ASCII case-insensitive.
    pub(crate) value: String,
}

impl AutoFollowState {
    pub(crate) fn new() -> Self {
        Self {
            post_turn_continuation_paused: false,
            completed_auto_turns: 0,
            max_auto_turns: DEFAULT_AUTO_FOLLOW_MAX_TURNS,
            runtime_phase: AutoFollowRuntimePhase::Idle,
            stop_rules: AutoFollowStopRules::default(),
        }
    }

    pub(crate) fn mode_label(&self) -> &'static str {
        AUTO_FOLLOW_MODE_LABEL
    }

    // Compact progress used by prompt/tail copy while the chain is active.
    pub(crate) fn progress_label(&self) -> String {
        format!(
            "{}/{}",
            self.completed_auto_turns,
            self.max_auto_turns_label()
        )
    }

    #[cfg(test)]
    pub(crate) fn compact_completed_progress_label(&self) -> String {
        format!(
            "{}/{} done",
            self.completed_auto_turns,
            self.max_auto_turns_label()
        )
    }

    #[cfg(test)]
    pub(crate) fn max_auto_turns_value(&self) -> usize {
        self.max_auto_turns
    }

    pub(crate) fn max_auto_turns_label(&self) -> String {
        format_max_auto_turns(self.max_auto_turns)
    }

    pub(crate) fn stop_keyword_value(&self) -> &str {
        self.stop_rules.stop_keyword.value()
    }

    // The next turn is one-based so user-facing copy matches "turn N/M".
    pub(crate) fn next_auto_turn_index(&self) -> usize {
        self.completed_auto_turns + 1
    }

    pub(crate) fn active_turn_index(&self) -> Option<usize> {
        self.runtime_phase.turn_index()
    }

    pub(crate) fn active_started_at(&self) -> Option<Instant> {
        self.runtime_phase.started_at()
    }

    pub(crate) fn has_live_activity(&self) -> bool {
        !matches!(self.runtime_phase, AutoFollowRuntimePhase::Idle)
    }

    // Human-facing phase summary shared by status panels and runtime diagnostics.
    pub(crate) fn activity_label(&self) -> String {
        let max_auto_turns = self.max_auto_turns_label();
        match &self.runtime_phase {
            AutoFollowRuntimePhase::Idle => "idle".to_string(),
            AutoFollowRuntimePhase::Evaluating { .. } => "evaluating next turn".to_string(),
            AutoFollowRuntimePhase::Queued { turn_index, .. } => {
                format!("queued turn {turn_index}/{max_auto_turns}")
            }
            AutoFollowRuntimePhase::Submitting { turn_index, .. } => {
                format!("submitting turn {turn_index}/{max_auto_turns}")
            }
            AutoFollowRuntimePhase::Running { turn_index, .. } => {
                format!("running turn {turn_index}/{max_auto_turns}")
            }
        }
    }

    // Budget and operator pause are checked before post-turn evaluation queues a prompt.
    pub(crate) fn can_queue_next(&self) -> bool {
        !self.post_turn_continuation_paused && self.completed_auto_turns < self.max_auto_turns
    }

    /*
     * Manual turns start a new operator-directed chain. They clear pause and
     * progress so auto-follow state from the previous chain cannot suppress or
     * relabel the user's explicit request.
     */
    pub(crate) fn reset_for_manual_turn(&mut self) {
        self.completed_auto_turns = 0;
        self.runtime_phase = AutoFollowRuntimePhase::Idle;
        self.post_turn_continuation_paused = false;
    }

    // Post-turn execution is now inspecting planning/runtime state for a follow-up prompt.
    pub(crate) fn begin_post_turn_evaluation(&mut self) {
        self.runtime_phase = AutoFollowRuntimePhase::Evaluating {
            started_at: Instant::now(),
        };
    }

    // Queue preserves the chosen turn index before async submission can start.
    pub(crate) fn mark_auto_turn_queued(&mut self) -> usize {
        let turn_index = self.next_auto_turn_index();
        self.runtime_phase = AutoFollowRuntimePhase::Queued {
            started_at: Instant::now(),
            turn_index,
        };
        turn_index
    }

    /*
     * Submission may be called after Queued or directly from recovery paths.
     * Reusing the active index keeps the displayed N/M stable across the queue
     * and submit phases.
     */
    pub(crate) fn mark_auto_turn_submitted(&mut self) -> usize {
        let turn_index = self
            .active_turn_index()
            .unwrap_or_else(|| self.next_auto_turn_index());
        self.runtime_phase = AutoFollowRuntimePhase::Submitting {
            started_at: Instant::now(),
            turn_index,
        };
        turn_index
    }

    // Only a queued/submitted auto turn can become Running; stale starts are ignored.
    pub(crate) fn mark_auto_turn_started(&mut self) -> Option<usize> {
        let turn_index = match &self.runtime_phase {
            AutoFollowRuntimePhase::Queued { turn_index, .. }
            | AutoFollowRuntimePhase::Submitting { turn_index, .. } => *turn_index,
            AutoFollowRuntimePhase::Idle
            | AutoFollowRuntimePhase::Evaluating { .. }
            | AutoFollowRuntimePhase::Running { .. } => return None,
        };
        self.runtime_phase = AutoFollowRuntimePhase::Running {
            started_at: Instant::now(),
            turn_index,
        };
        Some(turn_index)
    }

    /*
     * A completed submitting/running phase consumes budget. Evaluation or queued
     * phases that are cancelled/ignored return to idle without counting as a turn.
     */
    pub(crate) fn complete_auto_turn_if_running(&mut self) -> bool {
        match self.runtime_phase {
            AutoFollowRuntimePhase::Submitting { .. } | AutoFollowRuntimePhase::Running { .. } => {
                self.completed_auto_turns += 1;
                self.runtime_phase = AutoFollowRuntimePhase::Idle;
                true
            }
            AutoFollowRuntimePhase::Idle
            | AutoFollowRuntimePhase::Evaluating { .. }
            | AutoFollowRuntimePhase::Queued { .. } => {
                self.runtime_phase = AutoFollowRuntimePhase::Idle;
                false
            }
        }
    }

    pub(crate) fn clear_runtime_phase(&mut self) {
        self.runtime_phase = AutoFollowRuntimePhase::Idle;
    }

    pub(crate) fn pause_post_turn_continuation(&mut self) {
        self.post_turn_continuation_paused = true;
    }

    pub(crate) fn clear_post_turn_continuation_pause(&mut self) {
        self.post_turn_continuation_paused = false;
    }

    pub(crate) fn post_turn_continuation_paused(&self) -> bool {
        self.post_turn_continuation_paused
    }

    pub(crate) fn set_max_auto_turns(&mut self, value: usize) {
        self.max_auto_turns = value;
    }

    #[cfg(test)]
    pub(crate) fn set_stop_keyword_value(&mut self, value: String) {
        self.stop_rules.stop_keyword.set_value(value);
    }

    /*
     * The overlay editor accepts free text while typing, then commits through
     * this parser so UI validation and runtime budget semantics stay identical.
     */
    pub(crate) fn normalize_max_auto_turns_candidate(candidate: &str) -> Option<usize> {
        let normalized = candidate.trim();
        if normalized.eq_ignore_ascii_case(INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN) {
            return Some(INFINITE_AUTO_FOLLOW_MAX_TURNS);
        }
        let value = normalized.parse::<usize>().ok()?;
        if value == 0 { None } else { Some(value) }
    }
}

fn format_max_auto_turns(value: usize) -> String {
    if value == INFINITE_AUTO_FOLLOW_MAX_TURNS {
        INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN.to_string()
    } else {
        value.to_string()
    }
}

impl Default for StopKeywordRule {
    fn default() -> Self {
        Self {
            enabled: true,
            value: DEFAULT_AUTO_FOLLOW_STOP_KEYWORD.to_string(),
        }
    }
}

impl AutoFollowRuntimePhase {
    fn turn_index(&self) -> Option<usize> {
        match self {
            Self::Queued { turn_index, .. }
            | Self::Submitting { turn_index, .. }
            | Self::Running { turn_index, .. } => Some(*turn_index),
            Self::Idle | Self::Evaluating { .. } => None,
        }
    }

    fn started_at(&self) -> Option<Instant> {
        match self {
            Self::Evaluating { started_at }
            | Self::Queued { started_at, .. }
            | Self::Submitting { started_at, .. }
            | Self::Running { started_at, .. } => Some(*started_at),
            Self::Idle => None,
        }
    }
}

impl AutoFollowStopRules {
    // The no-file-change stop rule only applies after the stream reducer knows the turn artifact count.
    pub(crate) fn should_stop_on_no_file_changes(&self, file_change_count: usize) -> bool {
        self.stop_on_no_file_changes && file_change_count == 0
    }
}

impl StopKeywordRule {
    #[cfg(test)]
    pub(crate) fn normalize_candidate(candidate: &str) -> Option<String> {
        let normalized = candidate.trim();
        if normalized.is_empty()
            || !normalized
                .chars()
                .all(|character| character.is_alphanumeric() || character == '_')
        {
            None
        } else {
            Some(normalized.to_string())
        }
    }

    /*
     * Matching is token based so a normal sentence can contain punctuation around
     * the stop word. Substring matching is intentionally avoided: a keyword of
     * "stop" should not pause on "unstoppable".
     */
    pub(crate) fn matches(&self, text: &str) -> bool {
        self.enabled
            && text.split_whitespace().any(|token| {
                token
                    .trim_matches(|character: char| {
                        !character.is_alphanumeric() && character != '_'
                    })
                    .eq_ignore_ascii_case(&self.value)
            })
    }

    #[cfg(test)]
    pub(crate) fn set_value(&mut self, value: String) {
        self.value = value;
    }

    pub(crate) fn value(&self) -> &str {
        self.value.as_str()
    }
}
