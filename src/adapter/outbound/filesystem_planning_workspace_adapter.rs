use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::adapter::outbound::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
    PlanningDraftStageRecord, PlanningStagedFileRecord, PlanningWorkspaceLoadRecord,
    PlanningWorkspacePort,
};
use crate::application::service::planning_contract::{
    ACTIVE_PLANNING_FILE_PATHS, DIRECTIONS_FILE_PATH, PLANNING_DRAFTS_DIRECTORY,
    PLANNING_REJECTED_DIRECTORY, QUEUE_SNAPSHOT_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
    TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
};

#[derive(Default)]
pub struct FilesystemPlanningWorkspaceAdapter;

impl FilesystemPlanningWorkspaceAdapter {
    pub fn new() -> Self {
        Self
    }

    fn draft_directory(workspace_dir: &str, draft_name: &str) -> PathBuf {
        Path::new(workspace_dir)
            .join(PLANNING_DRAFTS_DIRECTORY)
            .join(draft_name)
    }

    fn rejected_directory(workspace_dir: &str, archive_name: &str) -> PathBuf {
        Self::active_workspace_root(workspace_dir)
            .join(PLANNING_REJECTED_DIRECTORY)
            .join(archive_name)
    }

    fn active_workspace_root(workspace_dir: &str) -> PathBuf {
        SqlitePlanningAuthorityAdapter::resolve_active_workspace_root(workspace_dir)
    }

    fn active_workspace_path(workspace_dir: &str, relative_path: &str) -> PathBuf {
        Self::active_workspace_root(workspace_dir).join(relative_path)
    }

    fn read_optional_workspace_file(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        let path = Self::active_workspace_path(workspace_dir, relative_path);
        if !path.is_file() {
            return Ok(None);
        }

        fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))
            .map(Some)
    }

    fn staged_draft_file_path(
        workspace_dir: &str,
        draft_name: &str,
        active_path: &str,
    ) -> Result<PathBuf> {
        let relative_path = Self::draft_relative_path(active_path)?;
        let relative_path = Path::new(&relative_path);
        Ok(Self::draft_directory(workspace_dir, draft_name).join(relative_path))
    }

    fn draft_relative_path(active_path: &str) -> Result<String> {
        let normalized = active_path.replace('\\', "/");
        let normalized = normalized.trim_start_matches("./");
        let relative_path = normalized
            .strip_prefix(".codex-exec-loop/planning/")
            .unwrap_or(normalized);
        normalize_workspace_relative_path(
            relative_path,
            &format!("planning draft file has invalid relative path: {active_path}"),
        )
    }

    fn canonical_draft_active_path(active_path: &str) -> Result<String> {
        Ok(format!(
            ".codex-exec-loop/planning/{}",
            Self::draft_relative_path(active_path)?
        ))
    }

    fn read_all_draft_files(
        directory: &Path,
        root_directory: &Path,
        records: &mut Vec<PlanningDraftLoadFileRecord>,
    ) -> Result<()> {
        for entry in fs::read_dir(directory)
            .with_context(|| format!("failed to read {}", directory.display()))?
        {
            let entry =
                entry.with_context(|| format!("failed to inspect {}", directory.display()))?;
            let path = entry.path();
            if path.is_dir() {
                Self::read_all_draft_files(&path, root_directory, records)?;
                continue;
            }

            let relative_path = path
                .strip_prefix(root_directory)
                .with_context(|| format!("failed to strip {}", root_directory.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let active_path = format!(".codex-exec-loop/planning/{relative_path}");
            let body = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            records.push(PlanningDraftLoadFileRecord {
                active_path,
                staged_path: path.display().to_string(),
                body,
            });
        }

        Ok(())
    }

    fn ensure_parent_directory(path: &Path) -> Result<()> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))
    }

    fn draft_sort_order(active_path: &str) -> (usize, &str) {
        let order = ACTIVE_PLANNING_FILE_PATHS
            .iter()
            .position(|candidate| *candidate == active_path)
            .unwrap_or(ACTIVE_PLANNING_FILE_PATHS.len());
        (order, active_path)
    }
}

fn normalize_workspace_relative_path(path: &str, context: &str) -> Result<String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || looks_like_windows_absolute_path(&normalized)
    {
        anyhow::bail!("{context}");
    }

    for component in Path::new(&normalized).components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment.to_string_lossy();
                let trimmed_segment = segment.trim();
                if trimmed_segment == "." || trimmed_segment == ".." {
                    anyhow::bail!("{context}");
                }
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!("{context}");
            }
        }
    }

    Ok(normalized)
}

fn looks_like_windows_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

impl PlanningWorkspacePort for FilesystemPlanningWorkspaceAdapter {
    fn stage_planning_draft_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord> {
        let canonical_files = files
            .iter()
            .map(|file| {
                Ok(PlanningDraftFileRecord {
                    active_path: Self::canonical_draft_active_path(&file.active_path)?,
                    body: file.body.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if SqlitePlanningAuthorityAdapter::is_git_backed_workspace(workspace_dir) {
            return SqlitePlanningAuthorityAdapter::stage_repo_scoped_draft_files(
                workspace_dir,
                draft_name,
                &canonical_files,
            );
        }

        let draft_directory = Self::draft_directory(workspace_dir, draft_name);
        fs::create_dir_all(&draft_directory)
            .with_context(|| format!("failed to create {}", draft_directory.display()))?;

        let staged_files = canonical_files
            .iter()
            .map(|file| {
                let staged_path =
                    Self::staged_draft_file_path(workspace_dir, draft_name, &file.active_path)?;
                Self::ensure_parent_directory(&staged_path)?;
                fs::write(&staged_path, &file.body)
                    .with_context(|| format!("failed to write {}", staged_path.display()))?;

                Ok(PlanningStagedFileRecord {
                    active_path: file.active_path.clone(),
                    staged_path: staged_path.display().to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(PlanningDraftStageRecord {
            draft_name: draft_name.to_string(),
            draft_directory: draft_directory.display().to_string(),
            staged_files,
        })
    }

    fn load_planning_draft_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord> {
        if SqlitePlanningAuthorityAdapter::is_git_backed_workspace(workspace_dir) {
            let mut loaded = SqlitePlanningAuthorityAdapter::load_repo_scoped_draft_files(
                workspace_dir,
                draft_name,
            )?;
            loaded.staged_files.sort_by(|left, right| {
                Self::draft_sort_order(&left.active_path)
                    .cmp(&Self::draft_sort_order(&right.active_path))
            });
            return Ok(loaded);
        }

        let draft_directory = Self::draft_directory(workspace_dir, draft_name);
        let mut staged_files = Vec::new();
        Self::read_all_draft_files(&draft_directory, &draft_directory, &mut staged_files)?;
        staged_files.sort_by(|left, right| {
            Self::draft_sort_order(&left.active_path)
                .cmp(&Self::draft_sort_order(&right.active_path))
        });

        Ok(PlanningDraftLoadRecord {
            draft_name: draft_name.to_string(),
            draft_directory: draft_directory.display().to_string(),
            staged_files,
        })
    }

    fn replace_planning_draft_file(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        active_path: &str,
        body: &str,
    ) -> Result<String> {
        let active_path = Self::canonical_draft_active_path(active_path)?;
        if SqlitePlanningAuthorityAdapter::is_git_backed_workspace(workspace_dir) {
            return SqlitePlanningAuthorityAdapter::replace_repo_scoped_draft_file(
                workspace_dir,
                draft_name,
                &active_path,
                body,
            );
        }

        let staged_path = Self::staged_draft_file_path(workspace_dir, draft_name, &active_path)?;
        Self::ensure_parent_directory(&staged_path)?;
        fs::write(&staged_path, body)
            .with_context(|| format!("failed to write {}", staged_path.display()))?;
        Ok(staged_path.display().to_string())
    }

    fn load_planning_workspace_files(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        Ok(PlanningWorkspaceLoadRecord {
            directions_toml: Self::read_optional_workspace_file(
                workspace_dir,
                DIRECTIONS_FILE_PATH,
            )?,
            task_ledger_json: Self::read_optional_workspace_file(
                workspace_dir,
                TASK_LEDGER_FILE_PATH,
            )?,
            task_ledger_schema_json: Self::read_optional_workspace_file(
                workspace_dir,
                TASK_LEDGER_SCHEMA_FILE_PATH,
            )?,
            queue_snapshot_json: Self::read_optional_workspace_file(
                workspace_dir,
                QUEUE_SNAPSHOT_FILE_PATH,
            )?,
            result_output_markdown: Self::read_optional_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
            )?,
        })
    }

    fn load_optional_planning_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        Self::read_optional_workspace_file(workspace_dir, &relative_path)
    }

    fn replace_planning_workspace_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()> {
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        let path = Self::active_workspace_path(workspace_dir, &relative_path);
        match body {
            Some(body) => {
                Self::ensure_parent_directory(&path)?;
                fs::write(&path, body)
                    .with_context(|| format!("failed to write {}", path.display()))?;
            }
            None => {
                if path.exists() {
                    fs::remove_file(&path)
                        .with_context(|| format!("failed to remove {}", path.display()))?;
                }
            }
        }

        Ok(())
    }

    fn remove_planning_workspace_entry(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<()> {
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        let path = Self::active_workspace_path(workspace_dir, &relative_path);
        if !path.exists() {
            return Ok(());
        }

        if path.is_dir() {
            fs::remove_dir_all(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        } else {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
        }

        Ok(())
    }

    fn archive_rejected_planning_file(
        &self,
        workspace_dir: &str,
        archive_name: &str,
        active_path: &str,
        body: &str,
    ) -> Result<String> {
        let archive_directory = Self::rejected_directory(workspace_dir, archive_name);
        fs::create_dir_all(&archive_directory)
            .with_context(|| format!("failed to create {}", archive_directory.display()))?;

        let file_name = Path::new(active_path)
            .file_name()
            .with_context(|| format!("planning file has no file name: {active_path}"))?;
        let archived_path = archive_directory.join(file_name);
        fs::write(&archived_path, body)
            .with_context(|| format!("failed to write {}", archived_path.display()))?;

        Ok(archived_path.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning_contract::{
        DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    };

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    struct TempGitRepo {
        root: PathBuf,
        repo_root: PathBuf,
        worktree_root: PathBuf,
    }

    impl TempGitRepo {
        fn new(label: &str) -> Self {
            let root = PathBuf::from(create_temp_workspace(label));
            let repo_root = root.join("repo");
            let worktree_root = root.join("worktrees").join("linked");
            fs::create_dir_all(&repo_root).expect("temp repo root should be created");
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
            .expect("worktree parent should be created");
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
                fs::create_dir_all(parent).expect("repo file parent should exist");
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
    fn stages_planning_files_under_drafts_directory() {
        let workspace_dir = create_temp_workspace("planning-workspace-adapter");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let result = adapter
            .stage_planning_draft_files(
                &workspace_dir,
                "bootstrap-20260409T120000Z",
                &[
                    PlanningDraftFileRecord {
                        active_path: ".codex-exec-loop/planning/directions.toml".to_string(),
                        body: "version = 1".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: ".codex-exec-loop/planning/task-ledger.json".to_string(),
                        body: "{\"version\":1,\"tasks\":[]}".to_string(),
                    },
                ],
            )
            .expect("planning draft files should stage");

        assert!(
            result
                .draft_directory
                .contains(".codex-exec-loop/planning/drafts/bootstrap-20260409T120000Z")
        );
        assert_eq!(result.staged_files.len(), 2);
        for staged_file in result.staged_files {
            assert!(Path::new(&staged_file.staged_path).exists());
        }

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn loads_active_planning_workspace_files_when_present() {
        let workspace_dir = create_temp_workspace("planning-workspace-load");
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        fs::write(planning_dir.join("directions.toml"), "version = 1")
            .expect("directions should write");
        fs::write(
            planning_dir.join("task-ledger.json"),
            "{\"version\":1,\"tasks\":[]}",
        )
        .expect("task ledger should write");
        fs::write(
            planning_dir.join("task-ledger.schema.json"),
            "{\"type\":\"object\"}",
        )
        .expect("schema should write");
        fs::write(
            planning_dir.join("queue.snapshot.json"),
            "{\"next_task\":null}",
        )
        .expect("queue snapshot should write");
        fs::write(planning_dir.join("result-output.md"), "# result")
            .expect("result output should write");

        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        let result = adapter
            .load_planning_workspace_files(&workspace_dir)
            .expect("planning workspace files should load");

        assert_eq!(result.directions_toml.as_deref(), Some("version = 1"));
        assert_eq!(
            result.task_ledger_json.as_deref(),
            Some("{\"version\":1,\"tasks\":[]}")
        );
        assert_eq!(
            result.task_ledger_schema_json.as_deref(),
            Some("{\"type\":\"object\"}")
        );
        assert_eq!(
            result.queue_snapshot_json.as_deref(),
            Some("{\"next_task\":null}")
        );
        assert_eq!(result.result_output_markdown.as_deref(), Some("# result"));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn active_workspace_files_resolve_to_canonical_repo_root_for_linked_worktrees() {
        let repo = TempGitRepo::new("planning-workspace-linked-root");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        repo.write_repo_file(DIRECTIONS_FILE_PATH, "version = 1\n");

        let linked_worktree_path = repo.worktree_root.join(DIRECTIONS_FILE_PATH);
        fs::create_dir_all(
            linked_worktree_path
                .parent()
                .expect("linked worktree directions should have a parent directory"),
        )
        .expect("linked worktree planning directory should exist");
        fs::write(&linked_worktree_path, "version = 0\n")
            .expect("linked worktree file should diverge");

        let body = adapter
            .load_optional_planning_file(
                repo.worktree_root.to_str().expect("valid worktree path"),
                DIRECTIONS_FILE_PATH,
            )
            .expect("canonical directions should load")
            .expect("directions.toml should exist");

        assert_eq!(body, "version = 1\n");
    }

    #[test]
    fn git_backed_draft_stage_uses_repo_scoped_authority_store() {
        let repo = TempGitRepo::new("planning-draft-linked-stage");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let stage = adapter
            .stage_planning_draft_files(
                repo.worktree_root.to_str().expect("valid worktree path"),
                "bootstrap-20260410T120000Z",
                &[
                    PlanningDraftFileRecord {
                        active_path: DIRECTIONS_FILE_PATH.to_string(),
                        body: "version = 1".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: TASK_LEDGER_FILE_PATH.to_string(),
                        body: "{\"version\":1,\"tasks\":[]}".to_string(),
                    },
                ],
            )
            .expect("git-backed draft should stage");

        assert!(
            stage
                .draft_directory
                .contains("planning-authority.db#drafts/")
        );
        assert!(
            !repo
                .worktree_root
                .join(".codex-exec-loop/planning/drafts/bootstrap-20260410T120000Z")
                .exists()
        );

        let loaded = adapter
            .load_planning_draft_files(
                repo.repo_root.to_str().expect("valid repo path"),
                "bootstrap-20260410T120000Z",
            )
            .expect("repo-scoped draft should load from canonical root");

        assert_eq!(loaded.staged_files.len(), 2);
        assert_eq!(loaded.staged_files[0].active_path, DIRECTIONS_FILE_PATH);
        assert_eq!(loaded.staged_files[0].body, "version = 1");
        assert!(
            loaded.staged_files[0]
                .staged_path
                .contains("planning-authority.db#drafts/")
        );
    }

    #[test]
    fn git_backed_draft_replace_is_visible_across_worktree_boundaries() {
        let repo = TempGitRepo::new("planning-draft-linked-replace");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        let draft_name = "bootstrap-20260410T120000Z";
        adapter
            .stage_planning_draft_files(
                repo.worktree_root.to_str().expect("valid worktree path"),
                draft_name,
                &[PlanningDraftFileRecord {
                    active_path: DIRECTIONS_FILE_PATH.to_string(),
                    body: "version = 1".to_string(),
                }],
            )
            .expect("git-backed draft should stage");

        let staged_path = adapter
            .replace_planning_draft_file(
                repo.repo_root.to_str().expect("valid repo path"),
                draft_name,
                DIRECTIONS_FILE_PATH,
                "version = 2",
            )
            .expect("repo-scoped staged draft should update");

        assert!(staged_path.contains("planning-authority.db#drafts/"));

        let loaded = adapter
            .load_planning_draft_files(
                repo.worktree_root.to_str().expect("valid worktree path"),
                draft_name,
            )
            .expect("updated repo-scoped draft should load");

        assert_eq!(loaded.staged_files.len(), 1);
        assert_eq!(loaded.staged_files[0].body, "version = 2");
    }

    #[test]
    fn loads_staged_planning_draft_files_by_active_contract_path() {
        let workspace_dir = create_temp_workspace("planning-draft-load");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        adapter
            .stage_planning_draft_files(
                &workspace_dir,
                "bootstrap-20260410T120000Z",
                &[
                    PlanningDraftFileRecord {
                        active_path: DIRECTIONS_FILE_PATH.to_string(),
                        body: "version = 1".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: TASK_LEDGER_FILE_PATH.to_string(),
                        body: "{\"version\":1,\"tasks\":[]}".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: ".codex-exec-loop/planning/task-ledger.schema.json"
                            .to_string(),
                        body: "{\"type\":\"object\"}".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                        body: "# result".to_string(),
                    },
                ],
            )
            .expect("draft files should stage");

        let loaded = adapter
            .load_planning_draft_files(&workspace_dir, "bootstrap-20260410T120000Z")
            .expect("draft files should load");

        assert_eq!(loaded.staged_files.len(), 4);
        assert_eq!(loaded.staged_files[0].active_path, DIRECTIONS_FILE_PATH);
        assert_eq!(loaded.staged_files[1].active_path, TASK_LEDGER_FILE_PATH);

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn replace_planning_draft_file_updates_existing_staged_file() {
        let workspace_dir = create_temp_workspace("planning-draft-replace");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        adapter
            .stage_planning_draft_files(
                &workspace_dir,
                "bootstrap-20260410T120000Z",
                &[
                    PlanningDraftFileRecord {
                        active_path: DIRECTIONS_FILE_PATH.to_string(),
                        body: "version = 1".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: TASK_LEDGER_FILE_PATH.to_string(),
                        body: "{\"version\":1,\"tasks\":[]}".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: ".codex-exec-loop/planning/task-ledger.schema.json"
                            .to_string(),
                        body: "{\"type\":\"object\"}".to_string(),
                    },
                    PlanningDraftFileRecord {
                        active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                        body: "# result".to_string(),
                    },
                ],
            )
            .expect("draft files should stage");

        let staged_path = adapter
            .replace_planning_draft_file(
                &workspace_dir,
                "bootstrap-20260410T120000Z",
                DIRECTIONS_FILE_PATH,
                "version = 2",
            )
            .expect("staged draft directions should update");

        assert_eq!(
            fs::read_to_string(staged_path).expect("updated staged file should read"),
            "version = 2"
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn rejects_absolute_draft_paths_outside_planning_workspace() {
        let workspace_dir = create_temp_workspace("planning-draft-invalid-absolute");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let error = adapter
            .stage_planning_draft_files(
                &workspace_dir,
                "bootstrap-20260410T120000Z",
                &[PlanningDraftFileRecord {
                    active_path: "/tmp/escape.txt".to_string(),
                    body: "escape".to_string(),
                }],
            )
            .expect_err("absolute draft path should be rejected");

        assert!(error.to_string().contains("invalid relative path"));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn rejects_parent_traversal_when_loading_optional_planning_files() {
        let workspace_dir = create_temp_workspace("planning-load-invalid-parent");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let error = adapter
            .load_optional_planning_file(&workspace_dir, "../secret.md")
            .expect_err("parent traversal should be rejected");

        assert!(error.to_string().contains("invalid planning relative path"));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn trims_relative_paths_before_loading_optional_planning_files() {
        let workspace_dir = create_temp_workspace("planning-load-trimmed");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        fs::write(planning_dir.join("directions.toml"), "version = 1")
            .expect("directions should write");

        let body = adapter
            .load_optional_planning_file(
                &workspace_dir,
                "  .codex-exec-loop/planning/directions.toml  ",
            )
            .expect("trimmed relative path should load")
            .expect("directions.toml should exist");

        assert_eq!(body, "version = 1");

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn rejects_parent_traversal_components_with_trailing_whitespace() {
        let workspace_dir = create_temp_workspace("planning-load-invalid-parent-whitespace");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let error = adapter
            .load_optional_planning_file(&workspace_dir, ".codex-exec-loop/planning/.. /secret.md")
            .expect_err("trimmed parent traversal component should be rejected");

        assert!(error.to_string().contains("invalid planning relative path"));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn missing_active_planning_workspace_files_return_none_fields() {
        let workspace_dir = create_temp_workspace("planning-workspace-empty");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let result = adapter
            .load_planning_workspace_files(&workspace_dir)
            .expect("missing planning workspace files should still load");

        assert!(result.directions_toml.is_none());
        assert!(result.task_ledger_json.is_none());
        assert!(result.task_ledger_schema_json.is_none());
        assert!(result.queue_snapshot_json.is_none());
        assert!(result.result_output_markdown.is_none());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn planning_workspace_loader_ignores_directory_entries_for_expected_files() {
        let workspace_dir = create_temp_workspace("planning-workspace-directory-entry");
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(planning_dir.join("directions.toml"))
            .expect("directory entry should be created");

        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        let result = adapter
            .load_planning_workspace_files(&workspace_dir)
            .expect("directory entries should not fail planning workspace load");

        assert!(result.directions_toml.is_none());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn replace_planning_workspace_file_writes_and_removes_files() {
        let workspace_dir = create_temp_workspace("planning-workspace-replace");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        let directions_path =
            Path::new(&workspace_dir).join(".codex-exec-loop/planning/directions.toml");

        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                DIRECTIONS_FILE_PATH,
                Some("version = 1"),
            )
            .expect("directions should write");
        assert_eq!(
            fs::read_to_string(&directions_path).expect("written directions should read"),
            "version = 1"
        );

        adapter
            .replace_planning_workspace_file(&workspace_dir, DIRECTIONS_FILE_PATH, None)
            .expect("directions should remove");
        assert!(!directions_path.exists());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn archive_rejected_planning_file_writes_copy_under_rejected_directory() {
        let workspace_dir = create_temp_workspace("planning-workspace-rejected");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let archived_path = adapter
            .archive_rejected_planning_file(
                &workspace_dir,
                "turn-1",
                TASK_LEDGER_FILE_PATH,
                "{\"version\":1}",
            )
            .expect("rejected planning file should archive");

        assert!(archived_path.contains(".codex-exec-loop/planning/rejected/turn-1"));
        assert_eq!(
            fs::read_to_string(&archived_path).expect("archived file should read"),
            "{\"version\":1}"
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn remove_planning_workspace_entry_removes_file_and_directory_tree() {
        let workspace_dir = create_temp_workspace("planning-workspace-remove-entry");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        let prompt_path = Path::new(&workspace_dir)
            .join(".codex-exec-loop/planning/prompts/queue-idle-review.md");
        fs::create_dir_all(
            prompt_path
                .parent()
                .expect("prompt path should have a parent directory"),
        )
        .expect("prompt directory should be created");
        fs::write(&prompt_path, "# prompt").expect("prompt file should write");
        let directions_path = Path::new(&workspace_dir).join(DIRECTIONS_FILE_PATH);
        fs::create_dir_all(
            directions_path
                .parent()
                .expect("directions path should have a parent directory"),
        )
        .expect("directions parent should be created");
        fs::write(&directions_path, "version = 1").expect("directions file should write");

        adapter
            .remove_planning_workspace_entry(&workspace_dir, ".codex-exec-loop/planning/prompts")
            .expect("prompt directory should be removable");
        adapter
            .remove_planning_workspace_entry(&workspace_dir, DIRECTIONS_FILE_PATH)
            .expect("directions file should be removable");

        assert!(!prompt_path.exists());
        assert!(
            !Path::new(&workspace_dir)
                .join(".codex-exec-loop/planning/prompts")
                .exists()
        );
        assert!(!directions_path.exists());

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
