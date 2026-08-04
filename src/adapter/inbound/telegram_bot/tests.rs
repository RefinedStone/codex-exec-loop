#[cfg(any(unix, windows))]
use super::config::read_telegram_environment_file;
use super::config::{
    TelegramBotEnvironment, apply_environment_file, load_environment_from_sources,
    parse_args_with_environment, validate_telegram_process_environment_keys,
};
#[cfg(unix)]
use super::config::{telegram_env_file_path_if_present, utf8_environment_value};
use super::{
    TelegramBotPolicy, TelegramBotRunner, TelegramBotRuntimeConfig, TelegramInboundCommand,
    TelegramParsedMessage, parse_message, telegram_max_curl_request_seconds,
    telegram_runner_lease_ttl_seconds,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::inbound::parallel_mode_control_port::{
    ParallelModeControlPort, ParallelModeControlStatusSnapshot, ParallelModeOrchestratorTickResult,
};
use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterRepositoryPort,
    ReviewCenterThreadProjection,
};
use crate::application::port::outbound::telegram_bot_port::{
    TelegramBotIdentity, TelegramBotPort, TelegramInboundMessage, TelegramPollRequest,
    TelegramSendMessageRequest, TelegramUpdate,
};
use crate::application::port::outbound::telegram_global_runner_lease_port::{
    TelegramGlobalRunnerLeaseClaimDecision, TelegramGlobalRunnerLeasePort,
    TelegramWorkspaceBindingPolicy,
};
use crate::application::port::outbound::telegram_update_ledger_port::{
    TelegramRunnerLeaseClaimDecision, TelegramUpdateClaimDecision, TelegramUpdateLedgerPort,
};
use crate::application::service::planning::PlanningResetTarget;
use crate::application::service::planning::control::{
    PlanningControlCommand, PlanningControlQueueEntry, PlanningControlResetOutcome,
    PlanningControlService, PlanningControlStatusSnapshot, PlanningControlSurface,
};
use crate::application::service::review_center::ReviewCenterReadService;
use anyhow::{Context, Result, anyhow, bail};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(any(unix, windows))]
use std::{fs, time::SystemTime};

/*
 * These tests pin the Telegram adapter as a thin remote-control surface over
 * planning control. The fixtures avoid network access and the real planning
 * store, but they preserve the same command parser, allowlist policy, poll-loop
 * resilience, and Korean operator copy that production chat users see.
 */
struct FakePlanningControlSurface;

impl PlanningControlSurface for FakePlanningControlSurface {
    fn workspace_dir(&self) -> &str {
        "/tmp/repo"
    }

    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        /*
         * Keep this snapshot intentionally small but not empty. Telegram status
         * replies are read outside the TUI, so the queue head title is the
         * easiest durable signal that the inbound adapter reached the shared
         * planning-control service instead of rendering a Telegram-only stub.
         */
        Ok(PlanningControlStatusSnapshot {
            workspace_dir: "/tmp/repo".to_string(),
            planning_state: "ready".to_string(),
            task_authority_signature: Some(42),
            queue_head_task_signature: Some(7),
            queue_summary: Some("queue head ready".to_string()),
            proposal_summary: None,
            health: Some("planning workspace ready".to_string()),
            issue: None,
            note: None,
            preview_status_label: "queue ready".to_string(),
            preview_detail: None,
            queue_head: Some(PlanningControlQueueEntry {
                task_id: "task-1".to_string(),
                task_title: "Ship Telegram control".to_string(),
                direction_id: "general-workstream".to_string(),
                status: "ready".to_string(),
                combined_priority: 90,
            }),
            visible_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
        })
    }

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        /*
         * Reset tests do not need file IO; they need proof that Telegram target
         * words have already been mapped into the same PlanningResetTarget labels
         * used by admin and TUI control surfaces.
         */
        Ok(PlanningControlResetOutcome {
            target: target.label().to_string(),
            rewritten_paths: vec!["DB task authority".to_string()],
            removed_paths: Vec::new(),
            planning_state: "ready".to_string(),
            health: Some("queue reset complete".to_string()),
            issue: None,
        })
    }
}

struct FlakyPlanningControlSurface {
    load_calls: AtomicUsize,
}

struct CountingPlanningControlSurface {
    reset_calls: Arc<AtomicUsize>,
    status_calls: Arc<AtomicUsize>,
}

impl PlanningControlSurface for CountingPlanningControlSurface {
    fn workspace_dir(&self) -> &str {
        "/tmp/repo"
    }

    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        self.status_calls.fetch_add(1, Ordering::SeqCst);
        FakePlanningControlSurface.load_status_snapshot()
    }

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        self.reset_calls.fetch_add(1, Ordering::SeqCst);
        FakePlanningControlSurface.reset_workspace(target)
    }
}

impl PlanningControlSurface for FlakyPlanningControlSurface {
    fn workspace_dir(&self) -> &str {
        "/tmp/repo"
    }

    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        let call = self.load_calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            /*
             * The first status call simulates a service-layer failure after
             * parsing and authorization have already succeeded. The runner must
             * convert that into one failed reply without poisoning the next
             * update in the same Telegram batch.
             */
            bail!("temporary planning failure");
        }
        FakePlanningControlSurface.load_status_snapshot()
    }
    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        FakePlanningControlSurface.reset_workspace(target)
    }
}

struct FakeTelegramParallelControlSurface;

impl ParallelModeControlPort for FakeTelegramParallelControlSurface {
    fn load_status(
        &self,
        _workspace_dir: &str,
        _recent_event_limit: usize,
    ) -> std::result::Result<ParallelModeControlStatusSnapshot, String> {
        Ok(ParallelModeControlStatusSnapshot {
            readiness_label: "ready".to_string(),
            reconcile_status: "supervised".to_string(),
            active_agent_count: 1,
            queue_depth: 2,
            visible_event_count: 3,
        })
    }

    fn run_manual_orchestrator_tick(
        &self,
        _workspace_dir: &str,
    ) -> std::result::Result<ParallelModeOrchestratorTickResult, String> {
        Err("manual tick is not used by Telegram tests".to_string())
    }
}

#[derive(Default)]
struct FakeTelegramBotPort {
    /*
     * Stored in reverse-pop order so each test can script a poll transcript
     * without an async runtime or real Telegram HTTP state. That keeps offset
     * assertions focused on runner behavior rather than mock bookkeeping.
     */
    poll_errors: Mutex<Vec<anyhow::Error>>,
    updates: Mutex<Vec<Vec<TelegramUpdate>>>,
    poll_requests: Mutex<Vec<TelegramPollRequest>>,
    // Captured send requests are the observable side effect for runner tests.
    sent_messages: Mutex<Vec<TelegramSendMessageRequest>>,
    authenticated_bot_id: Mutex<Option<u64>>,
}

#[derive(Default)]
struct FakeTelegramUpdateLedger {
    state: Mutex<FakeTelegramUpdateLedgerState>,
}

#[derive(Default)]
struct PermissiveTelegramGlobalRunnerLease;

impl TelegramGlobalRunnerLeasePort for PermissiveTelegramGlobalRunnerLease {
    fn try_acquire_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _lease_ttl_seconds: u64,
        _binding_policy: TelegramWorkspaceBindingPolicy,
    ) -> Result<TelegramGlobalRunnerLeaseClaimDecision> {
        Ok(TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 })
    }

    fn renew_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _generation: u64,
        _lease_ttl_seconds: u64,
    ) -> Result<()> {
        Ok(())
    }

    fn release_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _generation: u64,
    ) -> Result<bool> {
        Ok(true)
    }
}

fn permissive_global_runner_lease() -> Arc<dyn TelegramGlobalRunnerLeasePort> {
    Arc::new(PermissiveTelegramGlobalRunnerLease)
}

struct ExpiringAfterPollGlobalRunnerLease {
    renew_calls: AtomicUsize,
}

struct MismatchedTelegramWorkspaceBinding {
    bound_workspace_dir: String,
}

impl TelegramGlobalRunnerLeasePort for MismatchedTelegramWorkspaceBinding {
    fn try_acquire_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _lease_ttl_seconds: u64,
        _binding_policy: TelegramWorkspaceBindingPolicy,
    ) -> Result<TelegramGlobalRunnerLeaseClaimDecision> {
        Ok(
            TelegramGlobalRunnerLeaseClaimDecision::WorkspaceBindingMismatch {
                bound_workspace_dir: self.bound_workspace_dir.clone(),
            },
        )
    }

    fn renew_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _generation: u64,
        _lease_ttl_seconds: u64,
    ) -> Result<()> {
        bail!("mismatched binding must never renew")
    }

    fn release_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _generation: u64,
    ) -> Result<bool> {
        Ok(false)
    }
}

impl TelegramGlobalRunnerLeasePort for ExpiringAfterPollGlobalRunnerLease {
    fn try_acquire_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _lease_ttl_seconds: u64,
        _binding_policy: TelegramWorkspaceBindingPolicy,
    ) -> Result<TelegramGlobalRunnerLeaseClaimDecision> {
        Ok(TelegramGlobalRunnerLeaseClaimDecision::Acquired { generation: 1 })
    }

    fn renew_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _generation: u64,
        _lease_ttl_seconds: u64,
    ) -> Result<()> {
        let call = self.renew_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call >= 2 {
            bail!("injected global Telegram lease expiry and replacement");
        }
        Ok(())
    }

    fn release_global_runner_lease(
        &self,
        _canonical_workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _owner_pid: u32,
        _generation: u64,
    ) -> Result<bool> {
        Ok(false)
    }
}

#[derive(Default)]
struct FakeTelegramUpdateLedgerState {
    next_offset: Option<i64>,
    updates: BTreeMap<i64, &'static str>,
    lease_owner: Option<String>,
}

impl TelegramUpdateLedgerPort for FakeTelegramUpdateLedger {
    fn try_acquire_runner_lease(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        owner_token: &str,
        _lease_ttl_seconds: u64,
    ) -> Result<TelegramRunnerLeaseClaimDecision> {
        let mut state = self.state.lock().expect("ledger mutex should lock");
        match state.lease_owner.as_deref() {
            None => {
                state.lease_owner = Some(owner_token.to_string());
                Ok(TelegramRunnerLeaseClaimDecision::Acquired)
            }
            Some(owner) if owner == owner_token => Ok(TelegramRunnerLeaseClaimDecision::Acquired),
            Some(_) => Ok(TelegramRunnerLeaseClaimDecision::ActiveRunner),
        }
    }

    fn renew_runner_lease(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        owner_token: &str,
        _lease_ttl_seconds: u64,
    ) -> Result<()> {
        let state = self.state.lock().expect("ledger mutex should lock");
        if state.lease_owner.as_deref() != Some(owner_token) {
            bail!("fake Telegram runner lease is not owned");
        }
        Ok(())
    }

    fn release_runner_lease(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        owner_token: &str,
    ) -> Result<bool> {
        let mut state = self.state.lock().expect("ledger mutex should lock");
        if state.lease_owner.as_deref() != Some(owner_token) {
            return Ok(false);
        }
        state.lease_owner = None;
        Ok(true)
    }

    fn load_cursor_and_recover_inflight(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
    ) -> Result<Option<i64>> {
        let mut state = self.state.lock().expect("ledger mutex should lock");
        let interrupted = state
            .updates
            .iter()
            .filter_map(|(update_id, status)| (*status == "executing").then_some(*update_id))
            .max();
        if let Some(update_id) = interrupted {
            for status in state.updates.values_mut() {
                if *status == "executing" {
                    *status = "completed";
                }
            }
            let candidate = update_id
                .checked_add(1)
                .expect("test update should advance");
            state.next_offset = Some(
                state
                    .next_offset
                    .map_or(candidate, |old| old.max(candidate)),
            );
        }
        Ok(state.next_offset)
    }

    fn advance_cursor(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        next_offset: Option<i64>,
    ) -> Result<Option<i64>> {
        let mut state = self.state.lock().expect("ledger mutex should lock");
        if let Some(candidate) = next_offset {
            state.next_offset = Some(
                state
                    .next_offset
                    .map_or(candidate, |old| old.max(candidate)),
            );
        }
        Ok(state.next_offset)
    }

    fn claim_update(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        update_id: i64,
    ) -> Result<TelegramUpdateClaimDecision> {
        let mut state = self.state.lock().expect("ledger mutex should lock");
        if state.next_offset.is_some_and(|offset| update_id < offset) {
            return Ok(TelegramUpdateClaimDecision::AlreadyCompleted);
        }
        match state.updates.get(&update_id) {
            Some(&"completed") => Ok(TelegramUpdateClaimDecision::AlreadyCompleted),
            Some(&"executing") => Ok(TelegramUpdateClaimDecision::InFlight),
            Some(unknown) => bail!("unexpected fake ledger state {unknown}"),
            None => {
                state.updates.insert(update_id, "executing");
                Ok(TelegramUpdateClaimDecision::Execute)
            }
        }
    }

    fn complete_update(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        update_id: i64,
    ) -> Result<Option<i64>> {
        let mut state = self.state.lock().expect("ledger mutex should lock");
        let status = state
            .updates
            .get_mut(&update_id)
            .context("fake update must be claimed")?;
        *status = "completed";
        let candidate = update_id
            .checked_add(1)
            .expect("test update should advance");
        state.next_offset = Some(
            state
                .next_offset
                .map_or(candidate, |old| old.max(candidate)),
        );
        Ok(state.next_offset)
    }
}

struct RejectingTelegramUpdateLedger;

impl TelegramUpdateLedgerPort for RejectingTelegramUpdateLedger {
    fn try_acquire_runner_lease(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _lease_ttl_seconds: u64,
    ) -> Result<TelegramRunnerLeaseClaimDecision> {
        Ok(TelegramRunnerLeaseClaimDecision::Acquired)
    }

    fn renew_runner_lease(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _lease_ttl_seconds: u64,
    ) -> Result<()> {
        Ok(())
    }

    fn release_runner_lease(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
    ) -> Result<bool> {
        Ok(true)
    }

    fn load_cursor_and_recover_inflight(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
    ) -> Result<Option<i64>> {
        Ok(None)
    }

    fn advance_cursor(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        next_offset: Option<i64>,
    ) -> Result<Option<i64>> {
        Ok(next_offset)
    }

    fn claim_update(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _update_id: i64,
    ) -> Result<TelegramUpdateClaimDecision> {
        bail!("injected Telegram ledger failure")
    }

    fn complete_update(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _update_id: i64,
    ) -> Result<Option<i64>> {
        bail!("completion should not be reached")
    }
}

struct CompletionFailingTelegramUpdateLedger {
    delegate: Arc<dyn TelegramUpdateLedgerPort>,
}

impl TelegramUpdateLedgerPort for CompletionFailingTelegramUpdateLedger {
    fn try_acquire_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        lease_ttl_seconds: u64,
    ) -> Result<TelegramRunnerLeaseClaimDecision> {
        self.delegate.try_acquire_runner_lease(
            workspace_dir,
            stream_key,
            owner_token,
            lease_ttl_seconds,
        )
    }

    fn renew_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        lease_ttl_seconds: u64,
    ) -> Result<()> {
        self.delegate
            .renew_runner_lease(workspace_dir, stream_key, owner_token, lease_ttl_seconds)
    }

    fn release_runner_lease(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<bool> {
        self.delegate
            .release_runner_lease(workspace_dir, stream_key, owner_token)
    }

    fn load_cursor_and_recover_inflight(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
    ) -> Result<Option<i64>> {
        self.delegate
            .load_cursor_and_recover_inflight(workspace_dir, stream_key, owner_token)
    }

    fn advance_cursor(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        next_offset: Option<i64>,
    ) -> Result<Option<i64>> {
        self.delegate
            .advance_cursor(workspace_dir, stream_key, owner_token, next_offset)
    }

    fn claim_update(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        owner_token: &str,
        update_id: i64,
    ) -> Result<TelegramUpdateClaimDecision> {
        self.delegate
            .claim_update(workspace_dir, stream_key, owner_token, update_id)
    }

    fn complete_update(
        &self,
        _workspace_dir: &str,
        _stream_key: &str,
        _owner_token: &str,
        _update_id: i64,
    ) -> Result<Option<i64>> {
        bail!("injected crash before Telegram update completion")
    }
}

impl TelegramBotPort for FakeTelegramBotPort {
    fn get_me(&self) -> Result<TelegramBotIdentity> {
        Ok(TelegramBotIdentity {
            bot_id: self
                .authenticated_bot_id
                .lock()
                .expect("bot identity mutex should lock")
                .unwrap_or(123_456),
        })
    }

    fn get_updates(&self, request: &TelegramPollRequest) -> Result<Vec<TelegramUpdate>> {
        self.poll_requests
            .lock()
            .expect("poll request mutex should lock")
            .push(request.clone());
        if let Some(error) = self
            .poll_errors
            .lock()
            .expect("poll error mutex should lock")
            .pop()
        {
            return Err(error);
        }
        Ok(self
            .updates
            .lock()
            .expect("updates mutex should lock")
            .pop()
            .unwrap_or_default())
    }
    fn send_message(&self, request: &TelegramSendMessageRequest) -> Result<()> {
        self.sent_messages
            .lock()
            .expect("sent messages mutex should lock")
            .push(request.clone());
        Ok(())
    }
}

fn build_runner(allowed_chat_ids: &[i64]) -> (Arc<FakeTelegramBotPort>, TelegramBotRunner) {
    let gateway = Arc::new(FakeTelegramBotPort::default());
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        permissive_global_runner_lease(),
        Arc::new(FakeTelegramUpdateLedger::default()),
        PlanningControlService::new(Arc::new(FakePlanningControlSurface)),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            "/tmp/repo",
            "test-stream",
            TelegramBotPolicy::new(allowed_chat_ids.iter().copied().collect(), BTreeSet::new()),
        ),
    );
    runner
        .acquire_runner_lease()
        .expect("fake Telegram runner lease should acquire");
    (gateway, runner)
}

fn test_runtime_config(
    workspace_dir: &str,
    stream_key: &str,
    policy: TelegramBotPolicy,
) -> TelegramBotRuntimeConfig {
    TelegramBotRuntimeConfig {
        workspace_dir: workspace_dir.to_string(),
        binding_workspace_dir: workspace_dir.to_string(),
        stream_key: stream_key.to_string(),
        policy,
        // A one-second timeout keeps fake polling narrow without affecting network tests.
        poll_timeout_seconds: 1,
        drop_pending_updates: false,
        binding_policy: TelegramWorkspaceBindingPolicy::Preserve,
        failure_backoff: Duration::ZERO,
    }
}

fn temp_telegram_workspace(prefix: &str) -> String {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "akra-telegram-ledger-{prefix}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("Telegram ledger workspace should exist");
    path.display().to_string()
}

fn counting_control_service(
    reset_calls: Arc<AtomicUsize>,
    status_calls: Arc<AtomicUsize>,
) -> PlanningControlService {
    PlanningControlService::new(Arc::new(CountingPlanningControlSurface {
        reset_calls,
        status_calls,
    }))
}

#[derive(Default)]
struct FakeReviewCenterRepository {
    inbox: Vec<ReviewCenterInboxItem>,
    history: Vec<ReviewCenterHistoryEntry>,
    thread_reviews: Vec<ReviewCenterThreadProjection>,
}

impl ReviewCenterRepositoryPort for FakeReviewCenterRepository {
    fn load_thread_reviews(
        &self,
        _workspace_dir: &str,
        _thread_id: &str,
    ) -> Result<Vec<ReviewCenterThreadProjection>> {
        Ok(self.thread_reviews.clone())
    }

    fn load_pending_inbox(&self, _workspace_dir: &str) -> Result<Vec<ReviewCenterInboxItem>> {
        Ok(self.inbox.clone())
    }

    fn load_recent_history(&self, _workspace_dir: &str) -> Result<Vec<ReviewCenterHistoryEntry>> {
        Ok(self.history.clone())
    }

    fn upsert_thread_review(
        &self,
        _workspace_dir: &str,
        _review: &ReviewCenterThreadProjection,
    ) -> Result<()> {
        Ok(())
    }

    fn replace_pending_inbox(
        &self,
        _workspace_dir: &str,
        _inbox: &[ReviewCenterInboxItem],
    ) -> Result<()> {
        Ok(())
    }

    fn append_history_entry(
        &self,
        _workspace_dir: &str,
        _entry: &ReviewCenterHistoryEntry,
    ) -> Result<()> {
        Ok(())
    }
}

fn build_runner_with_reviews(
    allowed_chat_ids: &[i64],
    repository: Arc<FakeReviewCenterRepository>,
) -> (Arc<FakeTelegramBotPort>, TelegramBotRunner) {
    let (gateway, runner) = build_runner(allowed_chat_ids);
    (
        gateway,
        runner.with_review_center_query_port(ReviewCenterReadService::new("/tmp/repo", repository)),
    )
}

// Parser tests protect the user-facing chat grammar before service dispatch is involved.
#[test]
fn parse_message_accepts_plan_status_command_with_bot_mention() {
    /*
     * Group chats append the bot username to slash commands. This case protects
     * the normalization step that strips `@AkraBot` before the adapter maps
     * `/plan status` onto the shared planning Status command.
     */
    let parsed = parse_message(Some("/plan@AkraBot status"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Command(TelegramInboundCommand::Planning(
            PlanningControlCommand::Status
        ))
    );
}

#[test]
fn parse_message_maps_supported_planning_commands_to_shared_control_enum() {
    /*
     * Telegram command spelling is transport vocabulary. Planning operations
     * must leave the parser as PlanningControlCommand so CLI and Telegram keep
     * the same application command surface.
     */
    for (raw, expected) in [
        ("/start", PlanningControlCommand::Help),
        ("/help", PlanningControlCommand::Help),
        ("help", PlanningControlCommand::Help),
        ("/status", PlanningControlCommand::Status),
        ("status", PlanningControlCommand::Status),
        ("/queue", PlanningControlCommand::Queue),
        ("queue", PlanningControlCommand::Queue),
        ("/plan", PlanningControlCommand::Status),
        ("/plan status", PlanningControlCommand::Status),
        (
            "/reset queue",
            PlanningControlCommand::Reset(PlanningResetTarget::Queue),
        ),
        (
            "/reset_directions",
            PlanningControlCommand::Reset(PlanningResetTarget::Directions),
        ),
        (
            "/reset_all",
            PlanningControlCommand::Reset(PlanningResetTarget::All),
        ),
    ] {
        assert_eq!(
            parse_message(Some(raw)),
            TelegramParsedMessage::Command(TelegramInboundCommand::Planning(expected)),
            "Telegram input `{raw}` should map to the shared planning control command"
        );
    }
}

#[test]
fn parse_message_maps_parallel_status_to_parallel_control_surface() {
    for raw in [
        "/parallel",
        "parallel",
        "/parallel status",
        "/parallel_status",
    ] {
        assert_eq!(
            parse_message(Some(raw)),
            TelegramParsedMessage::Command(TelegramInboundCommand::ParallelStatus),
            "Telegram input `{raw}` should map to the shared parallel control surface"
        );
    }
    assert_eq!(
        parse_message(Some("/parallel now")),
        TelegramParsedMessage::Error("사용법: /parallel [status]".to_string())
    );
}

#[test]
fn parse_message_maps_reviews_to_review_center_command() {
    for raw in ["/reviews", "/reviews@AkraBot"] {
        assert_eq!(
            parse_message(Some(raw)),
            TelegramParsedMessage::Command(TelegramInboundCommand::Reviews),
            "Telegram input `{raw}` should map to the shared review center surface"
        );
    }
    assert_eq!(
        parse_message(Some("/reviews now")),
        TelegramParsedMessage::Error("사용법: /reviews".to_string())
    );
}

#[test]
fn help_reply_mentions_reviews_command() {
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            sender_user_id: Some(777),
            text: Some("/help".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed")
        .expect("reply should exist");
    assert!(reply.contains("/reviews"));
}

#[test]
fn parse_message_ignores_empty_and_plain_chat_text() {
    for raw in [None, Some(""), Some("   "), Some("hello akra")] {
        assert_eq!(parse_message(raw), TelegramParsedMessage::Ignore);
    }
}

#[test]
fn parse_message_reports_help_for_unknown_slash_command() {
    let parsed = parse_message(Some("/deploy"));

    match parsed {
        TelegramParsedMessage::Error(error) => {
            assert!(error.contains("지원하지 않는 명령어입니다: /deploy"));
            assert!(error.contains("/status"));
            assert!(error.contains("/reset queue"));
        }
        other => panic!("unknown slash command should produce help error, got {other:?}"),
    }
}

#[test]
fn parse_message_rejects_extra_arguments_for_query_commands() {
    for (raw, usage) in [
        ("/help now", "/help"),
        ("/whoami please", "/whoami"),
        ("/status detail", "/status"),
        ("/queue all", "/queue"),
        ("/parallel_status now", "/parallel_status"),
    ] {
        assert_eq!(
            parse_message(Some(raw)),
            TelegramParsedMessage::Error(format!("사용법: {usage}")),
            "Telegram input `{raw}` should reject trailing arguments"
        );
    }
}

#[test]
fn parse_message_rejects_unknown_plan_and_reset_targets() {
    assert_eq!(
        parse_message(Some("/plan repair")),
        TelegramParsedMessage::Error("사용법: /plan [status]".to_string())
    );
    assert_eq!(
        parse_message(Some("/reset cache")),
        TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string()
        )
    );
}

#[test]
fn parse_message_reports_usage_for_reset_without_target() {
    /*
     * `/reset` is destructive enough that Telegram must reject an omitted target
     * at the parser boundary. The application service should never receive a
     * best-guess reset command from ambiguous chat text.
     */
    let parsed = parse_message(Some("/reset"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string()
        )
    );
}

#[test]
fn parse_message_rejects_reset_with_extra_arguments() {
    /*
     * Extra words after a reset target often mean the operator thought another
     * scope or confirmation was available. Returning usage text is safer than
     * silently accepting a partial destructive command.
     */
    let parsed = parse_message(Some("/reset queue now"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error(
            "사용법: /reset queue | /reset directions | /reset all".to_string()
        )
    );
}

#[test]
fn parse_message_rejects_reset_alias_with_extra_arguments() {
    /*
     * Alias commands skip the generic `/reset <target>` parser path, so this
     * regression case keeps shorthand reset commands equally strict about
     * accepting no trailing chat text.
     */
    let parsed = parse_message(Some("/reset_queue now"));

    assert_eq!(
        parsed,
        TelegramParsedMessage::Error("사용법: /reset_queue".to_string())
    );
}

// Allowlist tests are security-sensitive: unauthorized chats must receive setup guidance, not data.
#[test]
fn runner_rejects_unauthorized_chat_with_current_chat_id() {
    /*
     * An empty allowlist is treated as "not configured", not "allow everyone".
     * The reply must reveal only the current chat id and environment key so an
     * operator can complete setup without leaking workspace status or queue data.
     */
    let (_gateway, runner) = build_runner(&[]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            sender_user_id: Some(777),
            text: Some("/status".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");
    let reply = reply.expect("reply should exist");
    assert!(reply.contains("현재 chat_id: 777"));
    assert!(reply.contains("AKRA_TELEGRAM_ALLOWED_CHAT_IDS"));
}

#[test]
fn runner_executes_planning_command_for_allowed_chat() {
    /*
     * The queue title assertion proves the allowed path crosses the adapter
     * boundary and executes PlanningControlService. Checking only a generic
     * heading would miss a regression that returned static Telegram help text.
     */
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 42,
            sender_user_id: Some(42),
            text: Some("/status".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");
    let reply = reply.expect("reply should exist");
    assert!(reply.contains("상태 요약"));
    assert!(reply.contains("Ship Telegram control"));
}

#[test]
fn runner_executes_parallel_status_for_allowed_chat() {
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 42,
            sender_user_id: Some(42),
            text: Some("/parallel".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");
    let reply = reply.expect("reply should exist");
    assert!(reply.contains("병렬 상태"));
    assert!(reply.contains("queue_depth: 2"));
}

#[test]
fn runner_reviews_summary_mentions_top_pending_inbox_thread() {
    let repository = Arc::new(FakeReviewCenterRepository {
        inbox: vec![ReviewCenterInboxItem::new(
            "review-1",
            "thread-1",
            "pending",
            "Need operator follow-up",
            "2026-07-06T10:00:00Z",
            "2026-07-06T10:01:00Z",
        )],
        history: vec![ReviewCenterHistoryEntry::new(
            "review-1",
            "thread-1",
            "review_requested",
            "Need operator follow-up",
            "2026-07-06T10:02:00Z",
        )],
        thread_reviews: vec![ReviewCenterThreadProjection::new(
            "thread-1",
            "review-1",
            "Manual review",
            "pending",
            "Need operator follow-up",
            "2026-07-06T10:00:00Z",
            "2026-07-06T10:01:00Z",
        )],
    });
    let (_gateway, runner) = build_runner_with_reviews(&[42], repository);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 2,
            chat_id: 42,
            sender_user_id: Some(42),
            text: Some("/reviews".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed")
        .expect("reply should exist");

    assert!(reply.contains("top pending inbox thread: thread-1"));
    assert!(reply.contains("Manual review [pending] Need operator follow-up"));
}

#[test]
fn help_reply_mentions_whoami_without_allowlist() {
    /*
     * Help remains open because it is the bootstrap surface for remote setup.
     * `/whoami` must be visible here so an operator can discover the exact chat
     * id before any privileged planning command is accepted.
     */
    let (_gateway, runner) = build_runner(&[]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            sender_user_id: Some(777),
            text: Some("/help".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed");
    let reply = reply.expect("reply should exist");
    assert!(reply.contains("/whoami"));
    assert!(reply.contains("/parallel"));
    assert!(reply.contains("/status"));
}

// Config tests cover precedence: process env overrides file values, flags add explicit chat IDs.
#[test]
fn parse_args_reads_token_and_chat_ids_from_environment_and_flags() {
    /*
     * CLI chat ids are additive so a one-off operator can be allowed without
     * rewriting the local env file. The token still comes from the environment
     * because the flag parser should not require secrets in shell history.
     */
    let args = parse_args_with_environment(
        [
            "--allow-chat-id".to_string(),
            "12".to_string(),
            "--allow-user-id".to_string(),
            "101".to_string(),
            "--poll-timeout-seconds".to_string(),
            "45".to_string(),
        ],
        TelegramBotEnvironment {
            token: Some("env-token".to_string()),
            allowed_chat_ids: [10, 11].into_iter().collect(),
            allowed_user_ids: [100].into_iter().collect(),
        },
    )
    .expect("args should parse");

    assert_eq!(args.token, "env-token");
    assert_eq!(args.allowed_chat_ids.len(), 3);
    assert!(args.allowed_chat_ids.contains(&10));
    assert!(args.allowed_chat_ids.contains(&12));
    assert_eq!(args.allowed_user_ids, [100, 101].into_iter().collect());
    assert_eq!(args.poll_timeout_seconds, 45);
    assert!(args.drop_pending_updates);
    assert!(!args.rebind_workspace);
}

#[test]
fn parse_args_preserves_environment_token_and_keep_pending_disables_drop() {
    let args = parse_args_with_environment(
        [
            "--allow-chat-id".to_string(),
            "-100".to_string(),
            "--keep-pending".to_string(),
            "--rebind-workspace".to_string(),
        ],
        TelegramBotEnvironment {
            token: Some("env-token".to_string()),
            allowed_chat_ids: [10].into_iter().collect(),
            allowed_user_ids: BTreeSet::new(),
        },
    )
    .expect("args should parse");

    assert_eq!(args.token, "env-token");
    assert_eq!(args.allowed_chat_ids, [-100, 10].into_iter().collect());
    assert_eq!(args.poll_timeout_seconds, 30);
    assert!(!args.drop_pending_updates);
    assert!(args.rebind_workspace);
}

#[test]
fn parse_args_rejects_missing_invalid_and_unsupported_values() {
    for (args, expected_error) in [
        (
            vec!["--token".to_string()],
            "--token is disabled because process arguments are observable".to_string(),
        ),
        (
            vec!["--allow-chat-id".to_string()],
            "missing value for --allow-chat-id".to_string(),
        ),
        (
            vec!["--allow-chat-id".to_string(), "not-a-chat".to_string()],
            "failed to parse telegram chat id; expected a non-zero integer".to_string(),
        ),
        (
            vec!["--allow-chat-id".to_string(), "0".to_string()],
            "telegram chat id must be non-zero".to_string(),
        ),
        (
            vec!["--allow-user-id".to_string()],
            "missing value for --allow-user-id".to_string(),
        ),
        (
            vec!["--allow-user-id".to_string(), "not-a-user".to_string()],
            "failed to parse telegram user id; expected a positive integer".to_string(),
        ),
        (
            vec!["--allow-user-id".to_string(), "-1".to_string()],
            "telegram user id must be a positive integer".to_string(),
        ),
        (
            vec!["--poll-timeout-seconds".to_string(), "abc".to_string()],
            "failed to parse poll timeout seconds from `abc`".to_string(),
        ),
        (
            vec!["--poll-timeout-seconds".to_string(), "0".to_string()],
            "--poll-timeout-seconds must be greater than zero".to_string(),
        ),
        (
            vec!["--poll-timeout-seconds".to_string(), "51".to_string()],
            "--poll-timeout-seconds must not exceed 50".to_string(),
        ),
        (
            vec!["--unknown".to_string()],
            "unsupported telegram-bot argument: --unknown".to_string(),
        ),
    ] {
        let error = parse_args_with_environment(
            args,
            TelegramBotEnvironment {
                token: Some("env-token".to_string()),
                allowed_chat_ids: BTreeSet::new(),
                allowed_user_ids: BTreeSet::new(),
            },
        )
        .expect_err("args should fail");
        assert!(
            error.to_string().contains(&expected_error),
            "expected `{expected_error}`, got `{error:#}`"
        );
    }

    let missing_token = parse_args_with_environment(Vec::<String>::new(), Default::default())
        .expect_err("missing token should fail");
    assert!(
        missing_token
            .to_string()
            .contains("telegram bot token is required")
    );
    let blank_token = parse_args_with_environment(
        Vec::<String>::new(),
        TelegramBotEnvironment {
            token: Some(" \n ".to_string()),
            ..Default::default()
        },
    )
    .expect_err("blank token should fail");
    assert!(
        blank_token
            .to_string()
            .contains("non-empty telegram bot token")
    );
    let oversized_token = "x".repeat(257);
    for invalid_token in [" env-token", "env-token ", oversized_token.as_str()] {
        let error = parse_args_with_environment(
            Vec::<String>::new(),
            TelegramBotEnvironment {
                token: Some(invalid_token.to_string()),
                ..Default::default()
            },
        )
        .expect_err("malformed token should fail at bootstrap");
        assert!(error.to_string().contains("non-empty telegram bot token"));
        assert!(!error.to_string().contains(invalid_token));
    }

    let cli_secret = "telegram-secret-that-must-not-appear";
    let rejected_token = parse_args_with_environment(
        ["--token".to_string(), cli_secret.to_string()],
        TelegramBotEnvironment {
            token: Some("env-token".to_string()),
            allowed_chat_ids: BTreeSet::new(),
            allowed_user_ids: BTreeSet::new(),
        },
    )
    .expect_err("CLI token must be rejected");
    assert!(rejected_token.to_string().contains("--token is disabled"));
    assert!(!rejected_token.to_string().contains(cli_secret));

    let inline_secret = "telegram-inline-secret-that-must-not-appear";
    let rejected_inline_token = parse_args_with_environment(
        [format!("--token={inline_secret}")],
        TelegramBotEnvironment {
            token: Some("env-token".to_string()),
            allowed_chat_ids: BTreeSet::new(),
            allowed_user_ids: BTreeSet::new(),
        },
    )
    .expect_err("inline CLI token must be rejected");
    assert!(
        rejected_inline_token
            .to_string()
            .contains("--token is disabled")
    );
    assert!(!rejected_inline_token.to_string().contains(inline_secret));
}

#[test]
fn invalid_chat_allowlist_errors_redact_the_supplied_value() {
    let secret_like_value = "123456:telegram-secret";
    let error = parse_args_with_environment(
        vec!["--allow-chat-id".to_string(), secret_like_value.to_string()],
        TelegramBotEnvironment {
            token: Some("123456:valid-token".to_string()),
            allowed_chat_ids: BTreeSet::new(),
            allowed_user_ids: BTreeSet::new(),
        },
    )
    .expect_err("non-numeric chat id should fail");

    assert!(!format!("{error:#}").contains(secret_like_value));
}

#[cfg(unix)]
#[test]
fn telegram_config_file_rejects_symlinked_and_writable_parent_directories() {
    use std::os::unix::fs::symlink;

    let unique = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "akra-telegram-config-parent-{}-{unique}",
        std::process::id()
    ));
    let real_parent = root.join("real");
    let linked_parent = root.join("linked");
    fs::create_dir_all(&real_parent).expect("real config parent should create");
    fs::set_permissions(&real_parent, fs::Permissions::from_mode(0o700))
        .expect("real config parent should become private");
    let config = real_parent.join("telegram.env");
    fs::write(&config, "AKRA_TELEGRAM_BOT_TOKEN=123456:secret\n").expect("config should write");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
        .expect("config should become private");
    symlink(&real_parent, &linked_parent).expect("parent symlink should create");

    read_telegram_environment_file(&linked_parent.join("telegram.env"))
        .expect_err("symlinked config parent must fail closed");
    fs::set_permissions(&real_parent, fs::Permissions::from_mode(0o777))
        .expect("config parent should become writable");
    let error = read_telegram_environment_file(&config)
        .expect_err("group/other-writable config parent must fail closed");
    assert!(format!("{error:#}").contains("config parent"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn load_environment_from_sources_merges_config_file_and_process_env() {
    /*
     * Process env wins over config file content for both token and allowlist.
     * That lets deployment wrappers rotate credentials or narrow access without
     * editing the user's persistent Telegram config file.
     */
    let environment = load_environment_from_sources(
        Some(
            r#"
            AKRA_TELEGRAM_BOT_TOKEN=config-token
            AKRA_TELEGRAM_ALLOWED_CHAT_IDS=10,11
            AKRA_TELEGRAM_ALLOWED_USER_IDS=100,101
            "#,
        ),
        Some("env-token".to_string()),
        Some("12,13".to_string()),
        Some("102,103".to_string()),
    )
    .expect("environment should load");

    assert_eq!(environment.token.as_deref(), Some("env-token"));
    assert_eq!(environment.allowed_chat_ids, [12, 13].into_iter().collect());
    assert_eq!(
        environment.allowed_user_ids,
        [102, 103].into_iter().collect()
    );
}

#[test]
fn load_environment_from_sources_accepts_empty_and_sparse_allowlists() {
    let environment = load_environment_from_sources(
        Some("AKRA_TELEGRAM_BOT_TOKEN=config-token\nAKRA_TELEGRAM_ALLOWED_CHAT_IDS=10,, -20,\n"),
        None,
        None,
        None,
    )
    .expect("environment should load");

    assert_eq!(environment.token.as_deref(), Some("config-token"));
    assert_eq!(
        environment.allowed_chat_ids,
        [-20, 10].into_iter().collect()
    );

    let environment = load_environment_from_sources(None, None, Some("".to_string()), None)
        .expect("empty process allowlist should load");
    assert!(environment.allowed_chat_ids.is_empty());
}

#[test]
fn apply_environment_file_reads_token_and_allowlist() {
    let mut environment = TelegramBotEnvironment::default();

    /*
     * The file parser accepts shell-like `export` and quoted values because the
     * default config path is meant to be hand-edited. Unrelated keys stay ignored
     * so users can keep local notes in the same file.
     */
    apply_environment_file(
        &mut environment,
        r#"
        # local bot config
        export AKRA_TELEGRAM_BOT_TOKEN="stored-token"
        AKRA_TELEGRAM_ALLOWED_CHAT_IDS='10,11'
        AKRA_TELEGRAM_ALLOWED_USER_IDS='100,101'
        UNUSED_KEY=ignored
        "#,
    )
    .expect("config file should parse");

    assert_eq!(environment.token.as_deref(), Some("stored-token"));
    assert_eq!(environment.allowed_chat_ids, [10, 11].into_iter().collect());
    assert_eq!(
        environment.allowed_user_ids,
        [100, 101].into_iter().collect()
    );
}

#[test]
fn apply_environment_file_reports_malformed_lines_and_bad_chat_ids() {
    let mut environment = TelegramBotEnvironment::default();
    let malformed = apply_environment_file(&mut environment, "AKRA_TELEGRAM_BOT_TOKEN");
    assert!(
        malformed
            .expect_err("missing equals should fail")
            .to_string()
            .contains("invalid Telegram config entry on line 1")
    );

    let bad_chat_id =
        apply_environment_file(&mut environment, "AKRA_TELEGRAM_ALLOWED_CHAT_IDS=10,abc");
    assert!(
        bad_chat_id
            .expect_err("bad chat id should fail")
            .to_string()
            .contains("failed to parse telegram chat id; expected a non-zero integer")
    );

    let raw_user_id = "sensitive-looking-user-input";
    let bad_user_id = apply_environment_file(
        &mut environment,
        &format!("AKRA_TELEGRAM_ALLOWED_USER_IDS={raw_user_id}"),
    )
    .expect_err("bad user id should fail");
    assert!(
        bad_user_id
            .to_string()
            .contains("expected a positive integer")
    );
    assert!(!bad_user_id.to_string().contains(raw_user_id));

    let unknown_security_key =
        apply_environment_file(&mut environment, "AKRA_TELEGRAM_ALLOWED_USERS_IDS=123")
            .expect_err("misspelled Telegram security key should fail closed");
    assert!(
        unknown_security_key
            .to_string()
            .contains("unsupported Telegram config key")
    );
}

#[test]
fn telegram_process_environment_rejects_prefixed_typos() {
    validate_telegram_process_environment_keys(
        [
            "PATH",
            "AKRA_TELEGRAM_BOT_TOKEN",
            "AKRA_TELEGRAM_ALLOWED_CHAT_IDS",
            "AKRA_TELEGRAM_ALLOWED_USER_IDS",
        ]
        .into_iter()
        .map(std::ffi::OsString::from),
    )
    .expect("documented Telegram environment keys should pass");

    let error = validate_telegram_process_environment_keys([std::ffi::OsString::from(
        "AKRA_TELEGRAM_ALLOWED_USERS_IDS",
    )])
    .expect_err("misspelled Telegram environment key should fail closed");
    assert!(
        error
            .to_string()
            .contains("unsupported Telegram environment key")
    );
}

#[cfg(unix)]
#[test]
fn telegram_environment_rejects_non_utf8_keys_and_values() {
    use std::os::unix::ffi::OsStringExt;

    let invalid_key = std::ffi::OsString::from_vec(b"AKRA_TELEGRAM_\xff".to_vec());
    let key_error = validate_telegram_process_environment_keys([invalid_key])
        .expect_err("non-UTF-8 Telegram environment key must fail closed");
    assert!(key_error.to_string().contains("must be valid UTF-8"));

    let invalid_value = std::ffi::OsString::from_vec(b"token-\xff".to_vec());
    let value_error = utf8_environment_value("AKRA_TELEGRAM_BOT_TOKEN", Some(invalid_value))
        .expect_err("non-UTF-8 Telegram environment value must fail closed");
    assert!(value_error.to_string().contains("must be valid UTF-8"));
}

#[cfg(unix)]
#[test]
fn telegram_config_file_rejects_group_or_other_permissions_before_reading_secret() {
    let unique = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "akra-telegram-config-permissions-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("fixture root should exist");
    let path = root.join("telegram.env");
    let secret = "telegram-config-secret-that-must-not-appear";
    fs::write(&path, format!("AKRA_TELEGRAM_BOT_TOKEN={secret}\n"))
        .expect("config fixture should write");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
        .expect("config fixture permissions should change");

    let error =
        read_telegram_environment_file(&path).expect_err("group-readable config must fail closed");
    let message = error.to_string();
    assert!(message.contains("set mode 0600"));
    assert!(!message.contains(secret));
    assert!(!message.contains(path.to_string_lossy().as_ref()));

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("config fixture should become private");
    let body = read_telegram_environment_file(&path).expect("private config should be readable");
    assert!(body.contains(secret));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn telegram_config_file_rejects_non_utf8_content() {
    let unique = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "akra-telegram-config-non-utf8-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("fixture root should exist");
    let path = root.join("telegram.env");
    fs::write(&path, b"AKRA_TELEGRAM_BOT_TOKEN=token-\xff\n")
        .expect("invalid UTF-8 config fixture should write");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("config fixture should be private");

    let error = read_telegram_environment_file(&path)
        .expect_err("non-UTF-8 Telegram config must fail closed");
    assert!(error.to_string().contains("must be valid UTF-8"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn telegram_config_file_rejects_symbolic_links() {
    use std::os::unix::fs::symlink;

    let unique = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "akra-telegram-config-symlink-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("fixture root should exist");
    let target = root.join("target.env");
    let link = root.join("telegram.env");
    fs::write(&target, "AKRA_TELEGRAM_BOT_TOKEN=secret\n").expect("target config should write");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
        .expect("target config should be private");
    symlink(&target, &link).expect("config symlink should create");

    let error = read_telegram_environment_file(&link)
        .expect_err("symbolic-link Telegram config must fail closed");
    assert!(error.to_string().contains("regular file with one link"));

    let dangling_link = root.join("dangling.env");
    symlink(root.join("missing.env"), &dangling_link)
        .expect("dangling config symlink should create");
    assert_eq!(
        telegram_env_file_path_if_present(dangling_link.clone())
            .expect("existing path inspection should succeed"),
        Some(dangling_link.clone()),
        "a dangling config symlink must reach the fail-closed reader instead of looking absent"
    );
    let error = read_telegram_environment_file(&dangling_link)
        .expect_err("dangling Telegram config symlink must fail closed");
    assert!(error.to_string().contains("regular file with one link"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn telegram_config_file_rejects_hardlinks() {
    let unique = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "akra-telegram-config-hardlink-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("fixture root should exist");
    let path = root.join("telegram.env");
    let second_link = root.join("telegram-copy.env");
    fs::write(&path, "AKRA_TELEGRAM_BOT_TOKEN=secret\n").expect("config fixture should write");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .expect("config fixture should be private");
    fs::hard_link(&path, &second_link).expect("hardlink fixture should create");

    let error = read_telegram_environment_file(&path)
        .expect_err("hardlinked Telegram config must fail closed");
    assert!(error.to_string().contains("one link"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
fn secure_windows_telegram_config(path: &std::path::Path) {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
        WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC, set_windows_private_acl,
    };

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(
            WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC,
        )
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .expect("Windows Telegram config should open for ACL setup");
    set_windows_private_acl(&file, false)
        .expect("Windows Telegram config ACL should become private");
}

#[cfg(windows)]
#[test]
fn telegram_config_windows_requires_private_acl_and_rejects_hardlinks() {
    let unique = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "akra-telegram-config-windows-security-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("fixture root should exist");
    let path = root.join("telegram.env");
    fs::write(&path, "AKRA_TELEGRAM_BOT_TOKEN=secret\n").expect("config fixture should write");
    let broad_acl_status = std::process::Command::new("icacls")
        .arg(&path)
        .args(["/grant", "*S-1-1-0:(R)"])
        .status()
        .expect("icacls should launch");
    assert!(
        broad_acl_status.success(),
        "broad Windows ACL fixture should create"
    );

    let inherited_acl_error =
        read_telegram_environment_file(&path).expect_err("broad Windows ACL must fail closed");
    assert!(format!("{inherited_acl_error:#}").contains("current-user-only DACL"));

    secure_windows_telegram_config(&path);
    let body = read_telegram_environment_file(&path)
        .expect("owner-only Windows Telegram config should be readable");
    assert!(body.contains("AKRA_TELEGRAM_BOT_TOKEN=secret"));

    let second_link = root.join("telegram-copy.env");
    fs::hard_link(&path, &second_link).expect("Windows hardlink fixture should create");
    let hardlink_error = read_telegram_environment_file(&path)
        .expect_err("hardlinked Windows Telegram config must fail closed");
    assert!(format!("{hardlink_error:#}").contains("single-link"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn telegram_config_windows_rejects_parent_junctions() {
    let unique = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "akra-telegram-config-windows-junction-{}-{unique}",
        std::process::id()
    ));
    let real_parent = root.join("real-config");
    let junction_parent = root.join("junction-config");
    fs::create_dir_all(&real_parent).expect("real config parent should create");
    let target = real_parent.join("telegram.env");
    fs::write(&target, "AKRA_TELEGRAM_BOT_TOKEN=secret\n").expect("config fixture should write");
    secure_windows_telegram_config(&target);
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction_parent)
        .arg(&real_parent)
        .status()
        .expect("mklink should launch");
    assert!(status.success(), "junction fixture should create");

    let error = read_telegram_environment_file(&junction_parent.join("telegram.env"))
        .expect_err("parent junction must fail closed");
    assert!(format!("{error:#}").contains("junction or reparse point"));
    fs::remove_dir(&junction_parent).expect("junction fixture should remove without traversal");
    let _ = fs::remove_dir_all(root);
}

// Poll-loop tests keep the bot alive across transport and per-message failures.
#[test]
fn run_poll_cycle_keeps_loop_alive_after_poll_error() {
    /*
     * A failed getUpdates call cannot advance the cursor; otherwise Telegram
     * could drop a command the bot never saw. Returning the previous offset is
     * the retry contract for transient network or API errors.
     */
    let (gateway, runner) = build_runner(&[42]);
    gateway
        .poll_errors
        .lock()
        .expect("poll error mutex should lock")
        .push(anyhow!("network unavailable"));
    let next_offset = runner
        .run_poll_cycle(Some(99))
        .expect("transport failures should retain the cursor without failing the runner");

    assert_eq!(next_offset, Some(99));
}

#[test]
fn drop_pending_updates_uses_telegram_negative_offset_for_the_latest_update() {
    let (gateway, mut runner) = build_runner(&[42]);
    runner.drop_pending_updates = true;
    gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![TelegramUpdate {
            update_id: 123,
            message: None,
        }]);

    let next_offset = runner
        .bootstrap_offset()
        .expect("latest pending update should establish a live cursor");

    assert_eq!(next_offset, Some(124));
    let requests = gateway
        .poll_requests
        .lock()
        .expect("poll request mutex should lock");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0], TelegramPollRequest::new(Some(-1), 0, 1));
}

#[test]
fn run_poll_cycle_advances_after_the_maximum_update_id_without_trusting_order() {
    let (gateway, runner) = build_runner(&[42]);
    gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![
            TelegramUpdate {
                update_id: 12,
                message: None,
            },
            TelegramUpdate {
                update_id: 10,
                message: None,
            },
            TelegramUpdate {
                update_id: 11,
                message: None,
            },
        ]);

    let next_offset = runner
        .run_poll_cycle(Some(10))
        .expect("valid update ids should advance the cursor");

    assert_eq!(next_offset, Some(13));
}

#[test]
fn run_poll_cycle_never_moves_a_cursor_backwards() {
    let (gateway, runner) = build_runner(&[42]);
    gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![TelegramUpdate {
            update_id: 12,
            message: None,
        }]);

    let next_offset = runner
        .run_poll_cycle(Some(99))
        .expect("an old batch must not regress the acknowledged cursor");

    assert_eq!(next_offset, Some(99));
}

#[test]
fn cursor_overflow_stops_before_executing_or_replying_to_the_batch() {
    let (gateway, runner) = build_runner(&[42]);
    gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![TelegramUpdate {
            update_id: i64::MAX,
            message: Some(TelegramInboundMessage {
                message_id: 1,
                chat_id: 42,
                sender_user_id: Some(42),
                text: Some("/status".to_string()),
                sender_display_name: None,
            }),
        }]);

    let error = runner
        .run_poll_cycle(None)
        .expect_err("an unacknowledgeable batch must stop safely");

    assert!(error.to_string().contains("cursor overflowed"));
    assert!(
        gateway
            .sent_messages
            .lock()
            .expect("sent message mutex should lock")
            .is_empty()
    );
}

#[test]
fn post_poll_global_fence_discards_the_batch_before_workspace_side_effects() {
    let gateway = Arc::new(FakeTelegramBotPort::default());
    gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![TelegramUpdate {
            update_id: 73,
            message: Some(TelegramInboundMessage {
                message_id: 730,
                chat_id: 42,
                sender_user_id: Some(42),
                text: Some("/reset_all".to_string()),
                sender_display_name: Some("operator".to_string()),
            }),
        }]);
    let ledger = Arc::new(FakeTelegramUpdateLedger::default());
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        Arc::new(ExpiringAfterPollGlobalRunnerLease {
            renew_calls: AtomicUsize::new(0),
        }),
        ledger.clone(),
        counting_control_service(reset_calls.clone(), Arc::new(AtomicUsize::new(0))),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            "/tmp/repo",
            "bot-id:post-poll-fence",
            TelegramBotPolicy::new([42].into_iter().collect(), BTreeSet::new()),
        ),
    );
    runner
        .acquire_runner_lease()
        .expect("initial global and workspace leases should acquire");

    let error = runner
        .run_poll_cycle(None)
        .expect_err("lost post-poll global ownership must fence the whole batch");

    assert!(format!("{error:#}").contains("global Telegram lease expiry"));
    assert_eq!(reset_calls.load(Ordering::SeqCst), 0);
    assert!(
        ledger
            .state
            .lock()
            .expect("ledger mutex should lock")
            .updates
            .is_empty(),
        "the workspace inbox must not claim an update after the global fence is lost"
    );
    assert!(
        gateway
            .sent_messages
            .lock()
            .expect("sent messages mutex should lock")
            .is_empty()
    );
}

#[test]
fn workspace_binding_mismatch_stops_before_repo_lease_cursor_or_polling() {
    let gateway = Arc::new(FakeTelegramBotPort::default());
    let ledger = Arc::new(FakeTelegramUpdateLedger::default());
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        Arc::new(MismatchedTelegramWorkspaceBinding {
            bound_workspace_dir: "/canonical/old-workspace".to_string(),
        }),
        ledger.clone(),
        PlanningControlService::new(Arc::new(FakePlanningControlSurface)),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            "/canonical/new-workspace",
            "bot-id:workspace-binding",
            TelegramBotPolicy::new([42].into_iter().collect(), BTreeSet::new()),
        ),
    );

    let error = runner
        .acquire_runner_lease()
        .expect_err("a bot bound to another workspace must fail closed");

    assert!(error.to_string().contains("/canonical/old-workspace"));
    assert!(error.to_string().contains("--rebind-workspace"));
    assert!(
        ledger
            .state
            .lock()
            .expect("ledger mutex should lock")
            .lease_owner
            .is_none(),
        "the repo-scoped lease must not be touched before the global binding gate"
    );
    assert!(
        gateway
            .poll_requests
            .lock()
            .expect("poll request mutex should lock")
            .is_empty()
    );
}

#[test]
fn keep_pending_restart_does_not_repeat_a_completed_reset_before_telegram_ack_poll() {
    let workspace_dir = temp_telegram_workspace("restart-idempotency");
    let stream_key = "bot-id:123456".to_string();
    let fake_ledger = Arc::new(FakeTelegramUpdateLedger::default());
    let ledger: Arc<dyn TelegramUpdateLedgerPort> = fake_ledger.clone();
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let status_calls = Arc::new(AtomicUsize::new(0));
    let update = TelegramUpdate {
        update_id: 77,
        message: Some(TelegramInboundMessage {
            message_id: 700,
            chat_id: 42,
            sender_user_id: Some(42),
            text: Some("/reset_all".to_string()),
            sender_display_name: Some("operator".to_string()),
        }),
    };

    let first_gateway = Arc::new(FakeTelegramBotPort::default());
    first_gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![update.clone()]);
    let first_runner = TelegramBotRunner::new(
        first_gateway,
        permissive_global_runner_lease(),
        Arc::new(CompletionFailingTelegramUpdateLedger {
            delegate: ledger.clone(),
        }),
        counting_control_service(reset_calls.clone(), status_calls.clone()),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            &workspace_dir,
            &stream_key,
            TelegramBotPolicy::new([42].into_iter().collect(), BTreeSet::new()),
        ),
    );
    first_runner
        .acquire_runner_lease()
        .expect("first SQLite Telegram runner lease should acquire");
    let initial_offset = first_runner
        .bootstrap_offset()
        .expect("keep-pending bootstrap should load an empty cursor");
    assert_eq!(initial_offset, None);
    let completion_error = first_runner
        .run_poll_cycle(initial_offset)
        .expect_err("the fixture should stop after the reset side effect but before completion");
    assert!(completion_error.to_string().contains("durably complete"));
    assert_eq!(reset_calls.load(Ordering::SeqCst), 1);

    // The global ownership adapter is tested separately. This runner-level test resumes only after
    // the old owner is assumed dead and verifies the ambiguous claim is never replayed.
    let restarted_gateway = Arc::new(FakeTelegramBotPort::default());
    restarted_gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![update]);
    let restarted_runner = TelegramBotRunner::new(
        restarted_gateway.clone(),
        permissive_global_runner_lease(),
        ledger.clone(),
        counting_control_service(reset_calls.clone(), status_calls),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            &workspace_dir,
            &stream_key,
            TelegramBotPolicy::new([42].into_iter().collect(), BTreeSet::new()),
        ),
    );
    {
        let state = fake_ledger.state.lock().expect("ledger mutex should lock");
        assert_eq!(state.updates.get(&77), Some(&"executing"));
        assert_eq!(state.next_offset, None);
    }
    restarted_runner
        .acquire_runner_lease()
        .expect("the replacement global Telegram runner lease should acquire");
    let recovered_offset = restarted_runner
        .bootstrap_offset()
        .expect("restart should recover the durable cursor");
    assert_eq!(recovered_offset, Some(78));
    let next_offset = restarted_runner
        .run_poll_cycle(recovered_offset)
        .expect("a redelivered completed update should be ignored");

    assert_eq!(next_offset, Some(78));
    assert_eq!(reset_calls.load(Ordering::SeqCst), 1);
    assert!(
        restarted_gateway
            .sent_messages
            .lock()
            .expect("sent message mutex should lock")
            .is_empty(),
        "the completed update reply must not be repeated"
    );
    assert_eq!(
        restarted_gateway
            .poll_requests
            .lock()
            .expect("poll request mutex should lock")[0]
            .offset,
        Some(78)
    );
}

#[test]
fn authorized_unauthorized_and_non_message_updates_all_advance_the_durable_cursor() {
    let workspace_dir = temp_telegram_workspace("all-update-kinds");
    let stream_key = "bot-id:654321".to_string();
    let ledger: Arc<dyn TelegramUpdateLedgerPort> = Arc::new(FakeTelegramUpdateLedger::default());
    let status_calls = Arc::new(AtomicUsize::new(0));
    let gateway = Arc::new(FakeTelegramBotPort::default());
    gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![
            TelegramUpdate {
                update_id: 12,
                message: Some(TelegramInboundMessage {
                    message_id: 12,
                    chat_id: 42,
                    sender_user_id: Some(42),
                    text: Some("/status".to_string()),
                    sender_display_name: None,
                }),
            },
            TelegramUpdate {
                update_id: 10,
                message: None,
            },
            TelegramUpdate {
                update_id: 11,
                message: Some(TelegramInboundMessage {
                    message_id: 11,
                    chat_id: 777,
                    sender_user_id: Some(777),
                    text: Some("/status".to_string()),
                    sender_display_name: None,
                }),
            },
        ]);
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        permissive_global_runner_lease(),
        ledger.clone(),
        counting_control_service(Arc::new(AtomicUsize::new(0)), status_calls.clone()),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            &workspace_dir,
            &stream_key,
            TelegramBotPolicy::new([42].into_iter().collect(), BTreeSet::new()),
        ),
    );
    runner
        .acquire_runner_lease()
        .expect("SQLite Telegram runner lease should acquire");

    let next_offset = runner
        .run_poll_cycle(None)
        .expect("every valid update kind should complete");
    let durable_offset = ledger
        .load_cursor_and_recover_inflight(&workspace_dir, &stream_key, &runner.lease_owner_token)
        .expect("durable cursor should load");

    assert_eq!(next_offset, Some(13));
    assert_eq!(durable_offset, Some(13));
    assert_eq!(status_calls.load(Ordering::SeqCst), 1);
    let sent_messages = gateway
        .sent_messages
        .lock()
        .expect("sent message mutex should lock");
    assert_eq!(sent_messages.len(), 2);
    assert!(
        sent_messages[0]
            .text
            .contains("허용되지 않은 chat_id입니다.")
    );
    assert!(sent_messages[1].text.contains("상태 요약"));
}

#[test]
fn ledger_claim_failure_stops_before_command_or_reply_side_effects() {
    let gateway = Arc::new(FakeTelegramBotPort::default());
    gateway
        .updates
        .lock()
        .expect("updates mutex should lock")
        .push(vec![TelegramUpdate {
            update_id: 1,
            message: Some(TelegramInboundMessage {
                message_id: 1,
                chat_id: 42,
                sender_user_id: Some(42),
                text: Some("/reset_all".to_string()),
                sender_display_name: None,
            }),
        }]);
    let reset_calls = Arc::new(AtomicUsize::new(0));
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        permissive_global_runner_lease(),
        Arc::new(RejectingTelegramUpdateLedger),
        counting_control_service(reset_calls.clone(), Arc::new(AtomicUsize::new(0))),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            "/tmp/repo",
            "test-stream",
            TelegramBotPolicy::new([42].into_iter().collect(), BTreeSet::new()),
        ),
    );
    runner
        .acquire_runner_lease()
        .expect("rejecting fixture runner lease should acquire");

    let error = runner
        .run_poll_cycle(None)
        .expect_err("ledger failure must stop the update before execution");

    assert!(error.to_string().contains("before execution"));
    assert_eq!(reset_calls.load(Ordering::SeqCst), 0);
    assert!(
        gateway
            .sent_messages
            .lock()
            .expect("sent message mutex should lock")
            .is_empty()
    );
}

#[test]
fn telegram_stream_key_is_stable_scoped_and_does_not_embed_the_token() {
    let token = "123456:very-secret-bot-token";
    let rotated_token = "123456:rotated-secret-bot-token";
    let key = super::telegram_stream_key(token).expect("valid Telegram token should parse");

    assert_eq!(key, "bot-id:123456");
    assert_eq!(
        key,
        super::telegram_stream_key(rotated_token)
            .expect("token rotation should retain the stable bot id")
    );
    assert_ne!(
        key,
        super::telegram_stream_key("654321:different-bot-token")
            .expect("a different bot id should parse")
    );
    assert!(!key.contains(token));
    assert!(!key.contains("secret"));

    for invalid_token in [
        "missing-prefix-separator",
        ":missing-bot-id",
        "0:zero-bot-id",
        "0123:ambiguous-bot-id",
        "bot:non-numeric-bot-id",
        "123456:",
        "123456:secret:extra-separator",
    ] {
        let error = super::telegram_stream_key(invalid_token)
            .expect_err("malformed Telegram token identity must fail closed");
        assert!(!error.to_string().contains(invalid_token));
    }
}

#[test]
fn get_me_identity_must_match_before_the_runner_can_bind() {
    let gateway = Arc::new(FakeTelegramBotPort::default());
    *gateway
        .authenticated_bot_id
        .lock()
        .expect("bot identity mutex should lock") = Some(999_999);
    let runner = TelegramBotRunner::new(
        gateway,
        permissive_global_runner_lease(),
        Arc::new(FakeTelegramUpdateLedger::default()),
        PlanningControlService::new(Arc::new(FakePlanningControlSurface)),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            "/tmp/repo",
            "bot-id:123456",
            TelegramBotPolicy::new([42].into_iter().collect(), BTreeSet::new()),
        ),
    );

    let error = runner
        .authenticate_bot_identity()
        .expect_err("mismatched getMe identity must fail closed");
    assert!(error.to_string().contains("did not match"));
    assert_eq!(runner.lease_generation.load(Ordering::Acquire), 0);
}

#[test]
fn runner_lease_deadline_outlives_every_allowed_curl_poll() {
    for timeout in [1, 30, super::MAX_POLL_TIMEOUT_SECONDS] {
        assert!(
            telegram_runner_lease_ttl_seconds(timeout) > telegram_max_curl_request_seconds(timeout)
        );
    }
}

#[test]
fn sqlite_telegram_ledger_keeps_cursor_monotonic_and_requires_the_live_owner() {
    let workspace_dir = temp_telegram_workspace("lease-owner-cas-monotonic-cursor");
    let stream_key = "bot-id:999001";
    let first_owner = "runner-owner-first";
    let second_owner = "runner-owner-second";
    let ledger: Arc<dyn TelegramUpdateLedgerPort> = Arc::new(SqlitePlanningAuthorityAdapter::new());

    assert_eq!(
        ledger
            .try_acquire_runner_lease(&workspace_dir, stream_key, first_owner, 300)
            .expect("first SQLite Telegram runner lease should acquire"),
        TelegramRunnerLeaseClaimDecision::Acquired
    );
    assert_eq!(
        ledger
            .try_acquire_runner_lease(&workspace_dir, stream_key, second_owner, 300)
            .expect("concurrent SQLite Telegram runner lease should be inspected"),
        TelegramRunnerLeaseClaimDecision::ActiveRunner
    );
    assert_eq!(
        ledger
            .advance_cursor(&workspace_dir, stream_key, first_owner, Some(100))
            .expect("live owner should advance the cursor"),
        Some(100)
    );
    assert_eq!(
        ledger
            .advance_cursor(&workspace_dir, stream_key, first_owner, Some(10))
            .expect("older cursor input should remain monotonic"),
        Some(100)
    );
    assert!(
        ledger
            .advance_cursor(&workspace_dir, stream_key, second_owner, Some(101))
            .expect_err("foreign owner must not mutate the durable cursor")
            .to_string()
            .contains("no longer owned")
    );
    assert!(
        !ledger
            .release_runner_lease(&workspace_dir, stream_key, second_owner)
            .expect("foreign release should be an owner-CAS no-op")
    );
    ledger
        .renew_runner_lease(&workspace_dir, stream_key, first_owner, 300)
        .expect("the original owner should retain its lease");
    assert!(
        ledger
            .release_runner_lease(&workspace_dir, stream_key, first_owner)
            .expect("the original owner should release its lease")
    );
    assert!(
        ledger
            .advance_cursor(&workspace_dir, stream_key, first_owner, Some(101))
            .expect_err("released owner must not mutate the durable cursor")
            .to_string()
            .contains("no longer owned")
    );
}

#[test]
fn process_updates_continues_after_individual_message_failure() {
    /*
     * Batch processing isolates each message. The first update exercises the
     * failure reply path, and the second proves the runner keeps draining the
     * batch instead of letting one bad planning call block later chat commands.
     */
    let gateway = Arc::new(FakeTelegramBotPort::default());
    let runner = TelegramBotRunner::new(
        gateway.clone(),
        permissive_global_runner_lease(),
        Arc::new(FakeTelegramUpdateLedger::default()),
        PlanningControlService::new(Arc::new(FlakyPlanningControlSurface {
            load_calls: AtomicUsize::new(0),
        })),
        Arc::new(FakeTelegramParallelControlSurface),
        test_runtime_config(
            "/tmp/repo",
            "test-stream",
            TelegramBotPolicy::new([42].into_iter().collect(), BTreeSet::new()),
        ),
    );
    runner
        .acquire_runner_lease()
        .expect("fake Telegram runner lease should acquire");

    runner
        .process_updates(
            &[
                // Same chat and same command isolate the variable to service call order.
                TelegramUpdate {
                    update_id: 1,
                    message: Some(TelegramInboundMessage {
                        message_id: 10,
                        chat_id: 42,
                        sender_user_id: Some(42),
                        text: Some("/status".to_string()),
                        sender_display_name: None,
                    }),
                },
                TelegramUpdate {
                    update_id: 2,
                    message: Some(TelegramInboundMessage {
                        message_id: 11,
                        chat_id: 42,
                        sender_user_id: Some(42),
                        text: Some("/status".to_string()),
                        sender_display_name: None,
                    }),
                },
            ],
            None,
        )
        .expect("valid updates should complete durably");
    let sent_messages = gateway
        .sent_messages
        .lock()
        .expect("sent messages mutex should lock");
    assert_eq!(sent_messages.len(), 2);
    // First message reports the injected planning failure; second proves the loop recovered.
    assert!(sent_messages[0].text.contains("명령 처리에 실패했습니다."));
    assert!(sent_messages[1].text.contains("상태 요약"));
}

#[test]
fn runner_whoami_reports_allowlist_state_without_planning_access() {
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 42,
            sender_user_id: Some(42),
            text: Some("/whoami".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed")
        .expect("reply should exist");

    assert!(reply.contains("chat_id: 42"));
    assert!(reply.contains("control_allowed: yes"));
    assert!(reply.contains("chat_allowlist_configured: yes"));
}

#[test]
fn runner_rejects_unauthorized_chat_against_configured_allowlist() {
    let (_gateway, runner) = build_runner(&[42]);
    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: 777,
            sender_user_id: Some(777),
            text: Some("/parallel".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("handler should succeed")
        .expect("reply should exist");

    assert!(reply.contains("허용되지 않은 chat_id입니다."));
    assert!(reply.contains("현재 chat_id: 777"));
}

#[test]
fn group_commands_require_both_chat_and_sender_user_allowlists() {
    let (_gateway, mut runner) = build_runner(&[-100]);
    runner.policy =
        TelegramBotPolicy::new([-100].into_iter().collect(), [9001].into_iter().collect());

    let allowed = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: -100,
            sender_user_id: Some(9001),
            text: Some("/status".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("authorized group command should be handled")
        .expect("authorized group command should reply");
    assert!(allowed.contains("Ship Telegram control"));

    for sender_user_id in [Some(9002), None] {
        let denied = runner
            .handle_message(&TelegramInboundMessage {
                message_id: 2,
                chat_id: -100,
                sender_user_id,
                text: Some("/status".to_string()),
                sender_display_name: None,
            })
            .expect("unauthorized group command should return guidance")
            .expect("unauthorized group command should reply");
        assert!(denied.contains("그룹 제어에는 허용된 발신자 user_id가 필요합니다."));
        assert!(!denied.contains("Ship Telegram control"));
    }
}

#[test]
fn unauthorized_group_reports_missing_chat_before_sender_setup() {
    let (_gateway, mut runner) = build_runner(&[-100]);
    runner.policy =
        TelegramBotPolicy::new([-100].into_iter().collect(), [9001].into_iter().collect());

    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 3,
            chat_id: -200,
            sender_user_id: Some(9002),
            text: Some("/status".to_string()),
            sender_display_name: None,
        })
        .expect("unauthorized group command should be handled")
        .expect("unauthorized group command should reply");

    assert!(reply.contains("허용되지 않은 chat_id입니다."));
    assert!(!reply.contains("그룹 제어에는 허용된 발신자"));
}

#[test]
fn whoami_exposes_sender_and_both_allowlist_decisions_for_group_setup() {
    let (_gateway, mut runner) = build_runner(&[-100]);
    runner.policy =
        TelegramBotPolicy::new([-100].into_iter().collect(), [9001].into_iter().collect());

    let reply = runner
        .handle_message(&TelegramInboundMessage {
            message_id: 1,
            chat_id: -100,
            sender_user_id: Some(9001),
            text: Some("/whoami".to_string()),
            sender_display_name: Some("operator".to_string()),
        })
        .expect("whoami should succeed")
        .expect("whoami should reply");

    for expected in [
        "chat_id: -100",
        "user_id: 9001",
        "control_allowed: yes",
        "chat_allowed: yes",
        "user_allowed: yes",
        "chat_allowlist_configured: yes",
        "user_allowlist_configured: yes",
    ] {
        assert!(
            reply.contains(expected),
            "missing `{expected}` in `{reply}`"
        );
    }
}
