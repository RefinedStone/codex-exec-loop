use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::service::planning_contract::{
    ACTIVE_PLANNING_FILE_PATHS, PLAN_OFF_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

const RUNTIME_DIRECTORY: &str = ".codex-exec-loop/runtime";
const AUTHORITY_STORE_FILE_NAME: &str = "planning-authority.db";
const SHADOW_STORE_SCHEMA_VERSION: i64 = 1;
const SHADOW_STORE_MODE: &str = "shadow-store";

#[derive(Default)]
pub struct SqlitePlanningAuthorityAdapter;

impl SqlitePlanningAuthorityAdapter {
    pub fn new() -> Self {
        Self
    }

    pub(crate) fn resolve_active_workspace_root(workspace_dir: &str) -> PathBuf {
        Self::resolve_authority_location_from_workspace(workspace_dir)
            .map(|location| PathBuf::from(location.canonical_repo_root))
            .unwrap_or_else(|_| canonicalize_best_effort(Path::new(workspace_dir)))
    }

    fn resolve_authority_location_from_workspace(
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityLocation> {
        let workspace_root = canonicalize_best_effort(Path::new(workspace_dir));
        let canonical_repo_root =
            resolve_canonical_repo_root(workspace_dir).unwrap_or_else(|| workspace_root.clone());
        let runtime_dir = canonical_repo_root.join(RUNTIME_DIRECTORY);
        let authority_store_path = runtime_dir.join(AUTHORITY_STORE_FILE_NAME);

        Ok(PlanningAuthorityLocation {
            workspace_root: workspace_root.display().to_string(),
            canonical_repo_root: canonical_repo_root.display().to_string(),
            runtime_dir: runtime_dir.display().to_string(),
            authority_store_path: authority_store_path.display().to_string(),
        })
    }

    fn collect_authority_documents(canonical_repo_root: &Path) -> Result<BTreeMap<String, String>> {
        let mut documents = BTreeMap::new();
        for relative_path in ACTIVE_PLANNING_FILE_PATHS
            .iter()
            .copied()
            .chain(std::iter::once(PLAN_OFF_FILE_PATH))
        {
            let path = canonical_repo_root.join(relative_path);
            if !path.is_file() {
                continue;
            }
            let body = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            documents.insert(relative_path.to_string(), body);
        }

        collect_directory_documents(
            canonical_repo_root,
            PLANNING_DIRECTION_DOCS_DIRECTORY,
            &mut documents,
        )?;
        collect_directory_documents(
            canonical_repo_root,
            PLANNING_PROMPTS_DIRECTORY,
            &mut documents,
        )?;

        Ok(documents)
    }

    fn inspect_shadow_store_impl(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        let location = self.resolve_authority_location(workspace_dir)?;
        let canonical_repo_root = PathBuf::from(&location.canonical_repo_root);
        let authority_store_path = PathBuf::from(&location.authority_store_path);
        let source_documents = Self::collect_authority_documents(&canonical_repo_root)?;
        let had_store = authority_store_path.is_file();
        let previous_documents = load_shadow_documents(&authority_store_path)?;
        let parity_issues = compare_documents(&source_documents, &previous_documents);

        if let Some(parent) = authority_store_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }

        let mut connection = Connection::open(&authority_store_path)
            .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
        ensure_schema(&connection)?;
        store_shadow_documents(&mut connection, &location, &source_documents)?;

        let mirrored_documents = load_shadow_documents(&authority_store_path)?;
        let post_sync_issues = compare_documents(&source_documents, &mirrored_documents);
        if !post_sync_issues.is_empty() {
            let summary = post_sync_issues.join(", ");
            return Err(anyhow!(
                "shadow store parity check failed after sync: {summary}"
            ));
        }

        let sync_state = if !had_store {
            PlanningAuthorityShadowStoreSyncState::Bootstrapped
        } else if parity_issues.is_empty() {
            PlanningAuthorityShadowStoreSyncState::InSync
        } else {
            PlanningAuthorityShadowStoreSyncState::Resynced
        };

        Ok(PlanningAuthorityShadowStoreInspection {
            location,
            sync_state,
            mirrored_document_count: source_documents.len(),
            parity_issue_count: parity_issues.len(),
            parity_issue_examples: parity_issues.into_iter().take(3).collect(),
        })
    }
}

impl PlanningAuthorityPort for SqlitePlanningAuthorityAdapter {
    fn resolve_authority_location(&self, workspace_dir: &str) -> Result<PlanningAuthorityLocation> {
        Self::resolve_authority_location_from_workspace(workspace_dir)
    }

    fn inspect_shadow_store(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthorityShadowStoreInspection> {
        self.inspect_shadow_store_impl(workspace_dir)
    }
}

fn ensure_schema(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS authority_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS shadow_documents (
                relative_path TEXT PRIMARY KEY,
                content TEXT NOT NULL
            );
            "#,
        )
        .context("failed to initialize shadow-store schema")?;
    Ok(())
}

fn store_shadow_documents(
    connection: &mut Connection,
    location: &PlanningAuthorityLocation,
    documents: &BTreeMap<String, String>,
) -> Result<()> {
    let transaction = connection
        .transaction()
        .context("failed to open shadow-store transaction")?;
    transaction
        .execute("DELETE FROM shadow_documents", [])
        .context("failed to clear shadow documents")?;
    for (relative_path, content) in documents {
        transaction
            .execute(
                "INSERT INTO shadow_documents (relative_path, content) VALUES (?1, ?2)",
                params![relative_path, content],
            )
            .with_context(|| format!("failed to mirror `{relative_path}` into the shadow store"))?;
    }

    upsert_metadata(
        &transaction,
        "schema_version",
        &SHADOW_STORE_SCHEMA_VERSION.to_string(),
    )?;
    upsert_metadata(&transaction, "mode", SHADOW_STORE_MODE)?;
    upsert_metadata(
        &transaction,
        "canonical_repo_root",
        &location.canonical_repo_root,
    )?;
    upsert_metadata(&transaction, "workspace_root", &location.workspace_root)?;
    upsert_metadata(&transaction, "last_synced_at", &Utc::now().to_rfc3339())?;

    transaction
        .commit()
        .context("failed to commit shadow-store transaction")?;
    Ok(())
}

fn upsert_metadata(transaction: &rusqlite::Transaction<'_>, key: &str, value: &str) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO authority_metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .with_context(|| format!("failed to update authority metadata `{key}`"))?;
    Ok(())
}

fn load_shadow_documents(authority_store_path: &Path) -> Result<BTreeMap<String, String>> {
    if !authority_store_path.is_file() {
        return Ok(BTreeMap::new());
    }

    let connection = Connection::open(authority_store_path)
        .with_context(|| format!("failed to open {}", authority_store_path.display()))?;
    let schema_version = connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("failed to read shadow-store schema version")?;
    if let Some(schema_version) = schema_version {
        let parsed = schema_version.parse::<i64>().ok();
        if parsed != Some(SHADOW_STORE_SCHEMA_VERSION) {
            return Err(anyhow!(
                "unsupported shadow-store schema version: {schema_version}"
            ));
        }
    }

    let mut statement = connection
        .prepare("SELECT relative_path, content FROM shadow_documents ORDER BY relative_path")
        .context("failed to read shadow-store documents")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("failed to iterate shadow-store documents")?;
    let mut documents = BTreeMap::new();
    for row in rows {
        let (relative_path, content) = row.context("failed to decode shadow-store row")?;
        documents.insert(relative_path, content);
    }

    Ok(documents)
}

fn collect_directory_documents(
    workspace_root: &Path,
    relative_directory: &str,
    documents: &mut BTreeMap<String, String>,
) -> Result<()> {
    let directory = workspace_root.join(relative_directory);
    if !directory.is_dir() {
        return Ok(());
    }

    let mut stack = vec![directory];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)
            .with_context(|| format!("failed to read {}", current.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to inspect {}", current.display()))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            let relative_path = path
                .strip_prefix(workspace_root)
                .with_context(|| format!("failed to strip {}", workspace_root.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let body = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            documents.insert(relative_path, body);
        }
    }

    Ok(())
}

fn compare_documents(
    source_documents: &BTreeMap<String, String>,
    mirrored_documents: &BTreeMap<String, String>,
) -> Vec<String> {
    let document_paths = source_documents
        .keys()
        .chain(mirrored_documents.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut issues = Vec::new();
    for relative_path in document_paths {
        match (
            source_documents.get(&relative_path),
            mirrored_documents.get(&relative_path),
        ) {
            (Some(_), None) => {
                issues.push(format!("{relative_path}: missing from shadow store"));
            }
            (None, Some(_)) => {
                issues.push(format!(
                    "{relative_path}: shadow store contains stale content"
                ));
            }
            (Some(source), Some(mirrored)) if source != mirrored => {
                issues.push(format!("{relative_path}: content mismatch"));
            }
            _ => {}
        }
    }

    issues
}

fn resolve_canonical_repo_root(workspace_dir: &str) -> Option<PathBuf> {
    let common_dir = git_stdout(workspace_dir, &["rev-parse", "--git-common-dir"])?;
    let common_dir_path = absolutize_path(Path::new(workspace_dir), Path::new(&common_dir));
    let canonical_common_dir = canonicalize_best_effort(&common_dir_path);
    canonical_common_dir.parent().map(Path::to_path_buf)
}

fn git_stdout(workspace_dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(workspace_dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

fn absolutize_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    base.join(path)
}

fn canonicalize_best_effort(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::SqlitePlanningAuthorityAdapter;
    use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
    use crate::domain::planning::PlanningAuthorityShadowStoreSyncState;

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
        assert!(location.runtime_dir.ends_with(".codex-exec-loop/runtime"));
        assert!(
            location
                .authority_store_path
                .ends_with(".codex-exec-loop/runtime/planning-authority.db")
        );
    }

    #[test]
    fn inspect_shadow_store_bootstraps_from_canonical_repo_root() {
        let repo = TempGitRepo::new("shadow-bootstrap");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 1\n");
        repo.write_repo_file(
            ".codex-exec-loop/planning/task-ledger.json",
            "{\"version\":1,\"tasks\":[]}\n",
        );
        repo.write_repo_file(
            ".codex-exec-loop/planning/prompts/queue-idle-review.md",
            "# review\n",
        );

        let inspection = adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect("shadow store should inspect");

        assert_eq!(
            inspection.sync_state,
            PlanningAuthorityShadowStoreSyncState::Bootstrapped
        );
        assert_eq!(inspection.mirrored_document_count, 3);
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
    fn inspect_shadow_store_reports_resynced_when_source_changes() {
        let repo = TempGitRepo::new("shadow-resync");
        let adapter = SqlitePlanningAuthorityAdapter::new();
        repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 1\n");

        adapter
            .inspect_shadow_store(repo.worktree_root.to_str().expect("valid path"))
            .expect("initial shadow store sync should succeed");

        repo.write_repo_file(".codex-exec-loop/planning/directions.toml", "version = 2\n");

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
                .any(|issue| issue.contains("directions.toml"))
        );
    }
}
