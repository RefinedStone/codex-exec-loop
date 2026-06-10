pub use crate::domain::planning::{
    ACTIVE_PLANNING_FILE_PATHS, RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
};
use std::fmt;

// result-output은 현재 accepted planning state를 대표하는 active planning artifact이다. admin draft,
// runtime validation, workspace adapter가 같은 domain runtime contract를 보도록 여기서 재수출한다.
// direction detail docs는 direction catalog의 항목별 상세 설명을 markdown으로 저장하는 디렉터리이다.
pub const PLANNING_DIRECTION_DOCS_DIRECTORY: &str = ".codex-exec-loop/planning/directions";
// prompt directory는 planning worker와 queue-idle review prompt 같은 prompt artifacts의 기준 위치이다.
pub const PLANNING_PROMPTS_DIRECTORY: &str = ".codex-exec-loop/planning/prompts";
// queue-idle prompt는 queue에 active task가 없을 때 다음 planning response를 유도하는 기본 prompt이다.
pub const DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH: &str =
    ".codex-exec-loop/planning/prompts/queue-idle-review.md";
// drafts directory는 accepted state에 쓰기 전 operator/admin이 검토하는 staged planning files를 둔다.
pub const PLANNING_DRAFTS_DIRECTORY: &str = ".codex-exec-loop/planning/drafts";
// rejected directory는 validation이나 operator review에서 받아들이지 않은 planning output을 보관한다.
pub const PLANNING_REJECTED_DIRECTORY: &str = ".codex-exec-loop/planning/rejected";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningDraftNameError {
    Empty,
    DotSegment,
    InvalidCharacter(char),
}

impl fmt::Display for PlanningDraftNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("draft name must not be empty"),
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

// shared contract tests는 path normalization policy를 application layer 가까이에 고정한다.
#[cfg(test)]
mod tests {
    // test는 canonical lookup 함수와 expected canonical constant만 사용해 public contract를 검증한다.
    use super::{
        PlanningDraftNameError, RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
        validate_planning_draft_name,
    };

    // 이 test는 absolute active file은 canonical path로 인정하고, legacy/raw authority 또는 일반
    // source file은 active planning artifact로 오인하지 않는다는 경계를 확인한다.
    #[test]
    fn canonical_active_planning_file_path_matches_relative_and_absolute_paths() {
        assert_eq!(
            // workspace absolute path라도 active planning suffix가 directory boundary에 맞으면
            // canonical relative path로 접힌다.
            canonical_active_planning_file_path(
                "/tmp/workspace/.codex-exec-loop/planning/result-output.md"
            ),
            Some(RESULT_OUTPUT_FILE_PATH)
        );
        assert!(
            // Windows separator는 정규화되지만, raw DB task authority path는 active allowlist에 없으므로
            // None이어야 한다.
            canonical_active_planning_file_path(
                r"C:\workspace\.codex-exec-loop\planning\DB task authority"
            )
            .is_none()
        );
        // 일반 repo source file은 planning artifact suffix가 없으므로 active planning file이 아니다.
        assert!(canonical_active_planning_file_path("src/main.rs").is_none());
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

        assert_eq!(
            validate_planning_draft_name(""),
            Err(PlanningDraftNameError::Empty)
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
}
