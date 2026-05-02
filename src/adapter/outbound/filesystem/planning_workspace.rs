use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
    PlanningDraftStageRecord, PlanningStagedFileRecord, PlanningWorkspaceLoadRecord,
    PlanningWorkspacePort, RepoScopedPlanningWorkspacePort,
};
use crate::application::service::planning::{
    ACTIVE_PLANNING_FILE_PATHS, PLANNING_DRAFTS_DIRECTORY, PLANNING_REJECTED_DIRECTORY,
    RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
};

/*
 * FilesystemPlanningWorkspaceAdapter is the fallback outbound adapter for planning workspace files.
 * It still knows about repo-scoped stores because git-backed worktrees keep the authoritative planning files
 * somewhere other than the candidate workspace directory. The adapter therefore routes active-state reads/writes
 * through RepoScopedPlanningWorkspacePort when possible, while preserving direct filesystem behavior for plain dirs.
 */
#[derive(Default)]
pub struct FilesystemPlanningWorkspaceAdapter {
    // None means legacy direct-filesystem mode; Some means git-backed workspaces can resolve an active workspace root.
    repo_scoped_store: Option<Arc<dyn RepoScopedPlanningWorkspacePort>>,
}

impl FilesystemPlanningWorkspaceAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_repo_scoped_store(
        repo_scoped_store: Arc<dyn RepoScopedPlanningWorkspacePort>,
    ) -> Self {
        Self {
            repo_scoped_store: Some(repo_scoped_store),
        }
    }

    fn repo_scoped_store(
        &self,
        workspace_dir: &str,
    ) -> Option<&dyn RepoScopedPlanningWorkspacePort> {
        // The store is only valid for git-backed workspaces; non-git temp fixtures must keep using direct paths.
        self.repo_scoped_store
            .as_deref()
            .filter(|store| store.is_git_backed_workspace(workspace_dir))
    }

    fn draft_directory(workspace_dir: &str, draft_name: &str) -> PathBuf {
        // Drafts are staged under the candidate workspace so an operator can inspect/reject them before promotion.
        Path::new(workspace_dir)
            .join(PLANNING_DRAFTS_DIRECTORY)
            .join(draft_name)
    }

    fn rejected_directory(&self, workspace_dir: &str, archive_name: &str) -> PathBuf {
        // Rejected archives belong to the active workspace root, not necessarily the candidate worktree path.
        self.active_workspace_root(workspace_dir)
            .join(PLANNING_REJECTED_DIRECTORY)
            .join(archive_name)
    }

    fn active_workspace_root(&self, workspace_dir: &str) -> PathBuf {
        // Repo-scoped mode lets a parallel slot write planning state to the integration checkout authority.
        self.repo_scoped_store
            .as_ref()
            .map(|store| store.resolve_active_workspace_root(workspace_dir))
            .unwrap_or_else(|| Path::new(workspace_dir).to_path_buf())
    }

    fn active_workspace_path(&self, workspace_dir: &str, relative_path: &str) -> PathBuf {
        // Active paths are used for committed planning state such as result output.
        self.active_workspace_root(workspace_dir)
            .join(relative_path)
    }

    fn candidate_workspace_path(workspace_dir: &str, relative_path: &str) -> PathBuf {
        // Candidate paths read the slot/worktree copy without consulting repo-scoped authority.
        Path::new(workspace_dir).join(relative_path)
    }

    fn read_optional_workspace_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        let path = self.active_workspace_path(workspace_dir, relative_path);
        if !path.is_file() {
            return Ok(None);
        }

        fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))
            .map(Some)
    }

    fn read_optional_candidate_workspace_file(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        let path = Self::candidate_workspace_path(workspace_dir, relative_path);
        if !path.is_file() {
            return Ok(None);
        }

        fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))
            .map(Some)
    }

    fn load_workspace_record_from(
        workspace_dir: &str,
        file_loader: impl Fn(&str, &str) -> Result<Option<String>>,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        // The workspace record intentionally excludes DB-backed task authority artifacts; this adapter only moves prompt files.
        Ok(PlanningWorkspaceLoadRecord {
            result_output_markdown: file_loader(workspace_dir, RESULT_OUTPUT_FILE_PATH)?,
        })
    }

    fn commit_workspace_record_to_filesystem(
        workspace_root: &Path,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        // Commit mirrors the load record shape: None removes stale files, Some writes the latest prompt body.
        write_optional_workspace_file(
            workspace_root,
            RESULT_OUTPUT_FILE_PATH,
            record.result_output_markdown.as_deref(),
        )?;
        Ok(())
    }

    fn authority_managed_path(relative_path: &str) -> bool {
        // Canonical active planning files are considered authoritative even when repo-scoped storage returns None.
        canonical_active_planning_file_path(relative_path).is_some()
    }

    fn staged_draft_file_path(
        workspace_dir: &str,
        draft_name: &str,
        active_path: &str,
    ) -> Result<PathBuf> {
        // Draft paths drop the `.codex-exec-loop/planning` prefix so staged trees are compact and movable.
        let relative_path = Self::draft_relative_path(active_path)?;
        let relative_path = Path::new(&relative_path);
        Ok(Self::draft_directory(workspace_dir, draft_name).join(relative_path))
    }

    fn draft_relative_path(active_path: &str) -> Result<String> {
        // Draft input may already be canonical or may be a short planning-relative path; both normalize to one safe path.
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
        // Promotion code expects active paths to be in the canonical planning namespace.
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
        // Recursive draft loading reconstructs active paths from staged file locations for review/promotion UI.
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
        // Known planning files appear first in a stable semantic order; extra files sort after them by path.
        let order = ACTIVE_PLANNING_FILE_PATHS
            .iter()
            .position(|candidate| *candidate == active_path)
            .unwrap_or(ACTIVE_PLANNING_FILE_PATHS.len());
        (order, active_path)
    }
}

fn normalize_workspace_relative_path(path: &str, context: &str) -> Result<String> {
    /*
     * Every externally supplied planning path passes through this guard before touching the filesystem.
     * It rejects absolute paths, Windows drive roots, and parent traversal so draft/replace/remove operations
     * cannot escape the selected workspace root.
     */
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
        /*
         * Staging writes proposed planning files under a draft namespace instead of mutating active authority.
         * Repo-scoped stores implement the same contract for git-backed slots; direct mode writes staged files
         * into `.codex-exec-loop/planning/drafts/<draft_name>`.
         */
        let canonical_files = files
            .iter()
            .map(|file| {
                Ok(PlanningDraftFileRecord {
                    active_path: Self::canonical_draft_active_path(&file.active_path)?,
                    body: file.body.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            return store.stage_repo_scoped_draft_files(
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
        // Loading drafts uses the same sort order for repo-scoped and direct filesystem modes so UI ordering stays stable.
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            let mut loaded = store.load_repo_scoped_draft_files(workspace_dir, draft_name)?;
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
        // Draft replacement edits staged proposal content only; active planning files are untouched until promotion.
        let active_path = Self::canonical_draft_active_path(active_path)?;
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            return store.replace_repo_scoped_draft_file(
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
        // Active workspace load is authority-aware: git-backed workspaces read through the repo-scoped store first.
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            return store.load_active_workspace_files(workspace_dir);
        }
        Self::load_workspace_record_from(workspace_dir, |workspace_dir, relative_path| {
            self.read_optional_workspace_file(workspace_dir, relative_path)
        })
    }

    fn load_planning_workspace_candidate_files(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        // Candidate load deliberately ignores repo-scoped authority so comparison code can inspect the slot copy.
        Self::load_workspace_record_from(
            workspace_dir,
            Self::read_optional_candidate_workspace_file,
        )
    }

    fn commit_planning_workspace_files(
        &self,
        workspace_dir: &str,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        // Commit writes active prompt files to the authority location, delegating to repo-scoped storage when present.
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            return store.commit_active_workspace_files(workspace_dir, record);
        }

        Self::commit_workspace_record_to_filesystem(Path::new(workspace_dir), record)
    }

    fn load_optional_planning_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        /*
         * Optional active-file reads prefer repo-scoped authority. For non-authority paths, a repo-scoped miss falls
         * back to the workspace filesystem so callers can still read operator-owned supporting files.
         */
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            let active_body = store.load_active_planning_file(workspace_dir, &relative_path)?;
            if active_body.is_some() || Self::authority_managed_path(&relative_path) {
                return Ok(active_body);
            }
            return self.read_optional_workspace_file(workspace_dir, &relative_path);
        }
        self.read_optional_workspace_file(workspace_dir, &relative_path)
    }

    fn load_optional_planning_candidate_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        // Candidate reads never touch repo-scoped authority; they answer "what does this workspace currently contain?"
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        Self::read_optional_candidate_workspace_file(workspace_dir, &relative_path)
    }

    fn replace_planning_workspace_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
        body: Option<&str>,
    ) -> Result<()> {
        // Replace is the low-level active-file write primitive used by planning services after validation.
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            return store.replace_active_planning_file(workspace_dir, &relative_path, body);
        }
        let path = self.active_workspace_path(workspace_dir, &relative_path);
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
        // Removal accepts files or directories because planning artifacts can be individual files or draft trees.
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            return store.remove_active_planning_entry(workspace_dir, &relative_path);
        }
        let path = self.active_workspace_path(workspace_dir, &relative_path);
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
        // Rejected proposals are copied into a named archive so the operator can recover or inspect them later.
        let archive_directory = self.rejected_directory(workspace_dir, archive_name);
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

fn write_optional_workspace_file(
    workspace_root: &Path,
    relative_path: &str,
    body: Option<&str>,
) -> Result<()> {
    // Shared helper for record-shaped writes: Some writes after creating parents, None deletes the old file if present.
    let path = workspace_root.join(relative_path);
    match body {
        Some(body) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
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

#[cfg(test)]
mod tests {
    use super::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };

    #[test]
    fn workspace_load_record_excludes_task_authority_artifacts() {
        // Task authority is DB-backed now; filesystem workspace load should only round-trip prompt file content.
        let workspace =
            std::env::temp_dir().join(format!("codex-exec-loop-fs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workspace);
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        adapter
            .commit_planning_workspace_files(
                workspace.to_str().expect("workspace path should be utf8"),
                &PlanningWorkspaceLoadRecord {
                    result_output_markdown: Some("# Result Output Prompt".to_string()),
                },
            )
            .expect("workspace files should commit");

        let loaded = adapter
            .load_planning_workspace_files(
                workspace.to_str().expect("workspace path should be utf8"),
            )
            .expect("workspace files should load");

        assert_eq!(
            loaded.result_output_markdown.as_deref(),
            Some("# Result Output Prompt")
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }
}
