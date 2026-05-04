use crate::application::port::outbound::github_automation_port::GithubAutomationPort;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::domain::parallel_mode::{
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeOrchestratorState, ParallelModeOrchestratorStateMachine, ParallelModePoolSlotState,
    ParallelModeReadinessSnapshot, ParallelModeReadinessState, ParallelModeSlotLeaseState,
    ParallelModeSupervisorSnapshot,
};
use crate::domain::planning::PlanningOfficialCompletionRefreshContract;
use crate::domain::planning::PriorityQueueTask;
use chrono::DateTime;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
mod branch_names;
mod completion;
pub(crate) mod distributor;
mod git_sequence;
mod orchestration;
mod pool;
mod readiness;
mod session_detail;
mod slot_lifecycle;
pub(crate) mod supervisor;
mod support;
pub(crate) mod turn;
use self::branch_names::{allocate_agent_branch_name, branch_exists};
#[cfg(test)]
use self::branch_names::{sanitize_task_slug, short_branch_slug_hash};
use self::distributor::ParallelModeDistributorService;
use self::orchestration::{
    inspect_akra_integration_worktree_blocker, parallel_dispatch_excluded_task_ids,
};
#[cfg(test)]
use self::pool::detect_canonical_repo_root;
use self::pool::{
    PoolBoardWithContextResult, PoolRuntimeContext, WorkspaceSlotLeaseResolution,
    acquire_pool_allocation_lock, branch_is_cleanup_ready, branch_is_integrated_into,
    build_pool_board, build_pool_slots, cleanup_slot, inspect_pool_board_and_context,
    inspect_slot_git_status, load_pool_runtime_context, pool_operator_recovery_notice,
    reconcile_pool_board, reconcile_pool_board_and_context, remove_slot_lease,
    reset_pool_for_parallel_enable, resolve_workspace_head_sha, resolve_workspace_slot_lease,
    short_sha, write_slot_lease,
};
#[cfg(test)]
use self::pool::{derive_default_pool_root, slot_id, slot_lease_file_path};
#[cfg(test)]
use self::readiness::parse_https_remote;
use self::readiness::{
    blocked_prerequisite_capability, command_succeeds, inspect_akra_branch,
    inspect_authority_store, inspect_gh_auth, inspect_gh_binary, inspect_git_worktree,
    inspect_planning, inspect_push_remote, run_command,
};
#[cfg(test)]
use self::session_detail::{agent_session_detail_record_path, read_agent_session_detail_record};
use self::session_detail::{
    default_authority_refresh_outcome, default_validation_summary,
    format_elapsed_label_from_timestamp, lease_session_key, record_assigned_session_detail,
    record_cleaned_session_detail, record_cleanup_pending_session_detail,
    record_distributor_failed_session_detail, record_failed_start_session_detail,
    record_integrating_session_detail, record_merge_pending_session_detail,
    record_merge_queued_session_detail, record_official_completion_recovery_needed_session_detail,
    record_pr_pending_session_detail, record_pushing_session_detail, record_running_session_detail,
    record_thread_prepared_session_detail,
};
use self::supervisor::ParallelModeSupervisorService;
pub(super) use self::support::{
    current_branch_name, current_timestamp, discard_unstarted_slot_branch, ensure_directory_exists,
};
const DISTRIBUTOR_INTEGRATION_BRANCH: &str = "prerelease";
const POOL_BASELINE_BRANCH: &str = DISTRIBUTOR_INTEGRATION_BRANCH;
const DEFAULT_PUSH_REMOTE_NAME: &str = "origin";
const DEFAULT_POOL_SIZE: usize = 3;
const AKRA_AGENT_BRANCH_PREFIX: &str = "akra-agent";
const MAX_AGENT_BRANCH_SLUG_LEN: usize = 96;
const AGENT_BRANCH_TRUNCATION_HASH_LEN: usize = 10;
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL: &str =
    "agent branch is not integrated into `prerelease` and has no lease metadata";
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION: &str =
    "inspect the slot branch, merge or discard it manually, then rerun reconcile";
fn remote_branch_name(remote_name: &str, branch_name: &str) -> String {
    format!("{remote_name}/{branch_name}")
}
fn remote_tracking_branch_ref(remote_name: &str, branch_name: &str) -> String {
    format!(
        "refs/remotes/{}",
        remote_branch_name(remote_name, branch_name)
    )
}
fn local_branch_ref(branch_name: &str) -> String {
    format!("refs/heads/{branch_name}")
}
pub type ParallelModeOfficialCompletionReport = PlanningOfficialCompletionRefreshContract;
#[derive(Debug, Clone, PartialEq, Eq)]
/*
dispatch plan은 "지금 몇 개의 병렬 agent를 새로 띄워도 되는가"를 TUI와 orchestrator가 판단할 때
쓰는 계산 결과다. idle slot 수는 물리적 실행 capacity이고, excluded_task_ids는 이미 lease나
distributor queue에 잡혀 있는 task를 다시 배정하지 않기 위한 중복 방지 목록이다. candidates는
planning queue에서 실제로 배정할 수 있는 작업만 capacity만큼 잘라낸 결과다.
*/
pub struct ParallelModeDispatchPlan {
    pub idle_slot_count: usize,
    pub excluded_task_ids: Vec<String>,
    pub candidates: Vec<PriorityQueueTask>,
}

fn failed_start_dispatch_blockers(context: &PoolRuntimeContext) -> BTreeMap<String, i64> {
    let mut blockers = BTreeMap::new();
    for detail in &context.session_details {
        let task_id = detail.task_id.trim();
        if task_id.is_empty() {
            continue;
        }
        let Ok(failed_at) = DateTime::parse_from_rfc3339(detail.updated_at.trim())
            .map(|timestamp| timestamp.timestamp_millis())
        else {
            continue;
        };
        if detail.state_label == "failed"
            && detail.completion_state_label == "aborted"
            && detail
                .latest_summary
                .contains("launch failed before the session reached the running state")
        {
            blockers
                .entry(task_id.to_string())
                .and_modify(|current| {
                    if failed_at > *current {
                        *current = failed_at;
                    }
                })
                .or_insert(failed_at);
        }
    }
    blockers
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
orchestrator trigger는 queue processing을 왜 실행했는지 남기는 provenance다. main turn이 끝나서
실행한 tick, official planning refresh가 끝나서 실행한 tick, 사용자가 수동 dispatch한 tick은
같은 distributor queue를 움직이지만, notice와 telemetry를 해석할 때 원인이 다르다. enum으로 두면
호출자가 임의 문자열을 만들지 않고 정해진 사건만 넘기게 된다.
*/
pub enum ParallelModeOrchestratorTrigger {
    MainTurnCompleted,
    PlanningRefreshCompleted,
    ManualDispatch,
}
#[derive(Debug, Clone, PartialEq, Eq)]
/*
orchestrator tick result는 distributor queue를 한 번 움직인 결과다. blocked가 true이면 integration
worktree 자체가 잘못되어 queue processing에 들어가지 못한 것이고, false이면 queue processing은
실행되었으며 notices에 실제 push/PR/integration/cleanup 결과가 들어간다. 이 구분은 TUI가
"운영자가 먼저 worktree를 고쳐야 함"과 "queue가 정상 처리됨"을 다르게 표시하게 한다.
*/
pub struct ParallelModeOrchestratorTickResult {
    pub trigger: ParallelModeOrchestratorTrigger,
    pub state: ParallelModeOrchestratorState,
    pub blocked: bool,
    pub notices: Vec<String>,
}
#[derive(Clone)]
/*
ParallelModeService는 병렬 모드 application 계층의 facade다. TUI는 이 타입을 통해 readiness,
supervisor snapshot, dispatch plan, slot lifecycle, official completion, distributor orchestration을
호출한다. 내부적으로는 planning authority, GitHub automation, runtime port를 조합하고, 실제 세부
정책은 pool/distributor/supervisor/session_detail 모듈로 분산되어 있다.

이 타입이 adapter를 직접 구현하지 않고 port trait을 받는 구조는 application layer가 git, GitHub,
sqlite 같은 outbound 세부 구현에 묶이지 않게 해 준다.
*/
pub struct ParallelModeService {
    distributor_service: ParallelModeDistributorService,
    supervisor_service: ParallelModeSupervisorService,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    parallel_runtime: Arc<dyn ParallelModeRuntimePort>,
}
impl std::fmt::Debug for ParallelModeService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParallelModeService")
            .finish_non_exhaustive()
    }
}
impl ParallelModeService {
    pub fn new(
        planning_authority: Arc<dyn PlanningAuthorityPort>,
        github_automation: Arc<dyn GithubAutomationPort>,
        parallel_runtime: Arc<dyn ParallelModeRuntimePort>,
    ) -> Self {
        Self {
            distributor_service: ParallelModeDistributorService::with_planning_authority(
                github_automation,
                planning_authority.clone(),
            ),
            supervisor_service: ParallelModeSupervisorService::new(),
            planning_authority,
            parallel_runtime,
        }
    }

    /*
    refresh order 예약은 official completion worker가 시작되기 전에 순번을 고정하는 경로다. slot
    workspace가 Running lease에 연결되어 있을 때만 예약하며, 일반 workspace나 아직 실행되지 않은
    lease에서는 None을 반환한다. 이렇게 해야 여러 hidden worker가 거의 동시에 시작되어도 planning
    ledger refresh 순서가 뒤섞이지 않는다.
    */
    pub fn reserve_workspace_official_completion_refresh_order(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<u64>, String> {
        let Some(resolution) =
            resolve_workspace_slot_lease(self.planning_authority.as_ref(), workspace_dir)?
        else {
            return Ok(None);
        };
        if resolution.lease.state != ParallelModeSlotLeaseState::Running {
            return Ok(None);
        }

        self.planning_authority
            .reserve_next_official_refresh_order(&resolution.lease.worktree_path)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    /*
    readiness inspection은 병렬 모드의 enable gate다. git repository, git worktree, integration
    branch, push remote, GitHub automation, planning runtime, authority shadow store를 capability
    snapshot으로 모은다. domain의 readiness state는 이 capability 목록에서 derive되며, 첫 non-ready
    capability는 top alert로 올라간다.

    readiness가 통과되면 distributor runtime recovery를 한 번 시도한다. 단순 readiness 조회가 queue
    record와 lease의 재시작 후 상태를 정리하는 이유는 supervisor를 열었을 때 오래된 blocked/cleaning
    상태가 현재 git/GitHub 현실과 최대한 맞아 있어야 하기 때문이다.
    */
    pub fn inspect_readiness(
        &self,
        workspace_dir: &str,
        planning_snapshot: &PlanningRuntimeSnapshot,
    ) -> ParallelModeReadinessSnapshot {
        let repo_root = self.parallel_runtime.detect_git_repo_root(workspace_dir);
        let git_repository = match &repo_root {
            Some(repo_root) => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Ready,
                format!("git repo detected at {repo_root}"),
                None,
            ),
            None => ParallelModeCapabilitySnapshot::new(
                ParallelModeCapabilityKey::GitRepository,
                ParallelModeCapabilityState::Blocked,
                "parallel mode only runs inside a git repository",
                Some("open a git-backed workspace before enabling parallel mode".to_string()),
            ),
        };
        let git_worktree = match &repo_root {
            Some(repo_root) => inspect_git_worktree(self.parallel_runtime.as_ref(), repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::GitWorktree,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let akra_branch = match &repo_root {
            Some(repo_root) => inspect_akra_branch(self.parallel_runtime.as_ref(), repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::AkraBranch,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let push_remote = match &repo_root {
            Some(repo_root) => inspect_push_remote(self.parallel_runtime.as_ref(), repo_root),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::PushRemote,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let gh_binary = inspect_gh_binary(self.parallel_runtime.as_ref());
        let gh_auth = inspect_gh_auth(
            self.parallel_runtime.as_ref(),
            &gh_binary,
            repo_root.as_deref(),
        );
        let planning = inspect_planning(planning_snapshot);
        let authority_store = inspect_authority_store(
            self.planning_authority.as_ref(),
            workspace_dir,
            &git_repository,
            &planning,
        );
        /*
        Capability ordering is the operator reading order in the supersession
        popup: repository primitives first, GitHub delivery next, planning
        authority last. top_alert below intentionally reports the first non-ready
        item in that dependency chain.
        */
        let capabilities = vec![
            git_repository,
            git_worktree,
            akra_branch,
            push_remote,
            gh_binary,
            gh_auth,
            planning,
            authority_store,
        ];
        let readiness = ParallelModeReadinessState::derive_from_capabilities(&capabilities);
        let top_alert = capabilities
            .iter()
            .find(|capability| capability.state != ParallelModeCapabilityState::Ready)
            .map(ParallelModeCapabilitySnapshot::summary);
        let snapshot =
            ParallelModeReadinessSnapshot::new(workspace_dir, readiness, capabilities, top_alert);
        if snapshot.allows_parallel_mode() {
            /*
            Recovery is best-effort because readiness is still a diagnostic path.
            A failed recovery should be visible later through supervisor/distributor
            snapshots, not turn a ready capability set into a readiness failure.
            */
            let _ = self
                .distributor_service
                .recover_runtime_state(workspace_dir);
        }
        snapshot
    }

    /*
    build_supervisor_snapshot은 읽기 전용 snapshot 경로다. readiness 결과와 mode enabled 여부를 넘겨
    pool/roster/detail/distributor 화면 모델을 만들지만, pool worktree를 새로 만들거나 cleanup하는
    reconcile 부작용은 실행하지 않는다. 단순 화면 refresh가 저장소 상태를 바꾸지 않게 하려는
    경계다.
    */
    pub fn build_supervisor_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeSupervisorSnapshot {
        self.supervisor_service.build_snapshot(
            self.planning_authority.as_ref(),
            workspace_dir,
            mode_enabled,
            readiness_snapshot,
            &self.distributor_service,
        )
    }

    /*
    reconcile_supervisor_snapshot은 같은 supervisor snapshot을 반환하지만, mode가 켜진 상태에서 pool
    baseline/slot worktree/cleanup 상태를 기대 형태로 수렴시키는 경로다. 사용자가 병렬 모드를
    켜거나 명시적으로 refresh할 때 사용하며, 읽기 전용 snapshot과 구분해 side effect가 있는 작업을
    예측 가능하게 한다.
    */
    pub fn reconcile_supervisor_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeSupervisorSnapshot {
        self.supervisor_service.reconcile_snapshot(
            self.planning_authority.as_ref(),
            workspace_dir,
            mode_enabled,
            readiness_snapshot,
            &self.distributor_service,
        )
    }

    pub fn reset_pool_on_parallel_enable(&self, workspace_dir: &str) -> Result<usize, String> {
        reset_pool_for_parallel_enable(self.planning_authority.as_ref(), workspace_dir)
    }

    /*
    dispatch plan은 planning queue와 pool capacity를 맞물려 계산한다. 먼저 pool을 reconcile해
    missing/cleanup 가능한 slot을 정리하고, 현재 idle slot 수만큼만 active planning task를 후보로
    뽑는다. 이미 lease 중이거나 distributor queue에 있는 task는 excluded로 제거해 같은 task가 중복
    agent에게 배정되지 않게 한다.
    */
    pub fn build_dispatch_plan(
        &self,
        workspace_dir: &str,
        planning_snapshot: &PlanningRuntimeSnapshot,
        requested_count: usize,
    ) -> Result<ParallelModeDispatchPlan, String> {
        let _ = reconcile_pool_board(self.planning_authority.as_ref(), workspace_dir);
        let context = load_pool_runtime_context(self.planning_authority.as_ref(), workspace_dir)
            .map_err(|(_, detail)| detail.to_string())?;
        let idle_slot_count = build_pool_slots(&context)
            .into_iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Idle)
            .count();
        let capacity = requested_count.min(idle_slot_count);
        let excluded_task_ids = parallel_dispatch_excluded_task_ids(&context);
        /*
        excluded_task_ids crosses two sources: live slot leases and distributor
        queue records. It is kept as a returned field so the TUI can explain why
        fewer tasks were dispatched than the planning queue appears to contain.
        */
        let failed_start_blockers = failed_start_dispatch_blockers(&context);
        let excluded = excluded_task_ids
            .iter()
            .map(|task_id| task_id.trim().to_string())
            .collect::<BTreeSet<_>>();
        let mut reported_excluded = excluded.clone();
        let candidates = planning_snapshot
            .queue_projection()
            .map(|projection| {
                projection
                    .active_tasks
                    .iter()
                    .filter(|task| {
                        let task_id = task.task_id.trim();
                        let task_updated_at =
                            DateTime::parse_from_rfc3339(task.updated_at.as_str())
                                .map(|timestamp| timestamp.timestamp_millis())
                                .ok();
                        let eligibility =
                            ParallelModeOrchestratorStateMachine::dispatch_eligibility(
                                excluded.contains(task_id),
                                failed_start_blockers.get(task_id).copied(),
                                task_updated_at,
                            );
                        if !eligibility.is_dispatchable() {
                            reported_excluded.insert(task_id.to_string());
                            return false;
                        }
                        true
                    })
                    /*
                    Capacity is applied after exclusion, not before. Otherwise a
                    leased task near the front of the queue could consume one of
                    the requested slots and hide a later dispatchable task.
                    */
                    .take(capacity)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(ParallelModeDispatchPlan {
            idle_slot_count,
            excluded_task_ids: reported_excluded.into_iter().collect(),
            candidates,
        })
    }

    /*
    orchestrator tick은 distributor queue head를 한 번 진행시키는 public entry다. queue 처리 전에
    integration worktree blocker를 먼저 검사한다. integration branch가 아니거나 dirty하면
    cherry-pick/push 흐름이 잘못된 workspace에 적용될 수 있으므로, 이 경우에는 process_queue를
    호출하지 않고 blocked result만 반환한다.
    */
    pub fn run_orchestrator_tick(
        &self,
        workspace_dir: &str,
        trigger: ParallelModeOrchestratorTrigger,
    ) -> Result<ParallelModeOrchestratorTickResult, String> {
        if let Some(blocked_notice) = inspect_akra_integration_worktree_blocker(
            self.planning_authority.as_ref(),
            workspace_dir,
        ) {
            /*
            The blocker check is deliberately outside distributor_service. The
            facade owns the public "one tick" contract and can return a blocked
            result without mutating queue records when the integration worktree is
            not safe to touch.
            */
            return Ok(ParallelModeOrchestratorTickResult {
                trigger,
                state: ParallelModeOrchestratorStateMachine::tick_state(true),
                blocked: true,
                notices: vec![blocked_notice],
            });
        }
        let notices = self.distributor_service.process_queue(workspace_dir)?;
        Ok(ParallelModeOrchestratorTickResult {
            trigger,
            state: ParallelModeOrchestratorStateMachine::tick_state(false),
            blocked: false,
            notices,
        })
    }
}

/*
기본 supervisor notice는 readiness나 pool recovery가 더 구체적인 알림을 제공하지 않을 때만
사용되는 fallback 메시지다. mode enabled와 readiness 존재 여부의 조합으로 "켜졌지만 준비 안 됨",
"꺼졌지만 검토 가능", "readiness를 다시 실행해야 함" 같은 화면 상단 문구를 고른다.
*/
fn default_supervisor_notice(
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> Option<String> {
    match (mode_enabled, readiness_snapshot) {
        (true, Some(snapshot)) if snapshot.allows_parallel_mode() => {
            Some("control tower is live in read-only supervisor mode".to_string())
        }
        (true, Some(_)) => Some("repair readiness blockers before assigning agents".to_string()),
        (false, Some(_)) => Some("run `:parallel` after reviewing the board".to_string()),
        (true, None) => Some("rerun readiness to hydrate the supervisor board".to_string()),
        (false, None) => None,
    }
}
#[cfg(test)]
mod tests;
