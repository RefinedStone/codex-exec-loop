pub use crate::application::port::inbound::parallel_mode_control_port::{
    ParallelModeOrchestratorTickResult, ParallelModeOrchestratorTrigger,
};
#[cfg(test)]
use crate::application::port::outbound::github_automation_port::DEFAULT_GITHUB_PUSH_REMOTE_NAME;
use crate::application::port::outbound::github_automation_port::{
    AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, AKRA_GITHUB_PUSH_REMOTE_ENV_VAR, GithubAutomationPort,
    resolve_github_push_remote_name_strict,
};
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::ParallelModeRuntimeEventLogRequest;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::service::parallel_agent_profile::ParallelAgentProfileService;
use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningRuntimeProjection,
};
use crate::domain::parallel_mode::{
    PARALLEL_DISPATCH_COMMAND_STALE_AFTER_SECS, ParallelModeAutomationTrigger,
    ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
    ParallelModeDispatchCommandSnapshot, ParallelModeDispatchTaskCandidate,
    ParallelModeOrchestratorStateMachine, ParallelModePoolResetPolicy, ParallelModePoolResetReport,
    ParallelModePoolSlotState, ParallelModeReadinessSnapshot, ParallelModeReadinessState,
    ParallelModeRuntimeEvent, ParallelModeRuntimeEventsSnapshot, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, ParallelModeSupervisorSnapshot, PrValidationRecord,
    PrValidationRecordKey,
};
use crate::domain::planning::PlanningOfficialCompletionRefreshContract;
use crate::domain::planning::PriorityQueueTask;
use chrono::{DateTime, Utc};
use std::path::Path;
use std::sync::Arc;
pub(crate) mod admin;
mod automation_guard;
mod branch_names;
mod completion;
pub mod control_plane;
pub(crate) mod distributor;
mod git_sequence;
mod orchestration;
mod orchestrator_loop;
mod pool;
mod pr_validation_store;
mod readiness;
mod session_detail;
mod slot_lifecycle;
pub(crate) mod supervisor;
mod support;
pub(crate) mod turn;
mod worker_commit;
pub(crate) use self::automation_guard::{
    ParallelModeAutomationGuard, ParallelModeAutomationPermit,
};
use self::branch_names::{allocate_agent_branch_name, branch_exists};
#[cfg(test)]
use self::branch_names::{sanitize_task_slug, short_branch_slug_hash};
use self::control_plane::ParallelModeControlPlaneWake;
use self::distributor::ParallelModeDistributorService;
#[cfg(test)]
use self::distributor::{
    install_after_distributor_enqueue_pool_busy_hook,
    install_after_distributor_enqueue_preflight_hook, install_before_distributor_cleanup_lock_hook,
};
use self::orchestration::{
    inspect_akra_integration_worktree_blocker, parallel_dispatch_excluded_task_ids,
    parallel_failed_start_dispatch_blockers,
};
pub use self::orchestrator_loop::{
    ParallelModeDispatchOrchestratorTickRequest, ParallelModeDispatchOrchestratorTickResult,
    ParallelModeOrchestratorLoopEvent,
};
#[cfg(test)]
use self::pool::detect_canonical_repo_root;
use self::pool::{
    PoolBoardWithContextResult, PoolMutationLock, PoolRuntimeContext, PoolSlotCleanupIdentity,
    PoolSlotCleanupLeaseAuthority, WorkspaceSlotLeaseResolution, acquire_pool_mutation_lock,
    branch_is_integrated_into, build_pool_board_with_runtime, build_pool_slots,
    cleanup_slot_to_ref_locked, derive_default_pool_root, derive_integration_worktree_path,
    inspect_pool_board_and_context_with_runtime, inspect_slot_git_status_with_runtime,
    load_pool_runtime_context_with_runtime, pool_operator_recovery_notice,
    pool_root_has_managed_state, reconcile_pool_board_and_context_with_target_locked,
    reset_pool_for_parallel_enable_with_target_locked, resolve_workspace_head_sha_with_runtime,
    resolve_workspace_slot_lease_with_runtime, rollback_slot_lease_write_failure, short_sha,
    transition_slot_lease, write_slot_lease,
};
#[cfg(test)]
use self::pool::{
    build_pool_board, cleanup_slot_to_ref_with_hooks, delete_cleaned_slot_branch_if_unchanged,
    inspect_slot_git_status, install_after_normalization_quarantine_move_hook,
    install_before_normalization_quarantine_hook, normalization_quarantine_path,
    reconcile_pool_board, reset_slot_worktree_to_ref, resolve_workspace_head_sha,
    resolve_workspace_slot_lease, slot_id, slot_lease_file_path,
};
#[cfg(all(test, unix))]
use self::pool::{
    install_before_normalization_atomic_rename_hook,
    install_before_normalization_staging_provision_hook,
};
#[cfg(test)]
use self::pr_validation_store::pr_validation_record_relative_path;
use self::pr_validation_store::{
    persist_pr_validation_record, recover_pr_validation_record_mirror,
};
use self::readiness::{
    blocked_prerequisite_capability, command_succeeds_with_runtime, inspect_authority_store,
    inspect_git_worktree, inspect_planning, inspect_planning_projection, run_command_with_runtime,
};
#[cfg(test)]
use self::readiness::{
    command_succeeds, inspect_akra_branch, inspect_gh_auth, inspect_gh_binary, inspect_push_remote,
    parse_https_remote, run_command,
};
#[cfg(test)]
use self::session_detail::{agent_session_detail_record_path, read_agent_session_detail_record};
use self::session_detail::{
    default_authority_refresh_outcome, default_validation_summary,
    format_elapsed_label_from_timestamp, lease_session_key, record_assigned_session_detail,
    record_cleaned_session_detail, record_cleanup_pending_session_detail,
    record_distributor_failed_session_detail, record_failed_start_dispatch_block,
    record_failed_start_session_detail, record_integrating_session_detail,
    record_merge_pending_session_detail, record_merge_queued_session_detail,
    record_official_completion_recovery_needed_session_detail, record_pr_pending_session_detail,
    record_pushing_session_detail, record_running_session_detail,
    record_thread_prepared_session_detail,
};
use self::supervisor::ParallelModeSupervisorService;
use self::support::discard_unstarted_slot_branch;
pub(super) use self::support::{current_branch_name, current_timestamp, ensure_directory_exists};
const AKRA_PARALLEL_INTEGRATION_BRANCH_ENV_VAR: &str = "AKRA_PARALLEL_INTEGRATION_BRANCH";
const AKRA_PARALLEL_INTEGRATION_BRANCH_CONFIG_KEY: &str = "akra.parallelIntegrationBranch";
const AKRA_PARALLEL_ALLOW_PUBLIC_REPOSITORY_ENV_VAR: &str = "AKRA_PARALLEL_ALLOW_PUBLIC_REPOSITORY";
const AKRA_PARALLEL_AUTONOMOUS_DELIVERY_ENV_VAR: &str = "AKRA_PARALLEL_AUTONOMOUS_DELIVERY";
pub(crate) const DEFAULT_PARALLEL_MODE_INTEGRATION_BRANCH: &str = "prerelease";
#[cfg(test)]
const DEFAULT_PUSH_REMOTE_NAME: &str = DEFAULT_GITHUB_PUSH_REMOTE_NAME;
const DEFAULT_POOL_SIZE: usize = 3;
const AKRA_AGENT_BRANCH_PREFIX: &str = "akra-agent";
const MAX_AGENT_BRANCH_SLUG_LEN: usize = 96;
const AGENT_BRANCH_TRUNCATION_HASH_LEN: usize = 10;
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_DETAIL: &str =
    "agent branch is not integrated into the baseline branch and has no lease metadata";
const NON_MERGED_SLOT_BRANCH_WITHOUT_LEASE_NEXT_ACTION: &str =
    "inspect the slot branch, merge or discard it manually, then rerun reconcile";

fn unavailable_integration_branch_label(error: &str) -> String {
    format!("unavailable (invalid integration branch configuration: {error})")
}

fn distributor_integration_branch_for_repo(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> String {
    try_parallel_mode_integration_branch_for_repo_with_runtime(runtime, repo_root)
        .unwrap_or_else(|error| unavailable_integration_branch_label(&error))
}

fn pool_baseline_branch_for_repo(runtime: &dyn ParallelModeRuntimePort, repo_root: &str) -> String {
    distributor_integration_branch_for_repo(runtime, repo_root)
}

pub(crate) fn parallel_mode_integration_branch_for_repo(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> Result<String, String> {
    try_parallel_mode_integration_branch_for_repo_with_runtime(runtime, repo_root)
}

fn try_parallel_mode_integration_branch_for_repo_with_runtime(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> Result<String, String> {
    let env_value = runtime.environment_variable(AKRA_PARALLEL_INTEGRATION_BRANCH_ENV_VAR)?;
    let config_value = run_command_with_runtime(
        runtime,
        "git",
        [
            "-C",
            repo_root,
            "config",
            "--get",
            AKRA_PARALLEL_INTEGRATION_BRANCH_CONFIG_KEY,
        ],
        None,
    );
    resolve_parallel_mode_integration_branch_strict(env_value.as_deref(), config_value.as_deref())
}

#[cfg(test)]
fn resolve_parallel_mode_integration_branch(
    env_value: Option<&str>,
    config_value: Option<&str>,
) -> String {
    normalize_parallel_mode_integration_branch(env_value)
        .or_else(|| normalize_parallel_mode_integration_branch(config_value))
        .unwrap_or_else(|| DEFAULT_PARALLEL_MODE_INTEGRATION_BRANCH.to_string())
}

fn resolve_parallel_mode_integration_branch_strict(
    env_value: Option<&str>,
    config_value: Option<&str>,
) -> Result<String, String> {
    if let Some(value) = env_value.filter(|value| !value.trim().is_empty()) {
        return normalize_parallel_mode_integration_branch(Some(value)).ok_or_else(|| {
            "AKRA_PARALLEL_INTEGRATION_BRANCH is invalid; delivery target fallback is disabled"
                .to_string()
        });
    }
    if let Some(value) = config_value.filter(|value| !value.trim().is_empty()) {
        return normalize_parallel_mode_integration_branch(Some(value)).ok_or_else(|| {
            "akra.parallelIntegrationBranch is invalid; delivery target fallback is disabled"
                .to_string()
        });
    }
    Ok(DEFAULT_PARALLEL_MODE_INTEGRATION_BRANCH.to_string())
}

fn try_push_remote_name_with_runtime(
    runtime: &dyn ParallelModeRuntimePort,
    repo_root: &str,
) -> Result<String, String> {
    let env_value = runtime.environment_variable(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR)?;
    let config_value = run_command_with_runtime(
        runtime,
        "git",
        [
            "-C",
            repo_root,
            "config",
            "--get",
            AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY,
        ],
        None,
    );
    resolve_github_push_remote_name_strict(env_value.as_deref(), config_value.as_deref())
        .map_err(str::to_string)
}

#[cfg(test)]
fn try_parallel_mode_integration_branch_for_repo(repo_root: &str) -> Result<String, String> {
    let runtime =
        crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter::new();
    try_parallel_mode_integration_branch_for_repo_with_runtime(&runtime, repo_root)
}

#[cfg(test)]
fn try_push_remote_name(repo_root: &str) -> Result<String, String> {
    let runtime =
        crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter::new();
    try_push_remote_name_with_runtime(&runtime, repo_root)
}

fn resolve_parent_high_risk_opt_in(
    variable_name: &str,
    value: Option<&str>,
) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some("1") => Ok(true),
        Some("0") => Ok(false),
        Some(_) => Err(format!(
            "{variable_name} must be exactly `1` or `0`; repository configuration cannot enable this policy"
        )),
    }
}

fn parent_high_risk_opt_in(
    runtime: &dyn ParallelModeRuntimePort,
    variable_name: &str,
) -> Result<bool, String> {
    let value = runtime
        .environment_variable(variable_name)
        .map_err(|error| format!("{error}; high-risk delivery remains disabled"))?;
    resolve_parent_high_risk_opt_in(variable_name, value.as_deref())
}

#[derive(Debug, Clone)]
struct ParallelModeDeliverySafetyPolicy {
    allow_public_repository: Result<bool, String>,
    allow_autonomous_delivery: Result<bool, String>,
}

impl ParallelModeDeliverySafetyPolicy {
    fn from_parent_environment(runtime: &dyn ParallelModeRuntimePort) -> Self {
        Self {
            allow_public_repository: parent_high_risk_opt_in(
                runtime,
                AKRA_PARALLEL_ALLOW_PUBLIC_REPOSITORY_ENV_VAR,
            ),
            allow_autonomous_delivery: parent_high_risk_opt_in(
                runtime,
                AKRA_PARALLEL_AUTONOMOUS_DELIVERY_ENV_VAR,
            ),
        }
    }

    #[cfg(test)]
    fn for_tests(allow_public_repository: bool, allow_autonomous_delivery: bool) -> Self {
        Self {
            allow_public_repository: Ok(allow_public_repository),
            allow_autonomous_delivery: Ok(allow_autonomous_delivery),
        }
    }
}

fn normalize_parallel_mode_integration_branch(value: Option<&str>) -> Option<String> {
    let raw_value = value?;
    if raw_value.is_empty()
        || raw_value != raw_value.trim()
        || raw_value == "HEAD"
        || raw_value == "@"
        || raw_value.starts_with('-')
        || raw_value.starts_with('/')
        || raw_value.ends_with('/')
        || raw_value.ends_with('.')
        || raw_value.contains("..")
        || raw_value.contains("@{")
        || raw_value.ends_with(".lock")
        || raw_value.chars().any(|ch| {
            ch <= '\u{1f}'
                || ch == '\u{7f}'
                || ch.is_whitespace()
                || matches!(ch, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
    {
        return None;
    }
    if raw_value
        .split('/')
        .any(|segment| segment.is_empty() || segment.starts_with('.') || segment.ends_with(".lock"))
    {
        return None;
    }
    Some(raw_value.to_string())
}
fn remote_branch_name(remote_name: &str, branch_name: &str) -> String {
    format!("{remote_name}/{branch_name}")
}
fn remote_tracking_branch_ref(remote_name: &str, branch_name: &str) -> String {
    format!(
        "refs/remotes/{}",
        remote_branch_name(remote_name, branch_name)
    )
}
#[cfg(test)]
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
    github_automation: Arc<dyn GithubAutomationPort>,
    delivery_safety_policy: ParallelModeDeliverySafetyPolicy,
    parallel_agent_profile_service: Option<ParallelAgentProfileService>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FreshPoolIntegrationTargetProof {
    repo_root: String,
    push_remote: String,
    integration_branch: String,
    credential_redacted_push_url: String,
    commit_sha: String,
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
        let delivery_safety_policy =
            ParallelModeDeliverySafetyPolicy::from_parent_environment(parallel_runtime.as_ref());
        Self {
            distributor_service: ParallelModeDistributorService::with_planning_authority(
                github_automation.clone(),
                planning_authority.clone(),
                parallel_runtime.clone(),
                delivery_safety_policy.clone(),
            ),
            supervisor_service: ParallelModeSupervisorService::new(),
            planning_authority,
            parallel_runtime,
            github_automation,
            delivery_safety_policy,
            parallel_agent_profile_service: None,
        }
    }

    pub fn with_parallel_agent_profile_service(
        mut self,
        service: ParallelAgentProfileService,
    ) -> Self {
        self.parallel_agent_profile_service = Some(service);
        self
    }

    pub fn persist_pr_validation_record(
        &self,
        workspace_dir: &str,
        pool_root: &Path,
        expected: Option<&PrValidationRecord>,
        replacement: &PrValidationRecord,
    ) -> Result<(), String> {
        persist_pr_validation_record(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            pool_root,
            expected,
            replacement,
        )
    }

    pub fn recover_pr_validation_record(
        &self,
        workspace_dir: &str,
        pool_root: &Path,
        record_key: &PrValidationRecordKey,
    ) -> Result<Option<PrValidationRecord>, String> {
        recover_pr_validation_record_mirror(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            pool_root,
            record_key,
        )
    }

    fn fetch_fresh_pool_integration_target(
        &self,
        workspace_dir: &str,
    ) -> Result<FreshPoolIntegrationTargetProof, String> {
        let repo_root = self
            .parallel_runtime
            .detect_git_repo_root(workspace_dir)
            .ok_or_else(|| "git repository is unavailable".to_string())?;
        let push_remote =
            try_push_remote_name_with_runtime(self.parallel_runtime.as_ref(), &repo_root)?;
        let integration_branch = try_parallel_mode_integration_branch_for_repo_with_runtime(
            self.parallel_runtime.as_ref(),
            &repo_root,
        )?;
        let credential_redacted_push_url = self
            .github_automation
            .credential_redacted_push_url_for_remote(&repo_root, &push_remote)
            .map_err(|error| {
                format!("credential-redacted pool delivery target could not be frozen: {error}")
            })?;
        let tracking_ref = remote_tracking_branch_ref(&push_remote, &integration_branch);
        let prior_tracking_oid = run_command_with_runtime(
            self.parallel_runtime.as_ref(),
            "git",
            ["-C", &repo_root, "rev-parse", tracking_ref.as_str()],
            None,
        );
        let authority_has_pool_state = self
            .planning_authority
            .load_runtime_projections(&repo_root)
            .map(|snapshot| {
                !snapshot.slot_leases.is_empty()
                    || !snapshot.invalid_slot_leases.is_empty()
                    || !snapshot.session_details.is_empty()
                    || !snapshot.distributor_queue_records.is_empty()
                    || !snapshot.dispatch_commands.is_empty()
            })
            .unwrap_or(true);
        let filesystem_has_pool_state = self
            .parallel_runtime
            .canonicalize_path(Path::new(&repo_root))
            .ok()
            .is_none_or(|canonical| {
                pool_root_has_managed_state(
                    self.parallel_runtime.as_ref(),
                    &derive_default_pool_root(&canonical),
                )
            });
        let existing_pool_requires_stable_observation =
            prior_tracking_oid.is_some() || authority_has_pool_state || filesystem_has_pool_state;
        let commit_sha = self
            .github_automation
            .fetch_branch_to_tracking_ref_for_delivery_target(
                &repo_root,
                &push_remote,
                &credential_redacted_push_url,
                &integration_branch,
                &tracking_ref,
            )
            .map_err(|error| {
                format!("exact pool integration target could not be fetched safely: {error}")
            })?;
        if existing_pool_requires_stable_observation
            && prior_tracking_oid.as_deref() != Some(commit_sha.as_str())
        {
            return Err(
                "pool integration target was absent or advanced during freshness verification; no slot mutation was attempted, rerun after reviewing the new baseline"
                    .to_string(),
            );
        }

        let verified_push_remote =
            try_push_remote_name_with_runtime(self.parallel_runtime.as_ref(), &repo_root)?;
        let verified_integration_branch =
            try_parallel_mode_integration_branch_for_repo_with_runtime(
                self.parallel_runtime.as_ref(),
                &repo_root,
            )?;
        let verified_push_url = self
            .github_automation
            .credential_redacted_push_url_for_remote(&repo_root, &verified_push_remote)
            .map_err(|error| {
                format!("credential-redacted pool delivery target could not be reverified: {error}")
            })?;
        if verified_push_remote != push_remote
            || verified_integration_branch != integration_branch
            || verified_push_url != credential_redacted_push_url
        {
            return Err(
                "parallel pool delivery target changed while its remote proof was being fetched"
                    .to_string(),
            );
        }

        Ok(FreshPoolIntegrationTargetProof {
            repo_root,
            push_remote,
            integration_branch,
            credential_redacted_push_url,
            commit_sha,
        })
    }

    #[cfg(test)]
    fn with_test_delivery_safety_policy(
        mut self,
        allow_public_repository: bool,
        allow_autonomous_delivery: bool,
    ) -> Self {
        let policy = ParallelModeDeliverySafetyPolicy::for_tests(
            allow_public_repository,
            allow_autonomous_delivery,
        );
        self.delivery_safety_policy = policy.clone();
        self.distributor_service.delivery_safety_policy = policy;
        self
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
        self.reserve_workspace_official_completion_refresh_order_inner(workspace_dir, None)
    }

    pub(crate) fn reserve_workspace_official_completion_refresh_order_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<Option<u64>, String> {
        self.reserve_workspace_official_completion_refresh_order_inner(
            &expected_lease.worktree_path,
            Some(expected_lease),
        )
    }

    fn reserve_workspace_official_completion_refresh_order_inner(
        &self,
        workspace_dir: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
    ) -> Result<Option<u64>, String> {
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
        )?;
        let Some(resolution) = resolve_workspace_slot_lease_with_runtime(
            self.parallel_runtime.as_ref(),
            self.planning_authority.as_ref(),
            workspace_dir,
        )?
        else {
            return Ok(None);
        };
        mutation_lock.verify_pool_root(&resolution.context.pool_root)?;
        if expected_lease.is_some_and(|expected| !resolution.lease.same_generation_as(expected)) {
            return Ok(None);
        }
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
        planning_projection: &PlanningRuntimeProjection,
    ) -> ParallelModeReadinessSnapshot {
        self.inspect_readiness_with_planning_capability(
            workspace_dir,
            inspect_planning(planning_projection),
        )
    }

    pub fn inspect_readiness_from_planning_projection(
        &self,
        workspace_dir: &str,
        planning_projection: &PlanningApplicationProjection,
    ) -> ParallelModeReadinessSnapshot {
        self.inspect_readiness_with_planning_capability(
            workspace_dir,
            inspect_planning_projection(planning_projection),
        )
    }

    pub fn inspect_readiness_passively_from_planning_projection(
        &self,
        workspace_dir: &str,
        planning_projection: &PlanningApplicationProjection,
    ) -> ParallelModeReadinessSnapshot {
        self.build_readiness_snapshot_with_planning_capability(
            workspace_dir,
            inspect_planning_projection(planning_projection),
        )
    }

    fn inspect_readiness_with_planning_capability(
        &self,
        workspace_dir: &str,
        planning: ParallelModeCapabilitySnapshot,
    ) -> ParallelModeReadinessSnapshot {
        let snapshot =
            self.build_readiness_snapshot_with_planning_capability(workspace_dir, planning);
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

    fn build_readiness_snapshot_with_planning_capability(
        &self,
        workspace_dir: &str,
        planning: ParallelModeCapabilitySnapshot,
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
        let github_capabilities = repo_root
            .as_ref()
            .map(|repo_root| self.github_automation.inspect_capabilities(repo_root));
        let akra_branch = match &repo_root {
            Some(repo_root) => {
                let exact_target =
                    try_push_remote_name_with_runtime(self.parallel_runtime.as_ref(), repo_root)
                        .and_then(|push_remote| {
                            let integration_branch =
                                try_parallel_mode_integration_branch_for_repo_with_runtime(
                                    self.parallel_runtime.as_ref(),
                                    repo_root,
                                )?;
                            let push_url = self
                                .github_automation
                                .credential_redacted_push_url_for_remote(repo_root, &push_remote)
                                .map_err(|error| error.to_string())?;
                            Ok((push_remote, integration_branch, push_url))
                        });
                match exact_target.and_then(|(push_remote, integration_branch, push_url)| {
                    self.github_automation
                        .remote_branch_head_for_delivery_target(
                            repo_root,
                            &push_remote,
                            &push_url,
                            &integration_branch,
                        )
                        .map_err(|error| error.to_string())
                        .map(|head| (push_remote, integration_branch, head))
                }) {
                    Ok((push_remote, integration_branch, Some(_))) => {
                        ParallelModeCapabilitySnapshot::new(
                            ParallelModeCapabilityKey::AkraBranch,
                            ParallelModeCapabilityState::Ready,
                            format!("{push_remote}/{integration_branch} is available"),
                            None,
                        )
                    }
                    Ok((push_remote, integration_branch, None)) => {
                        ParallelModeCapabilitySnapshot::new(
                            ParallelModeCapabilityKey::AkraBranch,
                            ParallelModeCapabilityState::Blocked,
                            format!(
                                "required remote integration branch `{push_remote}/{integration_branch}` is unavailable"
                            ),
                            Some("create the integration branch explicitly before enabling parallel mode".to_string()),
                        )
                    }
                    Err(detail) => ParallelModeCapabilitySnapshot::new(
                        ParallelModeCapabilityKey::AkraBranch,
                        ParallelModeCapabilityState::Blocked,
                        format!("integration branch could not be checked through the isolated delivery target: {detail}"),
                        Some("repair the frozen GitHub delivery target".to_string()),
                    ),
                }
            }
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::AkraBranch,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let push_remote = match &github_capabilities {
            Some(capabilities) => capabilities.push_remote.clone(),
            None => blocked_prerequisite_capability(
                ParallelModeCapabilityKey::PushRemote,
                "waiting for git repository detection",
                "enter a git repository first",
            ),
        };
        let gh_binary = github_capabilities
            .as_ref()
            .map(|capabilities| capabilities.gh_binary.clone())
            .unwrap_or_else(|| {
                blocked_prerequisite_capability(
                    ParallelModeCapabilityKey::GhBinary,
                    "waiting for git repository detection",
                    "enter a git repository first",
                )
            });
        let gh_auth = github_capabilities
            .as_ref()
            .map(|capabilities| capabilities.gh_auth.clone())
            .unwrap_or_else(|| {
                blocked_prerequisite_capability(
                    ParallelModeCapabilityKey::GhAuth,
                    "waiting for git repository detection",
                    "enter a git repository first",
                )
            });
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
        ParallelModeReadinessSnapshot::new(workspace_dir, readiness, capabilities, top_alert)
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
        let snapshot = self.supervisor_service.build_snapshot(
            self.parallel_runtime.as_ref(),
            self.planning_authority.as_ref(),
            workspace_dir,
            mode_enabled,
            readiness_snapshot,
            &self.distributor_service,
        );
        self.with_agent_profile_labels(workspace_dir, snapshot)
    }

    pub fn build_passive_supervisor_snapshot(
        &self,
        workspace_dir: &str,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeSupervisorSnapshot {
        let snapshot = self.supervisor_service.build_passive_snapshot(
            self.parallel_runtime.as_ref(),
            self.planning_authority.as_ref(),
            workspace_dir,
            readiness_snapshot,
            &self.distributor_service,
        );
        self.with_agent_profile_labels(workspace_dir, snapshot)
    }

    fn with_agent_profile_labels(
        &self,
        workspace_dir: &str,
        mut snapshot: ParallelModeSupervisorSnapshot,
    ) -> ParallelModeSupervisorSnapshot {
        let Some(service) = self.parallel_agent_profile_service.as_ref() else {
            return snapshot;
        };
        let Ok(config) = service.load_config(workspace_dir) else {
            return snapshot;
        };
        let profiles = config.enabled_profiles();
        for entry in &mut snapshot.roster.entries {
            let Some(profile) = profiles
                .iter()
                .find(|profile| profile.agent_id == entry.agent_id)
            else {
                continue;
            };
            entry.profile_display_name = Some(profile.display_name.clone());
            entry.role_label = Some(profile.role.clone());
        }
        snapshot
    }

    /*
    runtime event snapshot은 current supervisor projection과 분리된 감사/타임라인 read model이다.
    포트에서 limit/filter를 적용해 읽고, DB가 없거나 오류가 나면 화면 계층이 같은 타입을 유지할 수 있게
    unavailable empty snapshot으로 축약한다.
    */
    pub fn build_runtime_events_snapshot(
        &self,
        workspace_dir: &str,
        request: ParallelModeRuntimeEventLogRequest,
    ) -> ParallelModeRuntimeEventsSnapshot {
        self.planning_authority
            .load_runtime_event_log(workspace_dir, request)
            .unwrap_or_else(|error| {
                ParallelModeRuntimeEventsSnapshot::empty(format!(
                    "runtime event log unavailable / {error:#}"
                ))
            })
    }

    /*
    reconcile_supervisor_snapshot은 같은 supervisor snapshot을 반환하지만, mode가 켜진 상태에서 pool
    baseline/slot worktree/cleanup 상태를 기대 형태로 수렴시키는 경로다. 사용자가 병렬 모드를
    켜거나 명시적으로 refresh할 때 사용하며, 읽기 전용 snapshot과 구분해 side effect가 있는 작업을
    예측 가능하게 한다.
    */
    #[tracing::instrument(level = "trace", skip(self, readiness_snapshot))]
    pub fn reconcile_supervisor_snapshot(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    ) -> ParallelModeSupervisorSnapshot {
        self.reconcile_supervisor_snapshot_with_permit(
            workspace_dir,
            mode_enabled,
            readiness_snapshot,
            None,
        )
        .expect("unguarded supervisor reconciliation cannot reject an automation permit")
    }

    pub(crate) fn reconcile_supervisor_snapshot_guarded(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
        permit: &ParallelModeAutomationPermit,
    ) -> Result<ParallelModeSupervisorSnapshot, String> {
        self.reconcile_supervisor_snapshot_with_permit(
            workspace_dir,
            mode_enabled,
            readiness_snapshot,
            Some(permit),
        )
    }

    fn reconcile_supervisor_snapshot_with_permit(
        &self,
        workspace_dir: &str,
        mode_enabled: bool,
        readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
        permit: Option<&ParallelModeAutomationPermit>,
    ) -> Result<ParallelModeSupervisorSnapshot, String> {
        if permit.is_some_and(|permit| !permit.is_active()) {
            return Err("parallel supervisor reconciliation belongs to an inactive epoch".into());
        }
        if mode_enabled
            && readiness_snapshot.is_some_and(ParallelModeReadinessSnapshot::allows_parallel_mode)
            && let Ok(mutation_lock) = acquire_pool_mutation_lock(
                self.planning_authority.as_ref(),
                self.parallel_runtime.as_ref(),
                workspace_dir,
            )
        {
            if permit.is_some_and(|permit| !permit.is_active()) {
                return Err(
                    "parallel supervisor reconciliation belongs to an inactive epoch".into(),
                );
            }
            if let Ok(target) = self.fetch_fresh_pool_integration_target(workspace_dir) {
                let reconcile = || {
                    reconcile_pool_board_and_context_with_target_locked(
                        self.planning_authority.as_ref(),
                        self.parallel_runtime.as_ref(),
                        workspace_dir,
                        &target,
                        &mutation_lock,
                    )
                };
                if let Some(permit) = permit {
                    /*
                     * Linearize the final mutation with epoch cancellation. Cancellation
                     * remains immediate during lock/target preflight; once reconcile is
                     * accepted, Disable waits for that bounded mutation to finish.
                     */
                    if permit.with_active_commit(reconcile).is_none() {
                        return Err(
                            "parallel supervisor reconciliation belongs to an inactive epoch"
                                .into(),
                        );
                    }
                } else {
                    let _ = reconcile();
                }
            }
        }
        Ok(self.build_supervisor_snapshot(workspace_dir, mode_enabled, readiness_snapshot))
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub fn reset_pool_on_parallel_enable(&self, workspace_dir: &str) -> Result<usize, String> {
        let report = self.reset_pool_on_parallel_enable_report(workspace_dir)?;
        if report.has_reset_failures() {
            return Err(format!(
                "pool reset partially failed for {} slot(s)",
                report.failed_reset_count()
            ));
        }
        Ok(report.succeeded_reset_slot_count())
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub fn reset_pool_on_parallel_enable_report(
        &self,
        workspace_dir: &str,
    ) -> Result<ParallelModePoolResetReport, String> {
        self.reset_pool_on_parallel_enable_report_with_policy(
            workspace_dir,
            ParallelModePoolResetPolicy::ProtectLive,
        )
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub fn reset_pool_on_parallel_initial_setup_report(
        &self,
        workspace_dir: &str,
    ) -> Result<ParallelModePoolResetReport, String> {
        self.reset_pool_on_parallel_enable_report_with_policy(
            workspace_dir,
            ParallelModePoolResetPolicy::ForceDisposable,
        )
    }

    #[tracing::instrument(level = "trace", skip(self))]
    fn reset_pool_on_parallel_enable_report_with_policy(
        &self,
        workspace_dir: &str,
        policy: ParallelModePoolResetPolicy,
    ) -> Result<ParallelModePoolResetReport, String> {
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
        )?;
        let target = self.fetch_fresh_pool_integration_target(workspace_dir)?;
        reset_pool_for_parallel_enable_with_target_locked(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            policy,
            &target,
            &mutation_lock,
        )
    }

    /*
    dispatch plan은 planning queue와 pool capacity를 맞물려 계산한다. 먼저 pool을 reconcile해
    missing/cleanup 가능한 slot을 정리하고, 현재 idle slot 수만큼만 active planning task를 후보로
    뽑는다. 이미 lease 중이거나 distributor queue에 있는 task는 excluded로 제거해 같은 task가 중복
    agent에게 배정되지 않게 한다.
    */
    #[tracing::instrument(level = "trace", skip(self, planning_projection))]
    pub fn build_dispatch_plan(
        &self,
        workspace_dir: &str,
        planning_projection: &PlanningRuntimeProjection,
        requested_count: usize,
    ) -> Result<ParallelModeDispatchPlan, String> {
        let mutation_lock = acquire_pool_mutation_lock(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
        )?;
        let target = self.fetch_fresh_pool_integration_target(workspace_dir)?;
        let (context, _) = reconcile_pool_board_and_context_with_target_locked(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
            workspace_dir,
            &target,
            &mutation_lock,
        )
        .map_err(|error| error.1)?;
        let idle_slot_count = build_pool_slots(self.parallel_runtime.as_ref(), &context)
            .into_iter()
            .filter(|slot| slot.state == ParallelModePoolSlotState::Idle)
            .count();
        let excluded_task_ids = parallel_dispatch_excluded_task_ids(&context);
        let failed_start_blockers = parallel_failed_start_dispatch_blockers(&context);
        let (selection, candidates) = planning_projection
            .queue_projection()
            .map(|projection| {
                let active_task_inputs = projection
                    .active_tasks
                    .iter()
                    .map(|task| {
                        ParallelModeDispatchTaskCandidate::new(
                            task.task_id.clone(),
                            DateTime::parse_from_rfc3339(task.updated_at.as_str())
                                .map(|timestamp| timestamp.timestamp_millis())
                                .ok(),
                        )
                    })
                    .collect::<Vec<_>>();
                let selection = ParallelModeOrchestratorStateMachine::select_dispatch_candidates(
                    idle_slot_count,
                    requested_count,
                    excluded_task_ids.clone(),
                    &failed_start_blockers,
                    active_task_inputs,
                );
                let mut selected_task_ids = selection.selected_task_ids.iter();
                let mut next_selected_task_id = selected_task_ids.next();
                let candidates = projection
                    .active_tasks
                    .iter()
                    .filter_map(|task| {
                        let expected_task_id = next_selected_task_id?;
                        if task.task_id.trim() != expected_task_id {
                            return None;
                        }
                        next_selected_task_id = selected_task_ids.next();
                        Some(task.clone())
                    })
                    .collect::<Vec<_>>();
                (selection, candidates)
            })
            .unwrap_or_else(|| {
                (
                    ParallelModeOrchestratorStateMachine::select_dispatch_candidates(
                        idle_slot_count,
                        requested_count,
                        excluded_task_ids.clone(),
                        &failed_start_blockers,
                        Vec::new(),
                    ),
                    Vec::new(),
                )
            });
        Ok(ParallelModeDispatchPlan {
            idle_slot_count: selection.idle_slot_count,
            excluded_task_ids: selection.excluded_task_ids,
            candidates,
        })
    }

    pub fn enqueue_dispatch_commands_for_event(
        &self,
        workspace_dir: &str,
        event: ParallelModeRuntimeEvent,
        planning_projection: &PlanningRuntimeProjection,
        epoch_id: Option<u64>,
    ) -> Result<usize, String> {
        if event == ParallelModeRuntimeEvent::ModeDisabled {
            return self
                .planning_authority
                .cancel_runtime_dispatch_commands(workspace_dir, "parallel mode disabled")
                .map_err(|error| error.to_string());
        }
        let queue_head_signature = planning_projection
            .queue_head_task_signature()
            .map(|signature| signature.to_string())
            .or_else(|| {
                planning_projection
                    .queue_head()
                    .map(|task| task.task_id.clone())
            });
        let commands = ParallelModeOrchestratorStateMachine::runtime_dispatch_commands(
            true,
            event,
            planning_projection.has_actionable_queue_head(),
            queue_head_signature,
            epoch_id,
            current_timestamp(),
        );
        let mut inserted = 0;
        for command in commands {
            if self
                .planning_authority
                .enqueue_runtime_dispatch_command(workspace_dir, &command)
                .map_err(|error| error.to_string())?
            {
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    pub fn claim_next_dispatch_command(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<ParallelModeDispatchCommandSnapshot>, String> {
        self.planning_authority
            .try_claim_next_runtime_dispatch_command(
                workspace_dir,
                &dispatch_command_owner_token(self.parallel_runtime.as_ref()),
            )
            .map_err(|error| error.to_string())
    }

    pub fn pending_dispatch_command_count(&self, workspace_dir: &str) -> Result<usize, String> {
        let snapshot = self
            .planning_authority
            .load_runtime_projections(workspace_dir)
            .map_err(|error| error.to_string())?;
        Ok(snapshot
            .dispatch_commands
            .iter()
            .filter(|command| Self::dispatch_command_is_claimable(command))
            .count())
    }

    fn dispatch_command_is_claimable(command: &ParallelModeDispatchCommandSnapshot) -> bool {
        match command.state {
            crate::domain::parallel_mode::ParallelModeDispatchCommandState::Pending => true,
            crate::domain::parallel_mode::ParallelModeDispatchCommandState::Running => {
                DateTime::parse_from_rfc3339(command.updated_at.as_str())
                    .map(|timestamp| {
                        Utc::now()
                            .signed_duration_since(timestamp.with_timezone(&Utc))
                            .num_seconds()
                            >= PARALLEL_DISPATCH_COMMAND_STALE_AFTER_SECS
                    })
                    .unwrap_or(true)
            }
            _ => false,
        }
    }

    pub fn pending_dispatch_wake(
        &self,
        workspace_dir: &str,
        epoch_id: u64,
    ) -> Result<Option<ParallelModeControlPlaneWake>, String> {
        let snapshot = self
            .planning_authority
            .load_runtime_projections(workspace_dir)
            .map_err(|error| error.to_string())?;
        if let Some(command) = Self::next_current_epoch_dispatch_command(&snapshot, epoch_id) {
            return Ok(Some(ParallelModeControlPlaneWake::new(
                workspace_dir,
                command.trigger,
                command.epoch_id.unwrap_or(epoch_id),
                None,
            )));
        }
        if let Some(command) =
            Self::next_orphaned_current_epoch_running_dispatch_command(&snapshot, epoch_id)
        {
            return Ok(Some(ParallelModeControlPlaneWake::new(
                workspace_dir,
                command.trigger,
                epoch_id,
                Some(command.trigger),
            )));
        }
        let Some(command) = Self::next_stale_epoch_dispatch_command(&snapshot, epoch_id) else {
            return Ok(None);
        };
        Ok(Some(ParallelModeControlPlaneWake::new(
            workspace_dir,
            command.trigger,
            epoch_id,
            Some(command.trigger),
        )))
    }

    fn next_current_epoch_dispatch_command(
        snapshot: &PlanningAuthorityRuntimeProjectionSnapshot,
        epoch_id: u64,
    ) -> Option<ParallelModeDispatchCommandSnapshot> {
        Self::next_claimable_dispatch_command_by(snapshot, |command| {
            command
                .epoch_id
                .is_none_or(|command_epoch_id| command_epoch_id == epoch_id)
        })
    }

    fn next_stale_epoch_dispatch_command(
        snapshot: &PlanningAuthorityRuntimeProjectionSnapshot,
        epoch_id: u64,
    ) -> Option<ParallelModeDispatchCommandSnapshot> {
        Self::next_claimable_dispatch_command_by(snapshot, |command| {
            command
                .epoch_id
                .is_some_and(|command_epoch_id| command_epoch_id != epoch_id)
        })
    }

    fn next_orphaned_current_epoch_running_dispatch_command(
        snapshot: &PlanningAuthorityRuntimeProjectionSnapshot,
        epoch_id: u64,
    ) -> Option<ParallelModeDispatchCommandSnapshot> {
        if !snapshot.session_details.is_empty() || !snapshot.slot_leases.is_empty() {
            return None;
        }
        snapshot
            .dispatch_commands
            .iter()
            .filter(|command| {
                command.state
                    == crate::domain::parallel_mode::ParallelModeDispatchCommandState::Running
                    && !Self::dispatch_command_is_claimable(command)
                    && command.epoch_id == Some(epoch_id)
            })
            .min_by(|left, right| {
                left.updated_at
                    .cmp(&right.updated_at)
                    .then_with(|| left.command_id.cmp(&right.command_id))
            })
            .cloned()
    }
    fn next_claimable_dispatch_command_by(
        snapshot: &PlanningAuthorityRuntimeProjectionSnapshot,
        mut predicate: impl FnMut(&ParallelModeDispatchCommandSnapshot) -> bool,
    ) -> Option<ParallelModeDispatchCommandSnapshot> {
        if let Some(command) = snapshot.dispatch_commands.iter().find(|command| {
            command.state == crate::domain::parallel_mode::ParallelModeDispatchCommandState::Pending
                && predicate(command)
        }) {
            return Some(command.clone());
        }
        snapshot
            .dispatch_commands
            .iter()
            .filter(|command| {
                command.state
                    == crate::domain::parallel_mode::ParallelModeDispatchCommandState::Running
                    && Self::dispatch_command_is_claimable(command)
                    && predicate(command)
            })
            .min_by(|left, right| {
                left.updated_at
                    .cmp(&right.updated_at)
                    .then_with(|| left.command_id.cmp(&right.command_id))
            })
            .cloned()
    }

    pub fn update_dispatch_command(
        &self,
        workspace_dir: &str,
        command: &ParallelModeDispatchCommandSnapshot,
    ) -> Result<(), String> {
        self.planning_authority
            .update_runtime_dispatch_command(workspace_dir, command)
            .map_err(|error| error.to_string())
    }

    pub fn recover_fresh_running_dispatch_command(
        &self,
        workspace_dir: &str,
        planning_projection: &PlanningRuntimeProjection,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    ) -> Result<bool, String> {
        let queue_head_signature = planning_projection
            .queue_head_task_signature()
            .map(|signature| signature.to_string())
            .or_else(|| {
                planning_projection
                    .queue_head()
                    .map(|task| task.task_id.clone())
            });
        let replacement = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
            trigger,
            queue_head_signature,
            Some(epoch_id),
            current_timestamp(),
        );
        let snapshot = self
            .planning_authority
            .load_runtime_projections(workspace_dir)
            .map_err(|error| error.to_string())?;
        let current_task_id = planning_projection
            .queue_head()
            .map(|task| task.task_id.clone());
        if !snapshot.session_details.is_empty() {
            return Ok(false);
        }
        let has_orphaned_running = snapshot.dispatch_commands.iter().any(|command| {
            command.command_id == replacement.command_id
                && command.state
                    == crate::domain::parallel_mode::ParallelModeDispatchCommandState::Running
                && !Self::dispatch_command_is_claimable(command)
                && command.epoch_id == Some(epoch_id)
        });
        if !has_orphaned_running {
            return Ok(false);
        }
        let has_matching_slot_lease = current_task_id.as_ref().is_some_and(|task_id| {
            snapshot
                .slot_leases
                .values()
                .any(|lease| lease.task_id == *task_id)
        });
        if has_matching_slot_lease {
            return Ok(false);
        }
        self.update_dispatch_command(workspace_dir, &replacement)?;
        Ok(true)
    }

    pub fn cancel_dispatch_commands(
        &self,
        workspace_dir: &str,
        reason: &str,
    ) -> Result<usize, String> {
        self.planning_authority
            .cancel_runtime_dispatch_commands(workspace_dir, reason)
            .map_err(|error| error.to_string())
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
        self.run_orchestrator_tick_with_permit(workspace_dir, trigger, None)
    }

    pub(crate) fn pending_commit_ready_recovery_signature(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<String>, String> {
        self.distributor_service
            .pending_commit_ready_recovery_signature(workspace_dir)
    }

    pub(crate) fn run_orchestrator_tick_guarded(
        &self,
        workspace_dir: &str,
        trigger: ParallelModeOrchestratorTrigger,
        permit: &ParallelModeAutomationPermit,
    ) -> Result<ParallelModeOrchestratorTickResult, String> {
        self.run_orchestrator_tick_with_permit(workspace_dir, trigger, Some(permit))
    }

    fn run_orchestrator_tick_with_permit(
        &self,
        workspace_dir: &str,
        trigger: ParallelModeOrchestratorTrigger,
        permit: Option<&ParallelModeAutomationPermit>,
    ) -> Result<ParallelModeOrchestratorTickResult, String> {
        if permit.is_some_and(|permit| !permit.is_active()) {
            return Ok(ParallelModeOrchestratorTickResult {
                trigger,
                state: ParallelModeOrchestratorStateMachine::tick_state(true),
                blocked: true,
                notices: vec![
                    "parallel automation epoch closed before orchestrator delivery".to_string(),
                ],
            });
        }
        if let Some(blocked_notice) = inspect_akra_integration_worktree_blocker(
            self.planning_authority.as_ref(),
            self.parallel_runtime.as_ref(),
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
        let notices = match permit {
            Some(permit) => self
                .distributor_service
                .process_queue_guarded(workspace_dir, permit)?,
            None => self.distributor_service.process_queue(workspace_dir)?,
        };
        Ok(ParallelModeOrchestratorTickResult {
            trigger,
            state: ParallelModeOrchestratorStateMachine::tick_state(false),
            blocked: false,
            notices,
        })
    }
}

fn dispatch_command_owner_token(runtime: &dyn ParallelModeRuntimePort) -> String {
    format!(
        "pid={} created_at={}",
        runtime.current_process_id(),
        current_timestamp()
    )
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
