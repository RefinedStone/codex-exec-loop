use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneLoadingStage, ParallelModeControlPlanePresentationEvent,
};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModeReadinessSnapshot,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParallelModePresentationLoadingStage {
    Entering,
    ReconcilingPool,
    RefreshingBoard,
}

impl ParallelModePresentationLoadingStage {
    fn from_control_plane(stage: ParallelModeControlPlaneLoadingStage) -> Self {
        match stage {
            ParallelModeControlPlaneLoadingStage::ReconcilingPool => Self::ReconcilingPool,
        }
    }

    fn pool_root_label(self) -> &'static str {
        match self {
            Self::Entering => "loading: readiness checks",
            Self::ReconcilingPool => "loading: pool reconcile",
            Self::RefreshingBoard => "loading: supervisor refresh",
        }
    }

    fn pool_status(self) -> &'static str {
        match self {
            Self::Entering => "1/3 readiness checks running",
            Self::ReconcilingPool => "2/3 pool reconcile running",
            Self::RefreshingBoard => "3/3 refreshing supervisor board",
        }
    }

    fn roster_empty_state(self) -> &'static str {
        match self {
            Self::Entering => "waiting for readiness before slots can be assigned",
            Self::ReconcilingPool => "waiting for pool reset and reconcile results",
            Self::RefreshingBoard => "refreshing active agent roster",
        }
    }

    fn detail_empty_state(self) -> &'static str {
        match self {
            Self::Entering => "loading 1/3: readiness checks",
            Self::ReconcilingPool => "loading 2/3: pool reconcile",
            Self::RefreshingBoard => "loading 3/3: board refresh",
        }
    }

    fn distributor_head(self) -> &'static str {
        match self {
            Self::Entering => "waiting for readiness",
            Self::ReconcilingPool => "pool reconcile in progress",
            Self::RefreshingBoard => "refreshing distributor state",
        }
    }

    fn distributor_note(self) -> &'static str {
        match self {
            Self::Entering => "pipeline: [running] readiness -> [next] pool -> [next] board",
            Self::ReconcilingPool => "pipeline: [done] readiness -> [running] pool -> [next] board",
            Self::RefreshingBoard => "pipeline: [done] readiness -> [done] pool -> [running] board",
        }
    }

    fn top_notice(self) -> &'static str {
        match self {
            Self::Entering => {
                "loading 1/3: checking repository, planning, branch, pool, and GitHub readiness"
            }
            Self::ReconcilingPool => "loading 2/3: readiness passed; reconciling pool",
            Self::RefreshingBoard => {
                "loading 3/3: pool state changed; refreshing the supervisor board"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelModePresentationBridgeContext {
    current_workspace_directory: String,
    mode_enabled: bool,
}

impl ParallelModePresentationBridgeContext {
    pub(super) fn new(current_workspace_directory: String, mode_enabled: bool) -> Self {
        Self {
            current_workspace_directory,
            mode_enabled,
        }
    }

    fn accepts_workspace(&self, workspace_directory: &str) -> bool {
        self.current_workspace_directory == workspace_directory
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ParallelModePresentationAction {
    SyncReadinessProjection(ParallelModeReadinessSnapshot),
    SyncSupervisorProjection(Box<ParallelModeSupervisorSnapshot>),
    ShowStatus(String),
    ObserveRuntimeNotice(String),
    RefreshPlanningRuntimeProjection { workspace_directory: String },
}

pub(super) fn parallel_mode_presentation_actions(
    context: &ParallelModePresentationBridgeContext,
    events: Vec<ParallelModeControlPlanePresentationEvent>,
) -> Vec<ParallelModePresentationAction> {
    events
        .into_iter()
        .flat_map(|event| parallel_mode_presentation_actions_for_event(context, event))
        .collect()
}

fn parallel_mode_presentation_actions_for_event(
    context: &ParallelModePresentationBridgeContext,
    event: ParallelModeControlPlanePresentationEvent,
) -> Vec<ParallelModePresentationAction> {
    match event {
        ParallelModeControlPlanePresentationEvent::EnterProgress {
            workspace_directory,
            readiness_snapshot,
            loading_stage,
            status_text,
        } => {
            if !context.mode_enabled || !context.accepts_workspace(&workspace_directory) {
                return Vec::new();
            }
            let stage = ParallelModePresentationLoadingStage::from_control_plane(loading_stage);
            let supervisor_snapshot = pending_parallel_mode_supervisor_snapshot(
                &workspace_directory,
                true,
                readiness_snapshot.as_ref(),
                stage,
            );
            let mut actions = Vec::new();
            if let Some(readiness_snapshot) = readiness_snapshot {
                actions.push(ParallelModePresentationAction::SyncReadinessProjection(
                    readiness_snapshot,
                ));
            }
            actions.push(ParallelModePresentationAction::SyncSupervisorProjection(
                Box::new(supervisor_snapshot),
            ));
            actions.push(ParallelModePresentationAction::ShowStatus(status_text));
            actions
        }
        ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
            workspace_directory,
            snapshot,
        } => {
            if context.accepts_workspace(&workspace_directory) {
                vec![ParallelModePresentationAction::SyncReadinessProjection(
                    snapshot,
                )]
            } else {
                Vec::new()
            }
        }
        ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged {
            workspace_directory,
            snapshot,
        } => {
            if context.accepts_workspace(&workspace_directory) {
                vec![ParallelModePresentationAction::SyncSupervisorProjection(
                    snapshot,
                )]
            } else {
                Vec::new()
            }
        }
        ParallelModeControlPlanePresentationEvent::StatusShown { status_text } => {
            vec![ParallelModePresentationAction::ShowStatus(status_text)]
        }
        ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice { notice } => {
            vec![ParallelModePresentationAction::ObserveRuntimeNotice(notice)]
        }
        ParallelModeControlPlanePresentationEvent::PostTurnAutoFollowPromptConsumed => Vec::new(),
        ParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
            workspace_directory,
        } => vec![
            ParallelModePresentationAction::RefreshPlanningRuntimeProjection {
                workspace_directory,
            },
        ],
        ParallelModeControlPlanePresentationEvent::ModeDisabled { .. } => Vec::new(),
    }
}

pub(super) fn pending_parallel_mode_supervisor_snapshot(
    workspace_directory: &str,
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    stage: ParallelModePresentationLoadingStage,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot),
        workspace_directory,
        ParallelModePoolBoardSnapshot::new(
            0,
            stage.pool_root_label(),
            stage.pool_status(),
            Vec::new(),
        ),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), stage.roster_empty_state()),
        ParallelModeSupervisorDetailSnapshot::new(None, stage.detail_empty_state()),
        ParallelModeDistributorSnapshot::new(
            Vec::new(),
            Vec::new(),
            stage.distributor_head(),
            stage.distributor_note(),
        ),
        Some(stage.top_notice().to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::parallel_mode::ParallelModeReadinessState;

    fn readiness_snapshot(workspace: &str) -> ParallelModeReadinessSnapshot {
        ParallelModeReadinessSnapshot::new(
            workspace,
            ParallelModeReadinessState::Ready,
            Vec::new(),
            Some("ready".to_string()),
        )
    }

    #[test]
    fn enter_progress_maps_to_projection_and_status_actions_for_current_workspace() {
        let context = ParallelModePresentationBridgeContext::new("/work".to_string(), true);
        let readiness = readiness_snapshot("/work");

        let actions = parallel_mode_presentation_actions_for_event(
            &context,
            ParallelModeControlPlanePresentationEvent::EnterProgress {
                workspace_directory: "/work".to_string(),
                readiness_snapshot: Some(readiness.clone()),
                loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
                status_text: "parallel mode: loading".to_string(),
            },
        );

        assert_eq!(
            actions.first(),
            Some(&ParallelModePresentationAction::SyncReadinessProjection(
                readiness
            ))
        );
        assert!(matches!(
            actions.get(1),
            Some(ParallelModePresentationAction::SyncSupervisorProjection(snapshot))
                if snapshot.pool.reconcile_status == "2/3 pool reconcile running"
        ));
        assert_eq!(
            actions.get(2),
            Some(&ParallelModePresentationAction::ShowStatus(
                "parallel mode: loading".to_string()
            ))
        );
    }

    #[test]
    fn workspace_scoped_projection_events_ignore_stale_workspaces() {
        let context = ParallelModePresentationBridgeContext::new("/current".to_string(), true);

        let actions = parallel_mode_presentation_actions_for_event(
            &context,
            ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
                workspace_directory: "/old".to_string(),
                snapshot: readiness_snapshot("/old"),
            },
        );

        assert!(actions.is_empty());
    }

    #[test]
    fn enter_progress_is_ignored_after_mode_turns_off() {
        let context = ParallelModePresentationBridgeContext::new("/work".to_string(), false);

        let actions = parallel_mode_presentation_actions_for_event(
            &context,
            ParallelModeControlPlanePresentationEvent::EnterProgress {
                workspace_directory: "/work".to_string(),
                readiness_snapshot: None,
                loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
                status_text: "parallel mode: loading".to_string(),
            },
        );

        assert!(actions.is_empty());
    }
}
