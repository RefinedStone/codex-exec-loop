use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
    PlanningDraftStageRecord, PlanningStagedFileRecord, PlanningWorkspaceLoadRecord,
    PlanningWorkspacePort,
};
use crate::application::service::planning::{
    ACTIVE_PLANNING_FILE_PATHS, PLANNING_DRAFTS_DIRECTORY, PLANNING_REJECTED_DIRECTORY,
    RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
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

    fn candidate_workspace_path(workspace_dir: &str, relative_path: &str) -> PathBuf {
        Path::new(workspace_dir).join(relative_path)
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
        Ok(PlanningWorkspaceLoadRecord {
            result_output_markdown: file_loader(workspace_dir, RESULT_OUTPUT_FILE_PATH)?,
        })
    }

    fn commit_workspace_record_to_filesystem(
        workspace_root: &Path,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        write_optional_workspace_file(
            workspace_root,
            RESULT_OUTPUT_FILE_PATH,
            record.result_output_markdown.as_deref(),
        )?;
        Ok(())
    }

    fn authority_managed_path(relative_path: &str) -> bool {
        canonical_active_planning_file_path(relative_path).is_some()
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
        if SqlitePlanningAuthorityAdapter::is_git_backed_workspace(workspace_dir) {
            return SqlitePlanningAuthorityAdapter::load_active_workspace_files(workspace_dir);
        }
        Self::load_workspace_record_from(workspace_dir, Self::read_optional_workspace_file)
    }

    fn load_planning_workspace_candidate_files(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
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
        if SqlitePlanningAuthorityAdapter::is_git_backed_workspace(workspace_dir) {
            return SqlitePlanningAuthorityAdapter::commit_active_workspace_files(
                workspace_dir,
                record,
            );
        }

        Self::commit_workspace_record_to_filesystem(Path::new(workspace_dir), record)
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
        if SqlitePlanningAuthorityAdapter::is_git_backed_workspace(workspace_dir) {
            let active_body = SqlitePlanningAuthorityAdapter::load_active_planning_file(
                workspace_dir,
                &relative_path,
            )?;
            if active_body.is_some() || Self::authority_managed_path(&relative_path) {
                return Ok(active_body);
            }
            return Self::read_optional_workspace_file(workspace_dir, &relative_path);
        }
        Self::read_optional_workspace_file(workspace_dir, &relative_path)
    }

    fn load_optional_planning_candidate_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
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
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        if SqlitePlanningAuthorityAdapter::is_git_backed_workspace(workspace_dir) {
            return SqlitePlanningAuthorityAdapter::replace_active_planning_file(
                workspace_dir,
                &relative_path,
                body,
            );
        }
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
        if SqlitePlanningAuthorityAdapter::is_git_backed_workspace(workspace_dir) {
            return SqlitePlanningAuthorityAdapter::remove_active_planning_entry(
                workspace_dir,
                &relative_path,
            );
        }
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

fn write_optional_workspace_file(
    workspace_root: &Path,
    relative_path: &str,
    body: Option<&str>,
) -> Result<()> {
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
