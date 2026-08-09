use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use rand::{RngCore, rngs::OsRng};

use super::pr_validation::{
    PrValidationPollExecution, PrValidationPollFailure, PrValidationPollRequest,
    PrValidationPollResult,
};
use super::{ParallelModeService, derive_default_pool_root};
use crate::application::port::outbound::planning_authority_port::{
    PrValidationPollLeaseClaim, PrValidationPollLeaseClaimRequest,
    PrValidationPollLeaseRenewalRequest, PrValidationPollSettlement,
};
use crate::application::service::planning::PlanningQueueUseCases;
use crate::domain::parallel_mode::{
    PrValidationPollErrorClass, PrValidationRecordKey, PrValidationSchedulerMode,
};

pub const AKRA_PR_VALIDATION_MODE_CONFIG_KEY: &str = "akra.prValidationMode";
const DEFAULT_ACTIVE_INTERVAL_SECS: i64 = 30;
const DEFAULT_MAX_BACKOFF_SECS: i64 = 5 * 60;
const DEFAULT_LEASE_TTL_SECS: i64 = 3 * 60;
const DEFAULT_LEASE_RENEW_INTERVAL_SECS: u64 = 45;
const DEFAULT_SCAN_INTERVAL_MILLIS: u64 = 1_000;
const DEFAULT_DUE_SCAN_LIMIT: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationSchedulerConfig {
    pub mode: PrValidationSchedulerMode,
    active_interval: TimeDelta,
    max_backoff: TimeDelta,
    lease_ttl: TimeDelta,
    lease_renew_interval: Duration,
    scan_interval: Duration,
    due_scan_limit: usize,
}

impl Default for PrValidationSchedulerConfig {
    fn default() -> Self {
        Self {
            mode: PrValidationSchedulerMode::Observe,
            active_interval: TimeDelta::seconds(DEFAULT_ACTIVE_INTERVAL_SECS),
            max_backoff: TimeDelta::seconds(DEFAULT_MAX_BACKOFF_SECS),
            lease_ttl: TimeDelta::seconds(DEFAULT_LEASE_TTL_SECS),
            lease_renew_interval: Duration::from_secs(DEFAULT_LEASE_RENEW_INTERVAL_SECS),
            scan_interval: Duration::from_millis(DEFAULT_SCAN_INTERVAL_MILLIS),
            due_scan_limit: DEFAULT_DUE_SCAN_LIMIT,
        }
    }
}

impl PrValidationSchedulerConfig {
    pub fn from_repository(
        service: &ParallelModeService,
        workspace_dir: &str,
    ) -> Result<Self, String> {
        let repo_root = service
            .parallel_runtime
            .detect_git_repo_root(workspace_dir)
            .ok_or_else(|| {
                "git repository is unavailable for PR validation scheduler configuration"
                    .to_string()
            })?;
        let command_args = repository_mode_git_args(repo_root.as_str());
        let configured = service
            .parallel_runtime
            .run_command("git", &command_args, None);
        let mode = configured
            .as_deref()
            .map(PrValidationSchedulerMode::parse)
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            mode,
            ..Self::default()
        })
    }
}

fn repository_mode_git_args(repo_root: &str) -> [&str; 6] {
    [
        "-C",
        repo_root,
        "config",
        "--local",
        "--get",
        AKRA_PR_VALIDATION_MODE_CONFIG_KEY,
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrValidationSchedulerRunOutcome {
    Disabled,
    Idle,
    LeaseLost {
        record_key: PrValidationRecordKey,
    },
    PollSettled {
        record_key: PrValidationRecordKey,
        result: PrValidationPollResult,
        next_poll_at: DateTime<Utc>,
    },
    RetryScheduled {
        record_key: PrValidationRecordKey,
        error_class: PrValidationPollErrorClass,
        next_poll_at: DateTime<Utc>,
    },
    Terminal {
        record_key: PrValidationRecordKey,
        result: PrValidationPollResult,
        error_class: PrValidationPollErrorClass,
    },
}

#[derive(Clone)]
pub struct PrValidationSchedulerService {
    parallel_mode_service: ParallelModeService,
    queue: PlanningQueueUseCases,
    workspace_dir: String,
    config: PrValidationSchedulerConfig,
    owner: String,
}

impl std::fmt::Debug for PrValidationSchedulerService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PrValidationSchedulerService")
            .field("workspace_dir", &self.workspace_dir)
            .field("mode", &self.config.mode)
            .finish_non_exhaustive()
    }
}

impl PrValidationSchedulerService {
    pub fn new(
        parallel_mode_service: ParallelModeService,
        queue: PlanningQueueUseCases,
        workspace_dir: impl Into<String>,
        config: PrValidationSchedulerConfig,
    ) -> Result<Self, String> {
        let workspace_dir = workspace_dir.into();
        let (process_id, process_start) = parallel_mode_service
            .planning_authority
            .current_process_claim_identity()
            .map_err(|error| {
                format!("failed to resolve PR validation scheduler process identity: {error}")
            })?;
        let owner = format!(
            "pr-validation:{process_id}:{process_start}:{}",
            secure_nonce()?
        );
        Ok(Self {
            parallel_mode_service,
            queue,
            workspace_dir,
            config,
            owner,
        })
    }

    pub fn mode(&self) -> PrValidationSchedulerMode {
        self.config.mode
    }

    pub fn run_due_once_at(
        &self,
        now: DateTime<Utc>,
    ) -> Result<PrValidationSchedulerRunOutcome, String> {
        if self.config.mode == PrValidationSchedulerMode::Off {
            return Ok(PrValidationSchedulerRunOutcome::Disabled);
        }
        let due_keys = self
            .parallel_mode_service
            .planning_authority
            .load_due_runtime_pr_validation_record_keys(
                &self.workspace_dir,
                now,
                now - self.config.active_interval,
                self.config.due_scan_limit,
            )
            .map_err(|error| format!("failed to load due PR validations: {error}"))?;
        for record_key in due_keys {
            let token = secure_nonce()?;
            let expires_at = now + self.config.lease_ttl;
            let Some(claim) = self
                .parallel_mode_service
                .planning_authority
                .try_claim_runtime_pr_validation_poll(
                    &self.workspace_dir,
                    PrValidationPollLeaseClaimRequest {
                        record_key: &record_key,
                        owner: &self.owner,
                        token: &token,
                        claimed_at: now,
                        expires_at,
                        repository_cooldown_since: now - self.config.active_interval,
                    },
                )
                .map_err(|error| {
                    format!(
                        "failed to claim due PR validation `{}`: {error}",
                        record_key.as_str()
                    )
                })?
            else {
                continue;
            };
            return self.run_claim(now, claim);
        }
        Ok(PrValidationSchedulerRunOutcome::Idle)
    }

    fn run_claim(
        &self,
        started_at: DateTime<Utc>,
        mut claim: PrValidationPollLeaseClaim,
    ) -> Result<PrValidationSchedulerRunOutcome, String> {
        let started_instant = std::time::Instant::now();
        let repo_root = self
            .parallel_mode_service
            .parallel_runtime
            .detect_git_repo_root(&self.workspace_dir)
            .ok_or_else(|| {
                "git repository became unavailable during PR validation polling".to_string()
            })?;
        let pool_root = derive_default_pool_root(std::path::Path::new(&repo_root));
        let delivery_revision = claim
            .record
            .observation_revision()
            .checked_add(1)
            .ok_or_else(|| {
                format!(
                    "PR validation record `{}` exhausted delivery revisions",
                    claim.record.key().as_str()
                )
            })?;
        let request = PrValidationPollRequest {
            workspace_dir: self.workspace_dir.clone(),
            pool_root,
            record_key: claim.record.key().clone(),
            target_shas: claim.record.target_shas().clone(),
            delivery_revision,
        };
        let service = self.parallel_mode_service.clone();
        let queue = self.queue.clone();
        let mode = self.config.mode;
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("akra-pr-validation-poll".to_string())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    service
                        .poll_pr_validation_into_normal_queue_for_scheduler(&queue, request, mode)
                }))
                .unwrap_or_else(|_| {
                    Err(PrValidationPollFailure {
                        class: PrValidationPollErrorClass::IntegrityFailed,
                        message: "PR validation provider worker panicked".to_string(),
                        retry_after_at: None,
                        provider_metadata: Default::default(),
                    })
                });
                let _ = sender.send(result);
            })
            .map_err(|error| format!("failed to start PR validation poll worker: {error}"))?;

        let mut lease_lost = false;
        let execution = loop {
            match receiver.recv_timeout(self.config.lease_renew_interval) {
                Ok(result) => break result,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if lease_lost {
                        continue;
                    }
                    let renewed_at = Utc::now();
                    let renewed_expires_at = renewed_at + self.config.lease_ttl;
                    let renewed = self
                        .parallel_mode_service
                        .planning_authority
                        .renew_runtime_pr_validation_poll_lease(
                            &self.workspace_dir,
                            PrValidationPollLeaseRenewalRequest {
                                record_key: claim.record.key(),
                                owner: &claim.owner,
                                token: &claim.token,
                                expected_expires_at: claim.expires_at,
                                renewed_at,
                                renewed_expires_at,
                            },
                        )
                        .unwrap_or(false);
                    match renewed {
                        true => claim.expires_at = renewed_expires_at,
                        false => lease_lost = true,
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(PrValidationPollFailure {
                        class: PrValidationPollErrorClass::IntegrityFailed,
                        message: "PR validation provider worker disconnected".to_string(),
                        retry_after_at: None,
                        provider_metadata: Default::default(),
                    });
                }
            }
        };
        if lease_lost {
            return Ok(PrValidationSchedulerRunOutcome::LeaseLost {
                record_key: claim.record.key().clone(),
            });
        }
        let elapsed = TimeDelta::from_std(started_instant.elapsed()).unwrap_or_default();
        let settled_at = started_at + elapsed;
        self.settle_claim(settled_at, claim, execution)
    }

    fn settle_claim(
        &self,
        settled_at: DateTime<Utc>,
        claim: PrValidationPollLeaseClaim,
        execution: Result<PrValidationPollExecution, PrValidationPollFailure>,
    ) -> Result<PrValidationSchedulerRunOutcome, String> {
        let (result, error_class, error_count, retry_after_at, metadata) = match execution {
            Ok(execution) => {
                let error_class = match execution.result {
                    PrValidationPollResult::Blocked => {
                        Some(PrValidationPollErrorClass::PolicyBlocked)
                    }
                    PrValidationPollResult::Failed => {
                        Some(PrValidationPollErrorClass::IdentityFailed)
                    }
                    PrValidationPollResult::StaleDeliveryIgnored => {
                        Some(PrValidationPollErrorClass::AdmissionRetryable)
                    }
                    _ => None,
                };
                let error_count = if error_class.is_some() {
                    claim.consecutive_error_count.saturating_add(1)
                } else {
                    0
                };
                (
                    Some(execution.result),
                    error_class,
                    error_count,
                    None,
                    execution.provider_metadata,
                )
            }
            Err(failure) => (
                None,
                Some(failure.class),
                claim.consecutive_error_count.saturating_add(1),
                failure.retry_after_at,
                failure.provider_metadata,
            ),
        };
        let mut next_poll_at = match error_class {
            None => settled_at + self.config.active_interval,
            Some(PrValidationPollErrorClass::PolicyBlocked)
            | Some(PrValidationPollErrorClass::IdentityFailed)
            | Some(PrValidationPollErrorClass::IntegrityFailed) => {
                settled_at + self.config.max_backoff
            }
            Some(_) => settled_at + self.retry_delay(claim.record.key(), error_count),
        };
        if let Some(retry_after_at) = retry_after_at {
            next_poll_at = next_poll_at.max(retry_after_at);
        }
        if metadata.rate_limit_remaining == Some(0)
            && let Some(rate_limit_reset_at) = metadata.rate_limit_reset_at
        {
            next_poll_at = next_poll_at.max(rate_limit_reset_at);
        }
        let settlement = PrValidationPollSettlement {
            polled_at: settled_at,
            next_poll_at,
            consecutive_error_count: error_count,
            error_class,
            rate_limit_remaining: metadata.rate_limit_remaining,
            rate_limit_reset_at: metadata.rate_limit_reset_at,
        };
        let settled = self
            .parallel_mode_service
            .planning_authority
            .settle_runtime_pr_validation_poll(
                &self.workspace_dir,
                claim.record.key(),
                &claim.owner,
                &claim.token,
                claim.expires_at,
                &settlement,
            )
            .map_err(|error| {
                format!(
                    "failed to settle PR validation poll `{}`: {error}",
                    claim.record.key().as_str()
                )
            })?;
        if !settled {
            return Ok(PrValidationSchedulerRunOutcome::LeaseLost {
                record_key: claim.record.key().clone(),
            });
        }
        match (result, error_class) {
            (Some(result), None) => Ok(PrValidationSchedulerRunOutcome::PollSettled {
                record_key: claim.record.key().clone(),
                result,
                next_poll_at,
            }),
            (
                Some(result @ (PrValidationPollResult::Blocked | PrValidationPollResult::Failed)),
                Some(error_class),
            ) => Ok(PrValidationSchedulerRunOutcome::Terminal {
                record_key: claim.record.key().clone(),
                result,
                error_class,
            }),
            (_, Some(error_class)) => Ok(PrValidationSchedulerRunOutcome::RetryScheduled {
                record_key: claim.record.key().clone(),
                error_class,
                next_poll_at,
            }),
            (None, None) => Err("PR validation scheduler produced no outcome".to_string()),
        }
    }

    fn retry_delay(
        &self,
        record_key: &PrValidationRecordKey,
        consecutive_error_count: u32,
    ) -> TimeDelta {
        let base_seconds = match consecutive_error_count {
            0 | 1 => DEFAULT_ACTIVE_INTERVAL_SECS,
            2 => 60,
            3 => 120,
            _ => self.config.max_backoff.num_seconds(),
        }
        .min(self.config.max_backoff.num_seconds());
        let jitter_bound = (base_seconds / 5).max(1);
        let hash =
            record_key
                .as_str()
                .bytes()
                .fold(u64::from(consecutive_error_count), |state, byte| {
                    state
                        .wrapping_mul(1099511628211)
                        .wrapping_add(u64::from(byte))
                });
        let jitter = i64::try_from(hash % u64::try_from(jitter_bound).unwrap_or(1)).unwrap_or(0);
        TimeDelta::seconds((base_seconds + jitter).min(self.config.max_backoff.num_seconds()))
    }
}

fn secure_nonce() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|error| format!("OS randomness is required for PR validation leases: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Clone, Default)]
pub struct PrValidationSchedulerRuntime {
    inner: Option<Arc<PrValidationSchedulerRuntimeInner>>,
}

struct PrValidationSchedulerRuntimeInner {
    stop_sender: mpsc::Sender<()>,
    join: Mutex<Option<thread::JoinHandle<()>>>,
}

impl PrValidationSchedulerRuntime {
    pub fn start(service: PrValidationSchedulerService) -> Result<Self, String> {
        if service.mode() == PrValidationSchedulerMode::Off {
            return Ok(Self::default());
        }
        let scan_interval = service.config.scan_interval;
        let (stop_sender, stop_receiver) = mpsc::channel();
        let join = thread::Builder::new()
            .name("akra-pr-validation-scheduler".to_string())
            .spawn(move || {
                loop {
                    if let Err(error) = service.run_due_once_at(Utc::now()) {
                        crate::diagnostics::event_log::emit_lazy(
                            "pr_validation_scheduler_cycle_failed",
                            || {
                                serde_json::json!({
                                    "workspace": &service.workspace_dir,
                                    "error": error,
                                })
                            },
                        );
                    }
                    match stop_receiver.recv_timeout(scan_interval) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
            })
            .map_err(|error| format!("failed to start PR validation scheduler: {error}"))?;
        Ok(Self {
            inner: Some(Arc::new(PrValidationSchedulerRuntimeInner {
                stop_sender,
                join: Mutex::new(Some(join)),
            })),
        })
    }

    pub fn is_running(&self) -> bool {
        self.inner.is_some()
    }
}

impl Drop for PrValidationSchedulerRuntimeInner {
    fn drop(&mut self) {
        let _ = self.stop_sender.send(());
        if let Some(join) = self
            .join
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_mode_command_cannot_inherit_global_or_system_configuration() {
        assert_eq!(
            repository_mode_git_args("repo"),
            [
                "-C",
                "repo",
                "config",
                "--local",
                "--get",
                AKRA_PR_VALIDATION_MODE_CONFIG_KEY,
            ]
        );
    }
}
