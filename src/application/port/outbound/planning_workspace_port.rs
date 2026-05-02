use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: draft는 아직 실제 planning workspace에 반영되지 않은 제안 파일 묶음입니다.
 * application 서비스는 파일 시스템 경로를 직접 만들지 않고 `active_path`와 `body`만 넘기며,
 * outbound adapter가 이를 안전한 staging 위치나 repo-scoped 저장소 표현으로 바꿉니다.
 */
pub struct PlanningDraftFileRecord {
    // 학습 주석: 최종 반영될 때의 논리 경로입니다. adapter는 이 값을 정규화해 경로 탈출을 막습니다.
    pub active_path: String,
    // 학습 주석: draft 파일에 저장할 전체 본문입니다. 부분 patch가 아니라 완성 파일 내용을 운반합니다.
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: stage 결과는 "활성 workspace에서의 이름"과 "실제로 보관된 staging 위치"를 함께 반환합니다.
 * 이 둘을 분리해 두면 TUI나 planning service가 사용자에게는 active_path를 보여 주면서,
 * 내부적으로는 draft 디렉터리 또는 DB authority에 보관된 파일을 다시 찾아갈 수 있습니다.
 */
pub struct PlanningStagedFileRecord {
    // 학습 주석: 승격되면 workspace에 놓일 경로입니다.
    pub active_path: String,
    // 학습 주석: 현재 draft 저장소에서의 물리적 또는 저장소 식별 경로입니다.
    pub staged_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: stage 명령 전체의 응답입니다. draft 이름과 디렉터리/위치 정보를 함께 담아
 * 후속 로딩, 수정, 진단 메시지가 같은 draft 세트를 기준으로 움직이게 합니다.
 */
pub struct PlanningDraftStageRecord {
    // 학습 주석: 사용자가 선택하거나 서비스가 생성한 draft 묶음의 안정적인 이름입니다.
    pub draft_name: String,
    // 학습 주석: draft 파일들이 모이는 저장 위치입니다. filesystem adapter에서는 디렉터리 문자열입니다.
    pub draft_directory: String,
    // 학습 주석: 요청된 각 파일이 어느 active_path/staged_path 쌍으로 보관됐는지 알려 줍니다.
    pub staged_files: Vec<PlanningStagedFileRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: draft를 다시 읽을 때는 stage 메타데이터뿐 아니라 본문도 필요합니다.
 * authoring 흐름은 이 레코드를 사용해 사용자가 고친 draft 내용을 다시 prompt나 승격 로직에 연결합니다.
 */
pub struct PlanningDraftLoadFileRecord {
    // 학습 주석: 승격 대상 경로입니다. `staged_path`와 달리 사용자 의미를 가진 경로입니다.
    pub active_path: String,
    // 학습 주석: draft 저장소에서 읽어 온 실제 저장 위치입니다.
    pub staged_path: String,
    // 학습 주석: draft 파일 본문입니다. 호출자는 이 값을 그대로 비교하거나 다시 작성할 수 있습니다.
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: 하나의 draft 세트를 로드한 결과입니다. stage 때의 응답과 같은 상위 모양을 유지하되,
 * 각 파일에 `body`를 추가해 application 계층이 저장소 구현을 몰라도 편집과 검증을 계속할 수 있습니다.
 */
pub struct PlanningDraftLoadRecord {
    // 학습 주석: 로드한 draft 세트 이름입니다.
    pub draft_name: String,
    // 학습 주석: draft가 위치한 저장소 경로 또는 논리 디렉터리입니다.
    pub draft_directory: String,
    // 학습 주석: 정렬된 draft 파일 목록입니다. 구현체는 호출자가 안정적인 순서로 렌더링하도록 정렬합니다.
    pub staged_files: Vec<PlanningDraftLoadFileRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/*
 * 학습 주석: planning workspace에서 application 계층이 현재 다루는 canonical 파일 묶음입니다.
 * 지금은 `result_output_markdown` 한 파일만 표현하지만, 별도 레코드를 둔 덕분에 나중에
 * authority 파일이나 보조 산출물이 늘어도 포트 메서드 시그니처를 크게 흔들지 않을 수 있습니다.
 */
pub struct PlanningWorkspaceLoadRecord {
    // 학습 주석: planning 결과 문서의 본문입니다. 파일이 없을 수 있으므로 `Option`으로 부재를 명시합니다.
    pub result_output_markdown: Option<String>,
}

impl PlanningWorkspaceLoadRecord {
    // 학습 주석: commit/render 전에 "실제로 다룰 planning 파일이 있는가"를 빠르게 판단하는 도우미입니다.
    pub fn has_any_files(&self) -> bool {
        self.result_output_markdown.is_some()
    }
}

/*
 * 학습 주석: `PlanningWorkspacePort`는 planning service가 workspace 파일을 읽고 쓰는 유일한 outbound 계약입니다.
 * service는 draft, candidate, active workspace의 의미만 알고, 실제 저장소가 파일 시스템인지
 * repo-scoped SQLite authority인지 판단하는 책임은 adapter 쪽에 둡니다.
 */
pub trait PlanningWorkspacePort: Send + Sync {
    // 학습 주석: 새 draft 파일 묶음을 안전한 staging 영역에 저장하고, 저장된 위치를 호출자에게 돌려줍니다.
    fn stage_planning_draft_files(
        &self,
        // 학습 주석: planning 작업의 기준 디렉터리입니다. adapter는 이 값으로 일반 workspace와 repo-scoped 저장소를 구분합니다.
        workspace_dir: &str,
        // 학습 주석: draft 세트를 다시 찾기 위한 이름입니다.
        draft_name: &str,
        // 학습 주석: 저장할 draft 파일들입니다. slice를 받아 호출자가 소유권을 유지합니다.
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord>;

    // 학습 주석: 이전에 stage한 draft 세트를 본문까지 포함해 다시 읽습니다.
    fn load_planning_draft_files(
        &self,
        // 학습 주석: draft 저장소를 찾기 위한 workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: 읽을 draft 세트 이름입니다.
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord>;

    // 학습 주석: 이미 존재하는 draft의 한 파일을 전체 본문 단위로 교체합니다.
    fn replace_planning_draft_file(
        &self,
        // 학습 주석: draft가 속한 workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: 수정할 draft 세트 이름입니다.
        draft_name: &str,
        // 학습 주석: draft 내부에서 교체할 active 대상 경로입니다.
        active_path: &str,
        // 학습 주석: 교체 후 파일 전체 본문입니다.
        body: &str,
    ) -> Result<String>;

    // 학습 주석: 현재 active planning workspace의 canonical 파일들을 읽습니다.
    fn load_planning_workspace_files(
        &self,
        // 학습 주석: active workspace 루트 또는 repo-scoped workspace 식별자입니다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // 학습 주석: active가 아니라 candidate 영역의 planning 파일을 읽습니다. 승격 전 미리보기/검증에 쓰입니다.
    fn load_planning_workspace_candidate_files(
        &self,
        // 학습 주석: candidate 파일들이 연결된 workspace 기준입니다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // 학습 주석: `PlanningWorkspaceLoadRecord` 전체를 active workspace에 반영합니다.
    fn commit_planning_workspace_files(
        &self,
        // 학습 주석: 반영할 active workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: 파일별 존재/부재를 포함한 canonical workspace 스냅샷입니다.
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()>;

    // 학습 주석: active workspace에서 단일 planning 파일을 선택적으로 읽습니다.
    fn load_optional_planning_file(
        &self,
        // 학습 주석: 파일을 찾을 workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: workspace 안에서의 상대 경로입니다. adapter가 정규화하고 위험한 경로를 거부합니다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // 학습 주석: candidate 영역에서 단일 planning 파일을 선택적으로 읽습니다.
    fn load_optional_planning_candidate_file(
        &self,
        // 학습 주석: candidate 파일을 찾을 workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: candidate 영역 안에서의 상대 경로입니다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // 학습 주석: active workspace의 한 파일을 쓰거나 삭제합니다.
    fn replace_planning_workspace_file(
        &self,
        // 학습 주석: 수정할 workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: 수정할 파일의 상대 경로입니다.
        relative_path: &str,
        // 학습 주석: `Some`이면 전체 본문을 쓰고, `None`이면 해당 파일을 제거한다는 계약입니다.
        body: Option<&str>,
    ) -> Result<()>;

    // 학습 주석: active workspace에서 파일 또는 디렉터리 엔트리를 제거합니다.
    fn remove_planning_workspace_entry(
        &self,
        // 학습 주석: 제거 대상이 속한 workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: 제거할 상대 경로입니다. 파일과 디렉터리 모두 포트 계약에 포함됩니다.
        relative_path: &str,
    ) -> Result<()>;

    // 학습 주석: 거절된 planning 파일을 archive 영역에 보존합니다.
    fn archive_rejected_planning_file(
        &self,
        // 학습 주석: archive 위치를 계산할 workspace 기준입니다.
        workspace_dir: &str,
        // 학습 주석: 거절 묶음의 이름입니다. 여러 rejected 파일을 한 archive 아래 모을 때 쓰입니다.
        archive_name: &str,
        // 학습 주석: 원래 active workspace에서의 파일 경로입니다. archive 파일명 계산의 기준이 됩니다.
        active_path: &str,
        // 학습 주석: archive에 저장할 거절 당시의 본문입니다.
        body: &str,
    ) -> Result<String>;
}

/*
 * 학습 주석: `RepoScopedPlanningWorkspacePort`는 git-backed workspace의 특수 저장소 계약입니다.
 * 일반 `PlanningWorkspacePort` 구현체는 workspace가 repo-scoped인지 먼저 감지한 뒤,
 * 이 trait로 위임해 active 루트, draft 저장, 파일 교체를 repository authority 관점에서 처리합니다.
 */
pub trait RepoScopedPlanningWorkspacePort: Send + Sync {
    // 학습 주석: 주어진 workspace가 repo authority를 통해 관리되는 git-backed workspace인지 판단합니다.
    fn is_git_backed_workspace(&self, workspace_dir: &str) -> bool;

    // 학습 주석: repo-scoped 저장소에서 실제 active workspace 루트로 볼 경로를 계산합니다.
    fn resolve_active_workspace_root(&self, workspace_dir: &str) -> PathBuf;

    // 학습 주석: repo authority 안에 draft 파일 묶음을 stage합니다.
    fn stage_repo_scoped_draft_files(
        &self,
        // 학습 주석: repo-scoped workspace를 식별하는 기준 문자열입니다.
        workspace_dir: &str,
        // 학습 주석: 저장할 draft 세트 이름입니다.
        draft_name: &str,
        // 학습 주석: repo authority에 넣을 draft 파일 목록입니다.
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord>;

    // 학습 주석: repo authority에서 draft 파일 묶음을 본문까지 읽어 옵니다.
    fn load_repo_scoped_draft_files(
        &self,
        // 학습 주석: repo-scoped workspace 식별자입니다.
        workspace_dir: &str,
        // 학습 주석: 읽을 draft 세트 이름입니다.
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord>;

    // 학습 주석: repo authority에 저장된 draft 파일 하나를 전체 본문 단위로 교체합니다.
    fn replace_repo_scoped_draft_file(
        &self,
        // 학습 주석: repo-scoped workspace 식별자입니다.
        workspace_dir: &str,
        // 학습 주석: 수정할 draft 세트 이름입니다.
        draft_name: &str,
        // 학습 주석: draft 안에서 교체할 active 대상 경로입니다.
        active_path: &str,
        // 학습 주석: 교체할 전체 본문입니다.
        body: &str,
    ) -> Result<String>;

    // 학습 주석: repo authority가 관리하는 active workspace 파일 스냅샷을 읽습니다.
    fn load_active_workspace_files(
        &self,
        // 학습 주석: 읽을 repo-scoped workspace 식별자입니다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // 학습 주석: repo authority의 active workspace에 canonical 파일 스냅샷을 반영합니다.
    fn commit_active_workspace_files(
        &self,
        // 학습 주석: 반영 대상 repo-scoped workspace 식별자입니다.
        workspace_dir: &str,
        // 학습 주석: 저장할 planning workspace 파일 스냅샷입니다.
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()>;

    // 학습 주석: repo authority의 active workspace에서 단일 파일을 선택적으로 읽습니다.
    fn load_active_planning_file(
        &self,
        // 학습 주석: 파일을 찾을 repo-scoped workspace 식별자입니다.
        workspace_dir: &str,
        // 학습 주석: active workspace 기준 상대 경로입니다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // 학습 주석: repo authority의 active planning 파일을 쓰거나 삭제합니다.
    fn replace_active_planning_file(
        &self,
        // 학습 주석: 수정할 repo-scoped workspace 식별자입니다.
        workspace_dir: &str,
        // 학습 주석: 수정할 active 파일 상대 경로입니다.
        relative_path: &str,
        // 학습 주석: `Some`이면 저장, `None`이면 삭제를 의미해 일반 workspace 포트와 같은 계약을 유지합니다.
        body: Option<&str>,
    ) -> Result<()>;

    // 학습 주석: repo authority의 active workspace에서 파일 또는 디렉터리 엔트리를 제거합니다.
    fn remove_active_planning_entry(&self, workspace_dir: &str, relative_path: &str) -> Result<()>;
}
