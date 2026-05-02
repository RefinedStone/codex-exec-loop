use anyhow::Result;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * draft는 아직 실제 planning workspace에 반영되지 않은 제안 파일 묶음이다.
 * application 서비스는 파일 시스템 경로를 직접 만들지 않고 `active_path`와 `body`만 넘기며,
 * outbound adapter가 이를 안전한 staging 위치나 repo-scoped 저장소 표현으로 바꾼다.
 */
pub struct PlanningDraftFileRecord {
    // 최종 반영될 때의 논리 경로이다. adapter는 이 값을 정규화해 경로 탈출을 막는다.
    pub active_path: String,
    // draft 파일에 저장할 전체 본문이다. 부분 patch가 아니라 완성 파일 내용을 운반한다.
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * stage 결과는 "활성 workspace에서의 이름"과 "실제로 보관된 staging 위치"를 함께 반환한다.
 * 이 둘을 분리해 두면 TUI나 planning service가 사용자에게는 active_path를 보여 주면서,
 * 내부적으로는 draft 디렉터리 또는 DB authority에 보관된 파일을 다시 찾아갈 수 있다.
 */
pub struct PlanningStagedFileRecord {
    // 승격되면 workspace에 놓일 경로이다.
    pub active_path: String,
    // 현재 draft 저장소에서의 물리적 또는 저장소 식별 경로이다.
    pub staged_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * stage 명령 전체의 응답이다. draft 이름과 디렉터리/위치 정보를 함께 담아
 * 후속 로딩, 수정, 진단 메시지가 같은 draft 세트를 기준으로 움직이게 한다.
 */
pub struct PlanningDraftStageRecord {
    // 사용자가 선택하거나 서비스가 생성한 draft 묶음의 안정적인 이름이다.
    pub draft_name: String,
    // draft 파일들이 모이는 저장 위치이다. filesystem adapter에서는 디렉터리 문자열이다.
    pub draft_directory: String,
    // 요청된 각 파일이 어느 active_path/staged_path 쌍으로 보관됐는지 알려 준다.
    pub staged_files: Vec<PlanningStagedFileRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * draft를 다시 읽을 때는 stage 메타데이터뿐 아니라 본문도 필요하다.
 * authoring 흐름은 이 레코드를 사용해 사용자가 고친 draft 내용을 다시 prompt나 승격 로직에 연결한다.
 */
pub struct PlanningDraftLoadFileRecord {
    // 승격 대상 경로이다. `staged_path`와 달리 사용자 의미를 가진 경로이다.
    pub active_path: String,
    // draft 저장소에서 읽어 온 실제 저장 위치이다.
    pub staged_path: String,
    // draft 파일 본문이다. 호출자는 이 값을 그대로 비교하거나 다시 작성할 수 있다.
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 하나의 draft 세트를 로드한 결과이다. stage 때의 응답과 같은 상위 모양을 유지하되,
 * 각 파일에 `body`를 추가해 application 계층이 저장소 구현을 몰라도 편집과 검증을 계속할 수 있다.
 */
pub struct PlanningDraftLoadRecord {
    // 로드한 draft 세트 이름이다.
    pub draft_name: String,
    // draft가 위치한 저장소 경로 또는 논리 디렉터리이다.
    pub draft_directory: String,
    // 정렬된 draft 파일 목록이다. 구현체는 호출자가 안정적인 순서로 렌더링하도록 정렬한다.
    pub staged_files: Vec<PlanningDraftLoadFileRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/*
 * planning workspace에서 application 계층이 현재 다루는 canonical 파일 묶음이다.
 * 지금은 `result_output_markdown` 한 파일만 표현하지만, 별도 레코드를 둔 덕분에 나중에
 * authority 파일이나 보조 산출물이 늘어도 포트 메서드 시그니처를 크게 흔들지 않을 수 있다.
 */
pub struct PlanningWorkspaceLoadRecord {
    // planning 결과 문서의 본문이다. 파일이 없을 수 있으므로 `Option`으로 부재를 명시한다.
    pub result_output_markdown: Option<String>,
}

impl PlanningWorkspaceLoadRecord {
    // commit/render 전에 "실제로 다룰 planning 파일이 있는가"를 빠르게 판단하는 도우미이다.
    pub fn has_any_files(&self) -> bool {
        self.result_output_markdown.is_some()
    }
}

/*
 * `PlanningWorkspacePort`는 planning service가 workspace 파일을 읽고 쓰는 유일한 outbound 계약이다.
 * service는 draft, candidate, active workspace의 의미만 알고, 실제 저장소가 파일 시스템인지
 * repo-scoped SQLite authority인지 판단하는 책임은 adapter 쪽에 둔다.
 */
pub trait PlanningWorkspacePort: Send + Sync {
    // 새 draft 파일 묶음을 안전한 staging 영역에 저장하고, 저장된 위치를 호출자에게 돌려준다.
    fn stage_planning_draft_files(
        &self,
        // planning 작업의 기준 디렉터리이다. adapter는 이 값으로 일반 workspace와 repo-scoped 저장소를 구분한다.
        workspace_dir: &str,
        // draft 세트를 다시 찾기 위한 이름이다.
        draft_name: &str,
        // 저장할 draft 파일들이다. slice를 받아 호출자가 소유권을 유지한다.
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord>;

    // 이전에 stage한 draft 세트를 본문까지 포함해 다시 읽는다.
    fn load_planning_draft_files(
        &self,
        // draft 저장소를 찾기 위한 workspace 기준이다.
        workspace_dir: &str,
        // 읽을 draft 세트 이름이다.
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord>;

    // 이미 존재하는 draft의 한 파일을 전체 본문 단위로 교체한다.
    fn replace_planning_draft_file(
        &self,
        // draft가 속한 workspace 기준이다.
        workspace_dir: &str,
        // 수정할 draft 세트 이름이다.
        draft_name: &str,
        // draft 내부에서 교체할 active 대상 경로이다.
        active_path: &str,
        // 교체 후 파일 전체 본문이다.
        body: &str,
    ) -> Result<String>;

    // 현재 active planning workspace의 canonical 파일들을 읽는다.
    fn load_planning_workspace_files(
        &self,
        // active workspace 루트 또는 repo-scoped workspace 식별자이다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // active가 아니라 candidate 영역의 planning 파일을 읽는다. 승격 전 미리보기/검증에 쓰인다.
    fn load_planning_workspace_candidate_files(
        &self,
        // candidate 파일들이 연결된 workspace 기준이다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // `PlanningWorkspaceLoadRecord` 전체를 active workspace에 반영한다.
    fn commit_planning_workspace_files(
        &self,
        // 반영할 active workspace 기준이다.
        workspace_dir: &str,
        // 파일별 존재/부재를 포함한 canonical workspace 스냅샷이다.
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()>;

    // active workspace에서 단일 planning 파일을 선택적으로 읽는다.
    fn load_optional_planning_file(
        &self,
        // 파일을 찾을 workspace 기준이다.
        workspace_dir: &str,
        // workspace 안에서의 상대 경로이다. adapter가 정규화하고 위험한 경로를 거부한다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // candidate 영역에서 단일 planning 파일을 선택적으로 읽는다.
    fn load_optional_planning_candidate_file(
        &self,
        // candidate 파일을 찾을 workspace 기준이다.
        workspace_dir: &str,
        // candidate 영역 안에서의 상대 경로이다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // active workspace의 한 파일을 쓰거나 삭제한다.
    fn replace_planning_workspace_file(
        &self,
        // 수정할 workspace 기준이다.
        workspace_dir: &str,
        // 수정할 파일의 상대 경로이다.
        relative_path: &str,
        // `Some`이면 전체 본문을 쓰고, `None`이면 해당 파일을 제거한다는 계약이다.
        body: Option<&str>,
    ) -> Result<()>;

    // active workspace에서 파일 또는 디렉터리 엔트리를 제거한다.
    fn remove_planning_workspace_entry(
        &self,
        // 제거 대상이 속한 workspace 기준이다.
        workspace_dir: &str,
        // 제거할 상대 경로이다. 파일과 디렉터리 모두 포트 계약에 포함된다.
        relative_path: &str,
    ) -> Result<()>;

    // 거절된 planning 파일을 archive 영역에 보존한다.
    fn archive_rejected_planning_file(
        &self,
        // archive 위치를 계산할 workspace 기준이다.
        workspace_dir: &str,
        // 거절 묶음의 이름이다. 여러 rejected 파일을 한 archive 아래 모을 때 쓰인다.
        archive_name: &str,
        // 원래 active workspace에서의 파일 경로이다. archive 파일명 계산의 기준이 된다.
        active_path: &str,
        // archive에 저장할 거절 당시의 본문이다.
        body: &str,
    ) -> Result<String>;
}

/*
 * `RepoScopedPlanningWorkspacePort`는 git-backed workspace의 특수 저장소 계약이다.
 * 일반 `PlanningWorkspacePort` 구현체는 workspace가 repo-scoped인지 먼저 감지한 뒤,
 * 이 trait로 위임해 active 루트, draft 저장, 파일 교체를 repository authority 관점에서 처리한다.
 */
pub trait RepoScopedPlanningWorkspacePort: Send + Sync {
    // 주어진 workspace가 repo authority를 통해 관리되는 git-backed workspace인지 판단한다.
    fn is_git_backed_workspace(&self, workspace_dir: &str) -> bool;

    // repo-scoped 저장소에서 실제 active workspace 루트로 볼 경로를 계산한다.
    fn resolve_active_workspace_root(&self, workspace_dir: &str) -> PathBuf;

    // repo authority 안에 draft 파일 묶음을 stage한다.
    fn stage_repo_scoped_draft_files(
        &self,
        // repo-scoped workspace를 식별하는 기준 문자열이다.
        workspace_dir: &str,
        // 저장할 draft 세트 이름이다.
        draft_name: &str,
        // repo authority에 넣을 draft 파일 목록이다.
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord>;

    // repo authority에서 draft 파일 묶음을 본문까지 읽어 온다.
    fn load_repo_scoped_draft_files(
        &self,
        // repo-scoped workspace 식별자이다.
        workspace_dir: &str,
        // 읽을 draft 세트 이름이다.
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord>;

    // repo authority에 저장된 draft 파일 하나를 전체 본문 단위로 교체한다.
    fn replace_repo_scoped_draft_file(
        &self,
        // repo-scoped workspace 식별자이다.
        workspace_dir: &str,
        // 수정할 draft 세트 이름이다.
        draft_name: &str,
        // draft 안에서 교체할 active 대상 경로이다.
        active_path: &str,
        // 교체할 전체 본문이다.
        body: &str,
    ) -> Result<String>;

    // repo authority가 관리하는 active workspace 파일 스냅샷을 읽는다.
    fn load_active_workspace_files(
        &self,
        // 읽을 repo-scoped workspace 식별자이다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // repo authority의 active workspace에 canonical 파일 스냅샷을 반영한다.
    fn commit_active_workspace_files(
        &self,
        // 반영 대상 repo-scoped workspace 식별자이다.
        workspace_dir: &str,
        // 저장할 planning workspace 파일 스냅샷이다.
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()>;

    // repo authority의 active workspace에서 단일 파일을 선택적으로 읽는다.
    fn load_active_planning_file(
        &self,
        // 파일을 찾을 repo-scoped workspace 식별자이다.
        workspace_dir: &str,
        // active workspace 기준 상대 경로이다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // repo authority의 active planning 파일을 쓰거나 삭제한다.
    fn replace_active_planning_file(
        &self,
        // 수정할 repo-scoped workspace 식별자이다.
        workspace_dir: &str,
        // 수정할 active 파일 상대 경로이다.
        relative_path: &str,
        // `Some`이면 저장, `None`이면 삭제를 의미해 일반 workspace 포트와 같은 계약을 유지한다.
        body: Option<&str>,
    ) -> Result<()>;

    // repo authority의 active workspace에서 파일 또는 디렉터리 엔트리를 제거한다.
    fn remove_active_planning_entry(&self, workspace_dir: &str, relative_path: &str) -> Result<()>;
}
