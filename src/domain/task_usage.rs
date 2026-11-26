//! Observed token accounting. A missing observation is never a zero-cost execution.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::domain::conversation_progressive_activity::ConversationProgressiveTokenUsageBreakdown as Tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UsageGap {
    UsageNotObserved,
    PrefixNotObserved,
    CounterDiscontinuity,
    ArithmeticOverflow,
    TaskLinkMissing,
    BackendUsageUnavailable,
    SourceUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskUsageExecution {
    pub execution_id: String,
    pub kind: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub op_id: Option<i64>,
    pub observed_tokens: Option<Tokens>,
    pub gaps: Vec<UsageGap>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskUsageStatus {
    Available,
    Missing,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TaskUsageState {
    pub task_id: String,
    pub status: TaskUsageStatus,
    /// This is a lower bound from received observations, never billing or a complete cost.
    pub observed_tokens: Option<Tokens>,
    pub executions: Vec<TaskUsageExecution>,
    pub gaps: Vec<UsageGap>,
    pub runtime_available: bool,
    pub usage_available: bool,
    pub delegations_available: bool,
    /// Repository-wide old delegations without a task link, NOT attributed to this task.
    pub unlinked_repository_delegations: u64,
    pub truncated: bool,
}

impl TaskUsageState {
    pub fn unavailable(task_id: String) -> Self {
        Self {
            task_id,
            status: TaskUsageStatus::Unavailable,
            observed_tokens: None,
            executions: Vec::new(),
            gaps: Vec::new(),
            runtime_available: false,
            usage_available: false,
            delegations_available: false,
            unlinked_repository_delegations: 0,
            truncated: false,
        }
    }

    pub fn sum_observations(&mut self) {
        let mut sum = None;
        for tokens in self
            .executions
            .iter()
            .filter_map(|execution| execution.observed_tokens)
        {
            let Some(next) = add(sum.unwrap_or_default(), tokens) else {
                self.observed_tokens = None;
                self.gaps.push(UsageGap::ArithmeticOverflow);
                return;
            };
            sum = Some(next);
        }
        self.observed_tokens = sum;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskUsageLink {
    pub thread_id: String,
    pub op_id: i64,
}

#[derive(Debug, Clone, Default)]
pub struct UsageProjection {
    pub turns: Vec<UsageTurn>,
    pub links: Vec<TaskUsageLink>,
    pub truncated: bool,
}

/// A durable per-turn high water mark lives beside the existing kernel log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTurn {
    pub thread_id: String,
    pub turn_id: String,
    pub op_id: Option<i64>,
    pub last_sequence: i64,
    pub observed: Option<Tokens>,
    pub cumulative: Option<Tokens>,
    pub gaps: Vec<UsageGap>,
}

impl UsageTurn {
    pub fn new(thread_id: String, turn_id: String, op_id: Option<i64>, sequence: i64) -> Self {
        Self {
            thread_id,
            turn_id,
            op_id,
            last_sequence: sequence,
            observed: None,
            cumulative: None,
            gaps: Vec::new(),
        }
    }

    pub fn observe(&mut self, sequence: i64, last: Tokens, total: Tokens) {
        // Recorder replay and duplicate source snapshots never add another charge.
        if sequence <= self.last_sequence && self.cumulative.is_some() {
            return;
        }
        self.last_sequence = sequence;
        // A last-response counter cannot exceed the cumulative counter it belongs to.
        // Reject contradictory snapshots before they establish or advance a baseline.
        if subtract(total, last).is_none() {
            self.gap(UsageGap::CounterDiscontinuity);
            return;
        }
        match self.cumulative {
            None => {
                self.observed = Some(last);
                self.cumulative = Some(total);
                if last != total {
                    self.gap(UsageGap::PrefixNotObserved);
                }
            }
            Some(previous) if previous == total => {}
            Some(previous) => match subtract(total, previous) {
                Some(delta) => {
                    if let Some(observed) = self.observed {
                        self.observed = add(observed, delta);
                        if self.observed.is_none() {
                            self.gap(UsageGap::ArithmeticOverflow);
                        }
                    }
                    self.cumulative = Some(total);
                }
                // An out-of-order snapshot or reset cannot be safely assigned to this task.
                None => self.gap(UsageGap::CounterDiscontinuity),
            },
        }
    }

    fn gap(&mut self, gap: UsageGap) {
        if !self.gaps.contains(&gap) {
            self.gaps.push(gap);
        }
    }

    pub fn execution(self) -> TaskUsageExecution {
        let mut gaps = self.gaps;
        if self.observed.is_none() {
            gaps.push(UsageGap::UsageNotObserved);
        }
        TaskUsageExecution {
            execution_id: format!("{}/{}", self.thread_id, self.turn_id),
            kind: "kernel_turn".into(),
            thread_id: Some(self.thread_id),
            turn_id: Some(self.turn_id),
            op_id: self.op_id,
            observed_tokens: self.observed,
            gaps,
        }
    }
}

pub fn add(left: Tokens, right: Tokens) -> Option<Tokens> {
    Some(Tokens {
        cached_input_tokens: left
            .cached_input_tokens
            .checked_add(right.cached_input_tokens)?,
        input_tokens: left.input_tokens.checked_add(right.input_tokens)?,
        output_tokens: left.output_tokens.checked_add(right.output_tokens)?,
        reasoning_output_tokens: left
            .reasoning_output_tokens
            .checked_add(right.reasoning_output_tokens)?,
        total_tokens: left.total_tokens.checked_add(right.total_tokens)?,
    })
}

fn subtract(left: Tokens, right: Tokens) -> Option<Tokens> {
    Some(Tokens {
        cached_input_tokens: left
            .cached_input_tokens
            .checked_sub(right.cached_input_tokens)?,
        input_tokens: left.input_tokens.checked_sub(right.input_tokens)?,
        output_tokens: left.output_tokens.checked_sub(right.output_tokens)?,
        reasoning_output_tokens: left
            .reasoning_output_tokens
            .checked_sub(right.reasoning_output_tokens)?,
        total_tokens: left.total_tokens.checked_sub(right.total_tokens)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tokens(n: u64) -> Tokens {
        Tokens {
            input_tokens: n,
            total_tokens: n,
            ..Tokens::default()
        }
    }

    #[test]
    fn task_usage_repeated_cumulative_snapshots_and_replay_do_not_double_count() {
        let mut turn = UsageTurn::new("thread".into(), "turn".into(), Some(7), 1);
        turn.observe(2, tokens(10), tokens(110));
        turn.observe(3, tokens(10), tokens(110));
        turn.observe(4, tokens(20), tokens(130));
        let encoded = serde_json::to_string(&turn).unwrap();
        let mut resumed: UsageTurn = serde_json::from_str(&encoded).unwrap();
        resumed.observe(4, tokens(20), tokens(130));
        resumed.observe(5, tokens(5), tokens(135));
        assert_eq!(resumed.observed, Some(tokens(35)));
        assert_eq!(resumed.gaps, vec![UsageGap::PrefixNotObserved]);
    }

    #[test]
    fn task_usage_unobserved_zero_and_discontinuous_counters_are_distinct() {
        let mut turn = UsageTurn::new("thread".into(), "turn".into(), None, 1);
        assert_eq!(turn.clone().execution().observed_tokens, None);
        turn.observe(2, tokens(0), tokens(0));
        assert_eq!(turn.observed, Some(tokens(0)));
        turn.observe(3, tokens(20), tokens(20));
        turn.observe(4, tokens(3), tokens(3));
        assert_eq!(turn.observed, Some(tokens(20)));
        assert!(turn.gaps.contains(&UsageGap::CounterDiscontinuity));
    }

    #[test]
    fn task_usage_sums_independent_turns_and_reports_overflow() {
        let mut state = TaskUsageState::unavailable("task".into());
        state.sum_observations();
        assert_eq!(state.observed_tokens, None);
        let mut turn = UsageTurn::new("thread".into(), "turn".into(), None, 1);
        turn.observe(2, tokens(u64::MAX), tokens(u64::MAX));
        state.executions.push(turn.execution());
        let mut next = UsageTurn::new("thread".into(), "turn2".into(), None, 3);
        next.observe(4, tokens(1), tokens(1));
        state.executions.push(next.execution());
        state.sum_observations();
        assert_eq!(state.observed_tokens, None);
        assert!(state.gaps.contains(&UsageGap::ArithmeticOverflow));
    }

    #[test]
    fn task_usage_inconsistent_last_is_unobserved_and_does_not_advance_the_baseline() {
        let mut turn = UsageTurn::new("thread".into(), "turn".into(), Some(7), 1);
        turn.observe(2, tokens(10), tokens(5));
        assert_eq!(turn.observed, None);
        assert_eq!(turn.cumulative, None);
        assert!(
            turn.clone()
                .execution()
                .gaps
                .contains(&UsageGap::UsageNotObserved)
        );
        assert!(turn.gaps.contains(&UsageGap::CounterDiscontinuity));

        turn.observe(3, tokens(10), tokens(10));
        let invalid = Tokens {
            cached_input_tokens: 1,
            ..tokens(5)
        };
        turn.observe(4, invalid, tokens(15));
        assert_eq!(turn.observed, Some(tokens(10)));
        assert_eq!(turn.cumulative, Some(tokens(10)));
        turn.observe(5, tokens(5), tokens(15));
        assert_eq!(turn.observed, Some(tokens(15)));
    }
}
