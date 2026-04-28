pub const RESULT_OUTPUT_FILE_PATH: &str = ".codex-exec-loop/planning/result-output.md";
pub const PLANNING_DIRECTION_DOCS_DIRECTORY: &str = ".codex-exec-loop/planning/directions";
pub const PLANNING_PROMPTS_DIRECTORY: &str = ".codex-exec-loop/planning/prompts";
pub const DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH: &str =
    ".codex-exec-loop/planning/prompts/queue-idle-review.md";
pub const PLANNING_DRAFTS_DIRECTORY: &str = ".codex-exec-loop/planning/drafts";
pub const PLANNING_REJECTED_DIRECTORY: &str = ".codex-exec-loop/planning/rejected";
pub const ACTIVE_PLANNING_FILE_PATHS: [&str; 1] = [RESULT_OUTPUT_FILE_PATH];

pub fn canonical_active_planning_file_path(path: &str) -> Option<&'static str> {
    let normalized = path.replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");

    ACTIVE_PLANNING_FILE_PATHS
        .iter()
        .copied()
        .find(|candidate| {
            normalized
                .strip_suffix(candidate)
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('/'))
        })
}

pub fn default_direction_detail_doc_path(direction_id: &str) -> String {
    format!(
        "{PLANNING_DIRECTION_DOCS_DIRECTORY}/{}.md",
        direction_id.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::{RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path};

    #[test]
    fn canonical_active_planning_file_path_matches_relative_and_absolute_paths() {
        assert_eq!(
            canonical_active_planning_file_path(
                "/tmp/workspace/.codex-exec-loop/planning/result-output.md"
            ),
            Some(RESULT_OUTPUT_FILE_PATH)
        );
        assert!(
            canonical_active_planning_file_path(
                r"C:\workspace\.codex-exec-loop\planning\DB task authority"
            )
            .is_none()
        );
        assert!(canonical_active_planning_file_path("src/main.rs").is_none());
    }
}
