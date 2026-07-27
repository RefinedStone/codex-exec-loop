use std::time::Instant;

use crate::core::app::{AutoFollowAuthoritySnapshot, AutoFollowPhase};

use super::{
    DISABLED_AUTO_FOLLOW_MAX_TURNS_TOKEN, INFINITE_AUTO_FOLLOW_MAX_TURNS,
    INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN,
};

pub(crate) const AUTO_FOLLOW_MODE_LABEL: &str = "planning queue";

#[path = "auto_follow_decision.rs"]
mod decision;
pub(crate) use decision::AutoFollowSkipReason;

/*
 * Runtime policy belongs to Core. This trait formats an immutable authority
 * snapshot for terminal copy; it intentionally exposes no transition methods.
 */
pub(crate) trait AutoFollowSnapshotPresentation {
    fn mode_label(&self) -> &'static str;
    fn progress_label(&self) -> String;
    fn max_auto_turns_label(&self) -> String;
    fn next_auto_turn_index(&self) -> usize;
    fn active_turn_index(&self) -> Option<usize>;
    fn active_started_at(&self) -> Option<Instant>;
    fn activity_label(&self) -> String;
}

impl AutoFollowSnapshotPresentation for AutoFollowAuthoritySnapshot {
    fn mode_label(&self) -> &'static str {
        AUTO_FOLLOW_MODE_LABEL
    }

    fn progress_label(&self) -> String {
        if !self.is_enabled() {
            return DISABLED_AUTO_FOLLOW_MAX_TURNS_TOKEN.to_string();
        }
        format!(
            "{}/{}",
            self.completed_auto_turns,
            self.max_auto_turns_label()
        )
    }

    fn max_auto_turns_label(&self) -> String {
        format_max_auto_turns(self.max_auto_turns)
    }

    fn next_auto_turn_index(&self) -> usize {
        self.completed_auto_turns + 1
    }

    fn active_turn_index(&self) -> Option<usize> {
        self.phase.turn_index()
    }

    fn active_started_at(&self) -> Option<Instant> {
        self.phase.started_at()
    }

    fn activity_label(&self) -> String {
        let max_auto_turns = self.max_auto_turns_label();
        match &self.phase {
            AutoFollowPhase::Idle => "idle".to_string(),
            AutoFollowPhase::Queued { turn_index, .. } => {
                format!("queued turn {turn_index}/{max_auto_turns}")
            }
            AutoFollowPhase::Submitting { turn_index, .. } => {
                format!("submitting turn {turn_index}/{max_auto_turns}")
            }
            AutoFollowPhase::Running { turn_index, .. } => {
                format!("running turn {turn_index}/{max_auto_turns}")
            }
        }
    }
}

pub(crate) fn normalize_max_auto_turns_candidate(candidate: &str) -> Option<usize> {
    let normalized = candidate.trim();
    if normalized.eq_ignore_ascii_case(DISABLED_AUTO_FOLLOW_MAX_TURNS_TOKEN) {
        return Some(0);
    }
    if normalized.eq_ignore_ascii_case(INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN) {
        return Some(INFINITE_AUTO_FOLLOW_MAX_TURNS);
    }
    normalized.parse::<usize>().ok()
}

fn format_max_auto_turns(value: usize) -> String {
    if value == 0 {
        DISABLED_AUTO_FOLLOW_MAX_TURNS_TOKEN.to_string()
    } else if value == INFINITE_AUTO_FOLLOW_MAX_TURNS {
        INFINITE_AUTO_FOLLOW_MAX_TURNS_TOKEN.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_auto_turn_candidate_accepts_named_and_numeric_values() {
        assert_eq!(normalize_max_auto_turns_candidate("7"), Some(7));
        assert_eq!(normalize_max_auto_turns_candidate("off"), Some(0));
        assert_eq!(
            normalize_max_auto_turns_candidate("infinite"),
            Some(usize::MAX)
        );
        assert_eq!(normalize_max_auto_turns_candidate("three"), None);
    }

    #[test]
    fn immutable_snapshot_formats_runtime_activity() {
        let snapshot = AutoFollowAuthoritySnapshot {
            max_auto_turns: 3,
            completed_auto_turns: 1,
            phase: AutoFollowPhase::Queued {
                turn_index: 2,
                started_at: Instant::now(),
            },
            ..AutoFollowAuthoritySnapshot::default()
        };

        assert_eq!(snapshot.progress_label(), "1/3");
        assert_eq!(snapshot.activity_label(), "queued turn 2/3");
    }
}
