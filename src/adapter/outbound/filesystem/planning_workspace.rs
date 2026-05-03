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
 * FilesystemPlanningWorkspaceAdapter는 planning workspace file을 로컬 filesystem에 매핑하는 outbound adapter다.
 * 단순 파일 adapter처럼 보이지만 git-backed worktree에서는 candidate workspace와 authoritative planning store가
 * 서로 다른 root일 수 있다. 그래서 이 adapter는 repo-scoped store가 유효한 경우 active-state read/write를
 * RepoScopedPlanningWorkspacePort로 우회시키고, 일반 directory fixture나 legacy 실행에서는 직접 filesystem path를 쓴다.
 *
 * application layer는 PlanningWorkspacePort만 본다.
 * 이 구현이 active workspace, candidate workspace, staged draft, rejected archive의 물리 위치 차이를 숨겨야
 * service가 "무엇을 읽고 쓰는가"에 집중하고 "어느 checkout에 있는가"를 알 필요가 없어진다.
 */
#[derive(Default)]
pub struct FilesystemPlanningWorkspaceAdapter {
    // None은 direct-filesystem mode이고, Some은 git-backed workspace에서 active authority root를 resolve할 수 있다는 뜻이다.
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
        // repo-scoped store는 git-backed workspace에서만 의미가 있으므로 temp fixture나 plain directory는 direct path로 남긴다.
        self.repo_scoped_store
            .as_deref()
            .filter(|store| store.is_git_backed_workspace(workspace_dir))
    }

    fn draft_directory(workspace_dir: &str, draft_name: &str) -> PathBuf {
        // draft는 promotion 전 operator가 inspect/reject할 수 있도록 candidate workspace 아래 staged tree로 둔다.
        Path::new(workspace_dir)
            .join(PLANNING_DRAFTS_DIRECTORY)
            .join(draft_name)
    }

    fn rejected_directory(&self, workspace_dir: &str, archive_name: &str) -> PathBuf {
        // rejected archive는 candidate worktree가 아니라 active workspace root 쪽에 남겨 authority history와 같은 위치에 둔다.
        self.active_workspace_root(workspace_dir)
            .join(PLANNING_REJECTED_DIRECTORY)
            .join(archive_name)
    }

    fn active_workspace_root(&self, workspace_dir: &str) -> PathBuf {
        // repo-scoped mode에서는 parallel slot이 자기 worktree가 아니라 integration checkout authority에 planning state를 쓴다.
        self.repo_scoped_store
            .as_ref()
            .map(|store| store.resolve_active_workspace_root(workspace_dir))
            .unwrap_or_else(|| Path::new(workspace_dir).to_path_buf())
    }

    fn active_workspace_path(&self, workspace_dir: &str, relative_path: &str) -> PathBuf {
        // active path는 result output 같은 committed planning state를 읽고 쓸 때 사용한다.
        self.active_workspace_root(workspace_dir)
            .join(relative_path)
    }

    fn candidate_workspace_path(workspace_dir: &str, relative_path: &str) -> PathBuf {
        // candidate path는 repo-scoped authority를 보지 않고 현재 slot/worktree copy 자체를 검사할 때 사용한다.
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
        // workspace record는 DB-backed task authority artifact를 제외한다. 이 adapter는 prompt file만 round-trip한다.
        Ok(PlanningWorkspaceLoadRecord {
            result_output_markdown: file_loader(workspace_dir, RESULT_OUTPUT_FILE_PATH)?,
        })
    }

    fn commit_workspace_record_to_filesystem(
        workspace_root: &Path,
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        // commit은 load record shape를 mirror한다. None은 stale file 제거, Some은 최신 prompt body write를 뜻한다.
        write_optional_workspace_file(
            workspace_root,
            RESULT_OUTPUT_FILE_PATH,
            record.result_output_markdown.as_deref(),
        )?;
        Ok(())
    }

    fn authority_managed_path(relative_path: &str) -> bool {
        // canonical active planning file은 repo-scoped storage가 None을 돌려도 authority-managed path로 간주한다.
        canonical_active_planning_file_path(relative_path).is_some()
    }

    fn staged_draft_file_path(
        workspace_dir: &str,
        draft_name: &str,
        active_path: &str,
    ) -> Result<PathBuf> {
        // staged draft tree는 planning root prefix를 제거한 상대 경로를 써서 compact하고 이동 가능한 proposal tree로 만든다.
        let relative_path = Self::draft_relative_path(active_path)?;
        let relative_path = Path::new(&relative_path);
        Ok(Self::draft_directory(workspace_dir, draft_name).join(relative_path))
    }

    fn draft_relative_path(active_path: &str) -> Result<String> {
        /*
         * draft input은 canonical active path일 수도 있고 planning-relative short path일 수도 있다.
         * 두 형태를 하나의 safe relative path로 normalize해야 stage/load/promote가 같은 file identity를 바라본다.
         * 이 함수는 slash normalization 뒤 planning prefix를 제거하고, 마지막에는 workspace escape guard를 통과시킨다.
         */
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
        // promotion code는 active path가 canonical planning namespace에 있다고 가정하므로 draft-relative path를 다시 prefix한다.
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
        // recursive draft load는 staged file 위치에서 active path를 재구성해 review/promotion UI가 원래 planning file을 표시하게 한다.
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
        // 알려진 planning file은 semantic order로 먼저 보이고, extra file은 그 뒤에서 path 기준으로 안정 정렬된다.
        let order = ACTIVE_PLANNING_FILE_PATHS
            .iter()
            .position(|candidate| *candidate == active_path)
            .unwrap_or(ACTIVE_PLANNING_FILE_PATHS.len());
        (order, active_path)
    }
}

fn normalize_workspace_relative_path(path: &str, context: &str) -> Result<String> {
    /*
     * 외부에서 들어온 planning relative path는 filesystem을 만지기 전에 반드시 이 guard를 통과한다.
     * absolute path, Windows drive root, parent traversal을 모두 거부해 draft/replace/remove operation이 선택된
     * workspace root 밖으로 빠져나가지 못하게 한다. service는 planning-relative vocabulary를 다루고,
     * adapter는 그 vocabulary가 실제 OS path로 바뀌기 직전의 안전성을 책임진다.
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
         * staging은 active authority를 직접 mutate하지 않고 proposed planning file을 draft namespace 아래에 쓴다.
         * service 입장에서는 "이 draft_name에 어떤 active file 후보가 준비되었는가"만 중요하고,
         * git-backed slot인지 direct filesystem workspace인지는 adapter가 감춘다.
         *
         * 먼저 active_path를 canonical planning namespace로 맞춘다.
         * 이렇게 해야 repo-scoped store와 direct mode가 서로 다른 physical staging location을 쓰더라도
         * PlanningDraftStageRecord의 active_path vocabulary는 promotion/load UI에서 하나로 유지된다.
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
        /*
         * draft load는 staged proposal을 review/promotion UI가 그릴 read model로 되돌린다.
         * repo-scoped mode와 direct mode의 저장 위치는 다르지만 sort order는 같은 helper를 써서 UI ordering을 안정화한다.
         * known planning file이 먼저 오면 operator가 핵심 prompt/result file을 매번 같은 위치에서 확인할 수 있다.
         */
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
        /*
         * draft replacement는 staged proposal content만 편집한다.
         * active planning file은 promotion 전까지 untouched로 남아야 validation 실패나 operator 취소가 authority state를
         * 오염시키지 않는다. return 값은 실제 staged path라서 UI/debug surface가 "어디를 썼는가"를 보여줄 수 있다.
         */
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
        /*
         * active workspace load는 authority-aware read다.
         * git-backed workspace에서는 현재 slot directory가 아니라 repo-scoped authority store를 먼저 읽는다.
         * direct mode에서는 같은 record shape를 filesystem에서 조립해 legacy/plain workspace 실행과 test fixture를 유지한다.
         */
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
        /*
         * candidate workspace load는 의도적으로 repo-scoped authority를 무시한다.
         * comparison/review code가 "현재 slot worktree에는 무엇이 있는가"를 봐야 할 때 active authority로 fallback하면
         * candidate와 active의 차이를 잃는다.
         */
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
        /*
         * commit은 PlanningWorkspaceLoadRecord를 active authority location에 쓴다.
         * repo-scoped store가 있으면 integration checkout의 authority를 갱신하고, 없으면 workspace_dir 아래 direct filesystem에 쓴다.
         * record-shaped write를 유지해 load/commit이 같은 prompt file set을 round-trip한다.
         */
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
         * optional active-file read는 repo-scoped authority를 우선한다.
         * canonical authority-managed path에서 None이 나오면 "파일이 authority에 없다"는 의미가 있으므로 바로 None을 반환한다.
         * 반면 non-authority supporting file은 repo-scoped store가 모를 수 있어 workspace filesystem으로 fallback한다.
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
        // candidate optional read는 repo-scoped authority를 보지 않고 현재 workspace copy의 내용을 그대로 답한다.
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
        /*
         * replace는 validation 이후 service가 쓰는 low-level active-file write primitive다.
         * Some(body)는 parent directory를 만든 뒤 write하고, None은 같은 path의 stale file을 제거한다.
         * repo-scoped mode에서는 동일한 semantic을 authority store로 위임해 slot worktree가 active state를 몰래 바꾸지 않게 한다.
         */
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
        /*
         * removal은 file과 directory를 모두 받는다.
         * planning artifact는 단일 prompt file일 수도 있고 draft/rejected tree처럼 directory일 수도 있기 때문이다.
         * path normalization을 먼저 수행하고 repo-scoped mode에서는 authority store에 위임해 direct filesystem 삭제가
         * integration checkout authority를 우회하지 않게 한다.
         */
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
        /*
         * rejected proposal은 named archive 아래에 복사해 operator가 나중에 복구하거나 실패 원인을 조사할 수 있게 한다.
         * archive root는 active workspace root 기준이다. candidate slot이 사라져도 rejection record는 authority 쪽에 남아야 한다.
         * active_path 전체를 보존하지 않고 file name만 쓰는 이유는 rejected archive가 proposal snapshot의 leaf file 모음이기 때문이다.
         */
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
    /*
     * record-shaped write helper다.
     * PlanningWorkspaceLoadRecord의 Option field 의미를 filesystem operation으로 옮긴다.
     * Some은 parent directory를 만든 뒤 body를 쓰고, None은 이전 round-trip에서 남은 stale file을 제거한다.
     */
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
        /*
         * task authority는 이제 DB-backed이고 filesystem workspace record가 소유하지 않는다.
         * 이 테스트는 commit/load round-trip이 result output prompt file만 포함하고, 과거 raw task authority artifact를
         * record shape에 다시 끌어들이지 않는지 고정한다.
         */
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
