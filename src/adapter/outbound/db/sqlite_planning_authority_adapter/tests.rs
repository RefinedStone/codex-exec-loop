use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use super::{AUTHORITY_STORE_SCHEMA_VERSION, SqlitePlanningAuthorityAdapter};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::service::planning::{
    DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
};
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use crate::domain::planning::{
    PlanningAuthorityShadowStoreSyncState, PriorityQueueProjection, TaskLedgerDocument,
};

struct TempGitRepo {
    root: PathBuf,
    repo_root: PathBuf,
    worktree_root: PathBuf,
}

impl TempGitRepo {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{label}-{unique}"));
        let repo_root = root.join("repo");
        let worktree_root = root.join("worktrees").join("linked");
        fs::create_dir_all(&repo_root).expect("temp repo root should exist");
        run_git(&repo_root, &["init", "-q"]);
        run_git(&repo_root, &["config", "user.name", "RefinedStone"]);
        run_git(
            &repo_root,
            &["config", "user.email", "chem.en.9273@gmail.com"],
        );
        fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
        run_git(&repo_root, &["add", "README.md"]);
        run_git(&repo_root, &["commit", "-qm", "init"]);
        fs::create_dir_all(
            worktree_root
                .parent()
                .expect("worktree parent should exist"),
        )
        .expect("worktree parent should exist");
        run_git(
            &repo_root,
            &[
                "worktree",
                "add",
                "-b",
                "feature/worktree",
                worktree_root.to_str().expect("valid worktree path"),
            ],
        );

        Self {
            root,
            repo_root,
            worktree_root,
        }
    }

    fn write_repo_file(&self, relative_path: &str, body: &str) {
        let path = self.repo_root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent directory should exist");
        }
        fs::write(path, body).expect("repo file should write");
    }
}

impl Drop for TempGitRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn run_git(repo_root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .status()
        .expect("git command should spawn");
    assert!(
        status.success(),
        "git command should succeed: git {}",
        args.join(" ")
    );
}

fn runtime_exports_root(repo_root: &Path) -> PathBuf {
    repo_root.join(".codex-exec-loop/runtime/exports")
}

fn planning_snapshot_export_path(repo_root: &Path) -> PathBuf {
    runtime_exports_root(repo_root).join("planning-snapshot.json")
}

fn task_ledger_export_path(repo_root: &Path) -> PathBuf {
    runtime_exports_root(repo_root).join("task-ledger.json")
}

fn queue_projection_export_path(repo_root: &Path) -> PathBuf {
    runtime_exports_root(repo_root).join("queue.snapshot.json")
}

fn read_planning_snapshot_export(repo_root: &Path) -> BTreeMap<String, String> {
    let snapshot_body = fs::read_to_string(planning_snapshot_export_path(repo_root))
        .expect("planning snapshot export should exist");
    serde_json::from_str::<BTreeMap<String, String>>(&snapshot_body)
        .expect("planning snapshot export should parse")
}

fn task_ledger_with_ready_task_json() -> String {
    r#"{
  "version": 1,
  "tasks": [
{
  "id": "task-1",
  "direction_id": "direction-1",
  "direction_relation_note": "implements direction",
  "title": "Task One",
  "description": "Do task one.",
  "status": "ready",
  "base_priority": 10,
  "dynamic_priority_delta": 2,
  "priority_reason": "important",
  "depends_on": [],
  "blocked_by": [],
  "created_by": "user",
  "last_updated_by": "system",
  "source_turn_id": "turn-1",
  "updated_at": "2026-04-20T10:00:00Z"
}
  ]
}"#
    .to_string()
}

fn queue_projection_with_ready_task_json() -> String {
    r#"{
  "next_task": {
"rank": 1,
"task_id": "task-1",
"direction_id": "direction-1",
"direction_title": "Direction One",
"task_title": "Task One",
"status": "ready",
"combined_priority": 12,
"updated_at": "2026-04-20T10:00:00Z",
"rank_reasons": [
  "status=ready",
  "combined_priority=12 (base 10 + delta 2)"
]
  },
  "active_tasks": [
{
  "rank": 1,
  "task_id": "task-1",
  "direction_id": "direction-1",
  "direction_title": "Direction One",
  "task_title": "Task One",
  "status": "ready",
  "combined_priority": 12,
  "updated_at": "2026-04-20T10:00:00Z",
  "rank_reasons": [
    "status=ready",
    "combined_priority=12 (base 10 + delta 2)"
  ]
}
  ],
  "proposed_tasks": [],
  "skipped_tasks": []
}"#
    .to_string()
}

fn parse_task_ledger(body: &str) -> TaskLedgerDocument {
    serde_json::from_str(body).expect("task ledger should parse")
}

fn parse_queue_projection(body: &str) -> PriorityQueueProjection {
    serde_json::from_str(body).expect("queue projection should parse")
}

#[test]
fn resolve_authority_location_uses_canonical_repo_root_for_linked_worktree() {
    let repo = TempGitRepo::new("authority-location");
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let location = adapter
        .resolve_authority_location(repo.worktree_root.to_str().expect("valid path"))
        .expect("authority location should resolve");

    assert_eq!(
        location.canonical_repo_root,
        fs::canonicalize(&repo.repo_root)
            .expect("repo root should canonicalize")
            .display()
            .to_string()
    );
    assert_eq!(
        location.workspace_root,
        fs::canonicalize(&repo.worktree_root)
            .expect("worktree root should canonicalize")
            .display()
            .to_string()
    );
    let normalized_runtime_dir = location.runtime_dir.replace('\\', "/");
    let normalized_store_path = location.authority_store_path.replace('\\', "/");
    assert!(normalized_runtime_dir.contains("/.akra/tests/projects/"));
    assert!(normalized_runtime_dir.ends_with("/runtime"));
    assert!(normalized_store_path.ends_with("/runtime/planning-authority.db"));
}

#[test]
fn resolve_authority_location_uses_workspace_root_for_separate_git_dir_repo() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("separate-git-dir-{unique}"));
    let workspace_root = root.join("workspace");
    let external_git_dir = root.join("external-git-dir");
    fs::create_dir_all(&workspace_root).expect("workspace root should exist");
    let status = Command::new("git")
        .current_dir(&root)
        .args([
            "init",
            "--separate-git-dir",
            external_git_dir.to_str().expect("valid git dir path"),
            workspace_root.to_str().expect("valid workspace path"),
        ])
        .status()
        .expect("git init should spawn");
    assert!(
        status.success(),
        "git init with separate git dir should succeed"
    );

    let adapter = SqlitePlanningAuthorityAdapter::new();
    let location = adapter
        .resolve_authority_location(workspace_root.to_str().expect("valid path"))
        .expect("authority location should resolve");

    assert_eq!(
        location.canonical_repo_root,
        fs::canonicalize(&workspace_root)
            .expect("workspace root should canonicalize")
            .display()
            .to_string()
    );
    assert!(
        location
            .authority_store_path
            .replace('\\', "/")
            .ends_with("/runtime/planning-authority.db")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn inspect_shadow_store_bootstraps_from_active_store() {
    let repo = TempGitRepo::new("shadow-bootstrap");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        repo.worktree_root.to_str().expect("valid path"),
        ".codex-exec-loop/planning/directions.toml",
        Some("version = 1\n"),
    )
    .expect("directions should seed the authority store");
    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        repo.worktree_root.to_str().expect("valid path"),
        ".codex-exec-loop/planning/task-ledger.json",
        Some("{\"version\":1,\"tasks\":[]}\n"),
    )
    .expect("task ledger should seed the authority store");
    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        repo.worktree_root.to_str().expect("valid path"),
        ".codex-exec-loop/planning/prompts/queue-idle-review.md",
        Some("# review\n"),
    )
    .expect("prompt should seed the authority store");

    let inspection = adapter
        .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
        .expect("shadow store should inspect");

    assert_eq!(
        inspection.sync_state,
        PlanningAuthorityShadowStoreSyncState::Bootstrapped
    );
    assert_eq!(inspection.mirrored_document_count, 4);
    let connection = Connection::open(&inspection.location.authority_store_path)
        .expect("shadow store should open");
    let content = connection
        .query_row(
            "SELECT content FROM shadow_documents WHERE relative_path = ?1",
            [".codex-exec-loop/planning/directions.toml"],
            |row| row.get::<_, String>(0),
        )
        .expect("directions content should exist");
    assert_eq!(content, "version = 1\n");
}

#[test]
fn inspect_shadow_store_restores_diverged_runtime_exports_from_active_store() {
    let repo = TempGitRepo::new("shadow-resync");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
        repo.worktree_root.to_str().expect("valid path"),
        &PlanningWorkspaceLoadRecord {
            directions_toml: Some("version = 1\n".to_string()),
            task_ledger_json: None,
            task_ledger_schema_json: None,
            queue_snapshot_json: None,
            result_output_markdown: None,
        },
    )
    .expect("active planning should seed the authority store");
    adapter
        .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
        .expect("initial shadow store sync should succeed");

    fs::write(
        planning_snapshot_export_path(&repo.repo_root),
        "{\n  \".codex-exec-loop/planning/directions.toml\": \"version = 2\\n\"\n}\n",
    )
    .expect("runtime export snapshot should diverge");

    let inspection = adapter
        .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
        .expect("shadow store resync should succeed");

    assert_eq!(
        inspection.sync_state,
        PlanningAuthorityShadowStoreSyncState::Resynced
    );
    assert_eq!(inspection.parity_issue_count, 1);
    assert!(
        inspection
            .parity_issue_examples
            .iter()
            .any(|issue| issue.contains("runtime export"))
    );
    assert_eq!(
        read_planning_snapshot_export(&repo.repo_root)
            .get(DIRECTIONS_FILE_PATH)
            .expect("runtime export snapshot should be restored"),
        "version = 1\n"
    );
}

#[test]
fn inspect_shadow_store_rejects_export_only_legacy_state() {
    let repo = TempGitRepo::new("shadow-export-only");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    fs::create_dir_all(runtime_exports_root(&repo.repo_root))
        .expect("runtime exports root should exist");
    fs::write(
        planning_snapshot_export_path(&repo.repo_root),
        "{\n  \".codex-exec-loop/planning/directions.toml\": \"version = 1\\n\"\n}\n",
    )
    .expect("runtime export snapshot should write");

    let error = adapter
        .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
        .expect_err("export-only legacy state should be rejected");

    assert!(
        error
            .to_string()
            .contains("authority store is empty while runtime exports still exist")
    );
}

#[test]
fn inspect_shadow_store_rejects_legacy_schema_version_one_store() {
    let repo = TempGitRepo::new("shadow-upgrade-v1");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 1\n");
    let location = adapter
        .resolve_authority_location(repo.worktree_root.to_str().expect("valid path"))
        .expect("authority location should resolve");
    let runtime_dir = PathBuf::from(&location.runtime_dir);
    fs::create_dir_all(&runtime_dir).expect("runtime directory should exist");
    let authority_store_path = runtime_dir.join("planning-authority.db");
    let connection =
        Connection::open(&authority_store_path).expect("legacy authority store should open");
    connection
        .execute_batch(
            r#"
            CREATE TABLE authority_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE shadow_documents (
                relative_path TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );
            "#,
        )
        .expect("legacy shadow-store schema should initialize");
    connection
        .execute(
            "INSERT INTO authority_metadata (key, value) VALUES ('schema_version', '1')",
            [],
        )
        .expect("legacy schema version should insert");
    connection
        .execute(
            "INSERT INTO shadow_documents (relative_path, content) VALUES (?1, ?2)",
            [".codex-exec-loop/planning/directions.toml", "version = 0\n"],
        )
        .expect("legacy shadow document should insert");

    let error = adapter
        .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
        .expect_err("legacy schema version should be rejected");

    assert_eq!(
        error.to_string(),
        "unsupported authority-store schema version: 1"
    );
}

#[test]
fn active_commit_updates_repo_scoped_documents_for_linked_worktree() {
    let repo = TempGitRepo::new("authority-active-commit");
    let task_ledger_json = task_ledger_with_ready_task_json();
    let queue_snapshot_json = queue_projection_with_ready_task_json();

    SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
        repo.worktree_root.to_str().expect("valid worktree path"),
        &PlanningWorkspaceLoadRecord {
            directions_toml: Some("version = 4\n".to_string()),
            task_ledger_json: Some(task_ledger_json.clone()),
            task_ledger_schema_json: Some("{\"type\":\"object\"}\n".to_string()),
            queue_snapshot_json: Some(queue_snapshot_json.clone()),
            result_output_markdown: Some("# result\n".to_string()),
        },
    )
    .expect("active commit should succeed");

    assert_eq!(
        read_planning_snapshot_export(&repo.repo_root)
            .get(DIRECTIONS_FILE_PATH)
            .expect("runtime export directions should exist"),
        "version = 4\n"
    );
    assert!(
        fs::read_to_string(task_ledger_export_path(&repo.repo_root))
            .expect("runtime export task ledger should exist")
            .contains("\"id\": \"task-1\"")
    );
    assert!(
        !fs::read_to_string(queue_projection_export_path(&repo.repo_root))
            .expect("runtime export queue projection should exist")
            .contains("\"bucket\"")
    );
    let location = SqlitePlanningAuthorityAdapter::new()
        .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
        .expect("authority location should resolve");
    let connection =
        Connection::open(&location.authority_store_path).expect("authority store should open");
    let stored_task_count = connection
        .query_row(
            "SELECT COUNT(*) FROM planning_tasks WHERE task_id = 'task-1'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("planning task rows should be readable");
    assert_eq!(stored_task_count, 1);
    let active_document_task_count = connection
        .query_row(
            "SELECT COUNT(*) FROM active_documents WHERE relative_path IN (?1, ?2)",
            [TASK_LEDGER_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH],
            |row| row.get::<_, i64>(0),
        )
        .expect("active document rows should be readable");
    assert_eq!(active_document_task_count, 0);
}

#[test]
fn task_repository_commit_round_trips_relational_authority_projection() {
    let repo = TempGitRepo::new("authority-task-repository");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
    let task_ledger = parse_task_ledger(&task_ledger_with_ready_task_json());
    let queue_projection = parse_queue_projection(&queue_projection_with_ready_task_json());

    adapter
        .commit_task_authority_snapshot(
            workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_ledger: &task_ledger,
                queue_projection: &queue_projection,
            },
        )
        .expect("task authority should commit");

    let snapshot = adapter
        .load_task_authority_snapshot(workspace_dir)
        .expect("task authority should load")
        .expect("task authority should exist");
    assert_eq!(snapshot.task_ledger, task_ledger);
    assert_eq!(snapshot.queue_projection, queue_projection);
    assert!(
        !fs::read_to_string(task_ledger_export_path(&repo.repo_root))
            .expect("task ledger export should exist")
            .contains("\"task_id\"")
    );
    assert!(
        fs::read_to_string(queue_projection_export_path(&repo.repo_root))
            .expect("queue projection export should exist")
            .contains("\"task_id\": \"task-1\"")
    );
}

#[test]
fn task_repository_commit_rejects_stale_observed_revision() {
    let repo = TempGitRepo::new("authority-task-repository-conflict");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
    let task_ledger = parse_task_ledger(&task_ledger_with_ready_task_json());
    let queue_projection = parse_queue_projection(&queue_projection_with_ready_task_json());
    let first_result = adapter
        .commit_task_authority_snapshot(
            workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_ledger: &task_ledger,
                queue_projection: &queue_projection,
            },
        )
        .expect("initial task authority should commit");
    let PlanningTaskAuthorityCommitResult::Committed { planning_revision } = first_result else {
        panic!("initial commit should not conflict");
    };
    assert_eq!(planning_revision, 1);

    let mut stale_task_ledger = task_ledger.clone();
    stale_task_ledger.tasks[0].title = "Stale writer update".to_string();
    let stale_result = adapter
        .commit_task_authority_snapshot(
            workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(0),
                task_ledger: &stale_task_ledger,
                queue_projection: &queue_projection,
            },
        )
        .expect("stale task authority commit should return a conflict");

    assert_eq!(
        stale_result,
        PlanningTaskAuthorityCommitResult::Conflict {
            observed_planning_revision: 0,
            current_planning_revision: 1,
        }
    );
}

#[test]
fn replacing_directions_prunes_task_authority_for_removed_direction_ids() {
    let repo = TempGitRepo::new("authority-direction-prune");
    let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
    let initial_directions = r#"version = 1

[[directions]]
id = "direction-1"
title = "Direction One"
summary = "Keep this direction."
success_criteria = ["kept"]
state = "active"

[[directions]]
id = "direction-2"
title = "Direction Two"
summary = "Remove this direction."
success_criteria = ["removed"]
state = "active"
"#;
    let pruned_directions = r#"version = 1

[[directions]]
id = "direction-1"
title = "Direction One"
summary = "Keep this direction."
success_criteria = ["kept"]
state = "active"
"#;
    let task_ledger = r#"{
  "version": 1,
  "tasks": [
{
  "id": "task-1",
  "direction_id": "direction-1",
  "title": "Task One",
  "description": "Keep task one.",
  "status": "ready",
  "base_priority": 10,
  "dynamic_priority_delta": 0,
  "depends_on": ["task-2"],
  "blocked_by": ["task-2"],
  "created_by": "user",
  "last_updated_by": "system",
  "updated_at": "2026-04-20T10:00:00Z"
},
{
  "id": "task-2",
  "direction_id": "direction-2",
  "title": "Task Two",
  "description": "Drop task two.",
  "status": "ready",
  "base_priority": 9,
  "dynamic_priority_delta": 0,
  "depends_on": [],
  "blocked_by": [],
  "created_by": "user",
  "last_updated_by": "system",
  "updated_at": "2026-04-20T10:01:00Z"
}
  ]
}"#;

    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        workspace_dir,
        DIRECTIONS_FILE_PATH,
        Some(initial_directions),
    )
    .expect("initial directions should write");
    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        workspace_dir,
        TASK_LEDGER_FILE_PATH,
        Some(task_ledger),
    )
    .expect("initial task authority should write");

    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        workspace_dir,
        DIRECTIONS_FILE_PATH,
        Some(pruned_directions),
    )
    .expect("directions replacement should prune task authority");

    let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(workspace_dir)
        .expect("active workspace should load");
    let pruned_task_ledger = parse_task_ledger(
        loaded
            .task_ledger_json
            .as_deref()
            .expect("task authority should remain present"),
    );

    assert_eq!(pruned_task_ledger.tasks.len(), 1);
    assert_eq!(pruned_task_ledger.tasks[0].id, "task-1");
    assert!(pruned_task_ledger.tasks[0].depends_on.is_empty());
    assert!(pruned_task_ledger.tasks[0].blocked_by.is_empty());
    let queue_projection = parse_queue_projection(
        loaded
            .queue_snapshot_json
            .as_deref()
            .expect("queue projection should load"),
    );
    assert_eq!(queue_projection.next_task, None);
    assert!(queue_projection.active_tasks.is_empty());
}

#[test]
fn replacing_task_ledger_ignores_tasks_for_unknown_active_direction_ids() {
    let repo = TempGitRepo::new("authority-task-ledger-prune");
    let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
    let active_directions = r#"version = 1

[[directions]]
id = "direction-1"
title = "Direction One"
summary = "Keep this direction."
success_criteria = ["kept"]
state = "active"
"#;
    let task_ledger = r#"{
  "version": 1,
  "tasks": [
{
  "id": "task-1",
  "direction_id": "direction-1",
  "title": "Task One",
  "description": "Keep task one.",
  "status": "ready",
  "base_priority": 10,
  "dynamic_priority_delta": 0,
  "depends_on": ["task-2"],
  "blocked_by": ["task-2"],
  "created_by": "user",
  "last_updated_by": "system",
  "updated_at": "2026-04-20T10:00:00Z"
},
{
  "id": "task-2",
  "direction_id": "removed-direction",
  "title": "Task Two",
  "description": "Drop task two.",
  "status": "ready",
  "base_priority": 9,
  "dynamic_priority_delta": 0,
  "depends_on": [],
  "blocked_by": [],
  "created_by": "user",
  "last_updated_by": "system",
  "updated_at": "2026-04-20T10:01:00Z"
}
  ]
}"#;

    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        workspace_dir,
        DIRECTIONS_FILE_PATH,
        Some(active_directions),
    )
    .expect("active directions should write");

    SqlitePlanningAuthorityAdapter::replace_active_planning_file(
        workspace_dir,
        TASK_LEDGER_FILE_PATH,
        Some(task_ledger),
    )
    .expect("task ledger replacement should prune unknown directions");

    let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(workspace_dir)
        .expect("active workspace should load");
    let pruned_task_ledger = parse_task_ledger(
        loaded
            .task_ledger_json
            .as_deref()
            .expect("task authority should load"),
    );

    assert_eq!(pruned_task_ledger.tasks.len(), 1);
    assert_eq!(pruned_task_ledger.tasks[0].id, "task-1");
    assert!(pruned_task_ledger.tasks[0].depends_on.is_empty());
    assert!(pruned_task_ledger.tasks[0].blocked_by.is_empty());
}

#[test]
fn legacy_active_task_ledger_blob_is_ignored_as_export_only_state() {
    let repo = TempGitRepo::new("authority-task-backfill");
    let location = SqlitePlanningAuthorityAdapter::new()
        .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
        .expect("authority location should resolve");
    let runtime_dir = Path::new(&location.runtime_dir);
    fs::create_dir_all(runtime_dir).expect("runtime dir should exist");
    let connection =
        Connection::open(&location.authority_store_path).expect("authority store should open");
    connection
        .execute_batch(
            r#"
            CREATE TABLE authority_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE active_documents (
                relative_path TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );
            "#,
        )
        .expect("legacy authority store should initialize");
    connection
        .execute(
            "INSERT INTO authority_metadata (key, value) VALUES ('schema_version', '4')",
            [],
        )
        .expect("legacy schema version should insert");
    connection
        .execute(
            "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)",
            [
                TASK_LEDGER_FILE_PATH,
                task_ledger_with_ready_task_json().as_str(),
            ],
        )
        .expect("legacy task ledger should insert");
    connection
        .execute(
            "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)",
            [
                QUEUE_SNAPSHOT_FILE_PATH,
                queue_projection_with_ready_task_json().as_str(),
            ],
        )
        .expect("legacy queue projection should insert");
    drop(connection);

    let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(
        repo.worktree_root.to_str().expect("valid worktree path"),
    )
    .expect("active workspace should load");

    assert_eq!(loaded.task_ledger_json, None);
    assert_eq!(loaded.queue_snapshot_json, None);
    let connection =
        Connection::open(&location.authority_store_path).expect("authority store should open");
    let stored_task_count = connection
        .query_row("SELECT COUNT(*) FROM planning_tasks", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("planning task rows should be readable");
    let active_projection_count = connection
        .query_row(
            "SELECT COUNT(*) FROM planning_queue_projection WHERE bucket = 'active'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("planning queue projection rows should be readable");
    assert_eq!(stored_task_count, 0);
    assert_eq!(active_projection_count, 0);
    let schema_version = connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("schema version should be readable");
    assert_eq!(schema_version, "4");
}

#[test]
fn legacy_invalid_task_ledger_blob_is_ignored_as_export_only_state() {
    let repo = TempGitRepo::new("authority-task-backfill-invalid");
    let location = SqlitePlanningAuthorityAdapter::new()
        .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
        .expect("authority location should resolve");
    let runtime_dir = Path::new(&location.runtime_dir);
    fs::create_dir_all(runtime_dir).expect("runtime dir should exist");
    let connection =
        Connection::open(&location.authority_store_path).expect("authority store should open");
    connection
        .execute_batch(
            r#"
            CREATE TABLE authority_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE active_documents (
                relative_path TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );
            "#,
        )
        .expect("legacy authority store should initialize");
    connection
        .execute(
            "INSERT INTO authority_metadata (key, value) VALUES ('schema_version', '4')",
            [],
        )
        .expect("legacy schema version should insert");
    connection
        .execute(
            "INSERT INTO active_documents (relative_path, content) VALUES (?1, ?2)",
            [TASK_LEDGER_FILE_PATH, "{\"version\":1,\"tasks\":["],
        )
        .expect("legacy invalid task ledger should insert");
    drop(connection);

    let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(
        repo.worktree_root.to_str().expect("valid worktree path"),
    )
    .expect("active workspace should still load");

    assert_eq!(loaded.task_ledger_json, None);
    let connection =
        Connection::open(&location.authority_store_path).expect("authority store should open");
    let stored_task_count = connection
        .query_row("SELECT COUNT(*) FROM planning_tasks", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("planning task rows should be readable");
    let schema_version = connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("schema version should be readable");
    assert_eq!(stored_task_count, 0);
    assert_eq!(schema_version, "4");
}

#[test]
fn active_workspace_load_reads_store_when_tracked_export_is_missing() {
    let repo = TempGitRepo::new("authority-active-load");

    SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
        repo.worktree_root.to_str().expect("valid worktree path"),
        &PlanningWorkspaceLoadRecord {
            directions_toml: Some("version = 4\n".to_string()),
            task_ledger_json: None,
            task_ledger_schema_json: None,
            queue_snapshot_json: None,
            result_output_markdown: None,
        },
    )
    .expect("active commit should succeed");
    assert!(
        !repo.repo_root.join(DIRECTIONS_FILE_PATH).exists(),
        "tracked planning files should stay untouched in git-backed mode"
    );

    let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(
        repo.worktree_root.to_str().expect("valid worktree path"),
    )
    .expect("active workspace should load from store");
    let directions = SqlitePlanningAuthorityAdapter::load_active_planning_file(
        repo.worktree_root.to_str().expect("valid worktree path"),
        DIRECTIONS_FILE_PATH,
    )
    .expect("active directions should load");

    assert_eq!(loaded.directions_toml.as_deref(), Some("version = 4\n"));
    assert_eq!(directions.as_deref(), Some("version = 4\n"));
}

#[test]
fn active_workspace_load_does_not_bootstrap_tracked_exports() {
    let repo = TempGitRepo::new("authority-active-no-bootstrap");
    repo.write_repo_file(DIRECTIONS_FILE_PATH, "version = 9\n");

    let loaded = SqlitePlanningAuthorityAdapter::load_active_workspace_files(
        repo.worktree_root.to_str().expect("valid worktree path"),
    )
    .expect("active workspace should load without bootstrap");
    let directions = SqlitePlanningAuthorityAdapter::load_active_planning_file(
        repo.worktree_root.to_str().expect("valid worktree path"),
        DIRECTIONS_FILE_PATH,
    )
    .expect("active directions should inspect without bootstrap");

    assert_eq!(loaded, PlanningWorkspaceLoadRecord::default());
    assert_eq!(directions, None);
    assert_eq!(
        fs::read_to_string(repo.repo_root.join(DIRECTIONS_FILE_PATH))
            .expect("tracked export should remain untouched"),
        "version = 9\n"
    );
}

#[test]
fn runtime_projection_load_does_not_bootstrap_legacy_mirror_files() {
    let repo = TempGitRepo::new("runtime-projection-no-bootstrap");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let location = adapter
        .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
        .expect("authority location should resolve");
    let mirrored_lease = ParallelModeSlotLeaseSnapshot::new(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        "akra-agent/slot-1/task-one",
        repo.worktree_root.display().to_string(),
        ParallelModeSlotLeaseState::Running,
        "2026-04-18T10:00:00Z",
        Some("2026-04-18T10:05:00Z".to_string()),
    );
    let mirrored_path = super::runtime_slot_lease_path(&location, &mirrored_lease.slot_id);
    fs::create_dir_all(
        mirrored_path
            .parent()
            .expect("runtime mirror should have a parent directory"),
    )
    .expect("runtime mirror parent should exist");
    fs::write(
        &mirrored_path,
        serde_json::to_string_pretty(&mirrored_lease).expect("mirrored lease should serialize"),
    )
    .expect("runtime mirror should write");

    let snapshot = SqlitePlanningAuthorityAdapter::load_runtime_projections(
        repo.worktree_root.to_str().expect("valid worktree path"),
    )
    .expect("runtime projections should load without bootstrap");

    assert!(snapshot.slot_leases.is_empty());
    assert!(snapshot.invalid_slot_leases.is_empty());
    assert!(snapshot.session_details.is_empty());
    assert!(snapshot.distributor_queue_records.is_empty());
}

#[test]
fn authority_open_migrates_legacy_repo_local_runtime_store() {
    let repo = TempGitRepo::new("authority-legacy-runtime-migration");
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let location = adapter
        .resolve_authority_location(repo.worktree_root.to_str().expect("valid worktree path"))
        .expect("authority location should resolve");
    assert!(
        !Path::new(&location.authority_store_path).exists(),
        "new authority store should not exist before migration"
    );
    let legacy_runtime_dir = repo.repo_root.join(".codex-exec-loop/runtime");
    fs::create_dir_all(&legacy_runtime_dir).expect("legacy runtime dir should exist");
    let legacy_store_path = legacy_runtime_dir.join("planning-authority.db");
    let legacy_connection =
        Connection::open(&legacy_store_path).expect("legacy authority store should open");
    legacy_connection
        .execute_batch(
            r#"
            CREATE TABLE authority_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE runtime_slot_leases (
                slot_id TEXT PRIMARY KEY,
                updated_at TEXT NOT NULL,
                content TEXT NOT NULL
            );
            "#,
        )
        .expect("legacy runtime schema should initialize");
    legacy_connection
        .execute(
            "INSERT INTO authority_metadata (key, value) VALUES ('schema_version', ?1)",
            [AUTHORITY_STORE_SCHEMA_VERSION.to_string()],
        )
        .expect("legacy schema version should insert");
    let legacy_lease = ParallelModeSlotLeaseSnapshot::new(
        "slot-1",
        "task-1",
        "Task One",
        "agent-1",
        "akra-agent/slot-1/task-one",
        repo.worktree_root.display().to_string(),
        ParallelModeSlotLeaseState::Running,
        "2026-04-18T10:00:00Z",
        Some("2026-04-18T10:05:00Z".to_string()),
    );
    legacy_connection
        .execute(
            "INSERT INTO runtime_slot_leases (slot_id, updated_at, content) VALUES (?1, ?2, ?3)",
            (
                legacy_lease.slot_id.as_str(),
                "2026-04-18T10:05:00Z",
                serde_json::to_string(&legacy_lease).expect("legacy lease should serialize"),
            ),
        )
        .expect("legacy runtime lease should insert");
    drop(legacy_connection);

    let snapshot = SqlitePlanningAuthorityAdapter::load_runtime_projections(
        repo.worktree_root.to_str().expect("valid worktree path"),
    )
    .expect("runtime projections should load from migrated store");

    assert!(Path::new(&location.authority_store_path).is_file());
    assert_eq!(
        snapshot
            .slot_leases
            .get("slot-1")
            .expect("legacy slot lease should migrate")
            .branch_name,
        "akra-agent/slot-1/task-one"
    );
}

#[test]
fn official_refresh_claims_enforce_reserved_execution_order() {
    let repo = TempGitRepo::new("authority-official-claims");
    let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");

    let first = SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(workspace_dir)
        .expect("first order should reserve");
    let second = SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(workspace_dir)
        .expect("second order should reserve");

    assert_eq!(first, 1);
    assert_eq!(second, 2);
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            workspace_dir,
            second,
            "owner-2",
        )
        .expect("later order claim should inspect"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            workspace_dir,
            first,
            "owner-1",
        )
        .expect("first order claim should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            workspace_dir,
            first,
            "other-owner",
        )
        .expect("contended first order claim should inspect"),
        PlanningAuthorityOfficialRefreshClaimStatus::Waiting
    );

    SqlitePlanningAuthorityAdapter::release_official_refresh_claim(workspace_dir, first, "owner-1")
        .expect("first order claim should release");

    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            workspace_dir,
            first,
            "owner-1",
        )
        .expect("completed first order claim should inspect"),
        PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted
    );
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            workspace_dir,
            second,
            "owner-2",
        )
        .expect("second order claim should acquire after release"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn distributor_queue_claims_are_unique_until_release() {
    let repo = TempGitRepo::new("authority-distributor-claims");
    let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");

    assert!(
        SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
            workspace_dir,
            "queue-item-1",
            "owner-1",
        )
        .expect("first queue claim should succeed")
    );
    assert!(
        !SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
            workspace_dir,
            "queue-item-1",
            "owner-2",
        )
        .expect("duplicate queue claim should be rejected")
    );

    SqlitePlanningAuthorityAdapter::release_distributor_queue_claim(
        workspace_dir,
        "queue-item-1",
        "owner-1",
    )
    .expect("queue claim should release");

    assert!(
        SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
            workspace_dir,
            "queue-item-1",
            "owner-2",
        )
        .expect("released queue claim should be reacquired")
    );
}

#[test]
fn official_refresh_claims_can_reclaim_stale_owner() {
    let repo = TempGitRepo::new("authority-official-stale-claim");
    let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
    let refresh_order =
        SqlitePlanningAuthorityAdapter::reserve_next_official_refresh_order(workspace_dir)
            .expect("refresh order should reserve");
    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            workspace_dir,
            refresh_order,
            "stale-owner",
        )
        .expect("initial claim should acquire"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );

    let location = SqlitePlanningAuthorityAdapter::new()
        .resolve_authority_location(workspace_dir)
        .expect("authority location should resolve");
    let connection =
        Connection::open(&location.authority_store_path).expect("authority store should open");
    connection
        .execute(
            "UPDATE runtime_claims
             SET claimed_at = '2000-01-01T00:00:00Z'
             WHERE claim_kind = 'official-refresh' AND scope_key = ?1",
            ["official-refresh"],
        )
        .expect("stale official refresh claim should update");

    assert_eq!(
        SqlitePlanningAuthorityAdapter::acquire_official_refresh_claim(
            workspace_dir,
            refresh_order,
            "fresh-owner",
        )
        .expect("stale claim should be reclaimed"),
        PlanningAuthorityOfficialRefreshClaimStatus::Acquired
    );
}

#[test]
fn distributor_queue_claims_can_reclaim_stale_owner() {
    let repo = TempGitRepo::new("authority-distributor-stale-claim");
    let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
    assert!(
        SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
            workspace_dir,
            "queue-item-stale",
            "stale-owner",
        )
        .expect("initial queue claim should acquire")
    );

    let location = SqlitePlanningAuthorityAdapter::new()
        .resolve_authority_location(workspace_dir)
        .expect("authority location should resolve");
    let connection =
        Connection::open(&location.authority_store_path).expect("authority store should open");
    connection
        .execute(
            "UPDATE runtime_claims
             SET claimed_at = '2000-01-01T00:00:00Z'
             WHERE claim_kind = ?1 AND scope_key = ?2",
            ["distributor-queue-head", "queue-item-stale"],
        )
        .expect("stale distributor claim should update");

    assert!(
        SqlitePlanningAuthorityAdapter::try_acquire_distributor_queue_claim(
            workspace_dir,
            "queue-item-stale",
            "fresh-owner",
        )
        .expect("stale queue claim should be reclaimed")
    );
}
