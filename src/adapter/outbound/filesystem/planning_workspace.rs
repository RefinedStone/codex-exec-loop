use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
#[cfg(not(windows))]
use serde::{Deserialize, Serialize};

#[cfg(not(windows))]
use crate::application::port::outbound::planning_workspace_port::PlanningFileSyncBaselineRecord;
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadFileRecord, PlanningDraftLoadRecord,
    PlanningDraftStageRecord, PlanningFileSyncCandidateRecord, PlanningStagedFileRecord,
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort, RepoScopedPlanningWorkspacePort,
};
use crate::application::service::planning::{
    ACTIVE_PLANNING_FILE_PATHS, PLANNING_DRAFTS_DIRECTORY, PLANNING_REJECTED_DIRECTORY,
    RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path, validate_planning_draft_name,
};

use super::secure_fs;

#[cfg(not(windows))]
const FILE_SYNC_MANIFEST_PATH: &str =
    ".codex-exec-loop/planning/.akra-file-sync-result-output.json";

#[cfg(not(windows))]
#[derive(Debug, Serialize, Deserialize)]
struct PlanningFileSyncManifest {
    relative_path: String,
    observed_planning_revision: Option<i64>,
}

/*
 * FilesystemPlanningWorkspaceAdapter는 planning workspace file을 로컬 filesystem에 매핑하는 outbound adapter다.
 * 단순 파일 adapter처럼 보이지만 git-backed worktree에서는 candidate workspace와 authoritative planning store가
 * 서로 다른 root일 수 있다. 그래서 이 adapter는 repo-scoped store가 유효한 경우 active-state read/write를
 * RepoScopedPlanningWorkspacePort로 우회시키고, 일반 directory fixture나 legacy 실행에서는 직접 filesystem path를 쓴다.
 *
 * application layer는 PlanningWorkspacePort만 본다.
 * 이 구현이 active workspace, candidate workspace, staged draft, rejected archive의 물리 위치 차이를 숨겨야
 * service가 "무엇을 읽고 쓰는가"에 집중하고 "어느 checkout에 있는가"를 알 필요가 없어진다. Windows는
 * plain directory도 private SQLite authority로 보내 junction-safe direct filesystem 구현 부재를 기능 중단으로
 * 전파하지 않는다.
 */
#[derive(Default)]
pub struct FilesystemPlanningWorkspaceAdapter {
    // None은 direct-filesystem mode다. Some은 Unix Git workspace 또는 모든 Windows workspace에서 private authority를 제공한다.
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
        let store = self.repo_scoped_store.as_deref()?;
        #[cfg(windows)]
        {
            // Windows direct filesystem mutation is intentionally unavailable: a
            // full-path implementation cannot close junction/reparse replacement
            // races. The production composition always supplies this private,
            // workspace-keyed SQLite boundary, including for plain directories.
            let _ = workspace_dir;
            Some(store)
        }
        #[cfg(not(windows))]
        {
            // Unix plain directories retain the reviewable file-backed workflow;
            // Git worktrees share the repo-scoped authority store.
            store
                .is_git_backed_workspace(workspace_dir)
                .then_some(store)
        }
    }

    fn draft_directory(workspace_dir: &str, draft_name: &str) -> Result<PathBuf> {
        // draft는 promotion 전 operator가 inspect/reject할 수 있도록 candidate workspace 아래 staged tree로 둔다.
        validate_draft_name(draft_name)?;
        Ok(Path::new(workspace_dir)
            .join(PLANNING_DRAFTS_DIRECTORY)
            .join(draft_name))
    }

    fn draft_directory_relative(draft_name: &str) -> Result<PathBuf> {
        validate_draft_name(draft_name)?;
        Ok(Path::new(PLANNING_DRAFTS_DIRECTORY).join(draft_name))
    }

    fn active_workspace_root(&self, workspace_dir: &str) -> PathBuf {
        // repo-scoped mode에서는 parallel slot이 자기 worktree가 아니라 integration checkout authority에 planning state를 쓴다.
        self.repo_scoped_store
            .as_ref()
            .map(|store| store.resolve_active_workspace_root(workspace_dir))
            .unwrap_or_else(|| Path::new(workspace_dir).to_path_buf())
    }

    fn read_optional_workspace_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        let root = self.active_workspace_root(workspace_dir);
        secure_fs::read_optional_file(&root, Path::new(relative_path))
    }

    fn read_optional_candidate_workspace_file(
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        secure_fs::read_optional_file(Path::new(workspace_dir), Path::new(relative_path))
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
        Ok(Self::draft_directory(workspace_dir, draft_name)?.join(relative_path))
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

fn validate_draft_name(draft_name: &str) -> Result<()> {
    validate_planning_draft_name(draft_name)
        .map_err(|error| anyhow::anyhow!("invalid planning draft name `{draft_name}`: {error}"))
}

impl PlanningWorkspacePort for FilesystemPlanningWorkspaceAdapter {
    fn uses_repo_scoped_authority(&self, workspace_dir: &str) -> bool {
        self.repo_scoped_store(workspace_dir).is_some()
    }

    fn export_planning_file_sync_candidate(
        &self,
        workspace_dir: &str,
        relative_path: &str,
        body: &str,
        observed_planning_revision: Option<i64>,
    ) -> Result<String> {
        #[cfg(windows)]
        {
            let _ = (
                workspace_dir,
                relative_path,
                body,
                observed_planning_revision,
            );
            bail!(
                "external planning file sync is unavailable on Windows; use the repo-scoped admin draft editor"
            );
        }
        #[cfg(not(windows))]
        {
            let relative_path = normalize_workspace_relative_path(
                relative_path,
                &format!("invalid planning file-sync path: {relative_path}"),
            )?;
            if relative_path != RESULT_OUTPUT_FILE_PATH {
                bail!("unsupported planning file-sync path: {relative_path}");
            }
            let root = Path::new(workspace_dir);
            secure_fs::write_file_atomic(root, Path::new(&relative_path), body.as_bytes())?;
            if let Some(store) = self.repo_scoped_store(workspace_dir) {
                store.store_repo_scoped_file_sync_baseline(
                    workspace_dir,
                    &PlanningFileSyncBaselineRecord {
                        relative_path: relative_path.clone(),
                        observed_planning_revision,
                    },
                )?;
            } else {
                let manifest = serde_json::to_vec_pretty(&PlanningFileSyncManifest {
                    relative_path: relative_path.clone(),
                    observed_planning_revision,
                })?;
                secure_fs::write_file_atomic(root, Path::new(FILE_SYNC_MANIFEST_PATH), &manifest)?;
            }
            Ok(root.join(relative_path).display().to_string())
        }
    }

    fn load_planning_file_sync_candidate(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<PlanningFileSyncCandidateRecord>> {
        #[cfg(windows)]
        {
            let _ = (workspace_dir, relative_path);
            bail!(
                "external planning file sync is unavailable on Windows; use the repo-scoped admin draft editor"
            );
        }
        #[cfg(not(windows))]
        {
            let relative_path = normalize_workspace_relative_path(
                relative_path,
                &format!("invalid planning file-sync path: {relative_path}"),
            )?;
            if relative_path != RESULT_OUTPUT_FILE_PATH {
                bail!("unsupported planning file-sync path: {relative_path}");
            }
            let root = Path::new(workspace_dir);
            let Some(body) = secure_fs::read_optional_file(root, Path::new(&relative_path))? else {
                return Ok(None);
            };
            let observed_planning_revision = if let Some(store) =
                self.repo_scoped_store(workspace_dir)
            {
                store
                    .load_repo_scoped_file_sync_baseline(workspace_dir, &relative_path)?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "planning file-sync baseline is missing; export again before applying"
                        )
                    })?
                    .observed_planning_revision
            } else {
                let manifest = secure_fs::read_optional_file(
                    root,
                    Path::new(FILE_SYNC_MANIFEST_PATH),
                )?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "planning file-sync manifest is missing; export again before applying"
                    )
                })?;
                let manifest: PlanningFileSyncManifest = serde_json::from_str(&manifest)
                    .context("failed to decode planning file-sync manifest; export again")?;
                if manifest.relative_path != relative_path {
                    bail!(
                        "planning file-sync manifest does not match the exported path; export again"
                    );
                }
                manifest.observed_planning_revision
            };
            Ok(Some(PlanningFileSyncCandidateRecord {
                body,
                observed_planning_revision,
            }))
        }
    }

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
        validate_draft_name(draft_name)?;
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

        let draft_directory = Self::draft_directory(workspace_dir, draft_name)?;
        let draft_relative = Self::draft_directory_relative(draft_name)?;
        secure_fs::ensure_directory(Path::new(workspace_dir), &draft_relative)?;

        let staged_files = canonical_files
            .iter()
            .map(|file| {
                let staged_path =
                    Self::staged_draft_file_path(workspace_dir, draft_name, &file.active_path)?;
                let staged_relative = staged_path
                    .strip_prefix(workspace_dir)
                    .context("draft path escaped its workspace root")?;
                secure_fs::write_file_atomic(
                    Path::new(workspace_dir),
                    staged_relative,
                    file.body.as_bytes(),
                )?;

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
        validate_draft_name(draft_name)?;
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            let mut loaded = store.load_repo_scoped_draft_files(workspace_dir, draft_name)?;
            loaded.staged_files.sort_by(|left, right| {
                Self::draft_sort_order(&left.active_path)
                    .cmp(&Self::draft_sort_order(&right.active_path))
            });
            return Ok(loaded);
        }

        let draft_directory = Self::draft_directory(workspace_dir, draft_name)?;
        let draft_relative = Self::draft_directory_relative(draft_name)?;
        let mut staged_files = secure_fs::read_tree(Path::new(workspace_dir), &draft_relative)?
            .into_iter()
            .map(|(relative_path, body)| PlanningDraftLoadFileRecord {
                active_path: format!(".codex-exec-loop/planning/{relative_path}"),
                staged_path: draft_directory.join(&relative_path).display().to_string(),
                body,
            })
            .collect::<Vec<_>>();
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
        validate_draft_name(draft_name)?;
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
        let staged_relative = staged_path
            .strip_prefix(workspace_dir)
            .context("draft path escaped its workspace root")?;
        secure_fs::write_file_atomic(Path::new(workspace_dir), staged_relative, body.as_bytes())?;
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
        #[cfg(windows)]
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            // On Windows the private DB is also the candidate boundary. Reading
            // arbitrary full paths here would reintroduce the same junction race
            // that the active workspace routing avoids.
            return store.load_active_workspace_files(workspace_dir);
        }
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

    fn compare_and_swap_planning_workspace_files(
        &self,
        workspace_dir: &str,
        observed: &PlanningWorkspaceLoadRecord,
        replacement: &PlanningWorkspaceLoadRecord,
    ) -> Result<bool> {
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            return store.compare_and_swap_active_workspace_files(
                workspace_dir,
                observed,
                replacement,
            );
        }
        secure_fs::compare_and_swap_optional_file(
            &self.active_workspace_root(workspace_dir),
            Path::new(RESULT_OUTPUT_FILE_PATH),
            observed.result_output_markdown.as_deref(),
            replacement.result_output_markdown.as_deref(),
        )
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
            #[cfg(windows)]
            {
                // A DB miss is authoritative on Windows. Falling through to a
                // full-path filesystem read would reintroduce junction races.
                return Ok(None);
            }
            #[cfg(not(windows))]
            return self.read_optional_workspace_file(workspace_dir, &relative_path);
        }
        self.read_optional_workspace_file(workspace_dir, &relative_path)
    }

    fn load_optional_planning_candidate_file(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Result<Option<String>> {
        #[cfg(windows)]
        {
            let relative_path = normalize_workspace_relative_path(
                relative_path,
                &format!("invalid planning relative path: {relative_path}"),
            )?;
            if let Some(store) = self.repo_scoped_store(workspace_dir) {
                return store.load_active_planning_file(workspace_dir, &relative_path);
            }
            bail!(
                "planning candidate reads on Windows require the private workspace authority store"
            );
        }
        #[cfg(not(windows))]
        // candidate optional read는 repo-scoped authority를 보지 않고 현재 workspace copy의 내용을 그대로 답한다.
        let relative_path = normalize_workspace_relative_path(
            relative_path,
            &format!("invalid planning relative path: {relative_path}"),
        )?;
        #[cfg(not(windows))]
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
        let root = self.active_workspace_root(workspace_dir);
        match body {
            Some(body) => {
                secure_fs::write_file_atomic(&root, Path::new(&relative_path), body.as_bytes())?
            }
            None => secure_fs::remove_entry(&root, Path::new(&relative_path))?,
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
        secure_fs::remove_entry(
            &self.active_workspace_root(workspace_dir),
            Path::new(&relative_path),
        )
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
        validate_planning_draft_name(archive_name).map_err(|error| {
            anyhow::anyhow!("invalid planning archive name `{archive_name}`: {error}")
        })?;
        let active_path = normalize_workspace_relative_path(
            active_path,
            &format!("invalid planning archive source path: {active_path}"),
        )?;
        let file_name = Path::new(&active_path)
            .file_name()
            .with_context(|| format!("planning file has no file name: {active_path}"))?;
        let archive_relative = normalize_workspace_relative_path(
            &format!("{PLANNING_REJECTED_DIRECTORY}/{archive_name}"),
            &format!("invalid planning archive name: {archive_name}"),
        )?;
        let archived_relative = Path::new(&archive_relative).join(file_name);
        let archived_relative_string = archived_relative.to_string_lossy().replace('\\', "/");
        if let Some(store) = self.repo_scoped_store(workspace_dir) {
            store.replace_active_planning_file(
                workspace_dir,
                &archived_relative_string,
                Some(body),
            )?;
            return Ok(archived_relative_string);
        }
        let archive_relative = Path::new(&archive_relative);
        let root = self.active_workspace_root(workspace_dir);
        let archive_directory = secure_fs::ensure_directory(&root, archive_relative)?;
        secure_fs::write_file_atomic(&root, &archived_relative, body.as_bytes())?;
        let archived_path = archive_directory.join(file_name);

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
    let relative_path = Path::new(relative_path);
    match body {
        Some(body) => secure_fs::write_file_atomic(workspace_root, relative_path, body.as_bytes())?,
        None => secure_fs::remove_entry(workspace_root, relative_path)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::FilesystemPlanningWorkspaceAdapter;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningWorkspacePort, RepoScopedPlanningWorkspacePort,
    };
    use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
    use std::process::Command;
    use std::sync::{Arc, Barrier};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(not(windows))]
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
        std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
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

    #[cfg(not(windows))]
    #[test]
    fn direct_workspace_compare_and_swap_preserves_drifted_content() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-cas-test-{}-{unique_suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
        let workspace_dir = workspace.to_str().expect("workspace path should be utf8");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();
        let worker_candidate = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("worker candidate".to_string()),
        };
        let snapshot = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("pre-turn snapshot".to_string()),
        };
        adapter
            .commit_planning_workspace_files(workspace_dir, &worker_candidate)
            .expect("worker candidate should commit");

        assert!(
            adapter
                .compare_and_swap_planning_workspace_files(
                    workspace_dir,
                    &worker_candidate,
                    &snapshot,
                )
                .expect("matching candidate should restore")
        );
        assert!(
            !adapter
                .compare_and_swap_planning_workspace_files(
                    workspace_dir,
                    &worker_candidate,
                    &PlanningWorkspaceLoadRecord::default(),
                )
                .expect("stale candidate should miss")
        );
        assert_eq!(
            adapter
                .load_planning_workspace_files(workspace_dir)
                .expect("restored workspace should load"),
            snapshot
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[cfg(not(windows))]
    #[test]
    fn injected_store_does_not_hide_existing_plain_workspace_files() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-plain-routing-test-{}-{unique_suffix}",
            std::process::id()
        ));
        let result_output_path = workspace.join(RESULT_OUTPUT_FILE_PATH);
        std::fs::create_dir_all(result_output_path.parent().unwrap())
            .expect("plain planning directory should be created");
        std::fs::write(&result_output_path, "legacy plain result")
            .expect("legacy plain result should be seeded");
        let workspace_dir = workspace.to_str().expect("workspace should be utf8");
        let adapter = FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(Arc::new(
            SqlitePlanningAuthorityAdapter::new(),
        ));

        assert!(!adapter.uses_repo_scoped_authority(workspace_dir));
        assert_eq!(
            adapter
                .load_planning_workspace_files(workspace_dir)
                .expect("plain workspace should remain readable")
                .result_output_markdown
                .as_deref(),
            Some("legacy plain result")
        );
        adapter
            .replace_planning_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some("updated plain result"),
            )
            .expect("plain workspace should remain file-backed");
        assert_eq!(
            std::fs::read_to_string(&result_output_path)
                .expect("updated plain result should remain on disk"),
            "updated plain result"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn draft_storage_rejects_names_that_are_not_single_segments() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-draft-name-test-{}-{unique_suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&workspace);
        std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
        let workspace_dir = workspace.to_str().expect("workspace path should be utf8");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let stage_error = adapter
            .stage_planning_draft_files(
                workspace_dir,
                "../outside",
                &[PlanningDraftFileRecord {
                    active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                    body: "# Result Output Prompt\n".to_string(),
                }],
            )
            .expect_err("escaped draft name should not stage");
        assert!(
            stage_error
                .to_string()
                .contains("invalid planning draft name `../outside`")
        );
        assert!(
            !workspace.join(".codex-exec-loop/planning/outside").exists(),
            "invalid draft name must not escape the drafts directory"
        );

        let load_error = adapter
            .load_planning_draft_files(workspace_dir, "bad/name")
            .expect_err("slash draft name should not load");
        assert!(
            load_error
                .to_string()
                .contains("invalid planning draft name `bad/name`")
        );

        let replace_error = adapter
            .replace_planning_draft_file(
                workspace_dir,
                "bad:name",
                RESULT_OUTPUT_FILE_PATH,
                "# Edited\n",
            )
            .expect_err("colon draft name should not replace");
        assert!(
            replace_error
                .to_string()
                .contains("invalid planning draft name `bad:name`")
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[cfg(unix)]
    #[test]
    fn planning_write_rejects_symlink_and_hardlink_victims_without_touching_them() {
        use std::os::unix::fs::symlink;

        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-link-test-{}-{unique_suffix}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let planning = workspace.join(".codex-exec-loop/planning");
        std::fs::create_dir_all(&planning).expect("planning fixture should be created");
        let victim = root.join("victim.txt");
        std::fs::write(&victim, "do not touch").expect("victim should be seeded");
        let target = workspace.join(RESULT_OUTPUT_FILE_PATH);
        symlink(&victim, &target).expect("malicious symlink should be created");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let symlink_error = adapter
            .replace_planning_workspace_file(
                workspace.to_str().expect("workspace should be utf8"),
                RESULT_OUTPUT_FILE_PATH,
                Some("replacement"),
            )
            .expect_err("symlink destination must fail closed");
        assert!(
            symlink_error.to_string().contains("planning file")
                || symlink_error
                    .to_string()
                    .contains("without following links")
        );
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "do not touch");

        std::fs::remove_file(&target).expect("symlink should be removable by fixture");
        std::fs::hard_link(&victim, &target).expect("malicious hardlink should be created");
        adapter
            .replace_planning_workspace_file(
                workspace.to_str().expect("workspace should be utf8"),
                RESULT_OUTPUT_FILE_PATH,
                Some("replacement"),
            )
            .expect_err("hardlink destination must fail closed");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "do not touch");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "do not touch");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn planning_tree_remove_rejects_nested_symlink_without_traversing_victim() {
        use std::os::unix::fs::symlink;

        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-remove-link-test-{}-{unique_suffix}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let drafts = workspace.join(".codex-exec-loop/planning/drafts");
        let victim = root.join("victim");
        std::fs::create_dir_all(&drafts).expect("draft fixture should be created");
        std::fs::create_dir_all(&victim).expect("victim directory should be created");
        std::fs::write(victim.join("preserved.txt"), "preserved")
            .expect("victim file should be seeded");
        symlink(&victim, drafts.join("outside"))
            .expect("nested malicious symlink should be created");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        adapter
            .remove_planning_workspace_entry(
                workspace.to_str().expect("workspace should be utf8"),
                ".codex-exec-loop/planning/drafts",
            )
            .expect_err("tree removal must reject nested symlinks");

        assert_eq!(
            std::fs::read_to_string(victim.join("preserved.txt")).unwrap(),
            "preserved"
        );
        assert!(drafts.join("outside").symlink_metadata().is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn planning_tree_remove_atomically_preserves_contents_in_private_quarantine() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-quarantine-test-{}-{unique_suffix}",
            std::process::id()
        ));
        let draft = workspace.join(".codex-exec-loop/planning/drafts/review");
        std::fs::create_dir_all(draft.join("nested")).expect("draft fixture should be created");
        std::fs::write(draft.join("prompt.md"), "preserved prompt")
            .expect("prompt fixture should be seeded");
        std::fs::write(draft.join("nested/result.md"), "preserved result")
            .expect("result fixture should be seeded");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        adapter
            .remove_planning_workspace_entry(
                workspace.to_str().expect("workspace should be utf8"),
                ".codex-exec-loop/planning/drafts/review",
            )
            .expect("valid planning tree should move into quarantine");

        assert!(!draft.exists(), "logical planning path should be removed");
        let quarantine = workspace.join(".codex-exec-loop/runtime/planning-quarantine");
        let retained = std::fs::read_dir(&quarantine)
            .expect("quarantine should exist")
            .collect::<Result<Vec<_>, _>>()
            .expect("quarantine should enumerate");
        assert_eq!(retained.len(), 1);
        let retained = retained[0].path();
        assert_eq!(
            std::fs::read_to_string(retained.join("prompt.md")).unwrap(),
            "preserved prompt"
        );
        assert_eq!(
            std::fs::read_to_string(retained.join("nested/result.md")).unwrap(),
            "preserved result"
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[cfg(unix)]
    #[test]
    fn planning_remove_fails_without_mutation_when_quarantine_is_full() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-quarantine-limit-test-{}-{unique_suffix}",
            std::process::id()
        ));
        let planning = workspace.join(".codex-exec-loop/planning");
        std::fs::create_dir_all(&planning).expect("planning fixture should be created");
        let workspace_dir = workspace.to_str().expect("workspace should be utf8");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        for index in 0..16 {
            let relative = format!(".codex-exec-loop/planning/stale-{index}.md");
            std::fs::write(workspace.join(&relative), format!("stale {index}"))
                .expect("stale planning file should be seeded");
            adapter
                .remove_planning_workspace_entry(workspace_dir, &relative)
                .expect("retention below the entry limit should succeed");
        }

        let final_relative = ".codex-exec-loop/planning/must-remain.md";
        let final_path = workspace.join(final_relative);
        std::fs::write(&final_path, "must remain").expect("final planning file should be seeded");
        let error = adapter
            .remove_planning_workspace_entry(workspace_dir, final_relative)
            .expect_err("full quarantine must reject a new removal");
        assert!(error.to_string().contains("operator cleanup is required"));
        assert_eq!(std::fs::read_to_string(&final_path).unwrap(), "must remain");
        assert_eq!(
            std::fs::read_dir(workspace.join(".codex-exec-loop/runtime/planning-quarantine"))
                .unwrap()
                .count(),
            16
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn git_backed_supporting_documents_and_rejected_archives_stay_in_repo_db() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-repo-artifact-test-{}-{unique_suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
        let output = Command::new("git")
            .args(["init", "-q", workspace.to_str().unwrap()])
            .output()
            .expect("git init should run");
        assert!(output.status.success());
        let workspace_dir = workspace.to_str().expect("workspace should be utf8");
        let sqlite = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let adapter = FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(sqlite.clone());

        let archived = adapter
            .archive_rejected_planning_file(
                workspace_dir,
                "rejected-1",
                RESULT_OUTPUT_FILE_PATH,
                "rejected body",
            )
            .expect("rejected file should archive in the repo DB");
        let archived_relative = ".codex-exec-loop/planning/rejected/rejected-1/result-output.md";
        assert_eq!(archived, archived_relative);
        assert_eq!(
            sqlite
                .load_active_planning_file(workspace_dir, archived_relative)
                .expect("archived DB document should load")
                .as_deref(),
            Some("rejected body")
        );
        assert!(!workspace.join(archived_relative).exists());

        sqlite
            .replace_active_planning_file(
                workspace_dir,
                ".codex-exec-loop/planning/supporting.md",
                Some("supporting body"),
            )
            .expect("supporting DB document should persist");
        assert_eq!(
            adapter
                .load_optional_planning_file(
                    workspace_dir,
                    ".codex-exec-loop/planning/supporting.md",
                )
                .expect("supporting DB document should load")
                .as_deref(),
            Some("supporting body")
        );

        let invalid = adapter
            .archive_rejected_planning_file(
                workspace_dir,
                "../escaped",
                RESULT_OUTPUT_FILE_PATH,
                "must not persist",
            )
            .expect_err("archive name must be a single normalized segment");
        assert!(
            invalid
                .to_string()
                .contains("invalid planning archive name")
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn git_backed_workspace_compare_and_swap_uses_repo_authority_transaction() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-repo-cas-test-{}-{unique_suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
        let output = Command::new("git")
            .args(["init", "-q", workspace.to_str().unwrap()])
            .output()
            .expect("git init should run");
        assert!(output.status.success());
        let workspace_dir = workspace.to_str().expect("workspace should be utf8");
        let sqlite = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let adapter = FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(sqlite.clone());
        let worker_candidate = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("worker candidate".to_string()),
        };
        let snapshot = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("pre-turn snapshot".to_string()),
        };
        adapter
            .commit_planning_workspace_files(workspace_dir, &worker_candidate)
            .expect("worker candidate should commit to repo authority");

        assert!(
            adapter
                .compare_and_swap_planning_workspace_files(
                    workspace_dir,
                    &worker_candidate,
                    &snapshot,
                )
                .expect("repo authority CAS should succeed")
        );
        assert!(
            !adapter
                .compare_and_swap_planning_workspace_files(
                    workspace_dir,
                    &worker_candidate,
                    &PlanningWorkspaceLoadRecord::default(),
                )
                .expect("stale repo authority CAS should miss")
        );
        assert_eq!(
            sqlite
                .load_active_workspace_files(workspace_dir)
                .expect("repo authority snapshot should load"),
            snapshot
        );
        assert!(!workspace.join(RESULT_OUTPUT_FILE_PATH).exists());
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn concurrent_repo_authority_compare_and_swap_has_one_winner() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-repo-cas-race-test-{}-{unique_suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
        let output = Command::new("git")
            .args(["init", "-q", workspace.to_str().unwrap()])
            .output()
            .expect("git init should run");
        assert!(output.status.success());
        let workspace_dir = workspace.to_str().expect("workspace should be utf8");
        let sqlite = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let adapter = Arc::new(FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(
            sqlite,
        ));
        let observed = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("worker candidate".to_string()),
        };
        adapter
            .commit_planning_workspace_files(workspace_dir, &observed)
            .expect("worker candidate should commit to repo authority");
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for body in ["operator one", "operator two"] {
            let adapter = adapter.clone();
            let barrier = barrier.clone();
            let workspace_dir = workspace_dir.to_string();
            let observed = observed.clone();
            let replacement = PlanningWorkspaceLoadRecord {
                result_output_markdown: Some(body.to_string()),
            };
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                adapter.compare_and_swap_planning_workspace_files(
                    &workspace_dir,
                    &observed,
                    &replacement,
                )
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .expect("repo CAS worker should join")
                    .expect("repo CAS worker should complete")
            })
            .collect::<Vec<_>>();

        assert_eq!(outcomes.iter().filter(|outcome| **outcome).count(), 1);
        let final_record = adapter
            .load_planning_workspace_files(workspace_dir)
            .expect("winning repo authority snapshot should load");
        assert!(matches!(
            final_record.result_output_markdown.as_deref(),
            Some("operator one" | "operator two")
        ));
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[cfg(windows)]
    #[test]
    fn windows_plain_workspace_uses_private_authority_for_active_and_draft_flows() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-windows-authority-test-{}-{unique_suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
        let workspace_dir = workspace.to_str().expect("workspace should be utf8");
        let sqlite = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let adapter = FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(sqlite);

        assert!(adapter.uses_repo_scoped_authority(workspace_dir));
        let record = PlanningWorkspaceLoadRecord {
            result_output_markdown: Some("# Windows authority\n".to_string()),
        };
        adapter
            .commit_planning_workspace_files(workspace_dir, &record)
            .expect("plain Windows workspace should commit through SQLite");
        assert_eq!(
            adapter
                .load_planning_workspace_candidate_files(workspace_dir)
                .expect("Windows candidate view should use the private authority"),
            record
        );
        assert_eq!(
            adapter
                .load_optional_planning_candidate_file(workspace_dir, RESULT_OUTPUT_FILE_PATH)
                .expect("Windows candidate file should load from private authority")
                .as_deref(),
            Some("# Windows authority\n")
        );

        adapter
            .stage_planning_draft_files(
                workspace_dir,
                "review",
                &[PlanningDraftFileRecord {
                    active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
                    body: "# Draft\n".to_string(),
                }],
            )
            .expect("plain Windows draft should stage through SQLite");
        let draft = adapter
            .load_planning_draft_files(workspace_dir, "review")
            .expect("plain Windows draft should load through SQLite");
        assert_eq!(draft.staged_files.len(), 1);
        assert_eq!(draft.staged_files[0].body, "# Draft\n");

        adapter
            .replace_planning_workspace_file(workspace_dir, RESULT_OUTPUT_FILE_PATH, None)
            .expect("plain Windows active file removal should use SQLite");
        assert!(
            adapter
                .load_optional_planning_file(workspace_dir, RESULT_OUTPUT_FILE_PATH)
                .expect("removed Windows authority file should inspect")
                .is_none()
        );
        assert!(
            !workspace.join(RESULT_OUTPUT_FILE_PATH).exists(),
            "the Windows private authority route must not fall back to full-path file mutation"
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[cfg(windows)]
    #[test]
    fn direct_planning_filesystem_fails_closed_without_mutating_windows_workspace() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-windows-closed-test-{}-{unique_suffix}",
            std::process::id()
        ));
        let planning = workspace.join(".codex-exec-loop/planning");
        std::fs::create_dir_all(&planning).expect("planning fixture should be created");
        let target = workspace.join(RESULT_OUTPUT_FILE_PATH);
        std::fs::write(&target, "unchanged").expect("target should be seeded");
        let workspace_dir = workspace.to_str().expect("workspace should be utf8");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        let candidate_error = adapter
            .export_planning_file_sync_candidate(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                "replacement",
                Some(1),
            )
            .expect_err("external Windows file sync must be capability-gated");
        assert!(candidate_error.to_string().contains("admin draft editor"));
        let write_error = adapter
            .replace_planning_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some("replacement"),
            )
            .expect_err("direct Windows planning write must fail closed");
        assert!(write_error.to_string().contains("unsupported on Windows"));
        let remove_error = adapter
            .remove_planning_workspace_entry(workspace_dir, RESULT_OUTPUT_FILE_PATH)
            .expect_err("direct Windows planning removal must fail closed");
        assert!(remove_error.to_string().contains("unsupported on Windows"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "unchanged");
        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[cfg(not(windows))]
    #[test]
    fn planning_file_admission_accepts_boundary_and_rejects_oversize_without_data_loss() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "codex-exec-loop-fs-size-test-{}-{unique_suffix}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
        let workspace_dir = workspace.to_str().expect("workspace path should be utf8");
        let adapter = FilesystemPlanningWorkspaceAdapter::new();

        adapter
            .replace_planning_workspace_file(workspace_dir, RESULT_OUTPUT_FILE_PATH, Some("small"))
            .expect("small planning file should be admitted");
        let boundary = "x".repeat(super::secure_fs::MAX_FILE_BYTES);
        adapter
            .replace_planning_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some(&boundary),
            )
            .expect("boundary-sized planning file should be admitted");
        assert_eq!(
            adapter
                .load_optional_planning_file(workspace_dir, RESULT_OUTPUT_FILE_PATH)
                .expect("boundary file should load")
                .expect("boundary file should exist")
                .len(),
            super::secure_fs::MAX_FILE_BYTES
        );

        let oversize = format!("{boundary}x");
        let error = adapter
            .replace_planning_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some(&oversize),
            )
            .expect_err("oversized planning file must be rejected");
        assert!(error.to_string().contains("exceeds"));
        assert_eq!(
            std::fs::metadata(workspace.join(RESULT_OUTPUT_FILE_PATH))
                .expect("accepted boundary file should remain")
                .len(),
            super::secure_fs::MAX_FILE_BYTES as u64
        );
        let _ = std::fs::remove_dir_all(&workspace);
    }
}
