use std::time::Instant;

use crate::application::service::planning::PlanningRuntimeQueuedAutoFollowPrompt;

use super::turn_activity::TurnActivityState;
use super::{
    DEFAULT_AUTO_FOLLOW_MAX_TURNS, DEFAULT_AUTO_FOLLOW_STOP_KEYWORD,
    INFINITE_AUTO_FOLLOW_MAX_TURNS, INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN,
};

const AUTO_FOLLOW_MODE_LABEL: &str = "planning queue";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AutoFollowupDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Skip(AutoFollowupSkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AutoFollowupSkipReason {
    Disabled,
    LimitReached,
    NoAgentReply,
    StopKeywordMatched,
    NoFileChanges,
    PlanningDisabled,
    PlanningBlocked,
    PlanningQueueIdlePolicyStop,
    PlanningQueueHeadRequired,
    PlanningRepeatedQueueHead,
}

impl AutoFollowupSkipReason {
    pub(crate) fn detail(
        self,
        auto_follow_state: &AutoFollowState,
        turn_activity: &TurnActivityState,
    ) -> String {
        match self {
            Self::Disabled => {
                "post-turn automation is off; toggle Ctrl+a to re-enable it".to_string()
            }
            Self::LimitReached => format!(
                "reached the configured auto-turn budget ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                "a non-empty agent reply is required before the next auto turn can be queued"
                    .to_string()
            }
            Self::StopKeywordMatched => format!(
                "the latest agent reply matched the stop keyword {}",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => format!(
                "the last completed turn changed {} files while the no-file stop rule is on",
                turn_activity.last_completed_file_change_count()
            ),
            Self::PlanningDisabled => {
                "planning mode is off; run :planning to resume queue automation".to_string()
            }
            Self::PlanningBlocked => {
                "planning needs repair before automation can continue".to_string()
            }
            Self::PlanningQueueIdlePolicyStop => {
                "planning is valid but the queue is idle, and automation stops here".to_string()
            }
            Self::PlanningQueueHeadRequired => {
                "planning is valid but has no next task yet".to_string()
            }
            Self::PlanningRepeatedQueueHead => {
                "automation is paused because the queue did not advance past the previous task"
                    .to_string()
            }
        }
    }

    pub(crate) fn activity_summary(self) -> &'static str {
        match self {
            Self::Disabled => "stopped: automation off",
            Self::LimitReached => "stopped: turn limit reached",
            Self::NoAgentReply => "skipped: no agent reply",
            Self::StopKeywordMatched => "stopped: stop keyword matched",
            Self::NoFileChanges => "stopped: no file changes",
            Self::PlanningDisabled => "stopped: planning off",
            Self::PlanningBlocked => "paused: planning needs repair",
            Self::PlanningQueueIdlePolicyStop => "stopped: queue idle",
            Self::PlanningQueueHeadRequired => "paused: waiting for next task",
            Self::PlanningRepeatedQueueHead => "paused: queue did not advance",
        }
    }

    pub(crate) fn runtime_status(self, auto_follow_state: &AutoFollowState) -> String {
        match self {
            Self::Disabled => "turn completed / automation stopped: off".to_string(),
            Self::LimitReached => format!(
                "turn completed / auto follow-up stopped: turn limit reached ({})",
                auto_follow_state.progress_label()
            ),
            Self::NoAgentReply => {
                "turn completed / auto follow-up skipped: no agent reply".to_string()
            }
            Self::StopKeywordMatched => format!(
                "turn completed / auto follow-up stopped: stop keyword matched ({})",
                auto_follow_state.stop_rules.stop_keyword.value()
            ),
            Self::NoFileChanges => {
                "turn completed / auto follow-up stopped: no file changes".to_string()
            }
            Self::PlanningDisabled => {
                "turn completed / auto follow-up stopped: planning mode is off".to_string()
            }
            Self::PlanningBlocked => {
                "turn completed / auto follow-up paused: planning needs repair".to_string()
            }
            Self::PlanningQueueIdlePolicyStop => {
                "turn completed / auto follow-up stopped: planning queue is idle".to_string()
            }
            Self::PlanningQueueHeadRequired => {
                "turn completed / auto follow-up paused: planning has no next task yet"
                    .to_string()
            }
            Self::PlanningRepeatedQueueHead => {
                "turn completed / auto follow-up paused: the queue did not advance past the previous task"
                    .to_string()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AutoFollowState {
    pub(crate) enabled: bool,
    pub(crate) completed_auto_turns: usize,
    pub(crate) max_auto_turns: usize,
    pub(crate) runtime_phase: AutoFollowRuntimePhase,
    pub(crate) stop_rules: AutoFollowStopRules,
}

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
    pub(crate) stop_keyword: StopKeywordRule,
    pub(crate) stop_on_no_file_changes: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct StopKeywordRule {
    pub(crate) enabled: bool,
    pub(crate) value: String,
}

impl AutoFollowState {
    pub(crate) fn new() -> Self {
        Self {
            enabled: true,
            completed_auto_turns: 0,
            max_auto_turns: DEFAULT_AUTO_FOLLOW_MAX_TURNS,
            runtime_phase: AutoFollowRuntimePhase::Idle,
            stop_rules: AutoFollowStopRules::default(),
        }
    }

    pub(crate) fn status_label(&self) -> &'static str {
        if self.enabled { "on" } else { "off" }
    }

    pub(crate) fn mode_label(&self) -> &'static str {
        AUTO_FOLLOW_MODE_LABEL
    }

    pub(crate) fn progress_label(&self) -> String {
        format!(
            "{}/{}",
            self.completed_auto_turns,
            self.max_auto_turns_label()
        )
    }

    pub(crate) fn completed_progress_label(&self) -> String {
        format!(
            "{}/{} completed",
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

    pub(crate) fn max_auto_turns_value(&self) -> usize {
        self.max_auto_turns
    }

    pub(crate) fn max_auto_turns_label(&self) -> String {
        format_max_auto_turns(self.max_auto_turns)
    }

    pub(crate) fn stop_keyword_label(&self) -> String {
        self.stop_rules.stop_keyword.label()
    }

    pub(crate) fn stop_keyword_value(&self) -> &str {
        self.stop_rules.stop_keyword.value()
    }

    pub(crate) fn no_file_change_stop_label(&self) -> &'static str {
        self.stop_rules.no_file_change_label()
    }

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

    pub(crate) fn can_queue_next(&self) -> bool {
        self.enabled && self.completed_auto_turns < self.max_auto_turns
    }

    pub(crate) fn reset_for_manual_turn(&mut self) {
        self.completed_auto_turns = 0;
        self.runtime_phase = AutoFollowRuntimePhase::Idle;
    }

    pub(crate) fn begin_post_turn_evaluation(&mut self) {
        self.runtime_phase = AutoFollowRuntimePhase::Evaluating {
            started_at: Instant::now(),
        };
    }

    pub(crate) fn mark_auto_turn_queued(&mut self) -> usize {
        let turn_index = self.next_auto_turn_index();
        self.runtime_phase = AutoFollowRuntimePhase::Queued {
            started_at: Instant::now(),
            turn_index,
        };
        turn_index
    }

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

    pub(crate) fn enable(&mut self) {
        self.enabled = true;
    }

    pub(crate) fn stop(&mut self) {
        self.enabled = false;
        if matches!(
            self.runtime_phase,
            AutoFollowRuntimePhase::Idle
                | AutoFollowRuntimePhase::Evaluating { .. }
                | AutoFollowRuntimePhase::Queued { .. }
        ) {
            self.runtime_phase = AutoFollowRuntimePhase::Idle;
        }
    }

    pub(crate) fn set_max_auto_turns(&mut self, value: usize) {
        self.max_auto_turns = value;
    }

    pub(crate) fn toggle_stop_keyword(&mut self) {
        self.stop_rules.stop_keyword.toggle();
    }

    pub(crate) fn set_stop_keyword_value(&mut self, value: String) {
        self.stop_rules.stop_keyword.set_value(value);
    }

    pub(crate) fn toggle_no_file_change_stop(&mut self) {
        self.stop_rules.stop_on_no_file_changes = !self.stop_rules.stop_on_no_file_changes;
    }

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
    pub(crate) fn should_stop_on_no_file_changes(&self, file_change_count: usize) -> bool {
        self.stop_on_no_file_changes && file_change_count == 0
    }

    pub(crate) fn no_file_change_label(&self) -> &'static str {
        if self.stop_on_no_file_changes {
            "on"
        } else {
            "off"
        }
    }
}

impl StopKeywordRule {
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

    pub(crate) fn label(&self) -> String {
        if self.enabled {
            format!("on ({})", self.value)
        } else {
            format!("off ({})", self.value)
        }
    }

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

    pub(crate) fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub(crate) fn set_value(&mut self, value: String) {
        self.value = value;
    }

    pub(crate) fn value(&self) -> &str {
        self.value.as_str()
    }
}
