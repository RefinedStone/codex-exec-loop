/*
SQLite planning authority adapter가 application port의 snapshot 계약을 실제 DB 저장소 위에서도
지키는지 검증한다. service 계층은 `PlanningTaskRepositoryPort`만 보므로, 테스트는 concrete adapter를
포트 메서드로 호출해 task authority 문서와 queue projection의 동시 round-trip을 고정한다.
*/
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::app_server_prompt_log_port::{
    AppServerPromptInputRecord, AppServerPromptInteractionRecord, AppServerPromptLogPort,
    AppServerPromptOutputRecord,
};
use crate::application::port::outbound::github_automation_port::GithubRepositoryVisibility;
use crate::application::port::outbound::parallel_mode_runtime_event_log_port::{
    ParallelModeRuntimeEventLogPort, ParallelModeRuntimeEventLogRequest,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityActiveDocumentMutation, PlanningAuthorityDistributorDeliveryTarget,
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityDocumentCommit,
    PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityOfficialRefreshRecoveryStatus,
    PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit,
    PlanningTaskAuthorityCommitResult, PlanningTaskAuthorityMutationAudit,
    PlanningTaskAuthorityMutationKind, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningWorkspaceLoadRecord, RepoScopedPlanningWorkspacePort,
};
use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterRepositoryPort,
    ReviewCenterThreadProjection,
};
use crate::application::port::outbound::telegram_update_ledger_port::{
    TelegramRunnerLeaseClaimDecision, TelegramUpdateLedgerPort,
};
use crate::application::service::planning::{
    PlanningQueueCancellationRequest, PlanningQueueCancellationTarget, PlanningServices,
    RESULT_OUTPUT_FILE_PATH,
};
use crate::domain::parallel_mode::{
    IntegrationMethod, ParallelModeAgentSessionDetailSnapshot, ParallelModeAutomationTrigger,
    ParallelModeDispatchBlockReason, ParallelModeDispatchCommandSnapshot,
    ParallelModeDispatchCommandState, ParallelModePoolResetPolicy, ParallelModePoolResetReport,
    ParallelModePoolResetRunId, ParallelModePoolResetSlotAction, ParallelModePoolResetSlotOutcome,
    ParallelModePoolResetSlotReport, ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot,
    ParallelModeSlotLeaseState, ParallelModeTaskDispatchBlockSnapshot, PrValidationCommitSha,
    PrValidationRecord, PrValidationRecordKey, PrValidationTarget, PrValidationTargetShaSnapshot,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    PlanningAuthorityLocation, PriorityQueueProjection, PriorityQueueService,
    PriorityQueueSkippedTask, PriorityQueueTask, QueueIdleConfig, QueueIdlePolicy, TaskActor,
    TaskAuthorityDocument, TaskDefinition, TaskMutationProvenance, TaskStatus,
};
use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;
use std::sync::{Arc, Barrier};

use super::{
    ADMIN_FILE_SYNC_CLAIM_KIND, ADMIN_TASK_MUTATION_CLAIM_KIND, DISTRIBUTOR_QUEUE_CLAIM_KIND,
    OFFICIAL_REFRESH_CLAIM_KIND, OFFICIAL_REFRESH_SCOPE_KEY, open_authority_connection,
    task_authority_rows::replace_task_authority_tables,
};
#[cfg(windows)]
use super::{
    WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
    WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC,
};
#[cfg(any(unix, windows))]
use super::{
    authority_store_sidecar_path, prepare_private_authority_sidecar_files,
    secure_opened_private_authority_sidecar_file,
};

// 테스트마다 SQLite namespace를 분리하는 workspace directory를 만든다. adapter가 workspace path를
// DB 파일/row scope의 기준으로 쓰므로, 프로세스 id와 nanos를 섞어 병렬 테스트 충돌을 피한다.
fn temp_workspace(prefix: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "codex-exec-loop-db-{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos()
    ));
    // SQLite adapter가 workspace 아래에 database 파일을 열 수 있도록 directory를 먼저 만든다.
    // 실패는 테스트 환경 문제이므로 expect로 즉시 드러낸다.
    std::fs::create_dir_all(&path).expect("workspace should create");
    path.display().to_string()
}

#[cfg(any(unix, windows))]
fn authority_location_for_store(store_path: &std::path::Path) -> PlanningAuthorityLocation {
    let runtime_dir = store_path
        .parent()
        .expect("test authority store should have a runtime parent");
    PlanningAuthorityLocation {
        workspace_root: runtime_dir.display().to_string(),
        canonical_repo_root: runtime_dir.display().to_string(),
        repository_identity: runtime_dir.display().to_string(),
        runtime_dir: runtime_dir.display().to_string(),
        authority_store_path: store_path.display().to_string(),
    }
}
fn run_git_command(repo_root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command should spawn");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn temp_git_repo_with_linked_worktree(prefix: &str) -> (String, String) {
    let repo_root = std::path::PathBuf::from(temp_workspace(prefix)).join("repo");
    std::fs::create_dir_all(&repo_root).expect("temp git repo should be created");
    run_git_command(&repo_root, &["init", "-b", "prerelease"]);
    run_git_command(&repo_root, &["config", "user.name", "Akra Test"]);
    run_git_command(
        &repo_root,
        &["config", "user.email", "akra-test@example.com"],
    );
    std::fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
    run_git_command(&repo_root, &["add", "README.md"]);
    run_git_command(&repo_root, &["commit", "-m", "seed repo"]);

    let linked_worktree = repo_root
        .parent()
        .expect("temp git repo parent should exist")
        .join("linked-worktree");
    run_git_command(
        &repo_root,
        &[
            "worktree",
            "add",
            "-b",
            "feature/test-linked",
            linked_worktree.to_string_lossy().as_ref(),
            "prerelease",
        ],
    );

    (
        repo_root.display().to_string(),
        linked_worktree.display().to_string(),
    )
}

fn seed_git_checkout(repo_root: &std::path::Path) {
    std::fs::create_dir_all(repo_root).expect("seed checkout should create");
    run_git_command(repo_root, &["init", "-b", "prerelease"]);
    run_git_command(repo_root, &["config", "user.name", "Akra Test"]);
    run_git_command(
        repo_root,
        &["config", "user.email", "akra-test@example.com"],
    );
    std::fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
    run_git_command(repo_root, &["add", "README.md"]);
    run_git_command(repo_root, &["commit", "-m", "seed repo"]);
}

fn sibling_bare_backed_worktrees(prefix: &str) -> (String, String, String, String) {
    let fixture_root = std::path::PathBuf::from(temp_workspace(prefix));
    let seed = fixture_root.join("seed");
    seed_git_checkout(&seed);

    let mut worktrees = Vec::new();
    for label in ["a", "b"] {
        let bare_repo = fixture_root.join(format!("repo-{label}.git"));
        run_git_command(
            &fixture_root,
            &[
                "clone",
                "--bare",
                seed.to_string_lossy().as_ref(),
                bare_repo.to_string_lossy().as_ref(),
            ],
        );
        let worktree = fixture_root.join(format!("bare-worktree-{label}"));
        run_git_command(
            &bare_repo,
            &[
                "worktree",
                "add",
                "-b",
                &format!("feature/bare-{label}"),
                worktree.to_string_lossy().as_ref(),
                "prerelease",
            ],
        );
        let sibling_worktree = fixture_root.join(format!("bare-worktree-{label}-sibling"));
        run_git_command(
            &bare_repo,
            &[
                "worktree",
                "add",
                "-b",
                &format!("feature/bare-{label}-sibling"),
                sibling_worktree.to_string_lossy().as_ref(),
                "prerelease",
            ],
        );
        worktrees.push((
            worktree.display().to_string(),
            sibling_worktree.display().to_string(),
        ));
    }
    let (workspace_a, sibling_a) = worktrees.remove(0);
    let (workspace_b, sibling_b) = worktrees.remove(0);
    (workspace_a, sibling_a, workspace_b, sibling_b)
}

fn sibling_separate_git_dir_worktrees(prefix: &str) -> (String, String, String, String) {
    let fixture_root = std::path::PathBuf::from(temp_workspace(prefix));
    let mut worktrees = Vec::new();
    for label in ["a", "b"] {
        let checkout = fixture_root.join(format!("checkout-{label}"));
        let git_dir = fixture_root.join(format!("metadata-{label}.git"));
        std::fs::create_dir_all(&checkout).expect("separate-git-dir checkout should create");
        run_git_command(
            &checkout,
            &[
                "init",
                "-b",
                "prerelease",
                "--separate-git-dir",
                git_dir.to_string_lossy().as_ref(),
            ],
        );
        run_git_command(&checkout, &["config", "user.name", "Akra Test"]);
        run_git_command(
            &checkout,
            &["config", "user.email", "akra-test@example.com"],
        );
        std::fs::write(checkout.join("README.md"), "seed\n")
            .expect("separate checkout seed should write");
        run_git_command(&checkout, &["add", "README.md"]);
        run_git_command(&checkout, &["commit", "-m", "seed repo"]);

        let worktree = fixture_root.join(format!("separate-worktree-{label}"));
        run_git_command(
            &checkout,
            &[
                "worktree",
                "add",
                "-b",
                &format!("feature/separate-{label}"),
                worktree.to_string_lossy().as_ref(),
                "prerelease",
            ],
        );
        worktrees.push((
            checkout.display().to_string(),
            worktree.display().to_string(),
        ));
    }
    let (checkout_a, worktree_a) = worktrees.remove(0);
    let (checkout_b, worktree_b) = worktrees.remove(0);
    (checkout_a, worktree_a, checkout_b, worktree_b)
}

fn prompt_interaction_record(
    workspace_dir: &str,
    interaction_id: &str,
) -> AppServerPromptInteractionRecord {
    let completed_at = Utc::now().to_rfc3339();
    AppServerPromptInteractionRecord {
        sequence: 0,
        interaction_id: interaction_id.to_string(),
        session_kind: "main".to_string(),
        operation: "turn".to_string(),
        status: "completed".to_string(),
        workspace_dir: workspace_dir.to_string(),
        thread_id: Some("thread-isolation".to_string()),
        turn_id: Some("turn-isolation".to_string()),
        service_name: None,
        model: None,
        reasoning_effort: None,
        developer_instructions: Some("repository-private prompt".to_string()),
        input_items: vec![AppServerPromptInputRecord::new(
            "text",
            "turn input",
            "repository-private input",
        )],
        output_items: vec![AppServerPromptOutputRecord::new(
            "output-isolation",
            Some("final".to_string()),
            "repository-private output",
        )],
        error_message: None,
        started_at: completed_at.clone(),
        completed_at,
    }
}

fn assert_repository_authority_isolated(workspace_a: &str, workspace_b: &str) {
    let location_a =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace_a)
            .expect("repository A authority location should resolve");
    let location_b =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace_b)
            .expect("repository B authority location should resolve");
    assert_ne!(
        location_a.repository_identity,
        location_b.repository_identity
    );
    assert_ne!(
        location_a.authority_store_path, location_b.authority_store_path,
        "different Git common dirs must never share an authority DB"
    );

    let adapter = SqlitePlanningAuthorityAdapter::new();
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    adapter
        .commit_task_authority_snapshot(
            workspace_a,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("repository A task authority should persist");
    SqlitePlanningAuthorityAdapter::stage_repo_scoped_draft_files(
        workspace_a,
        "private-draft",
        &[PlanningDraftFileRecord {
            active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            body: "repository A draft".to_string(),
        }],
    )
    .expect("repository A draft should persist");
    adapter
        .append_app_server_prompt_interaction(
            workspace_a,
            prompt_interaction_record(workspace_a, "repository-a-prompt"),
        )
        .expect("repository A prompt should persist");
    assert_eq!(
        adapter
            .try_acquire_runner_lease(workspace_a, "bot-id:repository-isolation", "runner-a", 300)
            .expect("repository A Telegram lease should acquire"),
        TelegramRunnerLeaseClaimDecision::Acquired
    );

    assert!(
        adapter
            .load_task_authority_snapshot(workspace_b)
            .expect("repository B task authority should inspect")
            .is_none(),
        "task data must not cross repository identities"
    );
    assert!(
        SqlitePlanningAuthorityAdapter::load_repo_scoped_draft_files(workspace_b, "private-draft",)
            .is_err(),
        "draft data must not cross repository identities"
    );
    assert!(
        adapter
            .load_recent_app_server_prompt_interactions(workspace_b, 10)
            .expect("repository B prompt log should inspect")
            .records
            .is_empty(),
        "prompt data must not cross repository identities"
    );
    assert_eq!(
        adapter
            .try_acquire_runner_lease(workspace_b, "bot-id:repository-isolation", "runner-b", 300)
            .expect("repository B Telegram lease should be independent"),
        TelegramRunnerLeaseClaimDecision::Acquired,
        "Telegram data must not cross repository identities"
    );
}

fn assert_worktrees_share_repository_authority(workspace_a: &str, workspace_b: &str) {
    let location_a =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace_a)
            .expect("first worktree authority location should resolve");
    let location_b =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace_b)
            .expect("second worktree authority location should resolve");
    assert_eq!(
        location_a.repository_identity,
        location_b.repository_identity
    );
    assert_eq!(
        location_a.authority_store_path, location_b.authority_store_path,
        "worktrees backed by one Git common dir must share one authority DB"
    );
}

fn authority_connection(workspace_dir: &str) -> rusqlite::Connection {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace_dir)
            .expect("authority location should resolve");
    open_authority_connection(&location).expect("authority db should open")
}

#[test]
fn authority_schema_migrates_v7_through_v12_additively_and_rejects_unsupported_versions() {
    for legacy_version in [7, 8, 9, 10, 11, 12] {
        let workspace_dir = temp_workspace(&format!("schema-migrate-v{legacy_version}"));
        let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
            &workspace_dir,
        )
        .expect("authority location should resolve");
        let connection = open_authority_connection(&location).expect("current store should open");
        let v8_objects = if legacy_version == 7 {
            "DROP INDEX idx_telegram_update_inbox_stream_state_id;
             DROP TABLE telegram_update_inbox;
             DROP TABLE telegram_update_runner_leases;
             DROP TABLE telegram_update_streams;
             DROP TABLE retired_planning_tasks;"
        } else {
            ""
        };
        connection
            .execute_batch(&format!(
                "{v8_objects}
                 DROP INDEX idx_planning_task_mutation_events_revision;
                 DROP TABLE planning_task_mutation_events;
                 DROP TABLE planning_file_sync_baselines;
                 INSERT OR REPLACE INTO active_documents (relative_path, content)
                 VALUES ('legacy.md', 'legacy body');
                 UPDATE authority_metadata SET value = '{legacy_version}'
                 WHERE key = 'schema_version';"
            ))
            .expect("legacy schema fixture should install");
        drop(connection);

        let migrated = open_authority_connection(&location).expect("legacy store should migrate");
        let version: String = migrated
            .query_row(
                "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("migrated version should load");
        assert_eq!(version, "13");
        assert_eq!(
            migrated
                .query_row(
                    "SELECT content FROM active_documents WHERE relative_path = 'legacy.md'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("legacy authority row should survive migration"),
            "legacy body"
        );
        let expected_objects = if legacy_version == 7 {
            vec![
                ("table", "retired_planning_tasks"),
                ("table", "telegram_update_streams"),
                ("table", "telegram_update_runner_leases"),
                ("table", "telegram_update_inbox"),
                ("index", "idx_telegram_update_inbox_stream_state_id"),
                ("table", "planning_file_sync_baselines"),
                ("table", "planning_task_mutation_events"),
                ("index", "idx_planning_task_mutation_events_revision"),
            ]
        } else {
            vec![
                ("table", "planning_file_sync_baselines"),
                ("table", "planning_task_mutation_events"),
                ("index", "idx_planning_task_mutation_events_revision"),
            ]
        };
        for (object_type, object_name) in expected_objects {
            assert!(
                migrated
                    .query_row(
                        "SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2",
                        (object_type, object_name),
                        |_| Ok(()),
                    )
                    .optional()
                    .expect("migrated schema object should inspect")
                    .is_some(),
                "{object_type} `{object_name}` should be recreated from v{legacy_version}"
            );
        }
        assert!(
            migrated
                .query_row(
                    "SELECT 1 FROM sqlite_master
                     WHERE type = 'table' AND name = 'runtime_pr_validation_records'",
                    [],
                    |_| Ok(()),
                )
                .optional()
                .expect("PR validation authority table should inspect")
                .is_some(),
            "PR validation authority table should be created from v{legacy_version}"
        );
    }

    for unsupported_version in ["6", "14", "not-a-version"] {
        let workspace_dir = temp_workspace("schema-reject-unsupported");
        let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
            &workspace_dir,
        )
        .expect("authority location should resolve");
        let connection = open_authority_connection(&location).expect("current store should open");
        connection
            .execute(
                "UPDATE authority_metadata SET value = ?1 WHERE key = 'schema_version'",
                [unsupported_version],
            )
            .expect("unsupported version fixture should install");
        drop(connection);
        let error = open_authority_connection(&location)
            .expect_err("unsupported authority schema must fail closed");
        assert!(
            error
                .to_string()
                .contains("unsupported authority-store schema version")
        );
    }

    let workspace_dir = temp_workspace("schema-reject-missing-version");
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("authority location should resolve");
    let connection = open_authority_connection(&location).expect("current store should open");
    connection
        .execute(
            "DELETE FROM authority_metadata WHERE key = 'schema_version'",
            [],
        )
        .expect("schema version should delete");
    drop(connection);
    let error =
        open_authority_connection(&location).expect_err("missing schema marker must fail closed");
    assert!(error.to_string().contains("schema version is missing"));
}

#[test]
fn authority_schema_migrates_v12_pr_validation_schedule_without_data_loss() {
    let workspace_dir = temp_workspace("schema-migrate-v12-pr-validation-scheduler");
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("authority location should resolve");
    let connection = open_authority_connection(&location).expect("current store should open");
    let record = PrValidationRecord::register(
        PrValidationRecordKey::new("scheduler-validation-42").unwrap(),
        PrValidationTarget::new("acme/widgets", 42).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new("1111111111111111111111111111111111111111").unwrap(),
            PrValidationCommitSha::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
        ),
    );
    let updated_at = "2026-08-10T00:00:00+00:00";
    connection
        .execute_batch(
            "DROP TABLE runtime_pr_validation_records;
             CREATE TABLE runtime_pr_validation_records (
                 record_key TEXT PRIMARY KEY,
                 updated_at TEXT NOT NULL,
                 content TEXT NOT NULL,
                 integration_method TEXT,
                 integration_source_sha TEXT,
                 integration_evidence_sha TEXT,
                 integration_remote_verified_at TEXT
             );
             UPDATE authority_metadata SET value = '12' WHERE key = 'schema_version';",
        )
        .expect("v12 scheduler fixture should install");
    connection
        .execute(
            "INSERT INTO runtime_pr_validation_records (record_key, updated_at, content)
             VALUES (?1, ?2, ?3)",
            (
                record.key().as_str(),
                updated_at,
                serde_json::to_string(&record).unwrap(),
            ),
        )
        .expect("v12 validation record should persist");
    drop(connection);

    let migrated = open_authority_connection(&location).expect("v12 store should migrate");
    let row: (String, String, String, i64, i64) = migrated
        .query_row(
            "SELECT validation_repository, validation_phase, next_poll_at,
                    poll_attempt, consecutive_error_count
             FROM runtime_pr_validation_records WHERE record_key = ?1",
            [record.key().as_str()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("migrated scheduler columns should load");
    assert_eq!(
        row,
        (
            "acme/widgets".to_string(),
            "Registered".to_string(),
            updated_at.to_string(),
            0,
            0
        )
    );
    assert_eq!(
        migrated
            .query_row(
                "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "13"
    );
    drop(migrated);

    let due_at = "2026-08-10T00:00:01+00:00"
        .parse::<DateTime<Utc>>()
        .unwrap();
    assert_eq!(
        SqlitePlanningAuthorityAdapter::load_due_runtime_pr_validation_record_keys(
            &workspace_dir,
            due_at,
            due_at - chrono::TimeDelta::seconds(30),
            8,
        )
        .unwrap(),
        vec![record.key().clone()]
    );
}

#[test]
fn authority_schema_migrates_v11_legacy_merge_evidence_without_data_loss() {
    let workspace_dir = temp_workspace("schema-migrate-v11-pr-validation");
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("authority location should resolve");
    let connection = open_authority_connection(&location).expect("current store should open");
    let legacy = serde_json::json!({
        "key": "legacy-validation-42",
        "target": {
            "repository": "acme/widgets",
            "pull_request_number": 42
        },
        "target_shas": {
            "source_sha": "1111111111111111111111111111111111111111",
            "base_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        "phase": "PostMergeObservation",
        "findings": {},
        "remediations": {},
        "active_remediation": null,
        "merge_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "observation_revision": 3,
        "observation_cursor": "legacy-cursor",
        "evidence_fingerprint": "legacy-fingerprint",
        "post_merge_checkpoint_revision": 3,
        "completion": null,
        "terminal_reason": null
    });
    connection
        .execute_batch(
            "DROP TABLE runtime_pr_validation_records;
             CREATE TABLE runtime_pr_validation_records (
                 record_key TEXT PRIMARY KEY,
                 updated_at TEXT NOT NULL,
                 content TEXT NOT NULL
             );
             UPDATE authority_metadata SET value = '11' WHERE key = 'schema_version';",
        )
        .expect("v11 table fixture should install");
    connection
        .execute(
            "INSERT INTO runtime_pr_validation_records (record_key, updated_at, content)
             VALUES (?1, ?2, ?3)",
            (
                "legacy-validation-42",
                "2026-08-10T00:00:00Z",
                legacy.to_string(),
            ),
        )
        .expect("legacy validation fixture should persist");
    drop(connection);

    let migrated = open_authority_connection(&location).expect("v11 store should migrate");
    let version: String = migrated
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("migrated version should load");
    assert_eq!(version, "13");
    let (method, evidence_sha, content): (String, String, String) = migrated
        .query_row(
            "SELECT integration_method, integration_evidence_sha, content
             FROM runtime_pr_validation_records WHERE record_key = 'legacy-validation-42'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("migrated attestation columns should load");
    assert_eq!(method, "github_rebase_merge");
    assert_eq!(evidence_sha, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert!(!content.contains("\"merge_sha\""));

    drop(migrated);
    let record = SqlitePlanningAuthorityAdapter::load_runtime_pr_validation_record(
        &workspace_dir,
        &PrValidationRecordKey::new("legacy-validation-42").unwrap(),
    )
    .expect("migrated record should load")
    .expect("migrated record should remain present");
    let attestation = record
        .integration_attestation()
        .expect("legacy merge evidence should become an attestation");
    assert_eq!(attestation.method(), IntegrationMethod::GithubRebaseMerge);
    assert_eq!(record.observation_revision(), 3);
    assert_eq!(record.observation_cursor(), Some("legacy-cursor"));

    let migrated = open_authority_connection(&location).expect("migrated store should reopen");
    migrated
        .execute_batch(
            "CREATE TRIGGER reject_replayed_pr_validation_migration
             BEFORE UPDATE ON runtime_pr_validation_records
             BEGIN
                 SELECT RAISE(FAIL, 'PR validation data migration replayed');
             END;",
        )
        .expect("migration replay guard should install");
    drop(migrated);
    open_authority_connection(&location)
        .expect("schema v13 reopen must not replay the v11 data migration");
}

#[test]
fn authority_store_rejects_foreign_mode_and_repository_bindings() {
    for (metadata_key, foreign_value, expected_error) in [
        ("mode", "foreign-store", "unsupported authority-store mode"),
        (
            "repository_identity",
            "/foreign/repository.git",
            "bound to a different repository identity",
        ),
    ] {
        let workspace_dir = temp_workspace(&format!("foreign-binding-{metadata_key}"));
        let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
            &workspace_dir,
        )
        .expect("authority location should resolve");
        let connection = open_authority_connection(&location).expect("authority store should open");
        connection
            .execute(
                "UPDATE authority_metadata SET value = ?2 WHERE key = ?1",
                (metadata_key, foreign_value),
            )
            .expect("foreign authority marker should install");
        drop(connection);

        let error = open_authority_connection(&location)
            .expect_err("a foreign authority identity must fail closed before schema mutation");
        assert!(
            error.to_string().contains(expected_error),
            "unexpected foreign marker error: {error:#}"
        );
    }
}

#[test]
fn ordinary_repository_migrates_legacy_canonical_root_binding_in_place() {
    let (workspace_dir, _) = temp_git_repo_with_linked_worktree("legacy-repository-binding");
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("ordinary repository location should resolve");
    let connection = open_authority_connection(&location).expect("authority store should open");
    connection
        .execute(
            "DELETE FROM authority_metadata WHERE key = 'repository_identity'",
            [],
        )
        .expect("repository identity should be removed for legacy fixture");
    drop(connection);

    let reopened = open_authority_connection(&location)
        .expect("legacy canonical-root binding should migrate in place");
    assert_eq!(
        reopened
            .query_row(
                "SELECT value FROM authority_metadata WHERE key = 'repository_identity'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("migrated repository identity should load"),
        location.repository_identity
    );
}

#[cfg(unix)]
#[test]
fn authority_store_does_not_adopt_an_unmarked_nonempty_sqlite_database() {
    use std::os::unix::fs::PermissionsExt;

    let fixture_root = std::path::PathBuf::from(temp_workspace("foreign-unmarked-database"));
    let store = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");
    std::fs::create_dir_all(store.parent().expect("store parent should exist"))
        .expect("foreign database parent should create");
    let foreign = rusqlite::Connection::open(&store).expect("foreign SQLite database should open");
    foreign
        .execute_batch("CREATE TABLE foreign_customer_data (secret TEXT NOT NULL);")
        .expect("foreign schema should install");
    drop(foreign);
    std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o600))
        .expect("foreign database should use private fixture permissions");

    let error = open_authority_connection(&authority_location_for_store(&store))
        .expect_err("an unmarked non-empty database must not be mutated into an Akra store");
    assert!(error.to_string().contains("cannot be adopted"));
    let inspection = rusqlite::Connection::open(&store).expect("foreign database should reopen");
    assert_eq!(
        inspection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'foreign_customer_data'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("foreign table should remain inspectable"),
        1
    );
    assert_eq!(
        inspection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'authority_metadata'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("Akra marker absence should remain inspectable"),
        0,
        "rejection must occur before Akra creates any schema"
    );
}

#[test]
fn authority_schema_migration_rolls_back_additive_ddl_when_version_update_fails() {
    let workspace_dir = temp_workspace("schema-migration-rollback");
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("authority location should resolve");
    let connection = open_authority_connection(&location).expect("current store should open");
    connection
        .execute_batch(
            "DROP TABLE planning_file_sync_baselines;
             DROP TABLE runtime_pr_validation_records;
             CREATE TABLE runtime_pr_validation_records (
                 record_key TEXT PRIMARY KEY,
                 updated_at TEXT NOT NULL,
                 content TEXT NOT NULL
             );
             UPDATE authority_metadata SET value = '11' WHERE key = 'schema_version';
             CREATE TRIGGER fail_schema_migration_version
             BEFORE UPDATE OF value ON authority_metadata
             WHEN OLD.key = 'schema_version'
             BEGIN
                 SELECT RAISE(FAIL, 'forced schema version failure');
             END;",
        )
        .expect("failing migration fixture should install");
    drop(connection);

    let error = open_authority_connection(&location)
        .expect_err("forced schema metadata failure should abort migration");
    assert!(
        error
            .to_string()
            .contains("failed to update authority metadata")
    );
    let raw = rusqlite::Connection::open(&location.authority_store_path)
        .expect("raw authority store should open for rollback inspection");
    assert!(
        raw.query_row(
            "SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = 'planning_file_sync_baselines'",
            [],
            |_| Ok(()),
        )
        .optional()
        .expect("rolled-back table should inspect")
        .is_none(),
        "failed migration must roll back additive DDL"
    );
    assert_eq!(
        raw.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('runtime_pr_validation_records')
             WHERE name = 'integration_evidence_sha'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("rolled-back attestation column should inspect"),
        0,
        "failed migration must roll back PR validation attestation columns"
    );
    assert_eq!(
        raw.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('runtime_pr_validation_records')
             WHERE name = 'next_poll_at'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("rolled-back scheduler column should inspect"),
        0,
        "failed migration must roll back PR validation scheduler columns"
    );
}

#[test]
fn authority_connection_busy_timeout_waits_for_a_short_write_lock() {
    let workspace_dir = temp_workspace("busy-timeout-short-lock");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let stream_key = "bot-id:810001";
    let initial_owner = "runner-initial";
    assert_eq!(
        adapter
            .try_acquire_runner_lease(&workspace_dir, stream_key, initial_owner, 300)
            .expect("initial Telegram lease should create the authority schema"),
        TelegramRunnerLeaseClaimDecision::Acquired
    );
    assert!(
        adapter
            .release_runner_lease(&workspace_dir, stream_key, initial_owner)
            .expect("initial Telegram lease should release")
    );

    let lock_connection = authority_connection(&workspace_dir);
    lock_connection
        .execute_batch("BEGIN IMMEDIATE")
        .expect("fixture write lock should acquire");
    let (started_sender, started_receiver) = std::sync::mpsc::channel();
    let worker_workspace = workspace_dir.clone();
    let worker = std::thread::spawn(move || {
        started_sender
            .send(())
            .expect("busy-timeout worker start should publish");
        let started_at = std::time::Instant::now();
        let result = SqlitePlanningAuthorityAdapter::new().try_acquire_runner_lease(
            &worker_workspace,
            stream_key,
            "runner-after-short-lock",
            300,
        );
        (result, started_at.elapsed())
    });
    started_receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("busy-timeout worker should start");
    std::thread::sleep(std::time::Duration::from_millis(200));
    lock_connection
        .execute_batch("COMMIT")
        .expect("fixture write lock should release");

    let (result, elapsed) = worker.join().expect("busy-timeout worker should join");
    assert_eq!(
        result.expect("adapter write should wait for the short lock"),
        TelegramRunnerLeaseClaimDecision::Acquired
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(150),
        "adapter write returned before the fixture lock was released: {elapsed:?}"
    );
}

#[test]
fn authority_connection_busy_timeout_preserves_context_when_the_lock_outlives_it() {
    let workspace_dir = temp_workspace("busy-timeout-expired-lock");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let stream_key = "bot-id:810002";
    let initial_owner = "runner-initial";
    assert_eq!(
        adapter
            .try_acquire_runner_lease(&workspace_dir, stream_key, initial_owner, 300)
            .expect("initial Telegram lease should create the authority schema"),
        TelegramRunnerLeaseClaimDecision::Acquired
    );
    assert!(
        adapter
            .release_runner_lease(&workspace_dir, stream_key, initial_owner)
            .expect("initial Telegram lease should release")
    );

    let lock_connection = authority_connection(&workspace_dir);
    lock_connection
        .execute_batch("BEGIN IMMEDIATE")
        .expect("fixture write lock should acquire");
    let started_at = std::time::Instant::now();
    let error = adapter
        .try_acquire_runner_lease(&workspace_dir, stream_key, "runner-timeout", 300)
        .expect_err("a write lock beyond the configured timeout should fail");
    let elapsed = started_at.elapsed();
    lock_connection
        .execute_batch("ROLLBACK")
        .expect("fixture write lock should release after timeout");

    let message = format!("{error:#}");
    assert!(
        message.contains("failed to open Telegram runner lease transaction")
            || message.contains("failed to initialize authority-store schema")
            || message.contains("failed to open authority-store schema migration transaction"),
        "adapter context should be retained: {message}"
    );
    assert!(
        message.contains("database is locked") || message.contains("database table is locked"),
        "SQLite lock cause should be retained: {message}"
    );
    assert!(
        elapsed >= std::time::Duration::from_secs(4),
        "busy timeout failed too quickly: {elapsed:?}"
    );
}

fn set_claim_timestamp(workspace_dir: &str, claim_kind: &str, scope_key: &str, claimed_at: &str) {
    let connection = authority_connection(workspace_dir);
    let changed_rows = connection
        .execute(
            "UPDATE runtime_claims
             SET claimed_at = ?1
             WHERE claim_kind = ?2 AND scope_key = ?3",
            (claimed_at, claim_kind, scope_key),
        )
        .expect("runtime claim timestamp should update");
    assert_eq!(changed_rows, 1);
}

fn runtime_claim_owner_and_timestamp(
    workspace_dir: &str,
    claim_kind: &str,
    scope_key: &str,
) -> (String, String) {
    authority_connection(workspace_dir)
        .query_row(
            "SELECT owner_token, claimed_at
             FROM runtime_claims
             WHERE claim_kind = ?1 AND scope_key = ?2",
            (claim_kind, scope_key),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("runtime claim should exist")
}

fn set_authority_metadata(workspace_dir: &str, key: &str, value: &str) {
    authority_connection(workspace_dir)
        .execute(
            "INSERT INTO authority_metadata (key, value)
             VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            (key, value),
        )
        .expect("authority metadata should upsert");
}

fn replace_table_schema(connection: &rusqlite::Connection, table_name: &str, columns_sql: &str) {
    let sql = format!("DROP TABLE {table_name}; CREATE TABLE {table_name} ({columns_sql});");
    connection
        .execute_batch(&sql)
        .expect("runtime table schema should be replaced");
}

fn replace_runtime_table_schema(workspace_dir: &str, table_name: &str, columns_sql: &str) {
    replace_table_schema(
        &authority_connection(workspace_dir),
        table_name,
        columns_sql,
    );
}

fn corrupt_runtime_events_schema(workspace_dir: &str) {
    replace_runtime_table_schema(
        workspace_dir,
        "runtime_events",
        "sequence INTEGER PRIMARY KEY",
    );
}

fn install_failing_delete_trigger(workspace_dir: &str, table_name: &str, trigger_name: &str) {
    let sql = format!(
        "CREATE TRIGGER {trigger_name}
         BEFORE DELETE ON {table_name}
         BEGIN
             SELECT RAISE(FAIL, 'forced delete failure');
         END;"
    );
    authority_connection(workspace_dir)
        .execute_batch(&sql)
        .expect("failing delete trigger should install");
}

fn install_failing_insert_trigger(workspace_dir: &str, table_name: &str, trigger_name: &str) {
    let sql = format!(
        "CREATE TRIGGER {trigger_name}
         BEFORE INSERT ON {table_name}
         BEGIN
             SELECT RAISE(FAIL, 'forced insert failure');
         END;"
    );
    authority_connection(workspace_dir)
        .execute_batch(&sql)
        .expect("failing insert trigger should install");
}

fn dispatch_command_snapshot(seed: u64) -> ParallelModeDispatchCommandSnapshot {
    ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some(format!("queue-head-{seed}")),
        Some(seed),
        "2026-05-08T00:00:00+00:00",
    )
}

fn insert_pending_dispatch_command_row(
    workspace_dir: &str,
    command: &ParallelModeDispatchCommandSnapshot,
) {
    let payload_json =
        serde_json::to_string(command).expect("dispatch command payload should serialize");
    authority_connection(workspace_dir)
        .execute(
            "INSERT INTO runtime_dispatch_commands
                (command_id, command_state, created_at, content)
             VALUES (?1, ?2, ?3, ?4)",
            (
                command.command_id.as_str(),
                ParallelModeDispatchCommandState::Pending.label(),
                command.created_at.as_str(),
                payload_json.as_str(),
            ),
        )
        .expect("pending dispatch command row should insert");
}

fn insert_complete_pending_dispatch_command_row(
    workspace_dir: &str,
    command: &ParallelModeDispatchCommandSnapshot,
) {
    let payload_json =
        serde_json::to_string(command).expect("dispatch command payload should serialize");
    authority_connection(workspace_dir)
        .execute(
            "INSERT INTO runtime_dispatch_commands
                (command_id, command_kind, trigger, command_state, queue_head_signature,
                 epoch_id, created_at, updated_at, owner_token, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                command.command_id,
                command.kind.label(),
                command.trigger.label(),
                ParallelModeDispatchCommandState::Pending.label(),
                command.queue_head_signature,
                command.epoch_id.map(|value| value as i64),
                command.created_at,
                command.updated_at,
                command.owner_token,
                payload_json,
            ],
        )
        .expect("complete pending dispatch command row should insert");
}

fn assert_error_contains<T>(result: anyhow::Result<T>, expected: &str) {
    let Err(error) = result else {
        panic!("expected error containing `{expected}`");
    };
    let message = format!("{error:?}");
    assert!(message.contains(expected), "{message}");
}

fn insert_invalid_slot_marker(workspace_dir: &str, slot_id: &str) {
    let connection = authority_connection(workspace_dir);
    connection
        .execute(
            "INSERT OR REPLACE INTO runtime_invalid_slot_leases (slot_id, detected_at)
             VALUES (?1, ?2)",
            (slot_id, "2026-05-04T10:06:00+00:00"),
        )
        .expect("invalid slot marker should insert");
}
#[test]
fn review_center_repository_port_round_trips_workspace_scoped_data() {
    let (workspace_a, workspace_b) = temp_git_repo_with_linked_worktree("review-center");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let location_a =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_a)
            .expect("workspace A authority location should resolve");
    let location_b =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_b)
            .expect("workspace B authority location should resolve");
    assert_eq!(
        location_a.authority_store_path, location_b.authority_store_path,
        "linked worktrees in the same repo should share one authority DB"
    );
    assert_ne!(
        location_a.workspace_root, location_b.workspace_root,
        "linked worktrees should keep distinct workspace roots"
    );
    assert_eq!(
        location_a.repository_identity, location_b.repository_identity,
        "linked worktrees in the same repo should share one repository identity"
    );

    let mut thread_review = ReviewCenterThreadProjection::new(
        "thread-a",
        "review-a",
        "manual handoff",
        "review needed",
        "approval review needs human review",
        "2026-07-06T00:30:00Z",
        "2026-07-06T00:30:10Z",
    );
    thread_review.handoff_target = Some("operator".to_string());
    thread_review.handoff_note = Some("open inbox".to_string());
    let mut inbox_item = ReviewCenterInboxItem::new(
        "review-a",
        "thread-a",
        "pending",
        "approval review needs human review",
        "2026-07-06T00:30:00Z",
        "2026-07-06T00:30:10Z",
    );
    inbox_item.handoff_target = Some("operator".to_string());
    let history_entry = ReviewCenterHistoryEntry::new(
        "review-a",
        "thread-a",
        "review_requested",
        "approval review needs human review",
        "2026-07-06T00:30:11Z",
    );

    adapter
        .upsert_thread_review(&workspace_a, &thread_review)
        .expect("workspace A thread review should persist");
    adapter
        .replace_pending_inbox(&workspace_a, &[inbox_item.clone()])
        .expect("workspace A inbox should persist");
    adapter
        .append_history_entry(&workspace_a, &history_entry)
        .expect("workspace A history should persist");

    assert_eq!(
        adapter
            .load_thread_reviews(&workspace_a, "thread-a")
            .expect("workspace A thread review should load"),
        vec![thread_review.clone()]
    );
    assert_eq!(
        adapter
            .load_pending_inbox(&workspace_a)
            .expect("workspace A inbox should load"),
        vec![inbox_item.clone()]
    );
    assert_eq!(
        adapter
            .load_recent_history(&workspace_a)
            .expect("workspace A history should load"),
        vec![history_entry.clone()]
    );

    assert!(
        adapter
            .load_thread_reviews(&workspace_b, "thread-a")
            .expect("workspace B thread review should load")
            .is_empty(),
        "workspace B should stay isolated from workspace A review rows even in the shared repo DB"
    );
    assert!(
        adapter
            .load_pending_inbox(&workspace_b)
            .expect("workspace B inbox should load")
            .is_empty(),
        "workspace B should stay isolated from workspace A inbox rows even in the shared repo DB"
    );
    assert!(
        adapter
            .load_recent_history(&workspace_b)
            .expect("workspace B history should load")
            .is_empty(),
        "workspace B should stay isolated from workspace A history rows even in the shared repo DB"
    );
}

#[test]
fn sibling_bare_backed_worktrees_have_isolated_repository_authority() {
    let (workspace_a, sibling_a, workspace_b, sibling_b) =
        sibling_bare_backed_worktrees("bare-repository-authority-isolation");
    assert_worktrees_share_repository_authority(&workspace_a, &sibling_a);
    assert_worktrees_share_repository_authority(&workspace_b, &sibling_b);
    assert_repository_authority_isolated(&workspace_a, &workspace_b);
}

#[test]
fn sibling_separate_git_dir_worktrees_have_isolated_repository_authority() {
    let (checkout_a, workspace_a, checkout_b, workspace_b) =
        sibling_separate_git_dir_worktrees("separate-git-dir-authority-isolation");
    assert_worktrees_share_repository_authority(&checkout_a, &workspace_a);
    assert_worktrees_share_repository_authority(&checkout_b, &workspace_b);
    assert_repository_authority_isolated(&workspace_a, &workspace_b);
}

#[test]
fn repository_incarnation_is_stable_across_restart_and_move_but_not_clone() {
    let fixture_root = std::path::PathBuf::from(temp_workspace("repository-incarnation-move"));
    let original = fixture_root.join("original");
    seed_git_checkout(&original);

    let first = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
        original.to_string_lossy().as_ref(),
    )
    .expect("initial repository incarnation should resolve");
    let restarted = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
        original.to_string_lossy().as_ref(),
    )
    .expect("repository incarnation should survive another resolver instance");
    assert_eq!(first.repository_identity, restarted.repository_identity);
    assert_eq!(first.authority_store_path, restarted.authority_store_path);

    let moved = fixture_root.join("moved-and-renamed");
    std::fs::rename(&original, &moved).expect("repository including common dir should move");
    let moved_location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
        moved.to_string_lossy().as_ref(),
    )
    .expect("moved repository incarnation should resolve");
    assert_eq!(
        first.repository_identity,
        moved_location.repository_identity
    );
    assert_eq!(
        first.authority_store_path,
        moved_location.authority_store_path
    );
    assert_ne!(
        first.canonical_repo_root,
        moved_location.canonical_repo_root
    );

    let cloned = fixture_root.join("fresh-clone");
    run_git_command(
        &fixture_root,
        &[
            "clone",
            moved.to_string_lossy().as_ref(),
            cloned.to_string_lossy().as_ref(),
        ],
    );
    let clone_location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
        cloned.to_string_lossy().as_ref(),
    )
    .expect("fresh clone incarnation should resolve");
    assert_ne!(
        first.repository_identity,
        clone_location.repository_identity
    );
    assert_ne!(
        first.authority_store_path,
        clone_location.authority_store_path
    );
}

#[test]
fn same_path_repository_reinitialization_cannot_adopt_previous_authority() {
    let fixture_root = std::path::PathBuf::from(temp_workspace("repository-incarnation-reinit"));
    let repo_root = fixture_root.join("repo");
    seed_git_checkout(&repo_root);
    let workspace = repo_root.display().to_string();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let original =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace)
            .expect("original repository incarnation should resolve");

    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    adapter
        .commit_task_authority_snapshot(
            &workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("original task authority should persist");
    SqlitePlanningAuthorityAdapter::stage_repo_scoped_draft_files(
        &workspace,
        "previous-incarnation",
        &[PlanningDraftFileRecord {
            active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            body: "previous incarnation draft".to_string(),
        }],
    )
    .expect("original draft should persist");
    adapter
        .append_app_server_prompt_interaction(
            &workspace,
            prompt_interaction_record(&workspace, "previous-incarnation-prompt"),
        )
        .expect("original prompt should persist");

    std::fs::remove_dir_all(repo_root.join(".git")).expect("original Git metadata should remove");
    seed_git_checkout(&repo_root);
    let replacement =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace)
            .expect("replacement repository incarnation should resolve");
    assert_ne!(
        original.repository_identity,
        replacement.repository_identity
    );
    assert_ne!(
        original.authority_store_path,
        replacement.authority_store_path
    );
    let original_project = std::path::Path::new(&original.runtime_dir)
        .parent()
        .and_then(std::path::Path::file_name)
        .and_then(std::ffi::OsStr::to_str)
        .expect("original project namespace should be UTF-8");
    let replacement_project = std::path::Path::new(&replacement.runtime_dir)
        .parent()
        .and_then(std::path::Path::file_name)
        .and_then(std::ffi::OsStr::to_str)
        .expect("replacement project namespace should be UTF-8");
    assert!(
        replacement_project.starts_with(&format!("{original_project}-")),
        "occupied legacy namespace should remain a readable prefix but be isolated"
    );
    assert!(
        std::path::Path::new(&original.authority_store_path).is_file(),
        "old authority remains quarantined for explicit operator recovery"
    );
    assert!(
        adapter
            .load_task_authority_snapshot(&workspace)
            .expect("replacement task authority should inspect")
            .is_none(),
        "replacement repo must not inherit tasks"
    );
    assert!(
        SqlitePlanningAuthorityAdapter::load_repo_scoped_draft_files(
            &workspace,
            "previous-incarnation",
        )
        .is_err(),
        "replacement repo must not inherit staged drafts"
    );
    assert!(
        adapter
            .load_recent_app_server_prompt_interactions(&workspace, 10)
            .expect("replacement prompt log should inspect")
            .records
            .is_empty(),
        "replacement repo must not inherit prompt logs"
    );
}

#[test]
fn concurrent_repository_incarnation_creation_converges_on_one_identity() {
    let fixture_root = std::path::PathBuf::from(temp_workspace("repository-incarnation-race"));
    let repo_root = fixture_root.join("repo");
    seed_git_checkout(&repo_root);
    let workspace = std::sync::Arc::new(repo_root.display().to_string());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let workspace = std::sync::Arc::clone(&workspace);
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
                    workspace.as_str(),
                )
                .expect("concurrent repository incarnation should resolve")
            })
        })
        .collect::<Vec<_>>();
    let locations = handles
        .into_iter()
        .map(|handle| handle.join().expect("resolver thread should join"))
        .collect::<Vec<_>>();
    for location in &locations[1..] {
        assert_eq!(
            locations[0].repository_identity,
            location.repository_identity
        );
        assert_eq!(
            locations[0].authority_store_path,
            location.authority_store_path
        );
    }
}

#[cfg(unix)]
#[test]
fn repository_incarnation_marker_rejects_links_and_permissive_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    const VALID_MARKER: &str = concat!(
        "akra-repository-incarnation-v1\n",
        "id=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n",
        "project=fixture-0123456789ab\n",
    );

    for fixture in ["symlink", "hardlink", "permissive"] {
        let fixture_root =
            std::path::PathBuf::from(temp_workspace(&format!("repository-marker-{fixture}")));
        let repo_root = fixture_root.join("repo");
        seed_git_checkout(&repo_root);
        let marker = repo_root
            .join(".git")
            .join(super::workspace_paths::REPOSITORY_INCARNATION_MARKER_FILE_NAME);
        let target = fixture_root.join("marker-target");
        std::fs::write(&target, VALID_MARKER).expect("marker target should write");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
            .expect("marker target should be private");
        match fixture {
            "symlink" => symlink(&target, &marker).expect("marker symlink should create"),
            "hardlink" => {
                std::fs::hard_link(&target, &marker).expect("marker hard link should create")
            }
            "permissive" => {
                std::fs::write(&marker, VALID_MARKER).expect("marker should write");
                std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o644))
                    .expect("marker should become permissive");
            }
            _ => unreachable!(),
        }
        let before = std::fs::read(&target).expect("marker target should inspect");
        assert!(
            SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
                repo_root.to_string_lossy().as_ref(),
            )
            .is_err(),
            "{fixture} marker must fail closed"
        );
        assert_eq!(
            std::fs::read(&target).expect("marker target should remain readable"),
            before,
            "marker validation must not mutate a linked target"
        );
    }
}

#[test]
fn task_authority_snapshot_is_committed_to_db_tables() {
    // 빈 workspace로 시작해야 commit path가 schema bootstrap, initial revision 생성, table insert를
    // 모두 지난다. 기존 DB를 재사용하면 load-only 또는 update-only 경로만 검증할 위험이 있다.
    let workspace_dir = temp_workspace("workspace");
    // concrete adapter를 만들지만 아래 호출은 PlanningTaskRepositoryPort 메서드다. application boundary에서
    // 기대하는 포트 계약이 실제 SQLite 구현에서도 유지되는지 확인한다.
    let adapter = SqlitePlanningAuthorityAdapter::new();
    // 최소 task authority 문서로 round-trip을 검증한다. 내용이 비어 있어도 version과 tasks 배열이
    // DB 직렬화/역직렬화 후 같은 domain value로 돌아와야 한다.
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    // queue_projection은 task_authority와 같은 revision으로 저장되어야 하는 실행 관점 투영이다.
    // 빈 projection도 next/active/proposed/skipped 필드가 누락 없이 DB snapshot에 남는지 확인한다.
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    // observed_planning_revision이 None인 첫 commit은 lost-update 검사를 건너뛰고 새 snapshot을 쓴다.
    // 이 호출이 성공하면 adapter는 schema 준비, transaction, JSON 저장, revision 발급까지 완료해야 한다.
    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("task authority should commit");

    // 같은 adapter/workspace에서 다시 읽어야 persistence boundary를 통과한다. 반환값이 None이면 commit이
    // table에 snapshot을 남기지 못한 것이고, Some이어도 아래 equality가 직렬화 손실을 잡는다.
    let snapshot = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("task authority should load")
        .expect("snapshot should exist");

    // task_authority와 queue_projection을 따로 비교해 "문서만 저장됨" 또는 "큐 투영만 저장됨" 같은 반쪽
    // 성공을 막는다. 두 값이 같은 snapshot으로 돌아와야 planning runtime과 repair flow가 같은 authority를 본다.
    assert_eq!(snapshot.task_authority, task_authority);
    assert_eq!(snapshot.queue_projection, queue_projection);
}

#[test]
fn task_mutation_journal_commits_atomically_and_reloads_after_restart() {
    let workspace_dir = temp_workspace("task-mutation-journal-restart");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let empty_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let empty_queue = empty_test_queue_projection();
    let baseline = adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &empty_authority,
                queue_projection: &empty_queue,
            },
        )
        .expect("empty authority should seed");
    let PlanningTaskAuthorityCommitResult::Committed {
        planning_revision: baseline_revision,
        ..
    } = baseline
    else {
        panic!("baseline should commit")
    };
    let created = authority_task("journal-created", "direction-1");
    let created_id = created.id.clone();
    let next_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![created.clone()],
    };
    let provenance = TaskMutationProvenance::new(OriginSessionKind::Planner).with_parent(
        Some("parent-thread".to_string()),
        Some("parent-turn".to_string()),
    );
    let committed = adapter
        .commit_task_authority_mutation_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(baseline_revision),
                task_authority: &next_authority,
                queue_projection: &empty_queue,
            },
            PlanningTaskAuthorityMutationAudit {
                task_ids: std::slice::from_ref(&created_id),
                legacy_source_turn_id: None,
                provenance: &provenance,
            },
        )
        .expect("audited mutation should commit");
    let PlanningTaskAuthorityCommitResult::Committed {
        planning_revision: mutation_revision,
        changed: true,
    } = committed
    else {
        panic!("audited mutation should change authority")
    };
    let restarted = SqlitePlanningAuthorityAdapter::new();
    let records = restarted
        .load_task_authority_mutations(&workspace_dir, baseline_revision, mutation_revision)
        .expect("mutation journal should reload");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].task_id, created.id);
    assert_eq!(
        records[0].mutation_kind,
        PlanningTaskAuthorityMutationKind::Created
    );
    assert_eq!(records[0].provenance, provenance);

    let mut generic_task = created;
    generic_task.title = "Generic external rewrite".to_string();
    let generic_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![generic_task],
    };
    let generic = restarted
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(mutation_revision),
                task_authority: &generic_authority,
                queue_projection: &empty_queue,
            },
        )
        .expect("generic rewrite should commit");
    let PlanningTaskAuthorityCommitResult::Committed {
        planning_revision: generic_revision,
        ..
    } = generic
    else {
        panic!("generic rewrite should commit")
    };
    let generic_records = restarted
        .load_task_authority_mutations(&workspace_dir, mutation_revision, generic_revision)
        .expect("generic revision journal range should load");
    assert_eq!(generic_records.len(), 1);
    assert_eq!(
        generic_records[0].mutation_kind,
        PlanningTaskAuthorityMutationKind::Updated
    );
    assert_eq!(
        generic_records[0].provenance.origin_session_kind,
        Some(OriginSessionKind::System)
    );
    assert_eq!(
        generic_records[0]
            .after_task
            .as_ref()
            .map(|task| task.title.as_str()),
        Some("Generic external rewrite")
    );
}

#[test]
fn queue_cancellation_persists_cancelled_state_across_adapter_restart() {
    let workspace_dir = temp_workspace("queue-cancellation-restart");
    let adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let directions = DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig::default(),
        directions: vec![DirectionDefinition {
            id: "direction-1".to_string(),
            title: "Direction 1".to_string(),
            summary: "Queue cancellation persistence".to_string(),
            success_criteria: vec!["done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    };
    let ready_tasks = (0..17)
        .map(|index| authority_task(&format!("task-cancel-{index}"), "direction-1"))
        .collect::<Vec<_>>();
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: ready_tasks.clone(),
    };
    let queue_projection = PriorityQueueService::new()
        .build_projection(&directions, &task_authority)
        .expect("queue projection should build");
    adapter
        .commit_direction_authority_snapshot(
            &workspace_dir,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions,
                authority_mutation_owner_token: None,
            },
        )
        .expect("direction authority should seed");
    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("task authority should seed");
    let queue = PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        adapter.clone(),
        adapter.clone(),
        Arc::new(NoopPlanningWorkerPort),
    )
    .queue;
    let before = queue
        .load_authority_snapshot(&workspace_dir)
        .expect("queue authority should load");
    let cancellation = PlanningQueueCancellationRequest {
        workspace_directory: workspace_dir.clone(),
        expected_planning_revision: before.planning_revision,
        targets: ready_tasks
            .into_iter()
            .map(|task| PlanningQueueCancellationTarget {
                task_id: task.id,
                expected_status: task.status,
                expected_updated_at: task.updated_at,
            })
            .collect(),
    };

    adapter
        .upsert_runtime_slot_lease(
            &workspace_dir,
            &slot_lease_for_task(
                "slot-queue-cancel",
                "task-cancel-0",
                ParallelModeSlotLeaseState::Running,
            ),
        )
        .expect("running task lease should persist");
    let blocked = queue
        .cancel_tasks(cancellation.clone())
        .expect_err("running task must reject queue cancellation");
    assert!(blocked.to_string().contains("cannot be edited while slot"));
    let unchanged = queue
        .load_authority_snapshot(&workspace_dir)
        .expect("blocked cancellation should preserve authority");
    assert_eq!(unchanged.planning_revision, before.planning_revision);
    assert!(
        unchanged
            .tasks
            .iter()
            .all(|task| task.status == TaskStatus::Ready)
    );
    adapter
        .remove_runtime_slot_lease(&workspace_dir, "slot-queue-cancel")
        .expect("test lease should release");

    let cancelled = queue
        .cancel_tasks(cancellation)
        .expect("ready task should be cancelled");
    assert_eq!(cancelled.applied_command_count, 17);
    assert_eq!(
        cancelled.committed_planning_revision,
        before.planning_revision + 1
    );
    drop(queue);
    drop(adapter);

    let restarted = SqlitePlanningAuthorityAdapter::new();
    let snapshot = restarted
        .load_task_authority_snapshot(&workspace_dir)
        .expect("restarted adapter should load")
        .expect("task authority should persist");
    assert!(
        snapshot
            .task_authority
            .tasks
            .iter()
            .all(|task| task.status == TaskStatus::Cancelled)
    );
    assert!(snapshot.queue_projection.next_task.is_none());
    assert!(snapshot.queue_projection.active_tasks.is_empty());
}

#[test]
fn authority_document_commit_rolls_back_when_active_document_write_fails() {
    let workspace_dir = temp_workspace("authority-document-rollback");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let baseline_directions = DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig {
            policy: QueueIdlePolicy::Stop,
            prompt_path: String::new(),
        },
        directions: vec![DirectionDefinition {
            id: "direction-1".to_string(),
            title: "Direction 1".to_string(),
            summary: "Baseline direction".to_string(),
            success_criteria: vec!["done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    };
    let baseline_task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![TaskDefinition {
            id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_relation_note: "baseline relation".to_string(),
            title: "Baseline task".to_string(),
            description: "Baseline description".to_string(),
            status: TaskStatus::Ready,
            base_priority: 50,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: TaskMutationProvenance::new(OriginSessionKind::System),
            updated_at: "2026-05-07T09:00:00Z".to_string(),
        }],
    };
    let baseline_queue_projection = PriorityQueueProjection {
        next_task: Some(PriorityQueueTask {
            rank: 1,
            task_id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction 1".to_string(),
            task_title: "Baseline task".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 50,
            updated_at: "2026-05-07T09:00:00Z".to_string(),
            rank_reasons: vec!["baseline".to_string()],
        }),
        active_tasks: vec![PriorityQueueTask {
            rank: 1,
            task_id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction 1".to_string(),
            task_title: "Baseline task".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 50,
            updated_at: "2026-05-07T09:00:00Z".to_string(),
            rank_reasons: vec!["baseline".to_string()],
        }],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    let baseline_result = adapter
        .commit_planning_authority_documents(
            &workspace_dir,
            PlanningAuthorityDocumentCommit {
                observed_planning_revision: None,
                directions: &baseline_directions,
                task_authority: &baseline_task_authority,
                queue_projection: &baseline_queue_projection,
                result_output_markdown: Some("# Result Output\n\nBaseline"),
                active_document_mutations: &[],
                retired_task_ids: &[],
                authority_mutation_owner_token: None,
            },
        )
        .expect("baseline authority documents should commit");
    let PlanningTaskAuthorityCommitResult::Committed {
        planning_revision, ..
    } = baseline_result
    else {
        panic!("baseline authority documents should commit");
    };

    install_failing_insert_trigger(
        &workspace_dir,
        "active_documents",
        "fail_active_documents_insert",
    );
    let changed_directions = DirectionCatalogDocument {
        directions: vec![DirectionDefinition {
            title: "Changed direction".to_string(),
            ..baseline_directions.directions[0].clone()
        }],
        ..baseline_directions.clone()
    };
    let changed_task_authority = TaskAuthorityDocument {
        tasks: vec![TaskDefinition {
            title: "Changed task".to_string(),
            ..baseline_task_authority.tasks[0].clone()
        }],
        ..baseline_task_authority.clone()
    };
    let changed_queue_projection = PriorityQueueProjection {
        next_task: Some(PriorityQueueTask {
            task_title: "Changed task".to_string(),
            ..baseline_queue_projection
                .next_task
                .clone()
                .expect("baseline next task")
        }),
        active_tasks: vec![PriorityQueueTask {
            task_title: "Changed task".to_string(),
            ..baseline_queue_projection.active_tasks[0].clone()
        }],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    assert_error_contains(
        adapter.commit_planning_authority_documents(
            &workspace_dir,
            PlanningAuthorityDocumentCommit {
                observed_planning_revision: Some(planning_revision),
                directions: &changed_directions,
                task_authority: &changed_task_authority,
                queue_projection: &changed_queue_projection,
                result_output_markdown: Some("# Result Output\n\nChanged"),
                active_document_mutations: &[],
                retired_task_ids: &[],
                authority_mutation_owner_token: None,
            },
        ),
        "failed to store active document `.codex-exec-loop/planning/result-output.md`",
    );

    let reloaded_direction = adapter
        .load_direction_authority_snapshot(&workspace_dir)
        .expect("direction authority should load")
        .expect("direction authority should exist");
    let reloaded_task = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("task authority should load")
        .expect("task authority should exist");
    let reloaded_workspace = adapter
        .load_active_workspace_files(&workspace_dir)
        .expect("active workspace should load");
    assert_eq!(reloaded_direction.directions, baseline_directions);
    assert_eq!(reloaded_task.task_authority, baseline_task_authority);
    assert_eq!(reloaded_task.queue_projection, baseline_queue_projection);
    assert_eq!(
        reloaded_workspace.result_output_markdown.as_deref(),
        Some("# Result Output\n\nBaseline")
    );
}

#[test]
fn authority_document_commit_can_preserve_hidden_result_output() {
    let workspace_dir = temp_workspace("authority-document-preserve-result-output");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let baseline_directions = test_direction_catalog(&["direction-a"]);
    let empty_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let empty_queue = empty_test_queue_projection();
    let baseline = adapter
        .commit_planning_authority_documents(
            &workspace_dir,
            PlanningAuthorityDocumentCommit {
                observed_planning_revision: None,
                directions: &baseline_directions,
                task_authority: &empty_authority,
                queue_projection: &empty_queue,
                result_output_markdown: Some("# Result Output\n\nKeep this body.\n"),
                active_document_mutations: &[],
                retired_task_ids: &[],
                authority_mutation_owner_token: None,
            },
        )
        .expect("baseline authority documents should commit");
    let PlanningTaskAuthorityCommitResult::Committed {
        planning_revision, ..
    } = baseline
    else {
        panic!("baseline authority documents should commit");
    };
    let mut changed_directions = baseline_directions.clone();
    changed_directions.directions[0].title = "Changed direction".to_string();

    adapter
        .commit_planning_authority_documents(
            &workspace_dir,
            PlanningAuthorityDocumentCommit {
                observed_planning_revision: Some(planning_revision),
                directions: &changed_directions,
                task_authority: &empty_authority,
                queue_projection: &empty_queue,
                result_output_markdown: None,
                active_document_mutations: &[],
                retired_task_ids: &[],
                authority_mutation_owner_token: None,
            },
        )
        .expect("authority rewrite should preserve hidden result output");

    let reloaded = adapter
        .load_planning_authority_documents(&workspace_dir)
        .expect("authority documents should reload")
        .expect("authority documents should remain present");
    assert_eq!(reloaded.directions, changed_directions);
    assert_eq!(
        reloaded.result_output_markdown,
        "# Result Output\n\nKeep this body.\n"
    );
}

#[test]
fn authority_document_rewrite_atomically_mutates_support_files_and_retires_tasks() {
    let workspace_dir = temp_workspace("authority-document-support-retirement");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let directions = test_direction_catalog(&["direction-a"]);
    let baseline_tasks = task_authority_for_direction("task-retired", "direction-a");
    let queue = empty_test_queue_projection();
    let baseline_support = [PlanningAuthorityActiveDocumentMutation::Replace {
        relative_path: ".codex-exec-loop/planning/prompts/old.md",
        body: "old prompt",
    }];
    let baseline = adapter
        .commit_planning_authority_documents(
            &workspace_dir,
            PlanningAuthorityDocumentCommit {
                observed_planning_revision: None,
                directions: &directions,
                task_authority: &baseline_tasks,
                queue_projection: &queue,
                result_output_markdown: Some("# Result Output\n\nOld\n"),
                active_document_mutations: &baseline_support,
                retired_task_ids: &[],
                authority_mutation_owner_token: None,
            },
        )
        .expect("baseline documents should commit");
    let PlanningTaskAuthorityCommitResult::Committed {
        planning_revision: baseline_revision,
        ..
    } = baseline
    else {
        panic!("baseline documents should commit");
    };
    let dispatch_block = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-retired",
        "2026-07-10T00:00:00Z",
        "2026-07-10T00:01:00Z",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );
    adapter
        .upsert_runtime_task_dispatch_block(&workspace_dir, &dispatch_block)
        .expect("terminal runtime residue should persist before retirement");

    let empty_tasks = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let support_rewrite = [
        PlanningAuthorityActiveDocumentMutation::RemoveEntry {
            relative_path: ".codex-exec-loop/planning/prompts",
        },
        PlanningAuthorityActiveDocumentMutation::Replace {
            relative_path: ".codex-exec-loop/planning/prompts/queue-idle-review.md",
            body: "new prompt",
        },
    ];
    let retired_task_ids = vec!["task-retired".to_string()];
    install_failing_delete_trigger(
        &workspace_dir,
        "active_documents",
        "fail_support_document_delete",
    );
    let failed = adapter.commit_planning_authority_documents(
        &workspace_dir,
        PlanningAuthorityDocumentCommit {
            observed_planning_revision: Some(baseline_revision),
            directions: &directions,
            task_authority: &empty_tasks,
            queue_projection: &queue,
            result_output_markdown: Some("# Result Output\n\nNew\n"),
            active_document_mutations: &support_rewrite,
            retired_task_ids: &retired_task_ids,
            authority_mutation_owner_token: None,
        },
    );
    assert_error_contains(failed, "forced delete failure");
    let after_failure = adapter
        .load_planning_authority_documents(&workspace_dir)
        .expect("authority should reload after rollback")
        .expect("authority should remain present");
    assert_eq!(after_failure.planning_revision, baseline_revision);
    assert_eq!(after_failure.task_authority, baseline_tasks);
    assert_eq!(
        after_failure.result_output_markdown,
        "# Result Output\n\nOld\n"
    );
    assert_eq!(
        SqlitePlanningAuthorityAdapter::load_active_planning_file(
            &workspace_dir,
            ".codex-exec-loop/planning/prompts/old.md",
        )
        .expect("old support file should reload")
        .as_deref(),
        Some("old prompt")
    );
    let retired_after_failure: i64 = authority_connection(&workspace_dir)
        .query_row(
            "SELECT COUNT(*) FROM retired_planning_tasks WHERE task_id = 'task-retired'",
            [],
            |row| row.get(0),
        )
        .expect("retirement rollback should inspect");
    assert_eq!(retired_after_failure, 0);
    assert_eq!(
        adapter
            .load_runtime_projections(&workspace_dir)
            .expect("runtime residue should reload")
            .task_dispatch_blocks,
        vec![dispatch_block]
    );

    authority_connection(&workspace_dir)
        .execute_batch("DROP TRIGGER fail_support_document_delete")
        .expect("failure trigger should drop");
    let committed = adapter
        .commit_planning_authority_documents(
            &workspace_dir,
            PlanningAuthorityDocumentCommit {
                observed_planning_revision: Some(baseline_revision),
                directions: &directions,
                task_authority: &empty_tasks,
                queue_projection: &queue,
                result_output_markdown: Some("# Result Output\n\nNew\n"),
                active_document_mutations: &support_rewrite,
                retired_task_ids: &retired_task_ids,
                authority_mutation_owner_token: None,
            },
        )
        .expect("support and retirement rewrite should commit");
    assert!(matches!(
        committed,
        PlanningTaskAuthorityCommitResult::Committed { changed: true, .. }
    ));
    assert!(
        SqlitePlanningAuthorityAdapter::load_active_planning_file(
            &workspace_dir,
            ".codex-exec-loop/planning/prompts/old.md",
        )
        .expect("removed support file should inspect")
        .is_none()
    );
    assert_eq!(
        SqlitePlanningAuthorityAdapter::load_active_planning_file(
            &workspace_dir,
            ".codex-exec-loop/planning/prompts/queue-idle-review.md",
        )
        .expect("new support file should reload")
        .as_deref(),
        Some("new prompt")
    );
    assert!(
        adapter
            .load_runtime_projections(&workspace_dir)
            .expect("retired runtime residue should reload")
            .task_dispatch_blocks
            .is_empty()
    );
    let retired_after_success: i64 = authority_connection(&workspace_dir)
        .query_row(
            "SELECT COUNT(*) FROM retired_planning_tasks WHERE task_id = 'task-retired'",
            [],
            |row| row.get(0),
        )
        .expect("retirement should inspect");
    assert_eq!(retired_after_success, 1);
    let late_lease = adapter
        .upsert_runtime_slot_lease(
            &workspace_dir,
            &slot_lease_for_task(
                "slot-retired",
                "task-retired",
                ParallelModeSlotLeaseState::Running,
            ),
        )
        .expect_err("retired task must reject late runtime resurrection");
    assert!(late_lease.to_string().contains("retired planning task"));
}

#[test]
fn app_server_prompt_log_round_trips_recent_records() {
    let workspace_dir = temp_workspace("prompt-log");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .append_app_server_prompt_interaction(
            &workspace_dir,
            AppServerPromptInteractionRecord {
                sequence: 0,
                interaction_id: "interaction-1".to_string(),
                session_kind: "parallel-worker".to_string(),
                operation: "isolated_parallel_thread".to_string(),
                status: "completed".to_string(),
                workspace_dir: workspace_dir.clone(),
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                service_name: Some("akra-parallel-worker".to_string()),
                model: None,
                reasoning_effort: None,
                developer_instructions: Some("developer contract".to_string()),
                input_items: vec![AppServerPromptInputRecord::new(
                    "text",
                    "turn input",
                    "implement task",
                )],
                output_items: vec![AppServerPromptOutputRecord::new(
                    "agent-1",
                    Some("final".to_string()),
                    "done",
                )],
                error_message: None,
                started_at: Utc::now().to_rfc3339(),
                completed_at: Utc::now().to_rfc3339(),
            },
        )
        .expect("prompt log should append");

    let snapshot = adapter
        .load_recent_app_server_prompt_interactions(&workspace_dir, 10)
        .expect("prompt log should load");

    assert_eq!(snapshot.records.len(), 1);
    let record = &snapshot.records[0];
    assert!(record.sequence > 0);
    assert_eq!(record.session_kind, "parallel-worker");
    assert_eq!(record.service_name.as_deref(), Some("akra-parallel-worker"));
    assert_eq!(record.input_items[0].content, "implement task");
    assert_eq!(record.output_items[0].text, "done");
    assert_eq!(record.input_chars(), "implement task".chars().count());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
            &workspace_dir,
        )
        .expect("authority location should resolve");
        let runtime_mode = std::fs::metadata(&location.runtime_dir)
            .expect("authority runtime metadata should load")
            .permissions()
            .mode()
            & 0o777;
        let store_mode = std::fs::metadata(&location.authority_store_path)
            .expect("authority store metadata should load")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(runtime_mode, 0o700);
        assert_eq!(store_mode, 0o600);
        let connection = open_authority_connection(&location)
            .expect("authority store should reopen for pragma inspection");
        let secure_delete: i64 = connection
            .query_row("PRAGMA secure_delete", [], |row| row.get(0))
            .expect("secure_delete pragma should be readable");
        assert_eq!(secure_delete, 1);
    }
}

#[cfg(unix)]
#[test]
fn authority_store_rejects_parent_symlinks_without_touching_the_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-parent-symlink"));
    let akra_home = fixture_root.join("home");
    let projects = akra_home.join("projects");
    let victim = fixture_root.join("victim");
    std::fs::create_dir_all(&projects).expect("managed parent should create");
    std::fs::create_dir_all(&victim).expect("victim directory should create");
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o750))
        .expect("victim permissions should set");
    std::fs::write(victim.join("sentinel"), b"unchanged").expect("victim sentinel should write");
    symlink(&victim, projects.join("repo-deadbeef"))
        .expect("malicious project symlink should create");

    let store = projects
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");
    let error = open_authority_connection(&authority_location_for_store(&store))
        .expect_err("parent symlink must be rejected");

    assert!(error.to_string().contains("real directory"));
    assert_eq!(
        std::fs::read(victim.join("sentinel")).expect("victim sentinel should remain readable"),
        b"unchanged"
    );
    assert!(
        !victim.join("runtime").exists(),
        "validation must fail before creating content through the symlink"
    );
    assert_eq!(
        std::fs::metadata(&victim)
            .expect("victim metadata should load")
            .permissions()
            .mode()
            & 0o777,
        0o750,
        "validation must not chmod the symlink target"
    );
}

#[cfg(unix)]
#[test]
fn authority_store_does_not_change_akra_home_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-home-permissions"));
    let akra_home = fixture_root.join("shared-home");
    std::fs::create_dir(&akra_home).expect("AKRA_HOME fixture should create");
    std::fs::set_permissions(&akra_home, std::fs::Permissions::from_mode(0o750))
        .expect("AKRA_HOME fixture permissions should set");
    let store = akra_home
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");

    open_authority_connection(&authority_location_for_store(&store))
        .expect("authority store below shared AKRA_HOME should open");

    assert_eq!(
        std::fs::metadata(&akra_home)
            .expect("AKRA_HOME metadata should load")
            .permissions()
            .mode()
            & 0o777,
        0o750,
        "authority setup must not chmod the operator-selected AKRA_HOME"
    );
    for private_directory in [
        akra_home.join("projects"),
        akra_home.join("projects").join("repo-deadbeef"),
        akra_home
            .join("projects")
            .join("repo-deadbeef")
            .join("runtime"),
    ] {
        assert_eq!(
            std::fs::metadata(&private_directory)
                .expect("managed directory metadata should load")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "managed authority directory must remain private"
        );
    }
}

#[cfg(unix)]
#[test]
fn authority_store_rejects_replaceable_nonsticky_akra_home_without_creating_data() {
    use std::os::unix::fs::PermissionsExt;

    let fixture_root =
        std::path::PathBuf::from(temp_workspace("authority-replaceable-home-permissions"));
    let replaceable_parent = fixture_root.join("replaceable-parent");
    let akra_home = replaceable_parent.join("akra-home");
    std::fs::create_dir(&replaceable_parent).expect("replaceable parent fixture should create");
    std::fs::set_permissions(&replaceable_parent, std::fs::Permissions::from_mode(0o777))
        .expect("replaceable parent permissions should set");
    std::fs::create_dir(&akra_home).expect("private AKRA_HOME fixture should create");
    std::fs::set_permissions(&akra_home, std::fs::Permissions::from_mode(0o700))
        .expect("private AKRA_HOME permissions should set");
    let store = akra_home
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");

    let error = open_authority_connection(&authority_location_for_store(&store))
        .expect_err("a storage root replaceable by another user must fail closed");
    assert!(error.to_string().contains("writable"));
    assert!(
        !akra_home.join("projects").exists(),
        "rejection must occur before creating private data below an unsafe root"
    );
    assert_eq!(
        std::fs::metadata(&replaceable_parent)
            .expect("unsafe parent metadata should remain readable")
            .permissions()
            .mode()
            & 0o777,
        0o777,
        "Akra must not chmod an operator-selected unsafe ancestor while rejecting it"
    );
    assert_eq!(
        std::fs::metadata(&akra_home)
            .expect("private root metadata should remain readable")
            .permissions()
            .mode()
            & 0o777,
        0o700,
        "Akra must leave the selected storage root unchanged while rejecting its ancestor"
    );
}

#[cfg(unix)]
#[test]
fn authority_store_rejects_symlinked_akra_home_without_creating_projects() {
    use std::os::unix::fs::symlink;

    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-home-symlink"));
    let victim = fixture_root.join("victim");
    std::fs::create_dir(&victim).expect("victim directory should create");
    std::fs::write(victim.join("sentinel"), b"unchanged").expect("victim sentinel should write");
    let akra_home = fixture_root.join("home-link");
    symlink(&victim, &akra_home).expect("malicious AKRA_HOME symlink should create");
    let store = akra_home
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");

    open_authority_connection(&authority_location_for_store(&store))
        .expect_err("symlinked AKRA_HOME must be rejected");

    assert_eq!(
        std::fs::read(victim.join("sentinel")).expect("victim sentinel should remain readable"),
        b"unchanged"
    );
    assert!(
        !victim.join("projects").exists(),
        "validation must reject AKRA_HOME before creating the managed root"
    );
}

#[cfg(unix)]
#[test]
fn authority_store_rejects_final_symlinks_without_touching_the_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-file-symlink"));
    let runtime = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime");
    std::fs::create_dir_all(&runtime).expect("runtime directory should create");
    let victim = fixture_root.join("victim.db");
    std::fs::write(&victim, b"victim-content").expect("victim should write");
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o640))
        .expect("victim permissions should set");
    let store = runtime.join("planning-authority.db");
    symlink(&victim, &store).expect("malicious store symlink should create");

    open_authority_connection(&authority_location_for_store(&store))
        .expect_err("final symlink must be rejected");

    assert_eq!(
        std::fs::read(&victim).expect("victim should remain readable"),
        b"victim-content"
    );
    assert_eq!(
        std::fs::metadata(&victim)
            .expect("victim metadata should load")
            .permissions()
            .mode()
            & 0o777,
        0o640,
        "validation must not chmod the symlink target"
    );
}

#[cfg(unix)]
#[test]
fn authority_store_rejects_hardlinks_without_touching_the_target() {
    use std::os::unix::fs::PermissionsExt;

    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-file-hardlink"));
    let runtime = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime");
    std::fs::create_dir_all(&runtime).expect("runtime directory should create");
    let victim = fixture_root.join("victim.db");
    std::fs::write(&victim, b"victim-content").expect("victim should write");
    std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o640))
        .expect("victim permissions should set");
    let store = runtime.join("planning-authority.db");
    std::fs::hard_link(&victim, &store).expect("malicious store hardlink should create");

    let error = open_authority_connection(&authority_location_for_store(&store))
        .expect_err("hardlinked store must be rejected");

    assert!(error.to_string().contains("one link"));
    assert_eq!(
        std::fs::read(&victim).expect("victim should remain readable"),
        b"victim-content"
    );
    assert_eq!(
        std::fs::metadata(&victim)
            .expect("victim metadata should load")
            .permissions()
            .mode()
            & 0o777,
        0o640,
        "validation must reject the hardlink before chmod"
    );
}

#[cfg(unix)]
#[test]
fn authority_store_rejects_preexisting_hardlinked_sqlite_sidecars_without_touching_the_victim() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    for suffix in ["-journal", "-wal", "-shm"] {
        let fixture_root = std::path::PathBuf::from(temp_workspace(&format!(
            "authority-hardlinked-sidecar-{}",
            suffix.trim_start_matches('-')
        )));
        let store = fixture_root
            .join("home")
            .join("projects")
            .join("repo-deadbeef")
            .join("runtime")
            .join("planning-authority.db");
        let location = authority_location_for_store(&store);
        drop(open_authority_connection(&location).expect("authority store should initialize"));

        let sidecar = authority_store_sidecar_path(&store, suffix);
        if sidecar.exists() {
            std::fs::remove_file(&sidecar).expect("clean test sidecar should be removable");
        }
        let victim = fixture_root.join(format!("victim{suffix}"));
        std::fs::write(&victim, b"victim-content").expect("sidecar victim should write");
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600))
            .expect("sidecar victim should become private");
        std::fs::hard_link(&victim, &sidecar).expect("hardlinked sidecar fixture should create");

        let error = open_authority_connection(&location)
            .expect_err("hardlinked SQLite sidecar must fail before SQLite can write it");
        assert!(
            error.to_string().contains("one link"),
            "unexpected {suffix} rejection: {error:#}"
        );
        assert_eq!(
            std::fs::read(&victim).expect("sidecar victim should remain readable"),
            b"victim-content"
        );
        assert_eq!(
            std::fs::metadata(&victim)
                .expect("sidecar victim metadata should load")
                .nlink(),
            2,
            "rejection must not unlink the operator's victim fixture"
        );
        assert_eq!(
            std::fs::metadata(&victim)
                .expect("sidecar victim metadata should remain readable")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "rejection must happen before chmod or SQLite opens the hardlink"
        );
    }
}

#[cfg(unix)]
#[test]
fn authority_store_sidecar_retry_anchors_a_replacement_but_rejects_unsafe_replacements() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-sidecar-replacement"));
    let store = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");
    let location = authority_location_for_store(&store);
    drop(open_authority_connection(&location).expect("authority store should initialize"));

    let journal = authority_store_sidecar_path(&store, "-journal");
    std::fs::write(&journal, b"old-journal").expect("old journal should write");
    std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o600))
        .expect("old journal should become private");
    let old_journal = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&journal)
        .expect("old journal should open");
    std::fs::remove_file(&journal).expect("old journal should unlink");
    std::fs::write(&journal, b"new-journal").expect("replacement journal should write");
    std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o640))
        .expect("replacement journal fixture should be permissive");

    assert!(
        secure_opened_private_authority_sidecar_file(&journal, old_journal)
            .expect("an unlinked descriptor should be a retryable replacement")
            .is_none()
    );
    let anchors = prepare_private_authority_sidecar_files(&store)
        .expect("the current legitimate replacement should anchor");
    assert_eq!(anchors.len(), 1);
    let anchored = anchors[0]
        .metadata()
        .expect("replacement anchor metadata should load");
    let current = std::fs::metadata(&journal).expect("replacement path metadata should load");
    assert_eq!(
        (anchored.dev(), anchored.ino()),
        (current.dev(), current.ino())
    );
    assert_eq!(current.permissions().mode() & 0o777, 0o600);
    drop(anchors);
    std::fs::remove_file(&journal).expect("legitimate replacement should remove");

    for replacement_kind in ["symlink", "hardlink"] {
        std::fs::write(&journal, b"old-journal").expect("old journal should rewrite");
        std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o600))
            .expect("old journal should become private");
        let old_journal = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&journal)
            .expect("old journal should reopen");
        std::fs::remove_file(&journal).expect("old journal should unlink again");
        let victim = fixture_root.join(format!("{replacement_kind}-victim"));
        std::fs::write(&victim, b"victim-content").expect("replacement victim should write");
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o600))
            .expect("replacement victim should become private");
        if replacement_kind == "symlink" {
            symlink(&victim, &journal).expect("malicious sidecar symlink should create");
        } else {
            std::fs::hard_link(&victim, &journal)
                .expect("malicious sidecar hardlink should create");
        }

        assert!(
            secure_opened_private_authority_sidecar_file(&journal, old_journal)
                .expect("the unlinked old descriptor should remain retryable")
                .is_none()
        );
        prepare_private_authority_sidecar_files(&store)
            .expect_err("the current unsafe replacement must fail closed");
        assert_eq!(
            std::fs::read(&victim).expect("replacement victim should remain readable"),
            b"victim-content"
        );
        assert_eq!(
            std::fs::metadata(&victim)
                .expect("replacement victim metadata should load")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        std::fs::remove_file(&journal).expect("unsafe replacement should remove");
    }
}

#[cfg(unix)]
#[test]
fn authority_store_open_survives_concurrent_delete_journal_churn() {
    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-sidecar-churn"));
    let store = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");
    let location = authority_location_for_store(&store);
    let initialized =
        open_authority_connection(&location).expect("authority store should initialize");
    initialized
        .execute(
            "INSERT OR REPLACE INTO authority_metadata (key, value) VALUES ('churn-sentinel', 'kept')",
            [],
        )
        .expect("churn sentinel should initialize");
    drop(initialized);

    let barrier = Arc::new(Barrier::new(2));
    let writer_barrier = Arc::clone(&barrier);
    let writer_store = store.clone();
    let writer = std::thread::spawn(move || {
        let connection =
            rusqlite::Connection::open(writer_store).expect("journal churn connection should open");
        connection
            .busy_timeout(super::AUTHORITY_STORE_BUSY_TIMEOUT)
            .expect("journal churn busy timeout should configure");
        writer_barrier.wait();
        for _ in 0..128 {
            connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     UPDATE authority_metadata SET value = 'transient' WHERE key = 'churn-sentinel';
                     ROLLBACK;",
                )
                .expect("journal churn transaction should roll back");
        }
    });

    barrier.wait();
    for _ in 0..128 {
        let connection = open_authority_connection(&location)
            .expect("secure authority open should tolerate legitimate journal replacement");
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM authority_metadata WHERE key = 'churn-sentinel'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("churn sentinel should remain readable"),
            "kept"
        );
    }
    writer.join().expect("journal churn writer should finish");
}

#[cfg(windows)]
#[test]
fn authority_store_sidecar_retry_handles_delete_pending_and_rejects_hardlink_replacement() {
    use std::os::windows::fs::OpenOptionsExt;

    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-sidecar-replacement"));
    let store = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");
    let location = authority_location_for_store(&store);
    drop(open_authority_connection(&location).expect("authority store should initialize"));

    let journal = authority_store_sidecar_path(&store, "-journal");
    std::fs::write(&journal, b"old-journal").expect("old journal should write");
    let old_journal = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(
            WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC,
        )
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&journal)
        .expect("old journal should open with delete sharing");
    std::fs::remove_file(&journal).expect("old journal should become delete-pending");
    assert!(
        secure_opened_private_authority_sidecar_file(&journal, old_journal)
            .expect("a delete-pending descriptor should be retryable")
            .is_none()
    );

    std::fs::write(&journal, b"new-journal").expect("replacement journal should write");
    let anchors = prepare_private_authority_sidecar_files(&store)
        .expect("the legitimate replacement should anchor");
    assert_eq!(anchors.len(), 1);
    drop(anchors);
    std::fs::remove_file(&journal).expect("legitimate replacement should remove");

    let victim = fixture_root.join("hardlink-victim");
    std::fs::write(&victim, b"victim-content").expect("hardlink victim should write");
    std::fs::hard_link(&victim, &journal).expect("malicious sidecar hardlink should create");
    prepare_private_authority_sidecar_files(&store)
        .expect_err("the current hardlinked replacement must fail closed");
    assert_eq!(
        std::fs::read(&victim).expect("hardlink victim should remain readable"),
        b"victim-content"
    );
}

#[cfg(unix)]
#[test]
fn authority_store_migrates_wal_to_private_delete_journaling_without_data_loss() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-wal-migration"));
    let store = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");
    let location = authority_location_for_store(&store);
    let connection =
        open_authority_connection(&location).expect("authority store should initialize");
    let wal_mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .expect("test store should enter WAL mode");
    assert_eq!(wal_mode.to_ascii_lowercase(), "wal");
    connection
        .execute(
            "INSERT OR REPLACE INTO authority_metadata (key, value) VALUES ('wal-sentinel', 'kept')",
            [],
        )
        .expect("WAL sentinel should commit");
    drop(connection);

    let reopened =
        open_authority_connection(&location).expect("private open should checkpoint WAL safely");
    let journal_mode: String = reopened
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal mode should load");
    assert_eq!(journal_mode.to_ascii_lowercase(), "delete");
    assert_eq!(
        reopened
            .query_row(
                "SELECT value FROM authority_metadata WHERE key = 'wal-sentinel'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("checkpointed WAL sentinel should survive"),
        "kept"
    );

    reopened
        .execute_batch(
            "BEGIN IMMEDIATE;
             UPDATE authority_metadata SET value = 'kept-again' WHERE key = 'wal-sentinel';",
        )
        .expect("rollback-journal transaction should start");
    let journal = authority_store_sidecar_path(&store, "-journal");
    let journal_metadata =
        std::fs::metadata(&journal).expect("DELETE-mode write should create a rollback journal");
    assert!(journal_metadata.is_file());
    assert_eq!(journal_metadata.nlink(), 1);
    assert_eq!(journal_metadata.permissions().mode() & 0o777, 0o600);
    reopened
        .execute_batch("ROLLBACK")
        .expect("rollback-journal transaction should roll back");
}

#[cfg(windows)]
#[test]
fn authority_store_rejects_windows_junctions_without_touching_the_target() {
    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-parent-junction"));
    let projects = fixture_root.join("home").join("projects");
    let victim = fixture_root.join("victim");
    std::fs::create_dir_all(&projects).expect("managed parent should create");
    std::fs::create_dir_all(&victim).expect("victim directory should create");
    std::fs::write(victim.join("sentinel"), b"unchanged").expect("victim sentinel should write");
    let junction = projects.join("repo-deadbeef");
    let output = std::process::Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            junction.to_string_lossy().as_ref(),
            victim.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("junction command should run");
    assert!(
        output.status.success(),
        "junction should create: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let store = junction.join("runtime").join("planning-authority.db");
    let error = open_authority_connection(&authority_location_for_store(&store))
        .expect_err("parent junction must be rejected");

    assert!(error.to_string().contains("reparse point"));
    assert_eq!(
        std::fs::read(victim.join("sentinel")).expect("victim sentinel should remain readable"),
        b"unchanged"
    );
    assert!(
        !victim.join("runtime").exists(),
        "validation must fail before creating content through the junction"
    );
}

#[cfg(windows)]
#[test]
fn authority_store_rejects_windows_hardlinks_without_touching_the_target() {
    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-file-hardlink-windows"));
    let runtime = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime");
    std::fs::create_dir_all(&runtime).expect("runtime directory should create");
    let victim = fixture_root.join("victim.db");
    std::fs::write(&victim, b"victim-content").expect("victim should write");
    let store = runtime.join("planning-authority.db");
    std::fs::hard_link(&victim, &store).expect("malicious store hardlink should create");

    open_authority_connection(&authority_location_for_store(&store))
        .expect_err("hardlinked store must be rejected");

    assert_eq!(
        std::fs::read(&victim).expect("victim should remain readable"),
        b"victim-content"
    );
}

#[cfg(windows)]
#[test]
fn authority_store_windows_private_acl_allows_valid_reopen() {
    let fixture_root = std::path::PathBuf::from(temp_workspace("authority-private-acl-windows"));
    let store = fixture_root
        .join("home")
        .join("projects")
        .join("repo-deadbeef")
        .join("runtime")
        .join("planning-authority.db");
    let location = authority_location_for_store(&store);

    let connection = open_authority_connection(&location)
        .expect("new Windows authority store should receive a private ACL");
    drop(connection);
    open_authority_connection(&location)
        .expect("owner-only protected ACL should permit the same user to reopen the store");
}

#[test]
fn app_server_prompt_log_expires_old_records_and_bounds_large_payloads() {
    let workspace_dir = temp_workspace("prompt-log-retention");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let make_record = |interaction_id: &str, completed_at: String, content: String| {
        AppServerPromptInteractionRecord {
            sequence: 0,
            interaction_id: interaction_id.to_string(),
            session_kind: "main".to_string(),
            operation: "turn".to_string(),
            status: "completed".to_string(),
            workspace_dir: workspace_dir.clone(),
            thread_id: Some("thread".to_string()),
            turn_id: Some("turn".to_string()),
            service_name: None,
            model: None,
            reasoning_effort: None,
            developer_instructions: Some(content.clone()),
            input_items: (0..20)
                .map(|index| {
                    AppServerPromptInputRecord::new("text", format!("input-{index}"), &content)
                })
                .collect(),
            output_items: (0..20)
                .map(|index| {
                    AppServerPromptOutputRecord::new(
                        format!("output-{index}"),
                        Some("final".to_string()),
                        &content,
                    )
                })
                .collect(),
            error_message: None,
            started_at: completed_at.clone(),
            completed_at,
        }
    };

    let expired_secret = "AKRA_EXPIRED_PROMPT_SENTINEL_8f3c69f2";
    let malformed_secret = "AKRA_MALFORMED_TIME_SENTINEL_b84c7061";
    let future_secret = "AKRA_FUTURE_TIME_SENTINEL_1cf58f62";
    adapter
        .append_app_server_prompt_interaction(
            &workspace_dir,
            make_record(
                "expired",
                "2000-01-01T00:00:00Z".to_string(),
                expired_secret.to_string(),
            ),
        )
        .expect("expired prompt log append should remain valid");
    adapter
        .append_app_server_prompt_interaction(
            &workspace_dir,
            make_record(
                "malformed-time",
                "not-a-timestamp".repeat(2_000),
                malformed_secret.to_string(),
            ),
        )
        .expect("malformed timestamp record should be purged without retaining its payload");
    adapter
        .append_app_server_prompt_interaction(
            &workspace_dir,
            make_record(
                "future-time",
                "2999-01-01T00:00:00Z".to_string(),
                future_secret.to_string(),
            ),
        )
        .expect("future timestamp record should be purged without extending retention");
    adapter
        .append_app_server_prompt_interaction(
            &workspace_dir,
            make_record("bounded", Utc::now().to_rfc3339(), "한".repeat(20_000)),
        )
        .expect("bounded prompt log should append");

    let snapshot = adapter
        .load_recent_app_server_prompt_interactions(&workspace_dir, 10)
        .expect("bounded prompt log should load");
    assert_eq!(snapshot.records.len(), 1);
    let record = &snapshot.records[0];
    assert_eq!(record.interaction_id, "bounded");
    assert_eq!(record.input_items.len(), 16);
    assert_eq!(record.output_items.len(), 16);
    assert_eq!(
        record.input_items[0].content.chars().count(),
        crate::application::port::outbound::app_server_prompt_log_port::APP_SERVER_PROMPT_LOG_MAX_BODY_CHARS
    );
    assert!(record.input_items[0].content.ends_with("retention policy]"));

    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("authority location should resolve");
    let database_bytes = std::fs::read(&location.authority_store_path)
        .expect("authority store bytes should be readable");
    assert!(
        !database_bytes
            .windows(expired_secret.len())
            .any(|window| window == expired_secret.as_bytes()),
        "secure_delete must remove expired prompt bytes from SQLite pages"
    );
    for rejected_secret in [malformed_secret, future_secret] {
        assert!(
            !database_bytes
                .windows(rejected_secret.len())
                .any(|window| window == rejected_secret.as_bytes()),
            "secure_delete must remove malformed or future-dated prompt bytes from SQLite pages"
        );
    }
}

#[test]
fn disabling_app_server_prompt_log_securely_clears_retained_records() {
    let workspace_dir = temp_workspace("prompt-log-disabled-clear");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let now = Utc::now().to_rfc3339();
    let secret = "AKRA_DISABLED_PROMPT_SENTINEL_429c7785";
    adapter
        .append_app_server_prompt_interaction(
            &workspace_dir,
            AppServerPromptInteractionRecord {
                sequence: 0,
                interaction_id: "disabled-clear".to_string(),
                session_kind: "main".to_string(),
                operation: "turn".to_string(),
                status: "completed".to_string(),
                workspace_dir: workspace_dir.clone(),
                thread_id: None,
                turn_id: None,
                service_name: None,
                model: None,
                reasoning_effort: None,
                developer_instructions: Some(secret.to_string()),
                input_items: vec![AppServerPromptInputRecord::new("text", "input", secret)],
                output_items: vec![AppServerPromptOutputRecord::new("output", None, secret)],
                error_message: None,
                started_at: now.clone(),
                completed_at: now,
            },
        )
        .expect("prompt log fixture should append");

    let deleted =
        SqlitePlanningAuthorityAdapter::clear_app_server_prompt_interaction_records(&workspace_dir)
            .expect("disabled prompt logs should clear");
    assert_eq!(deleted, 1);

    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("authority location should resolve");
    let connection = open_authority_connection(&location).expect("authority store should reopen");
    let record_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM app_server_prompt_interactions",
            [],
            |row| row.get(0),
        )
        .expect("prompt record count should load");
    let metadata_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM authority_metadata
             WHERE key = 'last_app_server_prompt_log_at'",
            [],
            |row| row.get(0),
        )
        .expect("prompt metadata count should load");
    drop(connection);
    assert_eq!(record_count, 0);
    assert_eq!(metadata_count, 0);

    let database_bytes = std::fs::read(&location.authority_store_path)
        .expect("authority store bytes should be readable");
    assert!(
        !database_bytes
            .windows(secret.len())
            .any(|window| window == secret.as_bytes()),
        "secure_delete must remove disabled prompt bytes from SQLite pages"
    );
}

#[test]
fn app_server_prompt_log_retains_only_the_newest_hundred_records() {
    let workspace_dir = temp_workspace("prompt-log-count-retention");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    for index in 0..=100 {
        let now = Utc::now().to_rfc3339();
        adapter
            .append_app_server_prompt_interaction(
                &workspace_dir,
                AppServerPromptInteractionRecord {
                    sequence: 0,
                    interaction_id: format!("interaction-{index}"),
                    session_kind: "main".to_string(),
                    operation: "turn".to_string(),
                    status: "completed".to_string(),
                    workspace_dir: workspace_dir.clone(),
                    thread_id: None,
                    turn_id: None,
                    service_name: None,
                    model: None,
                    reasoning_effort: None,
                    developer_instructions: None,
                    input_items: Vec::new(),
                    output_items: Vec::new(),
                    error_message: None,
                    started_at: now.clone(),
                    completed_at: now,
                },
            )
            .expect("prompt log record should append");
    }

    let snapshot = adapter
        .load_recent_app_server_prompt_interactions(&workspace_dir, 200)
        .expect("retained prompt logs should load");
    assert_eq!(snapshot.records.len(), 100);
    assert_eq!(snapshot.records[0].interaction_id, "interaction-100");
    assert_eq!(snapshot.records[99].interaction_id, "interaction-1");
}

#[test]
fn task_authority_snapshot_persists_queryable_provenance_columns() {
    let workspace_dir = temp_workspace("task-provenance");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
        .with_thread_turn(
            Some("worker-thread-1".to_string()),
            Some("worker-turn-1".to_string()),
        )
        .with_parent(
            Some("main-thread-1".to_string()),
            Some("main-turn-1".to_string()),
        );
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![TaskDefinition {
            id: "task-provenance-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_relation_note: "covers provenance storage".to_string(),
            title: "Persist provenance".to_string(),
            description: "Persist generic provenance columns.".to_string(),
            status: TaskStatus::Ready,
            base_priority: 80,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::Worker,
            last_updated_by: TaskActor::Worker,
            source_turn_id: Some("worker-turn-1".to_string()),
            provenance,
            updated_at: "2026-05-07T09:00:00Z".to_string(),
        }],
    };
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("task authority should commit");

    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace_dir)
            .expect("authority location should resolve");
    let connection = open_authority_connection(&location).expect("authority db should open");
    let row = connection
        .query_row(
            "SELECT origin_session_kind, thread_id, turn_id, parent_thread_id, parent_turn_id
             FROM planning_tasks WHERE task_id = 'task-provenance-1'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .expect("provenance row should load");

    assert_eq!(
        row,
        (
            "planner".to_string(),
            "worker-thread-1".to_string(),
            "worker-turn-1".to_string(),
            "main-thread-1".to_string(),
            "main-turn-1".to_string(),
        )
    );
}

#[test]
fn task_authority_row_write_errors_keep_operation_context() {
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: vec![TaskDefinition {
            id: "task-edge".to_string(),
            direction_id: "direction-1".to_string(),
            direction_relation_note: "covers row error contexts".to_string(),
            title: "Persist row context".to_string(),
            description: "Persist relation and projection rows.".to_string(),
            status: TaskStatus::Ready,
            base_priority: 80,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: vec!["task-parent".to_string()],
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: TaskMutationProvenance::default(),
            updated_at: "2026-05-07T09:00:00Z".to_string(),
        }],
    };
    let empty_task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let empty_queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    let edge_workspace = temp_workspace("task-row-error-edge");
    let mut edge_connection = authority_connection(&edge_workspace);
    replace_table_schema(&edge_connection, "planning_task_edges", "task_id TEXT");
    let edge_transaction = edge_connection
        .transaction()
        .expect("edge transaction should open");
    assert_error_contains(
        replace_task_authority_tables(&edge_transaction, &task_authority, &empty_queue_projection),
        "failed to persist planning task edge `task-edge:depends_on`",
    );

    let active_projection_workspace = temp_workspace("task-row-error-active-projection");
    let mut active_projection_connection = authority_connection(&active_projection_workspace);
    replace_table_schema(
        &active_projection_connection,
        "planning_queue_projection",
        "bucket TEXT",
    );
    let active_queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: vec![PriorityQueueTask {
            rank: 1,
            task_id: "task-active".to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction 1".to_string(),
            task_title: "Active projection".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-05-07T09:00:00Z".to_string(),
            rank_reasons: vec!["highest priority".to_string()],
        }],
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };
    let active_projection_transaction = active_projection_connection
        .transaction()
        .expect("active projection transaction should open");
    assert_error_contains(
        replace_task_authority_tables(
            &active_projection_transaction,
            &empty_task_authority,
            &active_queue_projection,
        ),
        "failed to persist planning queue projection `active:task-active`",
    );

    let skipped_projection_workspace = temp_workspace("task-row-error-skipped-projection");
    let mut skipped_projection_connection = authority_connection(&skipped_projection_workspace);
    replace_table_schema(
        &skipped_projection_connection,
        "planning_queue_projection",
        "bucket TEXT",
    );
    let skipped_queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: vec![PriorityQueueSkippedTask {
            task_id: "task-skipped".to_string(),
            task_title: "Skipped projection".to_string(),
            direction_id: "direction-1".to_string(),
            status: TaskStatus::Blocked,
            reason: "blocked by dependency".to_string(),
        }],
    };
    let skipped_projection_transaction = skipped_projection_connection
        .transaction()
        .expect("skipped projection transaction should open");
    assert_error_contains(
        replace_task_authority_tables(
            &skipped_projection_transaction,
            &empty_task_authority,
            &skipped_queue_projection,
        ),
        "failed to persist skipped planning queue projection `task-skipped`",
    );

    let metadata_workspace = temp_workspace("task-row-error-metadata");
    let mut metadata_connection = authority_connection(&metadata_workspace);
    replace_table_schema(
        &metadata_connection,
        "authority_metadata",
        "broken_key TEXT",
    );
    let metadata_transaction = metadata_connection
        .transaction()
        .expect("metadata transaction should open");
    assert_error_contains(
        replace_task_authority_tables(
            &metadata_transaction,
            &empty_task_authority,
            &empty_queue_projection,
        ),
        "failed to update authority metadata `task_authority_version`",
    );
}

#[test]
fn active_workspace_artifact_removal_preserves_task_authority_snapshot() {
    let workspace_dir = temp_workspace("active-artifact-preserves-authority");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("task authority should commit");
    SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
        &workspace_dir,
        &PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("operator result output".to_string()),
        },
    )
    .expect("active workspace artifact should commit");
    assert_eq!(
        SqlitePlanningAuthorityAdapter::load_active_planning_file(
            &workspace_dir,
            RESULT_OUTPUT_FILE_PATH,
        )
        .expect("active artifact should load")
        .as_deref(),
        Some("operator result output")
    );

    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        &workspace_dir,
        RESULT_OUTPUT_FILE_PATH,
        None,
    )
    .expect("active workspace artifact should be removable");

    assert!(
        !SqlitePlanningAuthorityAdapter::load_active_workspace_files(&workspace_dir)
            .expect("active workspace should load after artifact removal")
            .has_any_files()
    );
    let snapshot = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("task authority should still load")
        .expect("task authority snapshot should remain accepted authority");
    assert_eq!(snapshot.task_authority, task_authority);
    assert_eq!(snapshot.queue_projection, queue_projection);
}

#[test]
fn active_document_prefix_removal_treats_sql_wildcards_as_literal_path_text() {
    let workspace_dir = temp_workspace("active-document-literal-prefix");
    for (path, body) in [
        (".codex-exec-loop/planning/prompts/a_b/target.md", "target"),
        (
            ".codex-exec-loop/planning/prompts/axb/preserved.md",
            "preserved underscore neighbor",
        ),
        (
            ".codex-exec-loop/planning/prompts/a%b/target.md",
            "percent target",
        ),
        (
            ".codex-exec-loop/planning/prompts/azzzb/preserved.md",
            "preserved percent neighbor",
        ),
    ] {
        SqlitePlanningAuthorityAdapter::replace_active_planning_file(
            &workspace_dir,
            path,
            Some(body),
        )
        .expect("active document should seed");
    }

    SqlitePlanningAuthorityAdapter::remove_active_planning_entry(
        &workspace_dir,
        ".codex-exec-loop/planning/prompts/a_b",
    )
    .expect("underscore path should remove literally");
    SqlitePlanningAuthorityAdapter::remove_active_planning_entry(
        &workspace_dir,
        ".codex-exec-loop/planning/prompts/a%b",
    )
    .expect("percent path should remove literally");

    for removed_path in [
        ".codex-exec-loop/planning/prompts/a_b/target.md",
        ".codex-exec-loop/planning/prompts/a%b/target.md",
    ] {
        assert!(
            SqlitePlanningAuthorityAdapter::load_active_planning_file(
                &workspace_dir,
                removed_path,
            )
            .expect("removed document should inspect")
            .is_none()
        );
    }
    for (preserved_path, expected) in [
        (
            ".codex-exec-loop/planning/prompts/axb/preserved.md",
            "preserved underscore neighbor",
        ),
        (
            ".codex-exec-loop/planning/prompts/azzzb/preserved.md",
            "preserved percent neighbor",
        ),
    ] {
        assert_eq!(
            SqlitePlanningAuthorityAdapter::load_active_planning_file(
                &workspace_dir,
                preserved_path,
            )
            .expect("neighbor document should inspect")
            .as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn staged_draft_rows_do_not_mutate_active_workspace_or_task_authority_snapshot() {
    let workspace_dir = temp_workspace("draft-preserves-authority");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("task authority should commit");
    SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
        &workspace_dir,
        &PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("active result output".to_string()),
        },
    )
    .expect("active workspace artifact should commit");

    SqlitePlanningAuthorityAdapter::stage_repo_scoped_draft_files(
        &workspace_dir,
        "draft-one",
        &[PlanningDraftFileRecord {
            active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            body: "draft result output".to_string(),
        }],
    )
    .expect("draft artifact should stage");

    assert_eq!(
        SqlitePlanningAuthorityAdapter::load_active_planning_file(
            &workspace_dir,
            RESULT_OUTPUT_FILE_PATH,
        )
        .expect("active artifact should load")
        .as_deref(),
        Some("active result output")
    );
    let snapshot = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("task authority should still load")
        .expect("task authority snapshot should remain accepted authority");
    assert_eq!(snapshot.task_authority, task_authority);
    assert_eq!(snapshot.queue_projection, queue_projection);
}

#[test]
fn repo_scoped_workspace_port_delegates_active_and_draft_operations() {
    let workspace_dir = temp_workspace("repo-scoped-port");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let port: &dyn RepoScopedPlanningWorkspacePort = &adapter;

    assert!(!port.is_git_backed_workspace(&workspace_dir));
    assert!(
        port.resolve_active_workspace_root(&workspace_dir)
            .is_absolute()
    );

    port.commit_active_workspace_files(
        &workspace_dir,
        &PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("active result output".to_string()),
        },
    )
    .expect("active workspace should commit through repo-scoped port");
    assert_eq!(
        port.load_active_workspace_files(&workspace_dir)
            .expect("active workspace should load through repo-scoped port")
            .result_output_markdown
            .as_deref(),
        Some("active result output")
    );

    port.replace_active_planning_file(
        &workspace_dir,
        RESULT_OUTPUT_FILE_PATH,
        Some("updated active result"),
    )
    .expect("active file should update through repo-scoped port");
    assert_eq!(
        port.load_active_planning_file(&workspace_dir, RESULT_OUTPUT_FILE_PATH)
            .expect("active file should load through repo-scoped port")
            .as_deref(),
        Some("updated active result")
    );

    let staged = port
        .stage_repo_scoped_draft_files(
            &workspace_dir,
            "draft-port",
            &[PlanningDraftFileRecord {
                active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                body: "draft result output".to_string(),
            }],
        )
        .expect("draft should stage through repo-scoped port");
    assert_eq!(staged.draft_name, "draft-port");

    let staged_path = port
        .replace_repo_scoped_draft_file(
            &workspace_dir,
            "draft-port",
            RESULT_OUTPUT_FILE_PATH,
            "updated draft result",
        )
        .expect("draft file should update through repo-scoped port");
    assert!(staged_path.contains("draft-port"));

    let loaded = port
        .load_repo_scoped_draft_files(&workspace_dir, "draft-port")
        .expect("draft should load through repo-scoped port");
    assert_eq!(loaded.staged_files.len(), 1);
    assert_eq!(loaded.staged_files[0].body, "updated draft result");

    let invalid_error = port
        .stage_repo_scoped_draft_files(
            &workspace_dir,
            "../outside",
            &[PlanningDraftFileRecord {
                active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                body: "escaped draft result output".to_string(),
            }],
        )
        .expect_err("invalid repo-scoped draft name should not stage");
    assert!(
        invalid_error
            .to_string()
            .contains("invalid planning draft name `../outside`")
    );

    port.remove_active_planning_entry(&workspace_dir, RESULT_OUTPUT_FILE_PATH)
        .expect("active file should remove through repo-scoped port");
    assert_eq!(
        port.load_active_planning_file(&workspace_dir, RESULT_OUTPUT_FILE_PATH)
            .expect("active file lookup should still succeed")
            .as_deref(),
        None
    );
}

#[test]
fn runtime_reset_preserves_latest_failed_start_dispatch_block_per_task() {
    let workspace_dir = temp_workspace("failed-start-blocks");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    adapter
        .upsert_runtime_slot_lease(
            &workspace_dir,
            &slot_lease_for_task("slot-reset", "task-1", ParallelModeSlotLeaseState::Running),
        )
        .expect("slot lease should persist before reset");
    insert_invalid_slot_marker(&workspace_dir, "slot-reset");
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &failed_start_session_detail("session-new", "task-1", "2026-05-04T12:00:00+00:00"),
        )
        .expect("newer failed-start detail should persist");
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &failed_start_session_detail("session-old", "task-1", "2026-05-04T11:00:00+00:00"),
        )
        .expect("older failed-start detail should persist");
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &running_session_detail(
                "session-running",
                "task-running",
                "2026-05-04T12:02:00+00:00",
            ),
        )
        .expect("non-failed session should persist before reset");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-reset", "session-new", "task-1"),
        )
        .expect("queue record should persist before reset");
    adapter
        .enqueue_runtime_dispatch_command(
            &workspace_dir,
            &ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
                ParallelModeAutomationTrigger::ParallelOfficialCompletion,
                Some("task-1:ready".to_string()),
                Some(501),
                "2026-05-08T00:00:00+00:00",
            ),
        )
        .expect("dispatch command should persist before reset");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "queue-owner")
            .expect("queue claim should acquire before reset")
    );
    let refresh_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("official refresh order should reserve before reset");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
            .expect("official refresh claim should acquire before reset"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );

    adapter
        .clear_parallel_runtime_projections(&workspace_dir, "test reset")
        .expect("runtime projections should clear");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert!(snapshot.slot_leases.is_empty());
    assert!(snapshot.invalid_slot_leases.is_empty());
    assert_eq!(snapshot.session_details.len(), 0);
    assert!(snapshot.distributor_queue_records.is_empty());
    assert!(snapshot.dispatch_commands.is_empty());
    assert_eq!(snapshot.task_dispatch_blocks.len(), 1);
    let block = &snapshot.task_dispatch_blocks[0];
    assert_eq!(block.task_id, "task-1");
    assert_eq!(block.blocked_at, "2026-05-04T12:00:00+00:00");
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "queue-owner-2")
            .expect("removed queue must not recreate an orphan claim")
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner-2")
            .expect("official refresh claim should clear during runtime reset"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn runtime_reset_preserves_admin_guards_for_guard_first_and_reset_first_orders() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let task_guard_workspace = temp_workspace("runtime-reset-task-guard-first");
    let collision_task_ids = vec![OFFICIAL_REFRESH_SCOPE_KEY.to_string()];
    adapter
        .acquire_admin_task_mutation_guard(
            &task_guard_workspace,
            &collision_task_ids,
            "task-guard-owner",
        )
        .expect("task guard should acquire before reset");
    adapter
        .clear_parallel_runtime_projections(&task_guard_workspace, "guard-first reset")
        .expect("runtime reset should complete without deleting task guard");
    let task_guard_count: i64 = authority_connection(&task_guard_workspace)
        .query_row(
            "SELECT COUNT(*) FROM runtime_claims WHERE claim_kind = ?1 AND scope_key = ?2",
            (ADMIN_TASK_MUTATION_CLAIM_KIND, OFFICIAL_REFRESH_SCOPE_KEY),
            |row| row.get(0),
        )
        .expect("task guard should inspect after reset");
    assert_eq!(
        task_guard_count, 1,
        "scope-key collision must not clear task guard"
    );
    let late_lease = adapter
        .upsert_runtime_slot_lease(
            &task_guard_workspace,
            &slot_lease_for_task(
                "slot-collision",
                OFFICIAL_REFRESH_SCOPE_KEY,
                ParallelModeSlotLeaseState::Leased,
            ),
        )
        .expect_err("late lease must remain excluded after reset");
    assert!(late_lease.to_string().contains("admin mutation guard"));
    adapter
        .release_admin_task_mutation_guard(
            &task_guard_workspace,
            &collision_task_ids,
            "task-guard-owner",
        )
        .expect("task guard should release");

    let file_guard_workspace = temp_workspace("runtime-reset-file-guard-first");
    adapter
        .acquire_admin_file_sync_guard(
            &file_guard_workspace,
            "file-guard-owner",
            "export planning support files",
        )
        .expect("file guard should acquire before reset");
    adapter
        .clear_parallel_runtime_projections(&file_guard_workspace, "file guard reset")
        .expect("runtime reset should preserve file guard");
    let file_guard_count: i64 = authority_connection(&file_guard_workspace)
        .query_row(
            "SELECT COUNT(*) FROM runtime_claims WHERE claim_kind = ?1",
            [ADMIN_FILE_SYNC_CLAIM_KIND],
            |row| row.get(0),
        )
        .expect("file guard should inspect after reset");
    assert_eq!(file_guard_count, 1);
    let late_command = adapter
        .enqueue_runtime_dispatch_command(&file_guard_workspace, &dispatch_command_snapshot(991))
        .expect_err("late dispatch command must remain excluded after reset");
    assert!(
        late_command
            .to_string()
            .contains("admin authority mutation guard")
    );
    adapter
        .release_admin_file_sync_guard(&file_guard_workspace, "file-guard-owner")
        .expect("file guard should release");

    let reset_first_workspace = temp_workspace("runtime-reset-first-guard");
    adapter
        .clear_parallel_runtime_projections(&reset_first_workspace, "reset first")
        .expect("empty runtime reset should succeed");
    adapter
        .acquire_admin_file_sync_guard(
            &reset_first_workspace,
            "reset-first-owner",
            "apply exported planning support files",
        )
        .expect("file guard should acquire after reset");
    let late_queue = adapter
        .upsert_runtime_distributor_queue_record(
            &reset_first_workspace,
            &queue_record_for_task(
                "queue-reset-first",
                "session-reset-first",
                "task-reset-first",
            ),
        )
        .expect_err("late queue must lose to reset-first guard");
    assert!(
        late_queue
            .to_string()
            .contains("admin authority mutation guard")
    );
    adapter
        .release_admin_file_sync_guard(&reset_first_workspace, "reset-first-owner")
        .expect("reset-first guard should release");
}

#[test]
fn runtime_dispatch_command_enqueue_claim_and_update_round_trips() {
    let workspace_dir = temp_workspace("dispatch-command");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-1".to_string()),
        Some(11),
        "2026-05-08T00:00:00+00:00",
    );

    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("command should enqueue")
    );
    assert!(
        !adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("duplicate command should not enqueue")
    );

    let claimed = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-1")
        .expect("command claim should succeed")
        .expect("pending command should be claimed");
    assert_eq!(claimed.command_id, command.command_id);
    assert_eq!(claimed.state, ParallelModeDispatchCommandState::Running);
    assert_eq!(claimed.owner_token.as_deref(), Some("owner-1"));
    assert!(
        adapter
            .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-2")
            .expect("second claim should inspect cleanly")
            .is_none()
    );

    let mut completed = claimed;
    completed.mark_completed("launched workers", "2026-05-08T00:00:10+00:00");
    adapter
        .update_runtime_dispatch_command(&workspace_dir, &completed)
        .expect("completed command should persist");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(snapshot.dispatch_commands.len(), 1);
    assert_eq!(
        snapshot.dispatch_commands[0].state,
        ParallelModeDispatchCommandState::Completed
    );
    assert_eq!(
        snapshot.dispatch_commands[0].status_detail.as_deref(),
        Some("launched workers")
    );
}

#[test]
fn admin_file_sync_guard_and_nonterminal_dispatch_commands_are_mutually_exclusive() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let session_workspace = temp_workspace("file-sync-runtime-session-first");
    adapter
        .upsert_runtime_session_detail(
            &session_workspace,
            &running_session_detail(
                "session-running",
                "task-running",
                "2026-05-08T00:00:00+00:00",
            ),
        )
        .expect("running session should persist");
    let session_error = adapter
        .acquire_admin_file_sync_guard(
            &session_workspace,
            "operator-session",
            "export planning support files",
        )
        .expect_err("running session must block file sync");
    assert!(
        session_error
            .to_string()
            .contains("session session-running is in_progress")
    );

    let command_workspace = temp_workspace("file-sync-runtime-command-first");
    let command = dispatch_command_snapshot(91);
    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&command_workspace, &command)
            .expect("pending command should enqueue")
    );
    let command_error = adapter
        .acquire_admin_file_sync_guard(
            &command_workspace,
            "operator-command",
            "apply exported planning support files",
        )
        .expect_err("pending command must block file sync");
    assert!(command_error.to_string().contains("dispatch command"));

    let guard_workspace = temp_workspace("file-sync-guard-first-dispatch");
    adapter
        .acquire_admin_file_sync_guard(
            &guard_workspace,
            "operator-first",
            "export planning support files",
        )
        .expect("idle runtime should admit file sync guard");
    let late_command = dispatch_command_snapshot(92);
    let enqueue_error = adapter
        .enqueue_runtime_dispatch_command(&guard_workspace, &late_command)
        .expect_err("late enqueue must lose to file sync guard");
    assert!(
        enqueue_error
            .to_string()
            .contains("admin authority mutation guard")
    );

    insert_complete_pending_dispatch_command_row(&guard_workspace, &late_command);
    let claim_error = adapter
        .try_claim_next_runtime_dispatch_command(&guard_workspace, "dispatcher-late")
        .expect_err("late dispatch claim must lose to file sync guard");
    assert!(
        claim_error
            .to_string()
            .contains("admin authority mutation guard")
    );
    let update_error = adapter
        .update_runtime_dispatch_command(&guard_workspace, &late_command)
        .expect_err("late nonterminal update must lose to file sync guard");
    assert!(
        update_error
            .to_string()
            .contains("admin authority mutation guard")
    );

    let mut terminal_command = late_command;
    terminal_command.mark_canceled("operator file sync cleanup", "2026-05-08T00:01:00+00:00");
    adapter
        .update_runtime_dispatch_command(&guard_workspace, &terminal_command)
        .expect("terminal cleanup update should remain available under file sync guard");
    adapter
        .release_admin_file_sync_guard(&guard_workspace, "operator-first")
        .expect("file sync guard should release");
}

#[test]
fn runtime_lease_first_blocks_authority_mutation_guard_after_barrier() {
    let workspace_dir = temp_workspace("authority-guard-runtime-first");
    let lease_persisted = Arc::new(Barrier::new(2));
    let worker_workspace = workspace_dir.clone();
    let worker_barrier = lease_persisted.clone();
    let worker = std::thread::spawn(move || {
        SqlitePlanningAuthorityAdapter::new()
            .upsert_runtime_slot_lease(
                &worker_workspace,
                &slot_lease_for_task(
                    "slot-runtime-first",
                    "task-runtime-first",
                    ParallelModeSlotLeaseState::Running,
                ),
            )
            .expect("runtime-first lease should persist");
        worker_barrier.wait();
    });

    lease_persisted.wait();
    let error = SqlitePlanningAuthorityAdapter::new()
        .acquire_admin_authority_mutation_guard(
            &workspace_dir,
            "direction-owner",
            "upsert planning direction",
        )
        .expect_err("active runtime lease must fence direction mutation");
    worker.join().expect("runtime-first worker should join");

    assert!(
        error
            .to_string()
            .contains("slot slot-runtime-first is running")
    );
}

#[test]
fn runtime_claim_first_blocks_authority_mutation_guard_after_barrier() {
    let workspace_dir = temp_workspace("authority-guard-claim-first");
    let claim_persisted = Arc::new(Barrier::new(2));
    let worker_workspace = workspace_dir.clone();
    let worker_barrier = claim_persisted.clone();
    let worker = std::thread::spawn(move || {
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let refresh_order = adapter
            .reserve_next_official_refresh_order(&worker_workspace)
            .expect("claim-first refresh order should reserve");
        assert_eq!(
            adapter
                .acquire_official_refresh_claim(&worker_workspace, refresh_order, "refresh-owner",)
                .expect("claim-first refresh claim should acquire"),
            PlanningAuthorityOfficialRefreshClaimStatus::Acquired
        );
        worker_barrier.wait();
        refresh_order
    });

    claim_persisted.wait();
    let error = SqlitePlanningAuthorityAdapter::new()
        .acquire_admin_authority_mutation_guard(
            &workspace_dir,
            "direction-owner",
            "delete planning direction",
        )
        .expect_err("active runtime claim must fence direction mutation");
    let refresh_order = worker.join().expect("claim-first worker should join");

    assert!(error.to_string().contains("official-refresh"), "{error}");
    SqlitePlanningAuthorityAdapter::new()
        .release_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
        .expect("claim-first refresh claim should release");
}

#[test]
fn authority_guard_first_fences_task_create_reassignment_and_runtime_lease() {
    let workspace_dir = temp_workspace("authority-guard-first");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let baseline_authority = task_authority_for_direction("task-existing", "direction-a");
    let queue_projection = empty_test_queue_projection();
    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &baseline_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("baseline task authority should commit");
    let refresh_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("official refresh order should reserve before the direction guard");
    adapter
        .acquire_admin_authority_mutation_guard(
            &workspace_dir,
            "direction-owner",
            "upsert planning direction",
        )
        .expect("idle workspace should admit direction guard");

    let attempts_started = Arc::new(Barrier::new(2));
    let worker_workspace = workspace_dir.clone();
    let worker_barrier = attempts_started.clone();
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        let worker_adapter = SqlitePlanningAuthorityAdapter::new();
        let queue_projection = empty_test_queue_projection();
        let mut created = task_authority_for_direction("task-existing", "direction-a");
        created
            .tasks
            .push(authority_task("task-created", "direction-b"));
        let create_error = worker_adapter
            .commit_task_authority_snapshot(
                &worker_workspace,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority: &created,
                    queue_projection: &queue_projection,
                },
            )
            .expect_err("direction guard must block task creation");

        let reassigned = task_authority_for_direction("task-existing", "direction-b");
        let reassign_error = worker_adapter
            .commit_task_authority_snapshot(
                &worker_workspace,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority: &reassigned,
                    queue_projection: &queue_projection,
                },
            )
            .expect_err("direction guard must block task reassignment");

        let lease_error = worker_adapter
            .upsert_runtime_slot_lease(
                &worker_workspace,
                &slot_lease_for_task(
                    "slot-late",
                    "task-existing",
                    ParallelModeSlotLeaseState::Leased,
                ),
            )
            .expect_err("direction guard must block a late runtime lease");
        let claim_error = worker_adapter
            .acquire_official_refresh_claim(&worker_workspace, refresh_order, "late-refresh-owner")
            .expect_err("direction guard must block a late runtime claim");
        let active_document_error = SqlitePlanningAuthorityAdapter::replace_active_planning_file(
            &worker_workspace,
            RESULT_OUTPUT_FILE_PATH,
            Some("late active document"),
        )
        .expect_err("direction guard must block a late active document write");
        (
            create_error.to_string(),
            reassign_error.to_string(),
            lease_error.to_string(),
            claim_error.to_string(),
            active_document_error.to_string(),
        )
    });

    attempts_started.wait();
    let (create_error, reassign_error, lease_error, claim_error, active_document_error) =
        worker.join().expect("guard-first worker should join");
    for error in [
        &create_error,
        &reassign_error,
        &lease_error,
        &claim_error,
        &active_document_error,
    ] {
        assert!(error.contains("admin authority mutation guard"), "{error}");
    }
    adapter
        .release_admin_authority_mutation_guard(&workspace_dir, "direction-owner")
        .expect("direction guard should release");
    let persisted = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("baseline task authority should reload")
        .expect("baseline task authority should remain");
    assert_eq!(persisted.task_authority, baseline_authority);
}

#[test]
fn stale_direction_child_snapshot_cannot_admit_phantom_runtime_task() {
    let workspace_dir = temp_workspace("authority-guard-phantom-child");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let empty_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let queue_projection = empty_test_queue_projection();
    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &empty_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("empty baseline should commit");

    let snapshot_taken = Arc::new(Barrier::new(2));
    let phantom_started = Arc::new(Barrier::new(2));
    let worker_workspace = workspace_dir.clone();
    let worker_snapshot_barrier = snapshot_taken.clone();
    let worker_phantom_barrier = phantom_started.clone();
    let direction_worker = std::thread::spawn(move || {
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let stale_snapshot = adapter
            .load_task_authority_snapshot(&worker_workspace)
            .expect("direction child snapshot should load")
            .expect("direction child snapshot should exist");
        assert!(stale_snapshot.task_authority.tasks.is_empty());
        worker_snapshot_barrier.wait();
        worker_phantom_barrier.wait();
        adapter
            .acquire_admin_authority_mutation_guard(
                &worker_workspace,
                "stale-direction-owner",
                "upsert planning direction",
            )
            .expect_err("new runtime child must fence the stale direction edit")
            .to_string()
    });

    snapshot_taken.wait();
    let phantom_authority = task_authority_for_direction("task-phantom", "direction-a");
    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &phantom_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("phantom child task should commit after the stale snapshot");
    adapter
        .upsert_runtime_slot_lease(
            &workspace_dir,
            &slot_lease_for_task(
                "slot-phantom",
                "task-phantom",
                ParallelModeSlotLeaseState::Running,
            ),
        )
        .expect("phantom child runtime should start before direction guard acquisition");
    phantom_started.wait();

    let error = direction_worker
        .join()
        .expect("stale direction worker should join");
    assert!(error.contains("slot slot-phantom is running"), "{error}");
}

#[test]
fn stale_direction_catalog_conflicts_without_deleting_new_authority() {
    let workspace_dir = temp_workspace("authority-guard-stale-direction-cas");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let direction_a = DirectionDefinition {
        id: "direction-a".to_string(),
        title: "Direction A".to_string(),
        summary: "baseline direction".to_string(),
        success_criteria: vec!["done".to_string()],
        scope_hints: Vec::new(),
        detail_doc_path: String::new(),
        state: DirectionState::Active,
    };
    let baseline_directions = DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig {
            policy: QueueIdlePolicy::Stop,
            prompt_path: String::new(),
        },
        directions: vec![direction_a.clone()],
    };
    let empty_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let empty_queue = empty_test_queue_projection();
    let baseline_result = adapter
        .commit_planning_authority_documents(
            &workspace_dir,
            PlanningAuthorityDocumentCommit {
                observed_planning_revision: None,
                directions: &baseline_directions,
                task_authority: &empty_authority,
                queue_projection: &empty_queue,
                result_output_markdown: Some("# Result Output\n"),
                active_document_mutations: &[],
                retired_task_ids: &[],
                authority_mutation_owner_token: None,
            },
        )
        .expect("baseline authority should commit");
    let PlanningTaskAuthorityCommitResult::Committed {
        planning_revision: baseline_revision,
        ..
    } = baseline_result
    else {
        panic!("baseline authority should commit");
    };

    let stale_snapshot_loaded = Arc::new(Barrier::new(2));
    let concurrent_commit_finished = Arc::new(Barrier::new(2));
    let worker_workspace = workspace_dir.clone();
    let worker_loaded = stale_snapshot_loaded.clone();
    let worker_commit_finished = concurrent_commit_finished.clone();
    let worker = std::thread::spawn(move || {
        let adapter = SqlitePlanningAuthorityAdapter::new();
        let stale_snapshot = adapter
            .load_direction_authority_snapshot(&worker_workspace)
            .expect("stale direction snapshot should load")
            .expect("stale direction snapshot should exist");
        assert_eq!(stale_snapshot.planning_revision, baseline_revision);
        worker_loaded.wait();
        worker_commit_finished.wait();

        adapter
            .acquire_admin_authority_mutation_guard(
                &worker_workspace,
                "stale-direction-owner",
                "edit stale direction catalog",
            )
            .expect("idle runtime should admit the operator guard");
        let result = adapter
            .commit_direction_authority_snapshot(
                &worker_workspace,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: Some(stale_snapshot.planning_revision),
                    directions: &stale_snapshot.directions,
                    authority_mutation_owner_token: Some("stale-direction-owner"),
                },
            )
            .expect("stale direction commit should return a conflict");
        adapter
            .release_admin_authority_mutation_guard(&worker_workspace, "stale-direction-owner")
            .expect("stale direction guard should release");
        result
    });

    stale_snapshot_loaded.wait();
    let changed_directions = DirectionCatalogDocument {
        directions: vec![
            direction_a,
            DirectionDefinition {
                id: "direction-b".to_string(),
                title: "Direction B".to_string(),
                summary: "concurrent direction".to_string(),
                success_criteria: vec!["preserved".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            },
        ],
        ..baseline_directions
    };
    let changed_authority = task_authority_for_direction("task-b", "direction-b");
    let concurrent_result = adapter
        .commit_planning_authority_documents(
            &workspace_dir,
            PlanningAuthorityDocumentCommit {
                observed_planning_revision: Some(baseline_revision),
                directions: &changed_directions,
                task_authority: &changed_authority,
                queue_projection: &empty_queue,
                result_output_markdown: Some("# Result Output\n\nConcurrent edit\n"),
                active_document_mutations: &[],
                retired_task_ids: &[],
                authority_mutation_owner_token: None,
            },
        )
        .expect("concurrent authority should commit");
    assert!(matches!(
        concurrent_result,
        PlanningTaskAuthorityCommitResult::Committed { changed: true, .. }
    ));
    concurrent_commit_finished.wait();

    let stale_result = worker.join().expect("stale direction worker should join");
    assert!(matches!(
        stale_result,
        PlanningTaskAuthorityCommitResult::Conflict {
            observed_planning_revision,
            current_planning_revision,
        } if observed_planning_revision == baseline_revision
            && current_planning_revision > baseline_revision
    ));
    let persisted_directions = adapter
        .load_direction_authority_snapshot(&workspace_dir)
        .expect("directions should reload")
        .expect("directions should remain present");
    assert!(
        persisted_directions
            .directions
            .directions
            .iter()
            .any(|direction| direction.id == "direction-b")
    );
    let persisted_tasks = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("tasks should reload")
        .expect("tasks should remain present");
    assert!(
        persisted_tasks
            .task_authority
            .tasks
            .iter()
            .any(|task| task.id == "task-b")
    );
}

#[test]
fn runtime_dispatch_command_reenqueue_revives_terminal_rows() {
    let workspace_dir = temp_workspace("dispatch-command-revive");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let mut command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-revive".to_string()),
        Some(71),
        "2026-05-08T00:00:00+00:00",
    );

    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("initial command should enqueue")
    );
    let mut claimed = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-1")
        .expect("initial command should claim")
        .expect("initial command should exist");
    claimed.mark_blocked("waiting for capacity", "2026-05-08T00:00:10+00:00");
    adapter
        .update_runtime_dispatch_command(&workspace_dir, &claimed)
        .expect("blocked command should persist");

    command.updated_at = "2026-05-08T00:01:00+00:00".to_string();
    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("terminal command should revive")
    );
    let revived = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-2")
        .expect("revived command should claim")
        .expect("revived command should be pending again");

    assert_eq!(revived.command_id, command.command_id);
    assert_eq!(revived.state, ParallelModeDispatchCommandState::Running);
    assert_eq!(revived.owner_token.as_deref(), Some("owner-2"));
    assert!(
        adapter
            .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-3")
            .expect("empty dispatch queue should inspect cleanly")
            .is_none()
    );
}

#[test]
fn runtime_dispatch_command_reenqueue_replaces_nonterminal_stale_epoch_row() {
    let workspace_dir = temp_workspace("dispatch-command-replace-stale-epoch");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let stale_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-shared".to_string()),
        Some(71),
        "2026-05-08T00:00:00+00:00",
    );
    let current_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some("queue-head-shared".to_string()),
        Some(72),
        "2026-05-08T00:01:00+00:00",
    );

    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &stale_command)
            .expect("stale epoch seed should enqueue")
    );
    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &current_command)
            .expect("current epoch enqueue should replace stale row")
    );

    let claimed = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-2")
        .expect("replaced command should claim")
        .expect("replaced command should exist");
    assert_eq!(claimed.command_id, current_command.command_id);
    assert_eq!(
        claimed.trigger,
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch
    );
    assert_eq!(claimed.epoch_id, Some(72));
}

#[test]
fn runtime_dispatch_command_claim_reclaims_stale_running_rows() {
    let workspace_dir = temp_workspace("dispatch-command-reclaim-running");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some("queue-head-running-stale".to_string()),
        Some(73),
        "2026-05-08T00:00:00+00:00",
    );

    assert!(
        adapter
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("stale-running seed should enqueue")
    );
    let mut running = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-1")
        .expect("running seed should claim")
        .expect("running seed should exist");
    running.updated_at = "2020-05-08T00:00:00+00:00".to_string();
    adapter
        .update_runtime_dispatch_command(&workspace_dir, &running)
        .expect("stale running command should persist");

    let reclaimed = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-2")
        .expect("stale running command should be reclaimable")
        .expect("stale running command should be returned");
    assert_eq!(reclaimed.command_id, command.command_id);
    assert_eq!(reclaimed.state, ParallelModeDispatchCommandState::Running);
    assert_eq!(reclaimed.owner_token.as_deref(), Some("owner-2"));
}

#[test]
fn runtime_dispatch_command_claim_handles_payload_row_id_mismatch_as_lost_claim() {
    let workspace_dir = temp_workspace("dispatch-command-lost-claim");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let mut payload_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("payload-head".to_string()),
        Some(72),
        "2026-05-08T00:00:00+00:00",
    );
    payload_command.command_id = "payload-command-id".to_string();
    let payload_json =
        serde_json::to_string(&payload_command).expect("dispatch command payload should serialize");
    authority_connection(&workspace_dir)
        .execute(
            "INSERT INTO runtime_dispatch_commands
                (command_id, command_kind, trigger, command_state, queue_head_signature,
                 epoch_id, created_at, updated_at, owner_token, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            (
                "row-command-id",
                payload_command.kind.label(),
                payload_command.trigger.label(),
                payload_command.state.label(),
                payload_command.queue_head_signature.as_deref(),
                payload_command.epoch_id.map(|value| value as i64),
                payload_command.created_at.as_str(),
                payload_command.updated_at.as_str(),
                payload_command.owner_token.as_deref(),
                payload_json.as_str(),
            ),
        )
        .expect("mismatched dispatch command row should insert");

    assert!(
        adapter
            .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner")
            .expect("mismatched dispatch command should be treated as lost claim")
            .is_none()
    );
}

#[test]
fn runtime_dispatch_command_cancel_marks_only_non_terminal_commands() {
    let workspace_dir = temp_workspace("dispatch-command-cancel");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let running_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some("queue-head-running".to_string()),
        Some(21),
        "2026-05-08T00:00:00+00:00",
    );
    let completed_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-completed".to_string()),
        Some(22),
        "2026-05-08T00:00:01+00:00",
    );
    let pending_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("queue-head-pending".to_string()),
        Some(23),
        "2026-05-08T00:00:02+00:00",
    );

    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &running_command)
        .expect("running seed should enqueue");
    let running = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-running")
        .expect("running command should claim")
        .expect("running command should exist");

    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &completed_command)
        .expect("completed seed should enqueue");
    let mut completed = adapter
        .try_claim_next_runtime_dispatch_command(&workspace_dir, "owner-completed")
        .expect("completed command should claim")
        .expect("completed command should exist");
    completed.mark_completed("already launched workers", "2026-05-08T00:00:03+00:00");
    adapter
        .update_runtime_dispatch_command(&workspace_dir, &completed)
        .expect("completed command should persist");

    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &pending_command)
        .expect("pending seed should enqueue");

    let canceled = adapter
        .cancel_runtime_dispatch_commands(&workspace_dir, "parallel mode disabled")
        .expect("non-terminal commands should cancel");
    assert_eq!(canceled, 2);

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(
        snapshot
            .dispatch_commands
            .iter()
            .find(|command| command.command_id == running.command_id)
            .map(|command| command.state),
        Some(ParallelModeDispatchCommandState::Canceled)
    );
    assert_eq!(
        snapshot
            .dispatch_commands
            .iter()
            .find(|command| command.command_id == completed.command_id)
            .map(|command| command.state),
        Some(ParallelModeDispatchCommandState::Completed)
    );
    assert_eq!(
        snapshot
            .dispatch_commands
            .iter()
            .find(|command| command.command_id == pending_command.command_id)
            .map(|command| command.state),
        Some(ParallelModeDispatchCommandState::Canceled)
    );
}

#[test]
fn runtime_task_cleanup_removes_deleted_task_projections_only() {
    let workspace_dir = temp_workspace("runtime-task-cleanup");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let deleted_lease = slot_lease_for_task(
        "slot-1",
        "task-deleted",
        ParallelModeSlotLeaseState::Running,
    );
    let kept_lease =
        slot_lease_for_task("slot-2", "task-kept", ParallelModeSlotLeaseState::Running);

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &deleted_lease)
        .expect("deleted task slot lease should persist");
    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &kept_lease)
        .expect("kept task slot lease should persist");
    insert_invalid_slot_marker(&workspace_dir, "slot-1");
    insert_invalid_slot_marker(&workspace_dir, "slot-2");
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &failed_start_session_detail(
                "session-deleted",
                "task-deleted",
                "2026-05-04T12:00:00+00:00",
            ),
        )
        .expect("deleted task session should persist");
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &failed_start_session_detail("session-kept", "task-kept", "2026-05-04T12:01:00+00:00"),
        )
        .expect("kept task session should persist");
    adapter
        .upsert_runtime_task_dispatch_block(
            &workspace_dir,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-deleted",
                "2026-05-04T11:55:00+00:00",
                "2026-05-04T12:00:00+00:00",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        )
        .expect("deleted task dispatch block should persist");
    adapter
        .upsert_runtime_task_dispatch_block(
            &workspace_dir,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-kept",
                "2026-05-04T11:56:00+00:00",
                "2026-05-04T12:01:00+00:00",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        )
        .expect("kept task dispatch block should persist");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-deleted", "session-deleted", "task-deleted"),
        )
        .expect("deleted task queue record should persist");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-kept", "session-kept", "task-kept"),
        )
        .expect("kept task queue record should persist");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-deleted", "owner")
            .expect("queue claim should be acquired")
    );

    adapter
        .clear_parallel_runtime_projections_for_tasks(
            &workspace_dir,
            &["task-deleted".to_string()],
            "test task delete",
        )
        .expect("deleted task runtime projections should clear");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(
        snapshot
            .slot_leases
            .values()
            .map(|lease| lease.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-kept"]
    );
    assert!(!snapshot.invalid_slot_leases.contains("slot-1"));
    assert!(snapshot.invalid_slot_leases.contains("slot-2"));
    assert_eq!(snapshot.session_details.len(), 1);
    assert_eq!(snapshot.session_details[0].task_id, "task-kept");
    assert_eq!(snapshot.task_dispatch_blocks.len(), 1);
    assert_eq!(snapshot.task_dispatch_blocks[0].task_id, "task-kept");
    assert_eq!(snapshot.distributor_queue_records.len(), 1);
    assert_eq!(snapshot.distributor_queue_records[0].task_id, "task-kept");
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-deleted", "owner-2")
            .expect("deleted queue must not recreate an orphan claim")
    );
}

#[test]
fn runtime_task_cleanup_reports_malformed_json_in_lookup_rows() {
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let slot_workspace = temp_workspace("runtime-task-cleanup-bad-slot");
    authority_connection(&slot_workspace)
        .execute(
            "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
             VALUES (?1, ?2, ?3)",
            ("slot-bad", "2026-05-04T10:00:00+00:00", "{bad-json"),
        )
        .expect("malformed slot cleanup row should insert");
    let slot_error = adapter
        .clear_parallel_runtime_projections_for_tasks(
            &slot_workspace,
            &["task-bad".to_string()],
            "cleanup malformed slot",
        )
        .expect_err("malformed slot JSON should fail task cleanup");
    let slot_message = format!("{slot_error:?}");
    assert!(
        slot_message.contains("failed to iterate runtime slot ids for `task-bad`")
            || slot_message.contains("failed to decode runtime slot id for `task-bad`")
            || slot_message.contains("failed to clear runtime slot leases for `task-bad`"),
        "{slot_message}"
    );

    let queue_workspace = temp_workspace("runtime-task-cleanup-bad-queue");
    authority_connection(&queue_workspace)
        .execute(
            "INSERT INTO runtime_distributor_queue
                (queue_item_id, session_key, queue_state, enqueued_at, updated_at, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "queue-bad",
                "session-bad",
                ParallelModeQueueItemState::Queued.label(),
                "2026-05-04T10:00:00+00:00",
                "2026-05-04T10:01:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed queue cleanup row should insert");
    let queue_error = adapter
        .clear_parallel_runtime_projections_for_tasks(
            &queue_workspace,
            &["task-bad".to_string()],
            "cleanup malformed queue",
        )
        .expect_err("malformed queue JSON should fail task cleanup");
    let queue_message = format!("{queue_error:?}");
    assert!(
        queue_message.contains("failed to iterate runtime distributor queue ids for `task-bad`")
            || queue_message
                .contains("failed to decode runtime distributor queue id for `task-bad`")
            || queue_message
                .contains("failed to clear runtime distributor queue records for `task-bad`"),
        "{queue_message}"
    );
}

#[test]
fn runtime_projection_snapshot_groups_current_rows_and_recent_events() {
    let workspace_dir = temp_workspace("runtime-projection-matrix");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let lease = slot_lease_for_task("slot-1", "task-1", ParallelModeSlotLeaseState::Running);
    let session = failed_start_session_detail("session-1", "task-1", "2026-05-04T12:00:00+00:00");
    let block = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-1",
        "2026-05-04T11:55:00+00:00",
        "2026-05-04T12:00:00+00:00",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );
    let queue_record = queue_record_for_task("queue-1", "session-1", "task-1");
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("task-1:ready".to_string()),
        Some(31),
        "2026-05-08T00:00:00+00:00",
    );

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &lease)
        .expect("slot lease should persist");
    adapter
        .upsert_runtime_session_detail(&workspace_dir, &session)
        .expect("session detail should persist");
    adapter
        .upsert_runtime_task_dispatch_block(&workspace_dir, &block)
        .expect("task dispatch block should persist");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace_dir, &queue_record)
        .expect("distributor queue record should persist");
    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &command)
        .expect("dispatch command should persist");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(snapshot.slot_leases.get("slot-1"), Some(&lease));
    assert_eq!(snapshot.session_details, vec![session]);
    assert_eq!(snapshot.task_dispatch_blocks, vec![block]);
    assert_eq!(snapshot.distributor_queue_records, vec![queue_record]);
    assert_eq!(snapshot.dispatch_commands, vec![command]);

    let event_kinds = snapshot
        .runtime_events
        .iter()
        .map(|event| event.event_kind.as_str())
        .collect::<Vec<_>>();
    assert!(event_kinds.contains(&"slot_lease_upsert"));
    assert!(event_kinds.contains(&"session_detail_upsert"));
    assert!(event_kinds.contains(&"task_dispatch_block_upsert"));
    assert!(event_kinds.contains(&"distributor_queue_upsert"));
    assert!(event_kinds.contains(&"dispatch_command_enqueued"));
}

#[test]
fn malformed_runtime_projection_rows_report_row_specific_context() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let slot_workspace = temp_workspace("runtime-bad-slot-json");
    authority_connection(&slot_workspace)
        .execute(
            "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
             VALUES (?1, ?2, ?3)",
            ("slot-bad", "2026-05-04T10:00:00+00:00", "{bad-json"),
        )
        .expect("malformed slot row should insert");
    let slot_error = adapter
        .load_runtime_projections(&slot_workspace)
        .expect_err("malformed slot row should fail projection load");
    assert!(
        format!("{slot_error:?}").contains("failed to deserialize runtime slot lease `slot-bad`")
    );

    let generation_workspace = temp_workspace("runtime-bad-slot-generation");
    let mut malformed_generation = serde_json::to_value(slot_lease(
        "slot-generation",
        ParallelModeSlotLeaseState::Leased,
    ))
    .expect("slot lease should serialize");
    malformed_generation["lease_generation"] =
        serde_json::Value::String("not-a-valid-generation".to_string());
    authority_connection(&generation_workspace)
        .execute(
            "INSERT INTO runtime_slot_leases (slot_id, updated_at, content)
             VALUES (?1, ?2, ?3)",
            (
                "slot-generation",
                "2026-05-04T10:00:00+00:00",
                serde_json::to_string(&malformed_generation)
                    .expect("malformed generation payload should serialize"),
            ),
        )
        .expect("malformed persisted generation row should insert");
    let generation_error = adapter
        .load_runtime_projections(&generation_workspace)
        .expect_err("malformed persisted generation must fail projection load");
    let generation_message = format!("{generation_error:?}");
    assert!(generation_message.contains("slot-generation"));
    assert!(generation_message.contains("64 lowercase hexadecimal"));

    let mut rejected_upsert = slot_lease(
        "slot-rejected-generation",
        ParallelModeSlotLeaseState::Leased,
    );
    rejected_upsert.lease_generation = Some("short".to_string());
    let upsert_error = adapter
        .upsert_runtime_slot_lease(&generation_workspace, &rejected_upsert)
        .expect_err("adapter must reject malformed generations before persistence");
    assert!(format!("{upsert_error:?}").contains("invalid lease generation"));

    let session_workspace = temp_workspace("runtime-bad-session-json");
    authority_connection(&session_workspace)
        .execute(
            "INSERT INTO runtime_session_details (session_key, slot_id, updated_at, content)
             VALUES (?1, ?2, ?3, ?4)",
            (
                "session-bad",
                "slot-1",
                "2026-05-04T10:00:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed session row should insert");
    let session_error = adapter
        .load_runtime_projections(&session_workspace)
        .expect_err("malformed session row should fail projection load");
    assert!(
        format!("{session_error:?}")
            .contains("failed to deserialize runtime session detail `session-bad`")
    );

    let block_workspace = temp_workspace("runtime-bad-block-json");
    authority_connection(&block_workspace)
        .execute(
            "INSERT INTO runtime_task_dispatch_blocks
                (task_id, reason, task_updated_at, blocked_at, content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                "task-bad",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges.label(),
                "2026-05-04T09:59:00+00:00",
                "2026-05-04T10:00:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed dispatch block row should insert");
    let block_error = adapter
        .load_runtime_projections(&block_workspace)
        .expect_err("malformed dispatch block row should fail projection load");
    assert!(
        format!("{block_error:?}")
            .contains("failed to deserialize runtime task dispatch block `task-bad`")
    );

    let queue_workspace = temp_workspace("runtime-bad-queue-json");
    authority_connection(&queue_workspace)
        .execute(
            "INSERT INTO runtime_distributor_queue
                (queue_item_id, session_key, queue_state, enqueued_at, updated_at, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                "queue-bad",
                "session-bad",
                ParallelModeQueueItemState::Queued.label(),
                "2026-05-04T10:00:00+00:00",
                "2026-05-04T10:01:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed queue row should insert");
    let queue_error = adapter
        .load_runtime_projections(&queue_workspace)
        .expect_err("malformed queue row should fail projection load");
    assert!(
        format!("{queue_error:?}")
            .contains("failed to deserialize runtime distributor queue record `queue-bad`")
    );

    let dispatch_workspace = temp_workspace("runtime-bad-dispatch-json");
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some("task-bad:ready".to_string()),
        Some(51),
        "2026-05-08T00:00:00+00:00",
    );
    adapter
        .enqueue_runtime_dispatch_command(&dispatch_workspace, &command)
        .expect("dispatch command should enqueue before corruption");
    authority_connection(&dispatch_workspace)
        .execute(
            "UPDATE runtime_dispatch_commands SET content = ?1 WHERE command_id = ?2",
            ("{bad-json", command.command_id.as_str()),
        )
        .expect("dispatch command content should corrupt");
    let claim_error = adapter
        .try_claim_next_runtime_dispatch_command(&dispatch_workspace, "owner")
        .expect_err("malformed dispatch command should fail claim");
    assert!(format!("{claim_error:?}").contains(&format!(
        "failed to deserialize runtime dispatch command `{}`",
        command.command_id
    )));

    let load_error = adapter
        .load_runtime_projections(&dispatch_workspace)
        .expect_err("malformed dispatch command should fail projection load");
    assert!(format!("{load_error:?}").contains(&format!(
        "failed to deserialize runtime dispatch command `{}`",
        command.command_id
    )));
}

#[test]
fn runtime_reset_reports_malformed_failed_start_session_before_clearing_rows() {
    let workspace_dir = temp_workspace("runtime-reset-bad-session-json");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    authority_connection(&workspace_dir)
        .execute(
            "INSERT INTO runtime_session_details (session_key, slot_id, updated_at, content)
             VALUES (?1, ?2, ?3, ?4)",
            (
                "session-bad",
                "slot-1",
                "2026-05-04T10:00:00+00:00",
                "{bad-json",
            ),
        )
        .expect("malformed session row should insert before reset");

    let error = adapter
        .clear_parallel_runtime_projections(&workspace_dir, "reset malformed session")
        .expect_err("malformed failed-start preservation row should fail reset");

    assert!(
        format!("{error:?}")
            .contains("failed to deserialize session detail `session-bad` before reset")
    );
}

#[test]
fn runtime_recoverable_projection_survives_adapter_restart_boundary() {
    let workspace_dir = temp_workspace("runtime-restart-boundary");
    let writer = SqlitePlanningAuthorityAdapter::new();
    let lease = slot_lease_for_task(
        "slot-1",
        "task-recoverable",
        ParallelModeSlotLeaseState::Running,
    );
    let mut session = failed_start_session_detail(
        "session-recoverable",
        "task-recoverable",
        "2026-05-04T12:00:00+00:00",
    );
    session.thread_id = Some("thread-recoverable".to_string());
    let block = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-recoverable",
        "2026-05-04T11:55:00+00:00",
        "2026-05-04T12:00:00+00:00",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );
    let queue_record = queue_record_for_task(
        "queue-recoverable",
        "session-recoverable",
        "task-recoverable",
    );
    let command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::ParallelOfficialCompletion,
        Some("task-recoverable:ready".to_string()),
        Some(41),
        "2026-05-08T00:00:00+00:00",
    );

    writer
        .upsert_runtime_slot_lease(&workspace_dir, &lease)
        .expect("slot lease should persist");
    writer
        .upsert_runtime_session_detail(&workspace_dir, &session)
        .expect("session detail should persist");
    writer
        .upsert_runtime_task_dispatch_block(&workspace_dir, &block)
        .expect("task dispatch block should persist");
    writer
        .upsert_runtime_distributor_queue_record(&workspace_dir, &queue_record)
        .expect("distributor queue should persist");
    assert!(
        writer
            .enqueue_runtime_dispatch_command(&workspace_dir, &command)
            .expect("dispatch command should persist")
    );
    assert!(
        writer
            .try_acquire_distributor_queue_claim(
                &workspace_dir,
                "queue-recoverable",
                "owner-before-restart",
            )
            .expect("queue claim should acquire")
    );
    assert_eq!(
        writer
            .reserve_next_official_refresh_order(&workspace_dir)
            .expect("first refresh order should reserve"),
        1
    );

    let restarted = SqlitePlanningAuthorityAdapter::new();
    let snapshot = restarted
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should survive a new adapter handle");

    assert_eq!(snapshot.slot_leases.get("slot-1"), Some(&lease));
    assert_eq!(snapshot.session_details, vec![session]);
    assert_eq!(snapshot.task_dispatch_blocks, vec![block]);
    assert_eq!(snapshot.distributor_queue_records, vec![queue_record]);
    assert_eq!(snapshot.dispatch_commands, vec![command]);
    assert!(
        !restarted
            .try_acquire_distributor_queue_claim(
                &workspace_dir,
                "queue-recoverable",
                "owner-after-restart",
            )
            .expect("persisted queue claim should block a competing owner")
    );
    assert_eq!(
        restarted
            .reserve_next_official_refresh_order(&workspace_dir)
            .expect("official refresh order should continue after restart"),
        2
    );
}

#[test]
fn official_refresh_claim_orders_are_enforced_by_authority_store() {
    let workspace_dir = temp_workspace("official-refresh-claims");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let first_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("first order should reserve");
    let second_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("second order should reserve");
    assert_eq!(first_order, 1);
    assert_eq!(second_order, 2);

    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, second_order, "owner-2")
            .expect("later order should inspect"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "owner-1")
            .expect("head order should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "owner-other")
            .expect("competing owner should wait"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );

    adapter
        .release_official_refresh_claim(&workspace_dir, first_order, "owner-1")
        .expect("first order should release");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "owner-1")
            .expect("completed order should inspect"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, second_order, "owner-2")
            .expect("next order should acquire after first release"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn official_refresh_claim_cancel_preserves_order_and_requires_exact_owner() {
    let workspace_dir = temp_workspace("official-refresh-claim-cancel");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let first_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("first order should reserve");
    let second_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("second order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "first-owner")
            .expect("first order should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );

    adapter
        .cancel_official_refresh_claim(&workspace_dir, second_order, "first-owner")
        .expect("wrong-order cancellation should be an idempotent no-op");
    adapter
        .cancel_official_refresh_claim(&workspace_dir, first_order, "wrong-owner")
        .expect("wrong-owner cancellation should be an idempotent no-op");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "replacement-owner")
            .expect("the original owner should still fence the order"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );

    adapter
        .cancel_official_refresh_claim(&workspace_dir, first_order, "first-owner")
        .expect("exact owner and order should cancel");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, first_order, "replacement-owner")
            .expect("canceled order should be reacquirable"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, second_order, "second-owner")
            .expect("later order should still wait"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );
    adapter
        .release_official_refresh_claim(&workspace_dir, first_order, "replacement-owner")
        .expect("reacquired order should complete");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, second_order, "second-owner")
            .expect("later order should run only after successful completion"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn official_refresh_claim_heartbeat_and_live_process_fence_prevent_stale_takeover() {
    let workspace_dir = temp_workspace("official-refresh-live-owner");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let refresh_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("refresh order should reserve");
    let owner = format!(
        "official-refresh-{}-{refresh_order}-live",
        std::process::id()
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, &owner)
            .expect("live owner should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );

    set_claim_timestamp(
        &workspace_dir,
        OFFICIAL_REFRESH_CLAIM_KIND,
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "competing-owner")
            .expect("live stale owner should remain fenced"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );
    assert!(
        adapter
            .renew_official_refresh_claim(&workspace_dir, refresh_order, &owner)
            .expect("exact owner should renew")
    );
    assert!(
        !adapter
            .renew_official_refresh_claim(&workspace_dir, refresh_order, "wrong-owner")
            .expect("wrong owner renewal should be rejected")
    );
}

#[test]
fn official_refresh_stale_claim_is_reclaimed_after_owner_process_exits() {
    #[cfg(unix)]
    let mut child = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .expect("short-lived Unix child should spawn");
    #[cfg(windows)]
    let mut child = std::process::Command::new("cmd")
        .args(["/C", "exit", "0"])
        .spawn()
        .expect("short-lived Windows child should spawn");
    let dead_pid = child.id();
    child.wait().expect("short-lived child should exit");

    let workspace_dir = temp_workspace("official-refresh-dead-owner");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let refresh_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("refresh order should reserve");
    let owner = format!("official-refresh-{dead_pid}-{refresh_order}-dead");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, &owner)
            .expect("initial owner should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    set_claim_timestamp(
        &workspace_dir,
        OFFICIAL_REFRESH_CLAIM_KIND,
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "replacement-owner")
            .expect("dead stale owner should be reclaimed"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
#[test]
fn official_refresh_stale_claim_is_reclaimed_after_pid_reuse() {
    let pid = std::process::id();
    let current_start_identity = crate::process_liveness::process_start_identity(pid)
        .expect("current process identity probe should succeed")
        .expect("supported OS should expose process start identity");
    let stale_start_identity = format!("{current_start_identity}-previous-lifetime");
    let workspace_dir = temp_workspace("official-refresh-reused-pid-owner");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let refresh_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("refresh order should reserve");
    let owner = format!(
        "official-refresh-{pid}-{refresh_order}-crashed-process-start:{stale_start_identity}"
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, &owner)
            .expect("old process lifetime should initially acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    set_claim_timestamp(
        &workspace_dir,
        OFFICIAL_REFRESH_CLAIM_KIND,
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );

    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "replacement-owner")
            .expect("same PID with a different start identity should be reclaimed"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn runtime_claim_release_and_stale_timestamp_edges_respect_claim_ownership() {
    let workspace_dir = temp_workspace("runtime-claim-release-edges");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-claim", "session-claim", "task-claim"),
        )
        .expect("claimable queue record should persist");

    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-owner")
            .expect("queue claim should acquire")
    );
    adapter
        .release_distributor_queue_claim(&workspace_dir, "queue-claim", "wrong-owner")
        .expect("wrong queue owner release should be harmless");
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-other")
            .expect("wrong release should not clear queue claim")
    );
    adapter
        .release_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-owner")
        .expect("matching queue owner should release");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-other")
            .expect("matching release should clear queue claim")
    );
    set_claim_timestamp(
        &workspace_dir,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-claim",
        "not-a-timestamp",
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-claim", "queue-reclaimer")
            .expect("invalid queue claim timestamp should be reclaimed")
    );

    let refresh_order = adapter
        .reserve_next_official_refresh_order(&workspace_dir)
        .expect("official refresh order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
            .expect("official refresh claim should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    adapter
        .release_official_refresh_claim(&workspace_dir, refresh_order, "wrong-refresh-owner")
        .expect("wrong official owner release should be harmless");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-other")
            .expect("wrong release should not clear official claim"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );
    set_authority_metadata(
        &workspace_dir,
        "next_executable_refresh_order",
        &(refresh_order + 5).to_string(),
    );
    adapter
        .release_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
        .expect("matching official owner should release without moving advanced pointer back");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&workspace_dir, refresh_order, "refresh-owner")
            .expect("advanced pointer should still mark old order completed"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
}

#[test]
fn distributor_queue_claim_renewal_is_owner_bound_after_stale_replacement() {
    let workspace_dir = temp_workspace("runtime-claim-renewal-owner");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-renew", "session-renew", "task-renew"),
        )
        .expect("renewable queue record should persist");

    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-renew", "owner-initial")
            .expect("initial queue claim should acquire")
    );
    set_claim_timestamp(
        &workspace_dir,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-renew",
        "2999-01-01T00:00:00+00:00",
    );
    assert!(
        adapter
            .renew_distributor_queue_claim(&workspace_dir, "queue-renew", "owner-initial")
            .expect("matching owner should renew its queue claim")
    );
    assert_ne!(
        runtime_claim_owner_and_timestamp(
            &workspace_dir,
            DISTRIBUTOR_QUEUE_CLAIM_KIND,
            "queue-renew",
        )
        .1,
        "2999-01-01T00:00:00+00:00"
    );

    set_claim_timestamp(
        &workspace_dir,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-renew",
        "2999-01-01T00:00:00+00:00",
    );
    assert!(
        !adapter
            .renew_distributor_queue_claim(&workspace_dir, "queue-renew", "owner-wrong")
            .expect("wrong owner renewal should be rejected")
    );
    assert_eq!(
        runtime_claim_owner_and_timestamp(
            &workspace_dir,
            DISTRIBUTOR_QUEUE_CLAIM_KIND,
            "queue-renew",
        ),
        (
            "owner-initial".to_string(),
            "2999-01-01T00:00:00+00:00".to_string(),
        )
    );

    set_claim_timestamp(
        &workspace_dir,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-renew",
        "2000-01-01T00:00:00+00:00",
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-renew", "owner-replacement")
            .expect("replacement owner should reclaim the stale queue claim")
    );
    let replacement_claim = runtime_claim_owner_and_timestamp(
        &workspace_dir,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-renew",
    );
    assert_eq!(replacement_claim.0, "owner-replacement");
    assert!(
        !adapter
            .renew_distributor_queue_claim(&workspace_dir, "queue-renew", "owner-initial")
            .expect("replaced owner renewal should be rejected")
    );
    assert_eq!(
        runtime_claim_owner_and_timestamp(
            &workspace_dir,
            DISTRIBUTOR_QUEUE_CLAIM_KIND,
            "queue-renew",
        ),
        replacement_claim
    );
    assert!(
        adapter
            .renew_distributor_queue_claim(&workspace_dir, "queue-renew", "owner-replacement")
            .expect("replacement owner should retain its queue claim")
    );
}

#[test]
fn official_refresh_recovery_handles_reentry_active_and_stale_claim_edges() {
    let idle_workspace = temp_workspace("official-refresh-recovery-idle");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    assert_eq!(
        adapter
            .abandon_next_official_refresh_order(&idle_workspace, "nothing pending")
            .expect("idle recovery should inspect"),
        PlanningAuthorityOfficialRefreshRecoveryStatus::NoPendingOrder
    );

    let first_order = adapter
        .reserve_next_official_refresh_order(&idle_workspace)
        .expect("first order should reserve");
    let second_order = adapter
        .reserve_next_official_refresh_order(&idle_workspace)
        .expect("second order should reserve");
    assert_eq!(
        adapter
            .abandon_next_official_refresh_order(&idle_workspace, "head worker exited")
            .expect("head order should recover"),
        PlanningAuthorityOfficialRefreshRecoveryStatus::Recovered {
            refresh_order: first_order,
        }
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&idle_workspace, first_order, "owner-1")
            .expect("recovered order should inspect as completed"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&idle_workspace, second_order, "owner-2")
            .expect("second order should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&idle_workspace, second_order, "owner-2")
            .expect("same owner should reenter"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );

    let active_workspace = temp_workspace("official-refresh-recovery-active");
    let active_order = adapter
        .reserve_next_official_refresh_order(&active_workspace)
        .expect("active order should reserve");
    adapter
        .reserve_next_official_refresh_order(&active_workspace)
        .expect("later active order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&active_workspace, active_order, "active-owner")
            .expect("active owner should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    assert_eq!(
        adapter
            .abandon_next_official_refresh_order(&active_workspace, "operator retry")
            .expect("active claim should block recovery"),
        PlanningAuthorityOfficialRefreshRecoveryStatus::WaitingForActiveClaim
    );

    let stale_workspace = temp_workspace("official-refresh-recovery-stale");
    let stale_order = adapter
        .reserve_next_official_refresh_order(&stale_workspace)
        .expect("stale order should reserve");
    adapter
        .reserve_next_official_refresh_order(&stale_workspace)
        .expect("later stale order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&stale_workspace, stale_order, "stale-owner")
            .expect("stale owner should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    set_claim_timestamp(
        &stale_workspace,
        "official-refresh",
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&stale_workspace, stale_order, "replacement-owner")
            .expect("stale claim should be reclaimed by acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    set_claim_timestamp(
        &stale_workspace,
        "official-refresh",
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );
    assert_eq!(
        adapter
            .abandon_next_official_refresh_order(&stale_workspace, "stale head")
            .expect("stale claim should not block recovery"),
        PlanningAuthorityOfficialRefreshRecoveryStatus::Recovered {
            refresh_order: stale_order,
        }
    );
}

#[test]
fn stale_distributor_claims_invalid_slots_pool_reset_and_zero_limit_events_are_projected() {
    let workspace_dir = temp_workspace("runtime-projection-edges");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let lease = slot_lease_for_task(
        "slot-reset",
        "task-reset",
        ParallelModeSlotLeaseState::Running,
    );
    let session =
        failed_start_session_detail("session-reset", "task-reset", "2026-05-04T12:00:00+00:00");
    let queue_record = queue_record_for_task("queue-reset", "session-reset", "task-reset");

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &lease)
        .expect("reset slot lease should persist");
    insert_invalid_slot_marker(&workspace_dir, "slot-reset");
    adapter
        .upsert_runtime_session_detail(&workspace_dir, &session)
        .expect("reset session should persist");
    adapter
        .upsert_runtime_distributor_queue_record(&workspace_dir, &queue_record)
        .expect("reset queue record should persist");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "owner-old")
            .expect("queue claim should acquire")
    );
    set_claim_timestamp(
        &workspace_dir,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-reset",
        "2000-01-01T00:00:00+00:00",
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "owner-new")
            .expect("stale queue claim should be reclaimed")
    );

    let snapshot_with_invalid = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load invalid marker");
    assert!(
        snapshot_with_invalid
            .invalid_slot_leases
            .contains("slot-reset")
    );

    let zero_limit_events = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", "slot-reset", 0),
        )
        .expect("zero-limit event request should load");
    assert_eq!(zero_limit_events.total_event_count, 1);
    assert_eq!(zero_limit_events.visible_count(), 0);
    assert_eq!(
        zero_limit_events.empty_state,
        "runtime events hidden by request limit"
    );

    let mut report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("reset-run-1"),
        ParallelModePoolResetPolicy::ForceDisposable,
    );
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-reset",
            ParallelModePoolResetSlotAction::Reset,
            ParallelModePoolResetSlotOutcome::Succeeded,
            "reset completed",
        ));
    report.reset_session_keys.push("session-reset".to_string());
    report.reset_queue_item_ids.push("queue-reset".to_string());
    adapter
        .apply_parallel_pool_reset_report(&workspace_dir, &report)
        .expect("pool reset report should apply");

    let reset_snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after reset");
    assert!(reset_snapshot.slot_leases.is_empty());
    assert!(reset_snapshot.invalid_slot_leases.is_empty());
    assert!(reset_snapshot.session_details.is_empty());
    assert!(reset_snapshot.distributor_queue_records.is_empty());
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-reset", "owner-after-reset")
            .expect("reset queue must not recreate an orphan claim")
    );

    adapter
        .clear_parallel_runtime_projections_for_tasks(
            &workspace_dir,
            &[" ".to_string(), String::new()],
            "blank task cleanup",
        )
        .expect("blank task cleanup should be a no-op");
}

#[test]
fn pool_reset_report_clears_only_successful_slots_and_reads_unfiltered_events() {
    let workspace_dir = temp_workspace("runtime-reset-report-mixed");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let success_lease = slot_lease_for_task(
        "slot-success",
        "task-success",
        ParallelModeSlotLeaseState::Running,
    );
    let blocked_lease = slot_lease_for_task(
        "slot-blocked",
        "task-blocked",
        ParallelModeSlotLeaseState::Running,
    );
    let failed_lease = slot_lease_for_task(
        "slot-failed",
        "task-failed",
        ParallelModeSlotLeaseState::Running,
    );

    for lease in [&success_lease, &blocked_lease, &failed_lease] {
        adapter
            .upsert_runtime_slot_lease(&workspace_dir, lease)
            .expect("slot lease should persist before mixed reset");
        insert_invalid_slot_marker(&workspace_dir, &lease.slot_id);
    }
    adapter
        .upsert_runtime_session_detail(
            &workspace_dir,
            &running_session_detail(
                "session-success",
                "task-success",
                "2026-05-04T12:00:00+00:00",
            ),
        )
        .expect("reset session should persist");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-success", "session-success", "task-success"),
        )
        .expect("reset queue should persist");
    adapter
        .upsert_runtime_distributor_queue_record(
            &workspace_dir,
            &queue_record_for_task("queue-blocked", "session-blocked", "task-blocked"),
        )
        .expect("blocked queue should persist");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-success", "owner-success")
            .expect("success queue claim should acquire")
    );
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-blocked", "owner-blocked")
            .expect("blocked queue claim should acquire")
    );

    let pending_command = ParallelModeDispatchCommandSnapshot::dispatch_ready_queue(
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        Some("queue-head-success".to_string()),
        Some(31),
        "2026-05-08T00:00:02+00:00",
    );
    adapter
        .enqueue_runtime_dispatch_command(&workspace_dir, &pending_command)
        .expect("pending dispatch command should persist before mixed reset");
    let mut report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("mixed-reset-run"),
        ParallelModePoolResetPolicy::ProtectLive,
    );
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-success",
            ParallelModePoolResetSlotAction::Reset,
            ParallelModePoolResetSlotOutcome::Succeeded,
            "reset completed",
        ));
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-blocked",
            ParallelModePoolResetSlotAction::PreserveLive,
            ParallelModePoolResetSlotOutcome::Blocked,
            "live turn running",
        ));
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-failed",
            ParallelModePoolResetSlotAction::Reset,
            ParallelModePoolResetSlotOutcome::Failed,
            "worktree reset failed",
        ));
    report
        .slot_reports
        .push(ParallelModePoolResetSlotReport::new(
            "slot-missing",
            ParallelModePoolResetSlotAction::SkipMissing,
            ParallelModePoolResetSlotOutcome::Skipped,
            "slot missing",
        ));
    report
        .reset_session_keys
        .push("session-success".to_string());
    report
        .reset_queue_item_ids
        .push("queue-success".to_string());
    report
        .reset_dispatch_command_ids
        .push(pending_command.command_id.clone());

    adapter
        .apply_parallel_pool_reset_report(&workspace_dir, &report)
        .expect("mixed pool reset report should apply");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after mixed reset");
    assert!(!snapshot.slot_leases.contains_key("slot-success"));
    assert!(snapshot.slot_leases.contains_key("slot-blocked"));
    assert!(snapshot.slot_leases.contains_key("slot-failed"));
    assert!(!snapshot.invalid_slot_leases.contains("slot-success"));
    assert!(snapshot.invalid_slot_leases.contains("slot-blocked"));
    assert!(snapshot.invalid_slot_leases.contains("slot-failed"));
    assert!(snapshot.session_details.is_empty());
    assert_eq!(snapshot.distributor_queue_records.len(), 1);
    assert!(
        snapshot
            .dispatch_commands
            .iter()
            .all(|command| command.command_id != pending_command.command_id)
    );
    assert_eq!(
        snapshot.distributor_queue_records[0].queue_item_id,
        "queue-blocked"
    );
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(
                &workspace_dir,
                "queue-success",
                "owner-success-after-reset",
            )
            .expect("removed success queue must not recreate an orphan claim")
    );
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(
                &workspace_dir,
                "queue-blocked",
                "owner-blocked-after-reset",
            )
            .expect("blocked queue claim should remain")
    );

    let events = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::recent(50),
        )
        .expect("unfiltered runtime event log should load");
    assert!(events.total_event_count >= snapshot.runtime_events.len());
    assert!(events.entries.iter().any(|event| {
        event.event_kind == "parallel_pool_reset_report_applied"
            && event.summary.contains("live_blockers: 1")
            && event.summary.contains("failures: 1")
    }));
}

#[test]
fn runtime_task_cleanup_trims_deduplicates_and_clears_multiple_tasks() {
    let workspace_dir = temp_workspace("runtime-task-cleanup-multi");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    for (slot_id, task_id, session_key, queue_item_id) in [
        ("slot-a", "task-a", "session-a", "queue-a"),
        ("slot-b", "task-b", "session-b", "queue-b"),
        ("slot-c", "task-c", "session-c", "queue-c"),
    ] {
        adapter
            .upsert_runtime_slot_lease(
                &workspace_dir,
                &slot_lease_for_task(slot_id, task_id, ParallelModeSlotLeaseState::Running),
            )
            .expect("task cleanup slot lease should persist");
        insert_invalid_slot_marker(&workspace_dir, slot_id);
        adapter
            .upsert_runtime_session_detail(
                &workspace_dir,
                &failed_start_session_detail(session_key, task_id, "2026-05-04T12:00:00+00:00"),
            )
            .expect("task cleanup session should persist");
        adapter
            .upsert_runtime_task_dispatch_block(
                &workspace_dir,
                &ParallelModeTaskDispatchBlockSnapshot::new(
                    task_id,
                    "2026-05-04T11:55:00+00:00",
                    "2026-05-04T12:00:00+00:00",
                    ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
                ),
            )
            .expect("task cleanup dispatch block should persist");
        adapter
            .upsert_runtime_distributor_queue_record(
                &workspace_dir,
                &queue_record_for_task(queue_item_id, session_key, task_id),
            )
            .expect("task cleanup queue should persist");
        assert!(
            adapter
                .try_acquire_distributor_queue_claim(&workspace_dir, queue_item_id, "owner")
                .expect("task cleanup queue claim should acquire")
        );
    }

    adapter
        .clear_parallel_runtime_projections_for_tasks(
            &workspace_dir,
            &[
                " task-b ".to_string(),
                "task-a".to_string(),
                "task-a".to_string(),
                String::new(),
            ],
            "multi task cleanup",
        )
        .expect("multi-task runtime cleanup should apply");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after multi-task cleanup");
    assert_eq!(
        snapshot
            .slot_leases
            .values()
            .map(|lease| lease.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-c"]
    );
    assert_eq!(
        snapshot
            .session_details
            .iter()
            .map(|detail| detail.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-c"]
    );
    assert_eq!(
        snapshot
            .task_dispatch_blocks
            .iter()
            .map(|block| block.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-c"]
    );
    assert_eq!(
        snapshot
            .distributor_queue_records
            .iter()
            .map(|record| record.task_id.as_str())
            .collect::<Vec<_>>(),
        vec!["task-c"]
    );
    assert!(!snapshot.invalid_slot_leases.contains("slot-a"));
    assert!(!snapshot.invalid_slot_leases.contains("slot-b"));
    assert!(snapshot.invalid_slot_leases.contains("slot-c"));
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-a", "owner-after")
            .expect("removed task-a queue must not recreate a claim")
    );
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-b", "owner-after")
            .expect("removed task-b queue must not recreate a claim")
    );
    assert!(
        !adapter
            .try_acquire_distributor_queue_claim(&workspace_dir, "queue-c", "owner-after")
            .expect("task-c queue claim should remain")
    );
    assert!(snapshot.runtime_events.iter().any(|event| {
        event.event_kind == "parallel_runtime_task_cleanup"
            && event.summary.contains("tasks: 2")
            && event.summary.contains("claims: 2")
    }));
}

#[test]
fn runtime_projection_write_error_contexts_report_broken_tables() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let enqueue_workspace = temp_workspace("runtime-write-error-dispatch-enqueue");
    replace_runtime_table_schema(
        &enqueue_workspace,
        "runtime_dispatch_commands",
        "command_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter
            .enqueue_runtime_dispatch_command(&enqueue_workspace, &dispatch_command_snapshot(801)),
        "failed to enqueue runtime dispatch command",
    );

    let revive_workspace = temp_workspace("runtime-write-error-dispatch-revive");
    let revive_command = dispatch_command_snapshot(802);
    adapter
        .enqueue_runtime_dispatch_command(&revive_workspace, &revive_command)
        .expect("revive seed should enqueue");
    let mut blocked = adapter
        .try_claim_next_runtime_dispatch_command(&revive_workspace, "owner-revive")
        .expect("revive seed should claim")
        .expect("revive seed should exist");
    blocked.mark_blocked("waiting for retry", "2026-05-08T00:00:10+00:00");
    adapter
        .update_runtime_dispatch_command(&revive_workspace, &blocked)
        .expect("revive seed should become terminal");
    authority_connection(&revive_workspace)
        .execute_batch(
            "CREATE TRIGGER fail_dispatch_reenqueue_update
             BEFORE UPDATE ON runtime_dispatch_commands
             BEGIN
                 SELECT RAISE(FAIL, 'forced dispatch update failure');
             END;",
        )
        .expect("dispatch update trigger should install");
    assert_error_contains(
        adapter.enqueue_runtime_dispatch_command(&revive_workspace, &revive_command),
        "failed to revive terminal runtime dispatch command",
    );

    let claim_workspace = temp_workspace("runtime-write-error-dispatch-claim");
    replace_runtime_table_schema(
        &claim_workspace,
        "runtime_dispatch_commands",
        "command_id TEXT PRIMARY KEY, command_state TEXT NOT NULL, created_at TEXT NOT NULL, content TEXT NOT NULL",
    );
    insert_pending_dispatch_command_row(&claim_workspace, &dispatch_command_snapshot(803));
    assert_error_contains(
        adapter.try_claim_next_runtime_dispatch_command(&claim_workspace, "owner-claim"),
        "failed to claim runtime dispatch command",
    );

    let update_workspace = temp_workspace("runtime-write-error-dispatch-update");
    replace_runtime_table_schema(
        &update_workspace,
        "runtime_dispatch_commands",
        "command_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.update_runtime_dispatch_command(&update_workspace, &dispatch_command_snapshot(804)),
        "failed to update runtime dispatch command",
    );

    let slot_workspace = temp_workspace("runtime-write-error-slot-invalid");
    replace_runtime_table_schema(
        &slot_workspace,
        "runtime_invalid_slot_leases",
        "broken_slot_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.upsert_runtime_slot_lease(
            &slot_workspace,
            &slot_lease_for_task(
                "slot-broken-invalid",
                "task-broken-invalid",
                ParallelModeSlotLeaseState::Running,
            ),
        ),
        "failed to clear invalid runtime slot lease `slot-broken-invalid`",
    );

    let session_workspace = temp_workspace("runtime-write-error-session");
    replace_runtime_table_schema(
        &session_workspace,
        "runtime_session_details",
        "session_key TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.upsert_runtime_session_detail(
            &session_workspace,
            &running_session_detail(
                "session-broken",
                "task-broken-session",
                "2026-05-04T12:00:00+00:00",
            ),
        ),
        "failed to persist runtime session detail `session-broken`",
    );

    let block_workspace = temp_workspace("runtime-write-error-block");
    replace_runtime_table_schema(
        &block_workspace,
        "runtime_task_dispatch_blocks",
        "task_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.upsert_runtime_task_dispatch_block(
            &block_workspace,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-broken-block",
                "2026-05-04T11:55:00+00:00",
                "2026-05-04T12:00:00+00:00",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        ),
        "failed to persist runtime task dispatch block `task-broken-block`",
    );

    let queue_workspace = temp_workspace("runtime-write-error-queue");
    replace_runtime_table_schema(
        &queue_workspace,
        "runtime_distributor_queue",
        "queue_item_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.upsert_runtime_distributor_queue_record(
            &queue_workspace,
            &queue_record_for_task("queue-broken", "session-broken", "task-broken-queue"),
        ),
        "failed to persist runtime distributor queue record `queue-broken`",
    );
}

#[test]
fn runtime_projection_event_append_errors_keep_projection_context() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let enqueue_workspace = temp_workspace("runtime-event-error-dispatch-enqueue");
    corrupt_runtime_events_schema(&enqueue_workspace);
    assert_error_contains(
        adapter
            .enqueue_runtime_dispatch_command(&enqueue_workspace, &dispatch_command_snapshot(811)),
        "failed to append runtime event `dispatch_command_enqueued`",
    );

    let claim_workspace = temp_workspace("runtime-event-error-dispatch-claim");
    adapter
        .enqueue_runtime_dispatch_command(&claim_workspace, &dispatch_command_snapshot(812))
        .expect("claim event seed should enqueue");
    corrupt_runtime_events_schema(&claim_workspace);
    assert_error_contains(
        adapter.try_claim_next_runtime_dispatch_command(&claim_workspace, "owner-event"),
        "failed to append runtime event `dispatch_command_claimed`",
    );

    let update_workspace = temp_workspace("runtime-event-error-dispatch-update");
    corrupt_runtime_events_schema(&update_workspace);
    assert_error_contains(
        adapter.update_runtime_dispatch_command(&update_workspace, &dispatch_command_snapshot(813)),
        "failed to append runtime event `dispatch_command_updated`",
    );

    let slot_workspace = temp_workspace("runtime-event-error-slot-upsert");
    corrupt_runtime_events_schema(&slot_workspace);
    assert_error_contains(
        adapter.upsert_runtime_slot_lease(
            &slot_workspace,
            &slot_lease_for_task(
                "slot-event",
                "task-event-slot",
                ParallelModeSlotLeaseState::Running,
            ),
        ),
        "failed to append runtime event `slot_lease_upsert`",
    );

    let remove_workspace = temp_workspace("runtime-event-error-slot-remove");
    adapter
        .upsert_runtime_slot_lease(
            &remove_workspace,
            &slot_lease_for_task(
                "slot-remove-event",
                "task-remove-event",
                ParallelModeSlotLeaseState::Running,
            ),
        )
        .expect("remove event seed should persist");
    corrupt_runtime_events_schema(&remove_workspace);
    assert_error_contains(
        SqlitePlanningAuthorityAdapter::remove_runtime_slot_lease(
            &remove_workspace,
            "slot-remove-event",
        ),
        "failed to append runtime event `slot_lease_removed`",
    );

    let reset_workspace = temp_workspace("runtime-event-error-reset");
    corrupt_runtime_events_schema(&reset_workspace);
    assert_error_contains(
        adapter.clear_parallel_runtime_projections(&reset_workspace, "broken event table"),
        "failed to append runtime event `parallel_runtime_reset`",
    );

    let task_cleanup_workspace = temp_workspace("runtime-event-error-task-cleanup");
    corrupt_runtime_events_schema(&task_cleanup_workspace);
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_cleanup_workspace,
            &["task-event-cleanup".to_string()],
            "broken event table",
        ),
        "failed to append runtime event `parallel_runtime_task_cleanup`",
    );

    let pool_reset_workspace = temp_workspace("runtime-event-error-pool-reset");
    corrupt_runtime_events_schema(&pool_reset_workspace);
    let report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("event-error-reset"),
        ParallelModePoolResetPolicy::ForceDisposable,
    );
    assert_error_contains(
        adapter.apply_parallel_pool_reset_report(&pool_reset_workspace, &report),
        "failed to append runtime event `parallel_pool_reset_report_applied`",
    );

    let session_workspace = temp_workspace("runtime-event-error-session");
    corrupt_runtime_events_schema(&session_workspace);
    assert_error_contains(
        adapter.upsert_runtime_session_detail(
            &session_workspace,
            &running_session_detail(
                "session-event",
                "task-event-session",
                "2026-05-04T12:00:00+00:00",
            ),
        ),
        "failed to append runtime event `session_detail_upsert`",
    );

    let block_workspace = temp_workspace("runtime-event-error-block");
    corrupt_runtime_events_schema(&block_workspace);
    assert_error_contains(
        adapter.upsert_runtime_task_dispatch_block(
            &block_workspace,
            &ParallelModeTaskDispatchBlockSnapshot::new(
                "task-event-block",
                "2026-05-04T11:55:00+00:00",
                "2026-05-04T12:00:00+00:00",
                ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
            ),
        ),
        "failed to append runtime event `task_dispatch_block_upsert`",
    );

    let queue_workspace = temp_workspace("runtime-event-error-queue");
    corrupt_runtime_events_schema(&queue_workspace);
    assert_error_contains(
        adapter.upsert_runtime_distributor_queue_record(
            &queue_workspace,
            &queue_record_for_task("queue-event", "session-event", "task-event-queue"),
        ),
        "failed to append runtime event `distributor_queue_upsert`",
    );

    let abandoned_workspace = temp_workspace("runtime-event-error-official-abandon");
    adapter
        .reserve_next_official_refresh_order(&abandoned_workspace)
        .expect("head order should reserve before abandoned event failure");
    adapter
        .reserve_next_official_refresh_order(&abandoned_workspace)
        .expect("tail order should reserve before abandoned event failure");
    corrupt_runtime_events_schema(&abandoned_workspace);
    assert_error_contains(
        adapter.abandon_next_official_refresh_order(&abandoned_workspace, "broken event table"),
        "failed to append runtime event `official_refresh_abandoned`",
    );
}

#[test]
fn runtime_projection_cleanup_error_contexts_report_sqlite_failures() {
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let reserve_workspace = temp_workspace("runtime-cleanup-error-reserve-metadata");
    authority_connection(&reserve_workspace)
        .execute_batch(
            "CREATE TRIGGER fail_next_official_order_metadata
             BEFORE INSERT ON authority_metadata
             WHEN NEW.key = 'next_official_refresh_order'
             BEGIN
                 SELECT RAISE(FAIL, 'forced metadata failure');
             END;",
        )
        .expect("next official metadata trigger should install");
    assert_error_contains(
        adapter.reserve_next_official_refresh_order(&reserve_workspace),
        "failed to update authority metadata `next_official_refresh_order`",
    );

    let release_workspace = temp_workspace("runtime-cleanup-error-release-metadata");
    let release_order = adapter
        .reserve_next_official_refresh_order(&release_workspace)
        .expect("release order should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&release_workspace, release_order, "release-owner")
            .expect("release order should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    authority_connection(&release_workspace)
        .execute_batch(
            "CREATE TRIGGER fail_next_executable_release_metadata
             BEFORE INSERT ON authority_metadata
             WHEN NEW.key = 'next_executable_refresh_order'
             BEGIN
                 SELECT RAISE(FAIL, 'forced metadata failure');
             END;",
        )
        .expect("release metadata trigger should install");
    assert_error_contains(
        adapter.release_official_refresh_claim(&release_workspace, release_order, "release-owner"),
        "failed to update authority metadata `next_executable_refresh_order`",
    );

    let abandon_workspace = temp_workspace("runtime-cleanup-error-abandon-metadata");
    adapter
        .reserve_next_official_refresh_order(&abandon_workspace)
        .expect("abandon head should reserve");
    adapter
        .reserve_next_official_refresh_order(&abandon_workspace)
        .expect("abandon tail should reserve");
    authority_connection(&abandon_workspace)
        .execute_batch(
            "CREATE TRIGGER fail_next_executable_abandon_metadata
             BEFORE INSERT ON authority_metadata
             WHEN NEW.key = 'next_executable_refresh_order'
             BEGIN
                 SELECT RAISE(FAIL, 'forced metadata failure');
             END;",
        )
        .expect("abandon metadata trigger should install");
    assert_error_contains(
        adapter.abandon_next_official_refresh_order(&abandon_workspace, "metadata failure"),
        "failed to update authority metadata `next_executable_refresh_order`",
    );

    let pool_queue_workspace = temp_workspace("runtime-cleanup-error-pool-queue");
    replace_runtime_table_schema(
        &pool_queue_workspace,
        "runtime_distributor_queue",
        "broken_queue_id TEXT PRIMARY KEY",
    );
    let mut pool_queue_report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("pool-queue-error"),
        ParallelModePoolResetPolicy::ForceDisposable,
    );
    pool_queue_report
        .reset_queue_item_ids
        .push("queue-pool-error".to_string());
    assert_error_contains(
        adapter.apply_parallel_pool_reset_report(&pool_queue_workspace, &pool_queue_report),
        "failed to clear reset distributor queue item `queue-pool-error`",
    );

    let pool_claim_workspace = temp_workspace("runtime-cleanup-error-pool-claim");
    replace_runtime_table_schema(
        &pool_claim_workspace,
        "runtime_claims",
        "broken_claim_id TEXT PRIMARY KEY",
    );
    let mut pool_claim_report = ParallelModePoolResetReport::new(
        ParallelModePoolResetRunId::new("pool-claim-error"),
        ParallelModePoolResetPolicy::ForceDisposable,
    );
    pool_claim_report
        .reset_queue_item_ids
        .push("queue-claim-error".to_string());
    assert_error_contains(
        adapter.apply_parallel_pool_reset_report(&pool_claim_workspace, &pool_claim_report),
        "failed to clear reset distributor claim `queue-claim-error`",
    );

    let task_invalid_workspace = temp_workspace("runtime-cleanup-error-task-invalid");
    adapter
        .upsert_runtime_slot_lease(
            &task_invalid_workspace,
            &slot_lease_for_task(
                "slot-cleanup-invalid",
                "task-cleanup-invalid",
                ParallelModeSlotLeaseState::Running,
            ),
        )
        .expect("task invalid seed slot should persist");
    replace_runtime_table_schema(
        &task_invalid_workspace,
        "runtime_invalid_slot_leases",
        "broken_slot_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_invalid_workspace,
            &["task-cleanup-invalid".to_string()],
            "broken invalid slot table",
        ),
        "failed to clear invalid runtime slot lease `slot-cleanup-invalid`",
    );

    let task_session_workspace = temp_workspace("runtime-cleanup-error-task-session");
    replace_runtime_table_schema(
        &task_session_workspace,
        "runtime_session_details",
        "session_key TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_session_workspace,
            &["task-cleanup-session".to_string()],
            "broken session table",
        ),
        "failed to clear runtime session details for `task-cleanup-session`",
    );

    let task_block_workspace = temp_workspace("runtime-cleanup-error-task-block");
    replace_runtime_table_schema(
        &task_block_workspace,
        "runtime_task_dispatch_blocks",
        "broken_task_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_block_workspace,
            &["task-cleanup-block".to_string()],
            "broken block table",
        ),
        "failed to clear runtime task dispatch blocks for `task-cleanup-block`",
    );

    let task_queue_workspace = temp_workspace("runtime-cleanup-error-task-queue");
    adapter
        .upsert_runtime_distributor_queue_record(
            &task_queue_workspace,
            &queue_record_for_task(
                "queue-cleanup-delete",
                "session-cleanup-delete",
                "task-cleanup-queue",
            ),
        )
        .expect("task queue seed should persist");
    install_failing_delete_trigger(
        &task_queue_workspace,
        "runtime_distributor_queue",
        "fail_task_queue_delete",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_queue_workspace,
            &["task-cleanup-queue".to_string()],
            "queue delete trigger",
        ),
        "failed to clear runtime distributor queue records for `task-cleanup-queue`",
    );

    let task_claim_workspace = temp_workspace("runtime-cleanup-error-task-claim");
    adapter
        .upsert_runtime_distributor_queue_record(
            &task_claim_workspace,
            &queue_record_for_task(
                "queue-cleanup-claim",
                "session-cleanup-claim",
                "task-cleanup-claim",
            ),
        )
        .expect("task claim queue seed should persist");
    replace_runtime_table_schema(
        &task_claim_workspace,
        "runtime_claims",
        "broken_claim_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections_for_tasks(
            &task_claim_workspace,
            &["task-cleanup-claim".to_string()],
            "broken claim table",
        ),
        "failed to clear runtime queue claim `queue-cleanup-claim`",
    );

    let preserve_workspace = temp_workspace("runtime-cleanup-error-preserve-block");
    adapter
        .upsert_runtime_session_detail(
            &preserve_workspace,
            &failed_start_session_detail(
                "session-preserve",
                "task-preserve",
                "2026-05-04T12:00:00+00:00",
            ),
        )
        .expect("preserve seed session should persist");
    replace_runtime_table_schema(
        &preserve_workspace,
        "runtime_task_dispatch_blocks",
        "task_id TEXT PRIMARY KEY",
    );
    assert_error_contains(
        adapter.clear_parallel_runtime_projections(&preserve_workspace, "broken block table"),
        "failed to preserve failed-start task dispatch block `task-preserve`",
    );

    let stale_queue_workspace = temp_workspace("runtime-cleanup-error-stale-queue-claim");
    adapter
        .upsert_runtime_distributor_queue_record(
            &stale_queue_workspace,
            &queue_record_for_task(
                "queue-stale-delete",
                "session-stale-delete",
                "task-stale-delete",
            ),
        )
        .expect("stale queue record should persist");
    assert!(
        adapter
            .try_acquire_distributor_queue_claim(
                &stale_queue_workspace,
                "queue-stale-delete",
                "owner-old",
            )
            .expect("stale queue seed claim should acquire")
    );
    set_claim_timestamp(
        &stale_queue_workspace,
        DISTRIBUTOR_QUEUE_CLAIM_KIND,
        "queue-stale-delete",
        "2000-01-01T00:00:00+00:00",
    );
    install_failing_delete_trigger(
        &stale_queue_workspace,
        "runtime_claims",
        "fail_stale_queue_claim_delete",
    );
    assert_error_contains(
        adapter.try_acquire_distributor_queue_claim(
            &stale_queue_workspace,
            "queue-stale-delete",
            "owner-new",
        ),
        "failed to clear stale runtime claim `distributor-queue-head:queue-stale-delete`",
    );

    let stale_official_workspace = temp_workspace("runtime-cleanup-error-stale-official-claim");
    let stale_order = adapter
        .reserve_next_official_refresh_order(&stale_official_workspace)
        .expect("stale official order should reserve");
    adapter
        .reserve_next_official_refresh_order(&stale_official_workspace)
        .expect("stale official tail should reserve");
    assert_eq!(
        adapter
            .acquire_official_refresh_claim(&stale_official_workspace, stale_order, "owner-old")
            .expect("stale official claim should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    set_claim_timestamp(
        &stale_official_workspace,
        "official-refresh",
        OFFICIAL_REFRESH_SCOPE_KEY,
        "2000-01-01T00:00:00+00:00",
    );
    install_failing_delete_trigger(
        &stale_official_workspace,
        "runtime_claims",
        "fail_stale_official_claim_delete",
    );
    assert_error_contains(
        adapter.abandon_next_official_refresh_order(&stale_official_workspace, "stale failure"),
        "failed to clear stale runtime claim `official-refresh:official-refresh`",
    );
}

#[test]
fn runtime_slot_removal_clears_current_and_invalid_slot_projection() {
    let workspace_dir = temp_workspace("runtime-slot-removal");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let lease = slot_lease("slot-remove", ParallelModeSlotLeaseState::Running);

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &lease)
        .expect("slot lease should persist before removal");
    insert_invalid_slot_marker(&workspace_dir, "slot-remove");
    adapter
        .remove_runtime_slot_lease(&workspace_dir, "slot-remove")
        .expect("slot lease should remove");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after slot removal");
    assert!(!snapshot.slot_leases.contains_key("slot-remove"));
    assert!(!snapshot.invalid_slot_leases.contains("slot-remove"));
    assert!(snapshot.runtime_events.iter().any(|event| {
        event.event_kind == "slot_lease_removed" && event.projection_key == "slot-remove"
    }));
}

#[test]
fn runtime_slot_removal_without_current_row_clears_invalid_marker_without_event() {
    let workspace_dir = temp_workspace("runtime-slot-removal-empty");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    insert_invalid_slot_marker(&workspace_dir, "slot-missing");

    adapter
        .remove_runtime_slot_lease(&workspace_dir, "slot-missing")
        .expect("missing slot removal should still clear invalid marker");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load after missing slot removal");
    assert!(snapshot.invalid_slot_leases.is_empty());
    assert!(
        !snapshot
            .runtime_events
            .iter()
            .any(|event| event.event_kind == "slot_lease_removed")
    );
}

#[test]
fn runtime_slot_compare_and_delete_preserves_replacement_generation() {
    let workspace_dir = temp_workspace("runtime-slot-removal-cas");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let stale = slot_lease("slot-cas", ParallelModeSlotLeaseState::CleanupPending);
    let mut replacement = stale.clone();
    replacement.task_id = "replacement-task".to_string();
    replacement.agent_id = "replacement-agent".to_string();
    replacement.branch_name = "akra-agent/slot-cas/replacement".to_string();
    replacement.leased_at = "2026-07-10T12:00:00+00:00".to_string();
    replacement.state = ParallelModeSlotLeaseState::Leased;

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &stale)
        .expect("stale lease should persist");
    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &replacement)
        .expect("replacement lease should persist");

    assert!(
        !adapter
            .remove_runtime_slot_lease_if_matches(&workspace_dir, &stale)
            .expect("stale compare-and-delete should be rejected")
    );
    let preserved = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("replacement projection should remain");
    assert_eq!(preserved.slot_leases.get("slot-cas"), Some(&replacement));

    assert!(
        adapter
            .remove_runtime_slot_lease_if_matches(&workspace_dir, &replacement)
            .expect("exact replacement compare-and-delete should succeed")
    );
    assert!(
        !adapter
            .load_runtime_projections(&workspace_dir)
            .expect("removed projection should load")
            .slot_leases
            .contains_key("slot-cas")
    );
}

#[test]
fn runtime_task_dispatch_block_keeps_newer_block_when_older_update_arrives() {
    let workspace_dir = temp_workspace("runtime-dispatch-block-older");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let newer = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-blocked",
        "2026-05-04T12:00:00+00:00",
        "2026-05-04T12:10:00+00:00",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );
    let older = ParallelModeTaskDispatchBlockSnapshot::new(
        "task-blocked",
        "2026-05-04T11:00:00+00:00",
        "2026-05-04T12:00:00+00:00",
        ParallelModeDispatchBlockReason::StartupFailedUntilTaskChanges,
    );

    adapter
        .upsert_runtime_task_dispatch_block(&workspace_dir, &newer)
        .expect("newer dispatch block should persist");
    adapter
        .upsert_runtime_task_dispatch_block(&workspace_dir, &older)
        .expect("older dispatch block should be ignored without failing");

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");
    assert_eq!(snapshot.task_dispatch_blocks, vec![newer]);
    let block_events = snapshot
        .runtime_events
        .iter()
        .filter(|event| event.event_kind == "task_dispatch_block_upsert")
        .count();
    assert_eq!(block_events, 1);
}

#[test]
fn runtime_event_log_without_authority_events_keeps_cursor_unknown() {
    let workspace_dir = temp_workspace("runtime-events-no-authority-events");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let snapshot = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::recent(5),
        )
        .expect("empty runtime event log should load");

    assert_eq!(snapshot.total_event_count, 0);
    assert_eq!(snapshot.visible_count(), 0);
    assert_eq!(snapshot.event_cursor, None);
}

#[test]
fn runtime_event_log_empty_filter_reports_no_events() {
    let workspace_dir = temp_workspace("runtime-events-empty");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .upsert_runtime_slot_lease(
            &workspace_dir,
            &slot_lease("slot-1", ParallelModeSlotLeaseState::Leased),
        )
        .expect("unrelated event should persist");

    let snapshot = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", "missing", 5),
        )
        .expect("empty runtime event log should load");

    assert_eq!(snapshot.total_event_count, 0);
    assert_eq!(snapshot.visible_count(), 0);
    assert_eq!(snapshot.event_cursor, Some(1));
    assert_eq!(snapshot.empty_state, "no runtime events captured yet");
}

#[test]
fn runtime_event_log_port_reads_recent_projection_events() {
    let workspace_dir = temp_workspace("runtime-events");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let first = slot_lease("slot-1", ParallelModeSlotLeaseState::Leased);
    let second = slot_lease("slot-1", ParallelModeSlotLeaseState::Running);

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &first)
        .expect("first slot lease event should persist");
    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &second)
        .expect("second slot lease event should persist");

    let snapshot = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", "slot-1", 1),
        )
        .expect("runtime event log should load");

    assert_eq!(snapshot.total_event_count, 2);
    assert_eq!(snapshot.visible_count(), 1);
    assert_eq!(snapshot.event_cursor, Some(2));
    let latest = snapshot.latest().expect("latest event should be visible");
    assert_eq!(latest.sequence, 2);
    assert_eq!(latest.event_kind, "slot_lease_upsert");
    assert_eq!(latest.projection_kind, "slot_lease");
    assert_eq!(latest.projection_key, "slot-1");
    assert!(
        latest
            .summary
            .contains("runtime slot lease stored / slot: slot-1 / state: running")
    );
}

#[test]
fn runtime_event_log_port_filters_events_after_sequence() {
    let workspace_dir = temp_workspace("runtime-events-after-sequence");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let first = slot_lease("slot-1", ParallelModeSlotLeaseState::Leased);
    let second = slot_lease("slot-1", ParallelModeSlotLeaseState::Running);

    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &first)
        .expect("first slot lease event should persist");
    adapter
        .upsert_runtime_slot_lease(&workspace_dir, &second)
        .expect("second slot lease event should persist");

    let snapshot = adapter
        .load_runtime_event_log(
            &workspace_dir,
            ParallelModeRuntimeEventLogRequest::for_projection("slot_lease", "slot-1", 10)
                .after_sequence(1),
        )
        .expect("incremental runtime event log should load");

    assert_eq!(snapshot.total_event_count, 1);
    assert_eq!(snapshot.visible_count(), 1);
    assert_eq!(snapshot.event_cursor, Some(2));
    let latest = snapshot.latest().expect("latest event should be visible");
    assert_eq!(latest.sequence, 2);
    assert!(latest.sequence > 1);
    assert!(latest.summary.contains("state: running"));
}

#[test]
fn runtime_projection_loads_recent_runtime_event_feed_newest_first() {
    let workspace_dir = temp_workspace("runtime-events");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    for index in 1..=10 {
        adapter
            .upsert_runtime_session_detail(
                &workspace_dir,
                &failed_start_session_detail(
                    &format!("session-{index:02}"),
                    &format!("task-{index:02}"),
                    &format!("2026-05-04T12:{index:02}:00+00:00"),
                ),
            )
            .expect("session detail event should persist");
    }

    let snapshot = adapter
        .load_runtime_projections(&workspace_dir)
        .expect("runtime projections should load");

    assert_eq!(snapshot.runtime_events.len(), 8);
    assert_eq!(snapshot.runtime_events[0].sequence, 10);
    assert_eq!(
        snapshot.runtime_events[0].event_kind,
        "session_detail_upsert"
    );
    assert_eq!(snapshot.runtime_events[0].projection_kind, "session_detail");
    assert_eq!(snapshot.runtime_events[0].projection_key, "session-10");
    assert_eq!(snapshot.runtime_events[0].observed_planning_revision, 0);
    assert!(snapshot.runtime_events[0].summary.contains("state: failed"));
    assert_eq!(snapshot.runtime_events[7].sequence, 3);
}

fn failed_start_session_detail(
    session_key: &str,
    task_id: &str,
    updated_at: &str,
) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::new(
        session_key,
        "agent-1",
        task_id,
        "Task One",
        "slot-1",
        None,
        "/tmp/worktree",
        "akra-agent/slot-1/task-one",
        "2026-05-04T10:00:00+00:00",
        "failed",
        "aborted",
        "launch failed before the session reached the running state",
        "validation unavailable",
        "startup failed",
        None,
        Vec::new(),
        updated_at,
    )
}

fn running_session_detail(
    session_key: &str,
    task_id: &str,
    updated_at: &str,
) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot::new(
        session_key,
        "agent-1",
        task_id,
        "Task One",
        "slot-1",
        Some("thread-running".to_string()),
        "/tmp/worktree",
        "akra-agent/slot-1/task-one",
        "2026-05-04T10:00:00+00:00",
        "running",
        "in_progress",
        "worker is running",
        "validation unavailable",
        "running",
        None,
        Vec::new(),
        updated_at,
    )
}

fn slot_lease(slot_id: &str, state: ParallelModeSlotLeaseState) -> ParallelModeSlotLeaseSnapshot {
    slot_lease_for_task(slot_id, "task-1", state)
}

fn slot_lease_for_task(
    slot_id: &str,
    task_id: &str,
    state: ParallelModeSlotLeaseState,
) -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
        slot_id,
        task_id,
        "Task One",
        "agent-1",
        format!("akra-agent/{slot_id}/{task_id}"),
        "/tmp/worktree",
        state,
        "2026-05-04T10:00:00+00:00",
        Some("2026-05-04T10:05:00+00:00".to_string()),
    )
}

fn queue_record_for_task(
    queue_item_id: &str,
    session_key: &str,
    task_id: &str,
) -> PlanningAuthorityDistributorQueueRecord {
    PlanningAuthorityDistributorQueueRecord {
        queue_item_id: queue_item_id.to_string(),
        queue_order_key: 1,
        session_key: session_key.to_string(),
        slot_id: "slot-1".to_string(),
        agent_id: "agent-1".to_string(),
        task_id: task_id.to_string(),
        task_title: "Task One".to_string(),
        delivery_target: Some(PlanningAuthorityDistributorDeliveryTarget::new(
            "origin",
            "acme/widgets",
            GithubRepositoryVisibility::Private,
            "prerelease",
        )),
        source_branch: "prerelease".to_string(),
        source_base_commit_sha: "base".to_string(),
        source_commit_sha: "source".to_string(),
        branch_name: format!("akra-agent/slot-1/{task_id}"),
        worktree_path: "/tmp/worktree".to_string(),
        commit_sha: "commit".to_string(),
        original_commit_sha: None,
        planning_refresh_state: "complete".to_string(),
        integration_state: "queued".to_string(),
        integration_base_commit_sha: None,
        integration_commit_sha: None,
        conflict_files: Vec::new(),
        recovery_note: None,
        validation_summary: "validation unavailable".to_string(),
        authority_refresh_outcome: "not refreshed".to_string(),
        github_capabilities: None,
        pull_request_number: None,
        pull_request_url: None,
        queue_state: ParallelModeQueueItemState::Queued,
        integration_note: "queued".to_string(),
        enqueued_at: "2026-05-04T10:00:00+00:00".to_string(),
        updated_at: "2026-05-04T10:00:00+00:00".to_string(),
        retry_attempts: 0,
        retry_not_before: None,
    }
}

fn empty_test_queue_projection() -> PriorityQueueProjection {
    PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    }
}

fn test_direction_catalog(direction_ids: &[&str]) -> DirectionCatalogDocument {
    DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig {
            policy: QueueIdlePolicy::Stop,
            prompt_path: String::new(),
        },
        directions: direction_ids
            .iter()
            .map(|direction_id| DirectionDefinition {
                id: (*direction_id).to_string(),
                title: format!("Direction {direction_id}"),
                summary: "test direction".to_string(),
                success_criteria: vec!["done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            })
            .collect(),
    }
}

fn task_authority_for_direction(task_id: &str, direction_id: &str) -> TaskAuthorityDocument {
    TaskAuthorityDocument {
        version: 1,
        tasks: vec![authority_task(task_id, direction_id)],
    }
}

fn authority_task(task_id: &str, direction_id: &str) -> TaskDefinition {
    TaskDefinition {
        id: task_id.to_string(),
        direction_id: direction_id.to_string(),
        direction_relation_note: "test direction relation".to_string(),
        title: format!("Task {task_id}"),
        description: "authority fence test task".to_string(),
        status: TaskStatus::Ready,
        base_priority: 50,
        dynamic_priority_delta: 0,
        priority_reason: String::new(),
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        created_by: TaskActor::User,
        last_updated_by: TaskActor::User,
        source_turn_id: None,
        provenance: TaskMutationProvenance::new(OriginSessionKind::System),
        updated_at: "2026-05-07T09:00:00Z".to_string(),
    }
}
