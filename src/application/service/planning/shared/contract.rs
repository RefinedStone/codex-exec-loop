// 학습 주석: result-output은 현재 accepted planning state를 대표하는 active planning artifact입니다.
// admin draft, runtime validation, workspace adapter가 같은 경로를 보도록 shared contract에 둡니다.
pub const RESULT_OUTPUT_FILE_PATH: &str = ".codex-exec-loop/planning/result-output.md";
// 학습 주석: direction detail docs는 direction catalog의 항목별 상세 설명을 markdown으로 저장하는 디렉터리입니다.
pub const PLANNING_DIRECTION_DOCS_DIRECTORY: &str = ".codex-exec-loop/planning/directions";
// 학습 주석: prompt directory는 planning worker와 queue-idle review prompt 같은 prompt artifacts의 기준 위치입니다.
pub const PLANNING_PROMPTS_DIRECTORY: &str = ".codex-exec-loop/planning/prompts";
// 학습 주석: queue-idle prompt는 queue에 active task가 없을 때 다음 planning response를 유도하는 기본 prompt입니다.
pub const DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH: &str =
    ".codex-exec-loop/planning/prompts/queue-idle-review.md";
// 학습 주석: drafts directory는 accepted state에 쓰기 전 operator/admin이 검토하는 staged planning files를 둡니다.
pub const PLANNING_DRAFTS_DIRECTORY: &str = ".codex-exec-loop/planning/drafts";
// 학습 주석: rejected directory는 validation이나 operator review에서 받아들이지 않은 planning output을 보관합니다.
pub const PLANNING_REJECTED_DIRECTORY: &str = ".codex-exec-loop/planning/rejected";
// 학습 주석: active planning files는 "현재 planning state로 간주되는 파일"의 allowlist입니다. 지금은
// result-output 하나뿐이라 배열 크기가 1이지만, canonical lookup은 확장을 전제로 작성되어 있습니다.
pub const ACTIVE_PLANNING_FILE_PATHS: [&str; 1] = [RESULT_OUTPUT_FILE_PATH];

// 학습 주석: 이 함수는 임의의 path 문자열이 active planning artifact를 가리키는지 canonical path로 판정합니다.
// absolute path, workspace-relative path, Windows separator가 섞여도 같은 contract path를 반환하게 합니다.
pub fn canonical_active_planning_file_path(path: &str) -> Option<&'static str> {
    // 학습 주석: Windows 입력도 `/` 기준으로 비교하기 위해 separator를 먼저 통일합니다.
    let normalized = path.replace('\\', "/");
    // 학습 주석: caller가 `./.codex-exec-loop/...`처럼 현재 directory prefix를 붙여도 같은 logical path로 봅니다.
    let normalized = normalized.trim_start_matches("./");

    // 학습 주석: allowlist 중 하나가 normalized path의 끝에 directory boundary를 두고 붙어 있는지 찾습니다.
    // 이렇게 하면 `/tmp/workspace/.codex-exec-loop/...` absolute path도 canonical relative contract로 접힙니다.
    ACTIVE_PLANNING_FILE_PATHS
        .iter()
        .copied()
        .find(|candidate| {
            normalized
                .strip_suffix(candidate)
                // 학습 주석: suffix 앞이 비어 있으면 exact relative path이고, `/`로 끝나면 absolute/path-prefixed
                // match입니다. 다른 문자로 끝나면 `fooresult-output.md` 같은 우연한 suffix match라 거부합니다.
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('/'))
        })
}

// 학습 주석: direction id에서 기본 detail doc path를 만듭니다. directions authoring은 id를 저장하고,
// detail body는 이 convention 아래의 markdown file로 연결합니다.
pub fn default_direction_detail_doc_path(direction_id: &str) -> String {
    format!(
        "{PLANNING_DIRECTION_DOCS_DIRECTORY}/{}.md",
        // 학습 주석: id 주변 공백은 file name convention에 포함하지 않습니다. caller가 입력 field에서 읽은
        // id를 넘겨도 stable path가 나오도록 trim합니다.
        direction_id.trim()
    )
}

// 학습 주석: shared contract tests는 path normalization policy를 application layer 가까이에 고정합니다.
#[cfg(test)]
mod tests {
    // 학습 주석: test는 canonical lookup 함수와 expected canonical constant만 사용해 public contract를 검증합니다.
    use super::{RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path};

    // 학습 주석: 이 test는 absolute active file은 canonical path로 인정하고, legacy/raw authority 또는 일반
    // source file은 active planning artifact로 오인하지 않는다는 경계를 확인합니다.
    #[test]
    fn canonical_active_planning_file_path_matches_relative_and_absolute_paths() {
        assert_eq!(
            // 학습 주석: workspace absolute path라도 active planning suffix가 directory boundary에 맞으면
            // canonical relative path로 접힙니다.
            canonical_active_planning_file_path(
                "/tmp/workspace/.codex-exec-loop/planning/result-output.md"
            ),
            Some(RESULT_OUTPUT_FILE_PATH)
        );
        assert!(
            // 학습 주석: Windows separator는 정규화되지만, raw DB task authority path는 active allowlist에 없으므로
            // None이어야 합니다.
            canonical_active_planning_file_path(
                r"C:\workspace\.codex-exec-loop\planning\DB task authority"
            )
            .is_none()
        );
        // 학습 주석: 일반 repo source file은 planning artifact suffix가 없으므로 active planning file이 아닙니다.
        assert!(canonical_active_planning_file_path("src/main.rs").is_none());
    }
}
