use crate::domain::parallel_mode::ParallelModeSupervisorSnapshot;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ParallelPanelUiState {
    pub overlay_visible: bool,
    pub mode_enabled: bool,
    pub supervisor_snapshot: Option<ParallelModeSupervisorSnapshot>,
    pub last_status_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ParallelPanelUiEvent {
    OverlayShown,
    OverlayHidden,
    ModeSet(bool),
    SupervisorSnapshotChanged(Option<Box<ParallelModeSupervisorSnapshot>>),
    StatusShown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ParallelPanelUiEffect {
    ShowOverlay,
    CloseOverlay,
    ShowStatus(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelPanelUiReduction {
    pub state: ParallelPanelUiState,
    pub effects: Vec<ParallelPanelUiEffect>,
}

pub(super) struct ParallelPanelStateController;

impl ParallelPanelStateController {
    pub(super) fn project(
        events: impl IntoIterator<Item = ParallelPanelUiEvent>,
    ) -> ParallelPanelUiState {
        let mut state = ParallelPanelUiState::default();
        for event in events {
            let ParallelPanelUiReduction {
                state: next_state,
                effects,
            } = Self::reduce(state, event);
            drop(effects);
            state = next_state;
        }
        state
    }

    pub(super) fn reduce(
        mut state: ParallelPanelUiState,
        event: ParallelPanelUiEvent,
    ) -> ParallelPanelUiReduction {
        let mut effects = Vec::new();
        match event {
            ParallelPanelUiEvent::OverlayShown => {
                state.overlay_visible = true;
                effects.push(ParallelPanelUiEffect::ShowOverlay);
            }
            ParallelPanelUiEvent::OverlayHidden => {
                state.overlay_visible = false;
                effects.push(ParallelPanelUiEffect::CloseOverlay);
            }
            ParallelPanelUiEvent::ModeSet(mode_enabled) => {
                state.mode_enabled = mode_enabled;
            }
            ParallelPanelUiEvent::SupervisorSnapshotChanged(snapshot) => {
                state.supervisor_snapshot = snapshot.map(|snapshot| *snapshot);
            }
            ParallelPanelUiEvent::StatusShown(status_text) => {
                state.last_status_text = Some(status_text.clone());
                effects.push(ParallelPanelUiEffect::ShowStatus(status_text));
            }
        }
        ParallelPanelUiReduction { state, effects }
    }

    pub(super) fn activity_pulse_visible(state: &ParallelPanelUiState) -> bool {
        if !state.overlay_visible || !state.mode_enabled {
            return false;
        }
        let Some(snapshot) = state.supervisor_snapshot.as_ref() else {
            return true;
        };
        Self::snapshot_is_loading(snapshot)
            || Self::snapshot_has_live_pool_slot(snapshot)
            || Self::snapshot_has_active_distributor_queue(snapshot)
            || Self::snapshot_has_recoverable_pool_issue(snapshot)
    }

    pub(super) fn loading_prompt_indicator_visible(state: &ParallelPanelUiState) -> bool {
        if !state.overlay_visible || !state.mode_enabled {
            return false;
        }
        let Some(snapshot) = state.supervisor_snapshot.as_ref() else {
            return true;
        };
        Self::snapshot_is_loading(snapshot)
    }

    pub(super) fn prompt_input_locked(state: &ParallelPanelUiState) -> bool {
        Self::loading_prompt_indicator_visible(state)
    }

    pub(super) fn snapshot_is_loading(snapshot: &ParallelModeSupervisorSnapshot) -> bool {
        snapshot
            .top_notice
            .as_deref()
            .is_some_and(|notice| notice.starts_with("loading "))
            || snapshot.pool.pool_root_label.starts_with("loading:")
    }

    pub(super) fn snapshot_has_live_pool_slot(snapshot: &ParallelModeSupervisorSnapshot) -> bool {
        snapshot.pool.leased_slots
            + snapshot.pool.running_slots
            + snapshot.pool.awaiting_cleanup_slots
            > 0
    }

    pub(super) fn snapshot_has_active_distributor_queue(
        snapshot: &ParallelModeSupervisorSnapshot,
    ) -> bool {
        !snapshot.distributor.queue_items.is_empty()
    }

    pub(super) fn snapshot_has_recoverable_pool_issue(
        snapshot: &ParallelModeSupervisorSnapshot,
    ) -> bool {
        snapshot.pool.blocked_slots > 0
            || snapshot.pool.missing_slots > 0
            || snapshot.pool.unavailable_slots > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterSnapshot, ParallelModeDistributorSnapshot,
        ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot, ParallelModePoolSlotState,
        ParallelModeReadinessState, ParallelModeSupervisorDetailSnapshot,
        ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
    };

    fn supervisor_snapshot(pool: ParallelModePoolBoardSnapshot) -> ParallelModeSupervisorSnapshot {
        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            "/repo",
            pool,
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "idle"),
            ParallelModeSupervisorDetailSnapshot::new(None, "idle"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "idle"),
            None,
        )
    }

    fn loading_snapshot() -> ParallelModeSupervisorSnapshot {
        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::derive(
                true,
                Some(
                    &crate::domain::parallel_mode::ParallelModeReadinessSnapshot::new(
                        "/repo",
                        ParallelModeReadinessState::Ready,
                        Vec::new(),
                        None,
                    ),
                ),
            ),
            "/repo",
            ParallelModePoolBoardSnapshot::new(
                0,
                "loading: supervisor refresh",
                "loading",
                Vec::new(),
            ),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "loading"),
            ParallelModeSupervisorDetailSnapshot::new(None, "loading"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "loading", "loading"),
            Some("loading 3/3: board refresh".to_string()),
        )
    }

    #[test]
    fn panel_controller_reduces_ui_state_and_emits_effects() {
        let initial = ParallelPanelUiState::default();
        let shown =
            ParallelPanelStateController::reduce(initial, ParallelPanelUiEvent::OverlayShown);
        assert!(shown.state.overlay_visible);
        assert_eq!(shown.effects, vec![ParallelPanelUiEffect::ShowOverlay]);

        let status = ParallelPanelStateController::reduce(
            shown.state,
            ParallelPanelUiEvent::StatusShown("parallel mode: on".to_string()),
        );
        assert_eq!(
            status.state.last_status_text.as_deref(),
            Some("parallel mode: on")
        );
        assert_eq!(
            status.effects,
            vec![ParallelPanelUiEffect::ShowStatus(
                "parallel mode: on".to_string()
            )]
        );
    }

    #[test]
    fn loading_snapshot_locks_prompt_only_when_overlay_is_visible() {
        let state = ParallelPanelUiState {
            overlay_visible: true,
            mode_enabled: true,
            supervisor_snapshot: Some(loading_snapshot()),
            last_status_text: None,
        };
        assert!(ParallelPanelStateController::activity_pulse_visible(&state));
        assert!(ParallelPanelStateController::prompt_input_locked(&state));

        let hidden = ParallelPanelUiState {
            overlay_visible: false,
            ..state
        };
        assert!(!ParallelPanelStateController::activity_pulse_visible(
            &hidden
        ));
        assert!(!ParallelPanelStateController::prompt_input_locked(&hidden));
    }

    #[test]
    fn ready_idle_snapshot_does_not_lock_prompt() {
        let state = ParallelPanelUiState {
            overlay_visible: true,
            mode_enabled: true,
            supervisor_snapshot: Some(supervisor_snapshot(ParallelModePoolBoardSnapshot::new(
                3,
                "/repo/.akra-worktrees",
                "idle",
                Vec::new(),
            ))),
            last_status_text: None,
        };

        assert!(!ParallelPanelStateController::activity_pulse_visible(
            &state
        ));
        assert!(!ParallelPanelStateController::prompt_input_locked(&state));
    }

    #[test]
    fn leased_pool_slot_keeps_activity_pulse_visible_without_locking_prompt() {
        let state = ParallelPanelUiState {
            overlay_visible: true,
            mode_enabled: true,
            supervisor_snapshot: Some(supervisor_snapshot(ParallelModePoolBoardSnapshot::new(
                3,
                "/repo/.akra-worktrees",
                "leased",
                vec![ParallelModePoolSlotSnapshot::new(
                    "slot-1",
                    ParallelModePoolSlotState::Leased,
                    "akra-agent/slot-1/task-one",
                    "akra-pool/slot-1",
                    "agent-1 / task-1",
                )],
            ))),
            last_status_text: None,
        };

        assert!(ParallelPanelStateController::activity_pulse_visible(&state));
        assert!(!ParallelPanelStateController::prompt_input_locked(&state));
    }
}
