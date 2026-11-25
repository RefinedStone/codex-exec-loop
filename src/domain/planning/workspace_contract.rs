use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningResetTarget {
    Queue,
    Directions,
    All,
}

impl PlanningResetTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Directions => "directions",
            Self::All => "all",
        }
    }

    // directions와 all은 authority 문서와 prompt 파일을 다시 쓴다. 모든 입구가 같은 확인을 거쳐야 한다.
    pub fn requires_confirmation(self) -> bool {
        matches!(self, Self::Directions | Self::All)
    }
}

/*
 * reset 범위와 operator 확인 여부를 함께 나른다. 확인 규칙을 입구(TUI, CLI, Telegram, admin)마다 두지 않고
 * reset service 한 곳에서 검사하기 위한 값이다. 확인 문법(`confirm`, `--confirm`, JSON `confirm`)은 입구가 정한다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningResetRequest {
    pub target: PlanningResetTarget,
    pub confirmed: bool,
}

impl PlanningResetRequest {
    pub fn new(target: PlanningResetTarget, confirmed: bool) -> Self {
        Self { target, confirmed }
    }

    // queue는 파생 상태만 지우므로 확인 없이 허용한다.
    pub fn is_allowed(self) -> bool {
        self.confirmed || !self.target.requires_confirmation()
    }
}

/*
 * Akra가 작업 폴더 안에 두는 디렉터리 이름은 여기 한 곳에서 정한다 (#2588).
 * `.akra/`에는 프로젝트 설정과 정책 파일(`config.toml`, `issue-policy.toml`, `issue-automation.toml`,
 * `parallel-agent-profiles.json`, `delegation.toml`)도 있다. 아래 하위 디렉터리 이름은 그 파일들과 겹치지 않는다.
 * 다른 곳의 경로 상수는 이 값으로 시작해야 한다. tests의 `workspace_paths_share_one_root`가 이를 지킨다.
 */
pub const AKRA_WORKSPACE_DIRECTORY: &str = ".akra";
// #2588 이전의 작업 폴더 디렉터리이다. 작업 폴더와 DB의 이전(migration) 코드만 이 이름을 읽는다.
pub const LEGACY_WORKSPACE_DIRECTORY: &str = ".codex-exec-loop";
// 이전 디렉터리에서 `.akra/`로 옮기는 하위 디렉터리이다.
pub const RELOCATED_WORKSPACE_ENTRIES: [&str; 3] = ["planning", "runtime", "followups"];
// planning 파일의 기준 디렉터리이다. 저장된 상대 경로는 모두 이 접두어로 시작한다.
pub const PLANNING_ROOT_DIRECTORY: &str = ".akra/planning";
pub const PLANNING_ROOT_PREFIX: &str = ".akra/planning/";
// runtime 디렉터리는 잠금, 격리(quarantine), trace log 같은 비공개 상태를 둔다. Git에 올리지 않는다.
pub const PLANNING_RUNTIME_DIRECTORY: &str = ".akra/runtime";
// result-output은 현재 accepted planning state를 대표하는 active planning artifact이다. admin draft,
// runtime validation, workspace adapter가 같은 domain runtime contract를 보도록 여기서 재수출한다.
// direction detail docs는 direction catalog의 항목별 상세 설명을 markdown으로 저장하는 디렉터리이다.
pub const PLANNING_DIRECTION_DOCS_DIRECTORY: &str = ".akra/planning/directions";
// prompt directory는 planning worker와 queue-idle review prompt 같은 prompt artifacts의 기준 위치이다.
pub const PLANNING_PROMPTS_DIRECTORY: &str = ".akra/planning/prompts";
// queue-idle prompt는 queue에 active task가 없을 때 다음 planning response를 유도하는 기본 prompt이다.
pub const DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH: &str = ".akra/planning/prompts/queue-idle-review.md";
// drafts directory는 accepted state에 쓰기 전 operator/admin이 검토하는 staged planning files를 둔다.
pub const PLANNING_DRAFTS_DIRECTORY: &str = ".akra/planning/drafts";
// rejected directory는 validation이나 operator review에서 받아들이지 않은 planning output을 보관한다.
pub const PLANNING_REJECTED_DIRECTORY: &str = ".akra/planning/rejected";
// draft name은 filesystem directory segment로도 쓰이므로 common filesystem filename limit 아래에 둔다.
pub const PLANNING_DRAFT_NAME_MAX_BYTES: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningDraftNameError {
    Empty,
    TooLong,
    DotSegment,
    InvalidCharacter(char),
}

impl fmt::Display for PlanningDraftNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("draft name must not be empty"),
            Self::TooLong => write!(
                formatter,
                "draft name is too long (max {PLANNING_DRAFT_NAME_MAX_BYTES} bytes)"
            ),
            Self::DotSegment => formatter.write_str("draft name must not be . or .."),
            Self::InvalidCharacter(character) => write!(
                formatter,
                "draft name contains invalid character `{character}`"
            ),
        }
    }
}

impl std::error::Error for PlanningDraftNameError {}

pub fn validate_planning_draft_name(draft_name: &str) -> Result<(), PlanningDraftNameError> {
    /*
     * draft_name is a storage segment, not a path. Keep the grammar smaller than
     * URL/path syntax so inbound routes, filesystem staging, and repo-scoped DB
     * records all share one stable identity boundary.
     */
    if draft_name.is_empty() {
        return Err(PlanningDraftNameError::Empty);
    }
    if draft_name.len() > PLANNING_DRAFT_NAME_MAX_BYTES {
        return Err(PlanningDraftNameError::TooLong);
    }
    if draft_name == "." || draft_name == ".." {
        return Err(PlanningDraftNameError::DotSegment);
    }
    for character in draft_name.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            continue;
        }
        return Err(PlanningDraftNameError::InvalidCharacter(character));
    }
    Ok(())
}

// direction id에서 기본 detail doc path를 만든다. directions authoring은 id를 저장하고, detail
// body는 이 convention 아래의 markdown file로 연결한다.
pub fn default_direction_detail_doc_path(direction_id: &str) -> String {
    format!(
        "{PLANNING_DIRECTION_DOCS_DIRECTORY}/{}.md",
        // id 주변 공백은 file name convention에 포함하지 않는다. caller가 입력 field에서 읽은 id를
        // 넘겨도 stable path가 나오도록 trim한다.
        direction_id.trim()
    )
}

/*
 * 이전 작업 폴더 경로(`.codex-exec-loop/<x>`)를 현재 경로(`.akra/<x>`)로 바꾼다. 이전 경로가 아니면 None이다.
 * 이전 이후에도 남아 있을 수 있는 기록(예: file-sync manifest)을 읽을 때 쓴다.
 */
// Only the Unix direct-filesystem file-sync manifest reads it; Windows keeps that baseline in SQLite.
#[cfg_attr(windows, allow(dead_code))]
pub fn relocate_legacy_workspace_path(path: &str) -> Option<String> {
    let rest = path
        .strip_prefix(LEGACY_WORKSPACE_DIRECTORY)?
        .strip_prefix('/')?;
    Some(format!("{AKRA_WORKSPACE_DIRECTORY}/{rest}"))
}

// workspace contract tests keep path and storage identity rules in the pure domain layer.
#[cfg(test)]
mod tests {
    // test는 canonical lookup 함수와 expected canonical constant만 사용해 public contract를 검증한다.
    use super::{
        PLANNING_DRAFT_NAME_MAX_BYTES, PlanningDraftNameError, PlanningResetRequest,
        PlanningResetTarget, validate_planning_draft_name,
    };
    use crate::domain::planning::runtime_contracts::{
        RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
    };

    // 이 test는 absolute active file은 canonical path로 인정하고, legacy/raw authority 또는 일반
    // source file은 active planning artifact로 오인하지 않는다는 경계를 확인한다.
    #[test]
    fn canonical_active_planning_file_path_matches_relative_and_absolute_paths() {
        assert_eq!(
            // workspace absolute path라도 active planning suffix가 directory boundary에 맞으면
            // canonical relative path로 접힌다.
            canonical_active_planning_file_path("/tmp/workspace/.akra/planning/result-output.md"),
            Some(RESULT_OUTPUT_FILE_PATH)
        );
        assert!(
            // Windows separator는 정규화되지만, raw DB task authority path는 active allowlist에 없으므로
            // None이어야 한다.
            canonical_active_planning_file_path(r"C:\workspace\.akra\planning\DB task authority")
                .is_none()
        );
        // 일반 repo source file은 planning artifact suffix가 없으므로 active planning file이 아니다.
        assert!(canonical_active_planning_file_path("src/main.rs").is_none());
    }

    #[test]
    fn workspace_paths_share_one_root() {
        use super::*;
        assert_eq!(
            PLANNING_ROOT_DIRECTORY,
            format!("{AKRA_WORKSPACE_DIRECTORY}/planning")
        );
        assert_eq!(PLANNING_ROOT_PREFIX, format!("{PLANNING_ROOT_DIRECTORY}/"));
        assert_eq!(
            PLANNING_RUNTIME_DIRECTORY,
            format!("{AKRA_WORKSPACE_DIRECTORY}/runtime")
        );
        for path in [
            RESULT_OUTPUT_FILE_PATH,
            PLANNING_DIRECTION_DOCS_DIRECTORY,
            PLANNING_PROMPTS_DIRECTORY,
            DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH,
            PLANNING_DRAFTS_DIRECTORY,
            PLANNING_REJECTED_DIRECTORY,
        ] {
            assert!(path.starts_with(PLANNING_ROOT_PREFIX), "{path}");
        }
    }

    #[test]
    fn legacy_workspace_paths_relocate_only_at_the_directory_boundary() {
        use super::relocate_legacy_workspace_path;
        assert_eq!(
            relocate_legacy_workspace_path(".codex-exec-loop/planning/result-output.md").as_deref(),
            Some(".akra/planning/result-output.md")
        );
        assert_eq!(
            relocate_legacy_workspace_path(".codex-exec-loop-copy/planning/x.md"),
            None
        );
        assert_eq!(relocate_legacy_workspace_path(".akra/planning/x.md"), None);
    }

    #[test]
    fn planning_draft_name_is_a_single_safe_storage_segment() {
        for valid in [
            "admin-20260610T101010Z-123456789",
            "bootstrap-20260610T101010Z-123456789",
            "directions-20260610T101010Z-123456789",
            "manual.v2_draft",
        ] {
            assert!(
                validate_planning_draft_name(valid).is_ok(),
                "{valid} should be accepted"
            );
        }
        assert!(validate_planning_draft_name(&"a".repeat(PLANNING_DRAFT_NAME_MAX_BYTES)).is_ok());

        assert_eq!(
            validate_planning_draft_name(""),
            Err(PlanningDraftNameError::Empty)
        );
        assert_eq!(
            validate_planning_draft_name(&"a".repeat(PLANNING_DRAFT_NAME_MAX_BYTES + 1)),
            Err(PlanningDraftNameError::TooLong)
        );
        assert_eq!(
            validate_planning_draft_name("."),
            Err(PlanningDraftNameError::DotSegment)
        );
        assert_eq!(
            validate_planning_draft_name(".."),
            Err(PlanningDraftNameError::DotSegment)
        );
        for invalid in [
            "../outside",
            "bad/name",
            "bad\\name",
            "bad:name",
            "bad name",
            "bad%2Fname",
            "한글",
            "bad\nname",
        ] {
            assert!(
                validate_planning_draft_name(invalid).is_err(),
                "{invalid:?} should be rejected"
            );
        }
    }

    #[test]
    fn planning_reset_targets_have_stable_labels() {
        assert_eq!(PlanningResetTarget::Queue.label(), "queue");
        assert_eq!(PlanningResetTarget::Directions.label(), "directions");
        assert_eq!(PlanningResetTarget::All.label(), "all");
    }

    #[test]
    fn only_queue_reset_is_allowed_without_confirmation() {
        let request = PlanningResetRequest::new;
        assert!(request(PlanningResetTarget::Queue, false).is_allowed());
        assert!(!request(PlanningResetTarget::Directions, false).is_allowed());
        assert!(!request(PlanningResetTarget::All, false).is_allowed());
        assert!(request(PlanningResetTarget::Directions, true).is_allowed());
        assert!(request(PlanningResetTarget::All, true).is_allowed());
    }
}
