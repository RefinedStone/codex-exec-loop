use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::parallel_mode::{
    ParallelModeOfficialCompletionReport, ParallelModeService,
};
use crate::domain::parallel_mode::ParallelModeSlotLeaseRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelTurnStreamLaunchRequest {
    pub workspace_directory: String,
    pub thread_id: Option<String>,
    pub prompt: String,
    pub slot_lease_request: Option<ParallelModeSlotLeaseRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelTurnStreamLaunchOutcome {
    pub request: ParallelTurnStreamLaunchRequest,
    pub launch_notice: Option<String>,
    pub invalidate_supervisor_snapshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelTurnStreamEventOutcome {
    pub runtime_notice: Option<String>,
    pub invalidate_supervisor_snapshot: bool,
    pub turn_started_observed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelTurnStreamCompletionOutcome {
    pub runtime_notice: Option<String>,
    pub invalidate_supervisor_snapshot: bool,
}

#[derive(Clone)]
pub struct ParallelModeTurnService {
    parallel_mode_service: ParallelModeService,
}

impl std::fmt::Debug for ParallelModeTurnService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParallelModeTurnService")
            .finish_non_exhaustive()
    }
}

impl ParallelModeTurnService {
    pub fn new(parallel_mode_service: ParallelModeService) -> Self {
        Self {
            parallel_mode_service,
        }
    }

    pub fn prepare_stream_launch(
        &self,
        request: ParallelTurnStreamLaunchRequest,
    ) -> Result<ParallelTurnStreamLaunchOutcome, String> {
        let Some(slot_lease_request) = request.slot_lease_request.clone() else {
            return Ok(ParallelTurnStreamLaunchOutcome {
                request,
                launch_notice: None,
                invalidate_supervisor_snapshot: false,
            });
        };

        let lease = self
            .parallel_mode_service
            .acquire_slot_lease(&request.workspace_directory, slot_lease_request)?;
        Ok(ParallelTurnStreamLaunchOutcome {
            request: ParallelTurnStreamLaunchRequest {
                workspace_directory: lease.worktree_path.clone(),
                thread_id: None,
                prompt: request.prompt,
                slot_lease_request: None,
            },
            launch_notice: Some(format!(
                "slot lease acquired before stream launch / slot: {} / agent: {} / task: {}",
                lease.slot_id, lease.agent_id, lease.task_id
            )),
            invalidate_supervisor_snapshot: true,
        })
    }

    pub fn sync_stream_event(
        &self,
        workspace_directory: &str,
        event: &ConversationStreamEvent,
    ) -> ParallelTurnStreamEventOutcome {
        if let ConversationStreamEvent::ThreadPrepared { thread_id, .. } = event {
            return match self
                .parallel_mode_service
                .record_workspace_slot_thread_prepared(workspace_directory, thread_id)
            {
                Ok(Some(_)) => ParallelTurnStreamEventOutcome {
                    runtime_notice: None,
                    invalidate_supervisor_snapshot: true,
                    turn_started_observed: false,
                },
                Ok(None) => ParallelTurnStreamEventOutcome {
                    runtime_notice: None,
                    invalidate_supervisor_snapshot: false,
                    turn_started_observed: false,
                },
                Err(error) => ParallelTurnStreamEventOutcome {
                    runtime_notice: Some(format!(
                        "slot lease thread-prepared transition failed: {error}"
                    )),
                    invalidate_supervisor_snapshot: false,
                    turn_started_observed: false,
                },
            };
        }

        if !matches!(event, ConversationStreamEvent::TurnStarted { .. }) {
            return ParallelTurnStreamEventOutcome {
                runtime_notice: None,
                invalidate_supervisor_snapshot: false,
                turn_started_observed: false,
            };
        }

        match self
            .parallel_mode_service
            .mark_workspace_slot_running(workspace_directory)
        {
            Ok(Some(_)) => ParallelTurnStreamEventOutcome {
                runtime_notice: None,
                invalidate_supervisor_snapshot: true,
                turn_started_observed: true,
            },
            Ok(None) => ParallelTurnStreamEventOutcome {
                runtime_notice: None,
                invalidate_supervisor_snapshot: false,
                turn_started_observed: true,
            },
            Err(error) => ParallelTurnStreamEventOutcome {
                runtime_notice: Some(format!("slot lease running transition failed: {error}")),
                invalidate_supervisor_snapshot: false,
                turn_started_observed: true,
            },
        }
    }

    pub fn finalize_stream_completion(
        &self,
        workspace_directory: &str,
        saw_turn_started: bool,
        saw_failed_before_turn_started: bool,
        saw_failed_event: bool,
        terminal_failure_observed: bool,
    ) -> ParallelTurnStreamCompletionOutcome {
        if should_release_unstarted_slot_lease(
            saw_turn_started,
            saw_failed_before_turn_started,
            terminal_failure_observed,
        ) {
            return match self
                .parallel_mode_service
                .release_workspace_slot_lease_after_failed_start(workspace_directory)
            {
                Ok(Some(lease)) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(format!(
                        "slot lease released after startup failure / slot: {} / agent: {}",
                        lease.slot_id, lease.agent_id
                    )),
                    invalidate_supervisor_snapshot: true,
                },
                Ok(None) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: None,
                    invalidate_supervisor_snapshot: false,
                },
                Err(error) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(format!(
                        "slot lease release failed after startup failure: {error}"
                    )),
                    invalidate_supervisor_snapshot: false,
                },
            };
        }

        if should_mark_cleanup_pending_after_success(
            saw_turn_started,
            saw_failed_event,
            terminal_failure_observed,
        ) {
            // Successful leased turns now wait for post-turn official completion/distributor
            // orchestration before cleanup and slot return.
            return ParallelTurnStreamCompletionOutcome {
                runtime_notice: None,
                invalidate_supervisor_snapshot: false,
            };
        }

        ParallelTurnStreamCompletionOutcome {
            runtime_notice: None,
            invalidate_supervisor_snapshot: false,
        }
    }

    pub fn begin_official_completion(
        &self,
        workspace_directory: &str,
        root_turn_id: &str,
        refresh_order: Option<u64>,
        latest_main_reply: Option<&str>,
        validation_summary: Option<&str>,
    ) -> Result<Option<ParallelModeOfficialCompletionReport>, String> {
        self.parallel_mode_service
            .begin_workspace_official_completion(
                workspace_directory,
                root_turn_id,
                refresh_order,
                latest_main_reply,
                validation_summary,
                None,
            )
    }

    pub fn mark_official_completion_failed(&self, workspace_directory: &str, failure_detail: &str) {
        let _ = self
            .parallel_mode_service
            .mark_workspace_official_completion_failed(workspace_directory, failure_detail);
    }

    pub fn mark_official_completion_refreshing(&self, workspace_directory: &str) -> Option<String> {
        match self
            .parallel_mode_service
            .mark_workspace_official_completion_refreshing(workspace_directory)
        {
            Ok(_) => None,
            Err(error) => Some(format!(
                "official completion refreshing state could not be recorded: {error}"
            )),
        }
    }

    pub fn finalize_official_completion_success(
        &self,
        workspace_directory: &str,
        ledger_refresh_outcome: &str,
    ) -> Vec<String> {
        let mut notices = Vec::new();
        if let Err(error) = self
            .parallel_mode_service
            .mark_workspace_commit_ready(workspace_directory, ledger_refresh_outcome)
        {
            notices.push(format!(
                "commit-ready state could not be recorded after official refresh: {error}"
            ));
        }
        match self
            .parallel_mode_service
            .enqueue_workspace_commit_ready_result(workspace_directory)
        {
            Ok(Some(item)) => notices.push(format!(
                "commit-ready result entered the distributor queue / agent: {} / task: {} / state: {}",
                item.source_agent,
                item.task_title,
                item.queue_state.label()
            )),
            Ok(None) => {}
            Err(error) => {
                notices.push(format!(
                    "distributor enqueue failed after official refresh: {error}"
                ));
                return notices;
            }
        }

        match self
            .parallel_mode_service
            .process_distributor_queue(workspace_directory)
        {
            Ok(mut delivery_notices) => notices.append(&mut delivery_notices),
            Err(error) => notices.push(format!(
                "distributor delivery failed after official refresh: {error}"
            )),
        }

        notices
    }
}

fn should_release_unstarted_slot_lease(
    saw_turn_started: bool,
    saw_failed_before_turn_started: bool,
    terminal_failure_observed: bool,
) -> bool {
    saw_failed_before_turn_started || (!saw_turn_started && terminal_failure_observed)
}

fn should_mark_cleanup_pending_after_success(
    saw_turn_started: bool,
    saw_failed_event: bool,
    terminal_failure_observed: bool,
) -> bool {
    saw_turn_started && !saw_failed_event && !terminal_failure_observed
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::adapter::outbound::github::GithubAutomationAdapter;
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::parallel_mode::ParallelModeService;

    use super::{
        ParallelModeTurnService, should_mark_cleanup_pending_after_success,
        should_release_unstarted_slot_lease,
    };

    struct TempGitWorkspace {
        root: String,
    }

    impl TempGitWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&root).expect("temp git workspace should be created");
            run_git(&root, &["init"]);
            run_git(&root, &["config", "user.name", "RefinedStone"]);
            run_git(&root, &["config", "user.email", "chem.en.9273@gmail.com"]);
            fs::write(root.join("README.md"), "temp repo\n")
                .expect("temp git workspace seed file should write");
            run_git(&root, &["add", "README.md"]);
            run_git(&root, &["commit", "-m", "Initial commit"]);

            Self {
                root: root.display().to_string(),
            }
        }
    }

    impl Drop for TempGitWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn create_temp_directory(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&root).expect("temp directory should be created");
        root.display().to_string()
    }

    fn run_git(repo_root: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .status()
            .expect("git command should launch");
        assert!(
            status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
    }

    fn test_parallel_mode_service() -> ParallelModeService {
        ParallelModeService::new(
            Arc::new(SqlitePlanningAuthorityAdapter::new()),
            Arc::new(GithubAutomationAdapter::new()),
            Arc::new(GitParallelModeRuntimeAdapter::new()),
        )
    }

    #[test]
    fn startup_failure_requests_unstarted_slot_release() {
        assert!(should_release_unstarted_slot_lease(false, true, true));
    }

    #[test]
    fn running_turn_does_not_request_unstarted_slot_release() {
        assert!(!should_release_unstarted_slot_lease(true, false, true));
    }

    #[test]
    fn successful_running_turn_is_cleanup_candidate() {
        assert!(should_mark_cleanup_pending_after_success(
            true, false, false
        ));
    }

    #[test]
    fn turn_started_without_slot_lease_keeps_snapshot_steady() {
        let workspace = TempGitWorkspace::new("parallel-turn-no-lease");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());

        let outcome = service.sync_stream_event(
            &workspace.root,
            &ConversationStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
            },
        );

        assert!(!outcome.invalidate_supervisor_snapshot);
        assert!(outcome.turn_started_observed);
        assert!(outcome.runtime_notice.is_none());
    }

    #[test]
    fn thread_prepared_without_slot_lease_keeps_snapshot_steady() {
        let workspace = TempGitWorkspace::new("parallel-thread-prepared-no-lease");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());

        let outcome = service.sync_stream_event(
            &workspace.root,
            &ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Temp".to_string(),
                cwd: workspace.root.clone(),
            },
        );

        assert!(!outcome.invalidate_supervisor_snapshot);
        assert!(!outcome.turn_started_observed);
        assert!(outcome.runtime_notice.is_none());
    }

    #[test]
    fn official_completion_refreshing_failure_becomes_runtime_notice() {
        let workspace = create_temp_directory("parallel-turn-refresh-failure");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());

        let notice = service.mark_official_completion_refreshing(&workspace);

        assert!(notice.as_deref().is_some_and(|value| {
            value.contains("official completion refreshing state could not be recorded")
        }));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn official_completion_finalize_surfaces_commit_ready_transition_failure() {
        let workspace = create_temp_directory("parallel-turn-commit-ready-failure");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());

        let notices = service
            .finalize_official_completion_success(&workspace, "official ledger refresh succeeded");

        assert!(notices.iter().any(|notice| {
            notice.contains("commit-ready state could not be recorded after official refresh")
        }));
        assert!(notices.iter().any(|notice| {
            notice.contains("distributor enqueue failed after official refresh")
        }));
        let _ = fs::remove_dir_all(workspace);
    }
}
