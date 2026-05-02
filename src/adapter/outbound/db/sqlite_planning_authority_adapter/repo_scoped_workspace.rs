use std::path::PathBuf;

use anyhow::Result;

use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
    PlanningWorkspaceLoadRecord, RepoScopedPlanningWorkspacePort,
};

use super::SqlitePlanningAuthorityAdapter;

/*
 * 학습 주석: 이 impl은 `FilesystemPlanningWorkspaceAdapter`가 git-backed workspace를 감지했을 때
 * 파일 시스템 대신 SQLite authority DB로 위임하기 위한 repo-scoped workspace 어댑터입니다.
 * 실제 저장/로드 로직은 `SqlitePlanningAuthorityAdapter`의 inherent 함수들과 하위 모듈에 있고,
 * 이 파일은 application port의 trait method를 그 구현 함수로 연결하는 경계 역할을 합니다.
 */
impl RepoScopedPlanningWorkspacePort for SqlitePlanningAuthorityAdapter {
    // 학습 주석: workspace가 `.git`/repo authority를 가진 저장소인지 빠르게 판별해 filesystem adapter의 분기 기준을 제공합니다.
    fn is_git_backed_workspace(&self, workspace_dir: &str) -> bool {
        Self::is_git_backed_workspace(workspace_dir)
    }

    // 학습 주석: repo-scoped active workspace의 실제 root를 계산해 파일 경로와 authority namespace 해석을 맞춥니다.
    fn resolve_active_workspace_root(&self, workspace_dir: &str) -> PathBuf {
        Self::resolve_active_workspace_root(workspace_dir)
    }

    /*
     * 학습 주석: draft 파일 묶음을 repo authority DB에 stage합니다.
     * filesystem adapter의 같은 이름 메서드와 달리 여기서는 staging path가 DB authority 안의 draft row/location이 됩니다.
     */
    fn stage_repo_scoped_draft_files(
        &self,
        // 학습 주석: repo authority store를 찾는 workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: draft row들을 묶는 논리 이름입니다.
        draft_name: &str,
        // 학습 주석: active path와 본문을 담은 draft 파일 요청들입니다.
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord> {
        Self::stage_repo_scoped_draft_files(workspace_dir, draft_name, files)
    }

    // 학습 주석: DB에 stage된 draft row들을 다시 읽어 application 계층의 `PlanningDraftLoadRecord`로 복원합니다.
    fn load_repo_scoped_draft_files(
        &self,
        // 학습 주석: draft가 저장된 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: 읽어 올 draft 묶음 이름입니다.
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord> {
        Self::load_repo_scoped_draft_files(workspace_dir, draft_name)
    }

    // 학습 주석: 이미 stage된 draft 파일 하나를 전체 본문 단위로 교체합니다.
    fn replace_repo_scoped_draft_file(
        &self,
        // 학습 주석: draft row가 속한 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: 수정할 draft 묶음 이름입니다.
        draft_name: &str,
        // 학습 주석: draft 안에서 교체할 active target 경로입니다.
        active_path: &str,
        // 학습 주석: 교체할 전체 본문입니다.
        body: &str,
    ) -> Result<String> {
        Self::replace_repo_scoped_draft_file(workspace_dir, draft_name, active_path, body)
    }

    // 학습 주석: active_documents 테이블을 `PlanningWorkspaceLoadRecord`로 읽어 repo-backed active workspace를 복원합니다.
    fn load_active_workspace_files(
        &self,
        // 학습 주석: 읽기 대상 authority namespace입니다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord> {
        Self::load_active_workspace_files(workspace_dir)
    }

    // 학습 주석: application이 만든 active workspace snapshot을 SQLite active_documents row들로 commit합니다.
    fn commit_active_workspace_files(
        &self,
        // 학습 주석: commit 대상 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: canonical planning workspace 파일 snapshot입니다.
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()> {
        Self::commit_active_workspace_files(workspace_dir, record)
    }

    // 학습 주석: active_documents에서 단일 planning 파일 본문을 선택적으로 읽습니다.
    fn load_active_planning_file(
        &self,
        // 학습 주석: 조회 대상 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: active workspace 기준 상대 경로입니다.
        relative_path: &str,
    ) -> Result<Option<String>> {
        Self::load_active_planning_file(workspace_dir, relative_path)
    }

    // 학습 주석: active_documents row 하나를 저장하거나 삭제해 repo-scoped active 파일 변경을 반영합니다.
    fn replace_active_planning_file(
        &self,
        // 학습 주석: 변경 대상 authority namespace입니다.
        workspace_dir: &str,
        // 학습 주석: active workspace 기준 상대 경로입니다.
        relative_path: &str,
        // 학습 주석: `Some`이면 저장, `None`이면 삭제라는 workspace port 계약을 그대로 전달합니다.
        body: Option<&str>,
    ) -> Result<()> {
        Self::replace_active_planning_file(workspace_dir, relative_path, body)
    }

    // 학습 주석: active_documents에서 파일 또는 디렉터리 prefix 전체를 제거합니다.
    fn remove_active_planning_entry(&self, workspace_dir: &str, relative_path: &str) -> Result<()> {
        Self::remove_active_planning_entry(workspace_dir, relative_path)
    }
}
