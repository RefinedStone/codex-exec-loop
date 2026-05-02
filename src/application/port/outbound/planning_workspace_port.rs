// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use anyhow::Result;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::path::PathBuf;

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct PlanningDraftFileRecord {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub active_path: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub body: String,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct PlanningStagedFileRecord {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub active_path: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub staged_path: String,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct PlanningDraftStageRecord {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub draft_name: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub draft_directory: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub staged_files: Vec<PlanningStagedFileRecord>,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct PlanningDraftLoadFileRecord {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub active_path: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub staged_path: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub body: String,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct PlanningDraftLoadRecord {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub draft_name: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub draft_directory: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub staged_files: Vec<PlanningDraftLoadFileRecord>,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct PlanningWorkspaceLoadRecord {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub result_output_markdown: Option<String>,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl PlanningWorkspaceLoadRecord {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn has_any_files(&self) -> bool {
        self.result_output_markdown.is_some()
    }
}

// 학습 주석: `trait`는 타입이 제공해야 하는 동작의 계약을 정의하며, 다른 구현체를 같은 방식으로 다루게 합니다.
pub trait PlanningWorkspacePort: Send + Sync {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn stage_planning_draft_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        draft_name: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_planning_draft_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn replace_planning_draft_file(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        draft_name: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        active_path: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        body: &str,
    ) -> Result<String>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_planning_workspace_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_planning_workspace_candidate_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn commit_planning_workspace_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_optional_planning_file(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_optional_planning_candidate_file(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn replace_planning_workspace_file(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        relative_path: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        body: Option<&str>,
    ) -> Result<()>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn remove_planning_workspace_entry(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        relative_path: &str,
    ) -> Result<()>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn archive_rejected_planning_file(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        archive_name: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        active_path: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        body: &str,
    ) -> Result<String>;
}

// 학습 주석: `trait`는 타입이 제공해야 하는 동작의 계약을 정의하며, 다른 구현체를 같은 방식으로 다루게 합니다.
pub trait RepoScopedPlanningWorkspacePort: Send + Sync {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn is_git_backed_workspace(&self, workspace_dir: &str) -> bool;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn resolve_active_workspace_root(&self, workspace_dir: &str) -> PathBuf;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn stage_repo_scoped_draft_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        draft_name: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        files: &[PlanningDraftFileRecord],
    ) -> Result<PlanningDraftStageRecord>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_repo_scoped_draft_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        draft_name: &str,
    ) -> Result<PlanningDraftLoadRecord>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn replace_repo_scoped_draft_file(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        draft_name: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        active_path: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        body: &str,
    ) -> Result<String>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_active_workspace_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceLoadRecord>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn commit_active_workspace_files(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        record: &PlanningWorkspaceLoadRecord,
    ) -> Result<()>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn load_active_planning_file(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        relative_path: &str,
    ) -> Result<Option<String>>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn replace_active_planning_file(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_dir: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        relative_path: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        body: Option<&str>,
    ) -> Result<()>;

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn remove_active_planning_entry(&self, workspace_dir: &str, relative_path: &str) -> Result<()>;
}
