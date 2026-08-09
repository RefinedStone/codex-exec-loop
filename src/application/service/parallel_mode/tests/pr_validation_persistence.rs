use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::github_automation_port::GithubRepositoryVisibility;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorDeliveryTarget, PlanningAuthorityDistributorQueueRecord,
    PlanningAuthorityPort,
};
use crate::domain::parallel_mode::{
    IntegrationAttestation, IntegrationMethod, ParallelModeQueueItemState, PrValidationCommitSha,
    PrValidationEvent, PrValidationFinding, PrValidationFindingKey, PrValidationFindingSource,
    PrValidationRecord, PrValidationRecordKey, PrValidationRemediationCorrelation,
    PrValidationTarget, PrValidationTargetShaSnapshot,
};
use chrono::{DateTime, Utc};

use super::super::pr_validation::{
    attest_distributor_pr_validation_with_ports,
    install_before_distributor_attestation_persist_hook,
};
use super::super::{
    persist_pr_validation_record, pr_validation_record_relative_path,
    recover_pr_validation_record_mirror,
};

#[derive(Default)]
struct ValidationMirrorRuntime {
    files: Mutex<BTreeMap<PathBuf, String>>,
    ensured_directories: Mutex<Vec<PathBuf>>,
    reject_next_compare_and_swap: AtomicBool,
}

impl ValidationMirrorRuntime {
    fn reject_next_compare_and_swap(&self) {
        self.reject_next_compare_and_swap
            .store(true, Ordering::SeqCst);
    }

    fn clear(&self) {
        self.files.lock().expect("mirror lock").clear();
    }

    fn body(&self, relative: &Path) -> Option<String> {
        self.files
            .lock()
            .expect("mirror lock")
            .get(relative)
            .cloned()
    }

    fn ensured_directories(&self) -> Vec<PathBuf> {
        self.ensured_directories
            .lock()
            .expect("directory lock")
            .clone()
    }

    fn clear_ensured_directories(&self) {
        self.ensured_directories
            .lock()
            .expect("directory lock")
            .clear();
    }
}

impl ParallelModeRuntimePort for ValidationMirrorRuntime {
    fn detect_git_repo_root(&self, workspace_dir: &str) -> Option<String> {
        Some(workspace_dir.to_string())
    }

    fn command_succeeds(&self, _program: &str, _args: &[&str]) -> bool {
        false
    }

    fn run_command(
        &self,
        _program: &str,
        _args: &[&str],
        _current_dir: Option<&str>,
    ) -> Option<String> {
        None
    }

    fn run_command_with_stdin(
        &self,
        _program: &str,
        _args: &[&str],
        _stdin_body: &str,
    ) -> Option<String> {
        None
    }

    fn find_executable(&self, _program: &str) -> Option<PathBuf> {
        None
    }

    fn gh_auth_status(&self, _repo_root: Option<&str>) -> bool {
        false
    }

    fn current_timestamp(&self) -> String {
        "2026-08-07T00:00:00Z".to_string()
    }

    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn ensure_directory_exists(&self, path: &Path) -> std::io::Result<()> {
        self.ensured_directories
            .lock()
            .expect("directory lock")
            .push(path.to_path_buf());
        Ok(())
    }

    fn write_runtime_mirror_atomic(
        &self,
        _pool_root: &Path,
        relative: &Path,
        body: &str,
    ) -> std::io::Result<()> {
        self.files
            .lock()
            .expect("mirror lock")
            .insert(relative.to_path_buf(), body.to_string());
        Ok(())
    }

    fn read_runtime_mirror_optional(
        &self,
        _pool_root: &Path,
        relative: &Path,
    ) -> std::io::Result<Option<String>> {
        Ok(self.body(relative))
    }

    fn read_runtime_mirror_directory(
        &self,
        _pool_root: &Path,
        relative: &Path,
    ) -> std::io::Result<Vec<(PathBuf, String)>> {
        Ok(self
            .files
            .lock()
            .expect("mirror lock")
            .iter()
            .filter_map(|(path, body)| {
                path.strip_prefix(relative)
                    .ok()
                    .map(|path| (path.to_path_buf(), body.clone()))
            })
            .collect())
    }

    fn remove_runtime_mirror_file(
        &self,
        _pool_root: &Path,
        relative: &Path,
    ) -> std::io::Result<()> {
        self.files.lock().expect("mirror lock").remove(relative);
        Ok(())
    }

    fn compare_and_swap_runtime_mirror_file(
        &self,
        _pool_root: &Path,
        relative: &Path,
        expected_body: Option<&str>,
        replacement_body: Option<&str>,
    ) -> std::io::Result<bool> {
        if self
            .reject_next_compare_and_swap
            .swap(false, Ordering::SeqCst)
        {
            return Ok(false);
        }
        let mut files = self.files.lock().expect("mirror lock");
        if files.get(relative).map(String::as_str) != expected_body {
            return Ok(false);
        }
        match replacement_body {
            Some(body) => {
                files.insert(relative.to_path_buf(), body.to_string());
            }
            None => {
                files.remove(relative);
            }
        }
        Ok(true)
    }
}

fn temp_workspace(prefix: &str) -> String {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "akra-pr-validation-{prefix}-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("workspace should create");
    path.display().to_string()
}

fn registered_record(key: &str) -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new(key).expect("record key"),
        PrValidationTarget::new("acme/widgets", 42).expect("target"),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new("1111111111111111111111111111111111111111")
                .expect("source sha"),
            PrValidationCommitSha::new("2222222222222222222222222222222222222222")
                .expect("base sha"),
        ),
    )
}

fn distributor_attestation() -> IntegrationAttestation {
    let observed_at = DateTime::parse_from_rfc3339("2026-08-10T00:00:00Z")
        .expect("attestation timestamp")
        .with_timezone(&Utc);
    IntegrationAttestation::new(
        IntegrationMethod::DistributorCherryPick,
        PrValidationCommitSha::new("1111111111111111111111111111111111111111").expect("source sha"),
        Some(
            PrValidationCommitSha::new("2222222222222222222222222222222222222222")
                .expect("base sha"),
        ),
        PrValidationCommitSha::new("3333333333333333333333333333333333333333")
            .expect("evidence sha"),
        Some(42),
        None,
        observed_at,
        observed_at,
    )
    .expect("distributor attestation")
}

fn distributor_queue_record(key: &str) -> PlanningAuthorityDistributorQueueRecord {
    PlanningAuthorityDistributorQueueRecord {
        queue_item_id: key.to_string(),
        queue_order_key: 1,
        session_key: "session-attestation-race".to_string(),
        slot_id: "slot-1".to_string(),
        agent_id: "agent-1".to_string(),
        task_id: "task-attestation-race".to_string(),
        task_title: "Attestation race".to_string(),
        delivery_target: Some(PlanningAuthorityDistributorDeliveryTarget::new(
            "origin",
            "acme/widgets",
            GithubRepositoryVisibility::Private,
            "prerelease",
        )),
        source_branch: "akra-agent/slot-1/attestation-race".to_string(),
        source_base_commit_sha: "2222222222222222222222222222222222222222".to_string(),
        source_commit_sha: "1111111111111111111111111111111111111111".to_string(),
        branch_name: "akra-agent/slot-1/attestation-race".to_string(),
        worktree_path: "worktree-attestation-race".to_string(),
        commit_sha: "1111111111111111111111111111111111111111".to_string(),
        original_commit_sha: None,
        planning_refresh_state: "complete".to_string(),
        integration_state: "integrating".to_string(),
        integration_base_commit_sha: Some("2222222222222222222222222222222222222222".to_string()),
        integration_commit_sha: Some("3333333333333333333333333333333333333333".to_string()),
        conflict_files: Vec::new(),
        recovery_note: None,
        validation_summary: "passed".to_string(),
        authority_refresh_outcome: "complete".to_string(),
        github_capabilities: None,
        pull_request_number: Some(42),
        pull_request_url: Some("https://github.com/acme/widgets/pull/42".to_string()),
        queue_state: ParallelModeQueueItemState::Integrating,
        integration_note: "remote integration head verified".to_string(),
        enqueued_at: "2026-08-10T00:00:00Z".to_string(),
        updated_at: "2026-08-10T00:00:00Z".to_string(),
        retry_attempts: 0,
        retry_not_before: None,
    }
}

#[test]
fn validation_record_survives_authority_restart_and_repairs_a_missing_mirror() {
    let workspace = temp_workspace("restart-mirror");
    let pool_root = PathBuf::from(&workspace).join("pool");
    let authority = SqlitePlanningAuthorityAdapter::new();
    let runtime = ValidationMirrorRuntime::default();
    let record = registered_record("validation/restart");

    persist_pr_validation_record(&authority, &runtime, &workspace, &pool_root, None, &record)
        .expect("initial record should persist");
    let relative = pr_validation_record_relative_path(record.key());
    assert_eq!(
        runtime.body(&relative).as_deref(),
        Some(
            serde_json::to_string_pretty(&record)
                .expect("record should serialize")
                .as_str()
        )
    );

    runtime.clear();
    runtime.clear_ensured_directories();
    let restarted = SqlitePlanningAuthorityAdapter::new();
    let recovered = recover_pr_validation_record_mirror(
        &restarted,
        &runtime,
        &workspace,
        &pool_root,
        record.key(),
    )
    .expect("authority-backed recovery should succeed")
    .expect("record should survive restart");

    assert_eq!(recovered, record);
    assert_eq!(
        runtime.body(&relative),
        Some(serde_json::to_string_pretty(&record).expect("record should serialize"))
    );
    assert_eq!(runtime.ensured_directories(), vec![pool_root]);
}

#[test]
fn validation_attestation_and_remediation_correlation_survive_authority_restart() {
    let workspace = temp_workspace("restart-attestation-correlation");
    let pool_root = PathBuf::from(&workspace).join("pool");
    let authority = SqlitePlanningAuthorityAdapter::new();
    let runtime = ValidationMirrorRuntime::default();
    let finding = PrValidationFinding::new(
        PrValidationFindingKey::new(
            PrValidationFindingSource::new("check_run").unwrap(),
            "check:post-merge-gate",
        )
        .unwrap(),
        PrValidationCommitSha::new("1111111111111111111111111111111111111111").unwrap(),
        "Post-Merge Gate failed",
    )
    .unwrap();
    let record = registered_record("validation/restart-attested")
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::FindingObserved(finding.clone()))
        .unwrap()
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                finding.key().clone(),
                PrValidationRecordKey::new("remediation-restart-42").unwrap(),
            ),
        ))
        .unwrap()
        .transition(PrValidationEvent::RemediationStarted {
            finding_key: finding.key().clone(),
        })
        .unwrap()
        .transition(PrValidationEvent::IntegrationAttested(
            distributor_attestation(),
        ))
        .unwrap();

    persist_pr_validation_record(&authority, &runtime, &workspace, &pool_root, None, &record)
        .expect("attested record should persist");
    runtime.clear();
    let restarted = SqlitePlanningAuthorityAdapter::new();
    let recovered = recover_pr_validation_record_mirror(
        &restarted,
        &runtime,
        &workspace,
        &pool_root,
        record.key(),
    )
    .expect("restart recovery should succeed")
    .expect("attested record should survive restart");

    assert_eq!(recovered, record);
    assert_eq!(
        recovered.integration_attestation(),
        Some(&distributor_attestation())
    );
    assert!(
        recovered
            .remediation_for_task("remediation-restart-42")
            .is_some()
    );
}

#[test]
fn validation_persistence_prepares_pool_root_before_touching_mirror() {
    let workspace = temp_workspace("prepare-mirror-root");
    let pool_root = PathBuf::from(&workspace).join("pool");
    let authority = SqlitePlanningAuthorityAdapter::new();
    let runtime = ValidationMirrorRuntime::default();
    let record = registered_record("validation/prepare-mirror-root");

    persist_pr_validation_record(&authority, &runtime, &workspace, &pool_root, None, &record)
        .expect("validation record should persist");

    assert_eq!(runtime.ensured_directories(), vec![pool_root]);
}

#[test]
fn validation_record_authority_compare_and_swap_rejects_a_stale_transition() {
    let workspace = temp_workspace("authority-cas");
    let pool_root = PathBuf::from(&workspace).join("pool");
    let authority = SqlitePlanningAuthorityAdapter::new();
    let runtime = ValidationMirrorRuntime::default();
    let registered = registered_record("validation/cas");
    let observing = registered
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .expect("observation transition");
    let competing = registered
        .transition(PrValidationEvent::TargetShaChanged(
            PrValidationTargetShaSnapshot::new(
                PrValidationCommitSha::new("3333333333333333333333333333333333333333")
                    .expect("replacement source sha"),
                PrValidationCommitSha::new("4444444444444444444444444444444444444444")
                    .expect("replacement base sha"),
            ),
        ))
        .expect("competing transition");

    persist_pr_validation_record(
        &authority,
        &runtime,
        &workspace,
        &pool_root,
        None,
        &registered,
    )
    .expect("registration should persist");
    persist_pr_validation_record(
        &authority,
        &runtime,
        &workspace,
        &pool_root,
        Some(&registered),
        &observing,
    )
    .expect("first transition should persist");

    let error = persist_pr_validation_record(
        &authority,
        &runtime,
        &workspace,
        &pool_root,
        Some(&registered),
        &competing,
    )
    .expect_err("stale transition must lose authority CAS");

    assert!(error.contains("changed before validation transition"));
    assert_eq!(
        authority
            .load_runtime_pr_validation_record(&workspace, registered.key())
            .expect("authority record should load"),
        Some(observing.clone())
    );

    let attested = registered
        .transition(PrValidationEvent::IntegrationAttested(
            distributor_attestation(),
        ))
        .expect("attestation transition");
    let error = persist_pr_validation_record(
        &authority,
        &runtime,
        &workspace,
        &pool_root,
        Some(&registered),
        &attested,
    )
    .expect_err("stale integration attestation must lose authority CAS");
    assert!(error.contains("changed before validation transition"));
    assert_eq!(
        authority
            .load_runtime_pr_validation_record(&workspace, registered.key())
            .expect("authority record should load after stale attestation"),
        Some(observing)
    );
}

#[test]
fn distributor_attestation_retries_a_benign_authority_cas_race() {
    let workspace = temp_workspace("attestation-cas-retry");
    let pool_root = PathBuf::from(&workspace).join("pool");
    let authority = SqlitePlanningAuthorityAdapter::new();
    let runtime = ValidationMirrorRuntime::default();
    let registered = registered_record("validation/attestation-race");
    persist_pr_validation_record(
        &authority,
        &runtime,
        &workspace,
        &pool_root,
        None,
        &registered,
    )
    .expect("registration should persist before the scripted race");
    let race_workspace = workspace.clone();
    let race_registered = registered.clone();
    install_before_distributor_attestation_persist_hook(move || {
        let concurrent = race_registered
            .transition(PrValidationEvent::BeginPreMergeObservation)
            .expect("scripted concurrent observation should be valid");
        assert!(
            SqlitePlanningAuthorityAdapter::new()
                .compare_and_swap_runtime_pr_validation_record(
                    &race_workspace,
                    race_registered.key(),
                    Some(&race_registered),
                    Some(&concurrent),
                )
                .expect("scripted concurrent authority CAS should execute"),
            "scripted concurrent observation should win the first authority CAS"
        );
    });

    attest_distributor_pr_validation_with_ports(
        &authority,
        &runtime,
        &workspace,
        &pool_root,
        &distributor_queue_record("validation/attestation-race"),
    )
    .expect("benign concurrent observation should be retried");

    let stored = authority
        .load_runtime_pr_validation_record(&workspace, registered.key())
        .expect("authority should remain readable")
        .expect("attested record should remain present");
    assert_eq!(
        stored
            .integration_attestation()
            .expect("distributor evidence should be attested")
            .evidence_sha()
            .as_str(),
        "3333333333333333333333333333333333333333"
    );
    assert_eq!(
        runtime.body(&pr_validation_record_relative_path(stored.key())),
        Some(serde_json::to_string_pretty(&stored).expect("stored record should serialize"))
    );
}

#[test]
fn validation_record_mirror_cas_failure_rolls_back_the_exact_authority_write() {
    let workspace = temp_workspace("mirror-cas-rollback");
    let pool_root = PathBuf::from(&workspace).join("pool");
    let authority = SqlitePlanningAuthorityAdapter::new();
    let runtime = ValidationMirrorRuntime::default();
    let record = registered_record("validation/rollback");
    runtime.reject_next_compare_and_swap();

    let error =
        persist_pr_validation_record(&authority, &runtime, &workspace, &pool_root, None, &record)
            .expect_err("mirror CAS failure should fail persistence");

    assert!(error.contains("exact previous authority snapshot restored"));
    assert_eq!(
        authority
            .load_runtime_pr_validation_record(&workspace, record.key())
            .expect("authority should remain readable"),
        None
    );
    assert_eq!(
        runtime.body(&pr_validation_record_relative_path(record.key())),
        None
    );
}
