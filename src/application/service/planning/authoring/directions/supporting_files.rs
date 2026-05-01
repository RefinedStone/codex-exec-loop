use anyhow::{Result, anyhow};

use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
#[cfg(test)]
use crate::application::service::planning::shared::contract::{
    PLANNING_DIRECTION_DOCS_DIRECTORY, default_direction_detail_doc_path,
};
#[cfg(test)]
use crate::application::service::planning::shared::planning_paths::is_valid_planning_markdown_path;
use crate::domain::planning::DirectionCatalogDocument;

pub(super) fn trimmed_non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

pub(super) fn normalize_queue_idle_review_prompt_markdown(prompt_markdown: &str) -> String {
    if is_legacy_queue_idle_review_prompt(prompt_markdown) {
        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string()
    } else {
        prompt_markdown.to_string()
    }
}

fn is_legacy_queue_idle_review_prompt(prompt_markdown: &str) -> bool {
    let normalized = prompt_markdown.to_lowercase();
    let source_markers = ["directions.toml", "task-ledger"];
    let legacy_behavior_markers = [
        "latest answer clearly implies",
        "latest accepted answer",
        "task catalog compatibility",
    ];

    source_markers
        .iter()
        .any(|legacy_marker| normalized.contains(legacy_marker))
        && legacy_behavior_markers
            .iter()
            .any(|legacy_marker| normalized.contains(legacy_marker))
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn build_default_detail_doc_markdown(
    direction: &crate::domain::planning::DirectionDefinition,
) -> String {
    let mut lines = vec![
        format!("# {}", direction.title.trim()),
        String::new(),
        format!("- Direction id: `{}`", direction.id.trim()),
        String::new(),
        "## Goal".to_string(),
        String::new(),
        direction.summary.trim().to_string(),
    ];
    if !direction.success_criteria.is_empty() {
        lines.push(String::new());
        lines.push("## Success criteria".to_string());
        lines.push(String::new());
        lines.extend(
            direction
                .success_criteria
                .iter()
                .map(|criterion| format!("- {}", criterion.trim())),
        );
    }
    if !direction.scope_hints.is_empty() {
        lines.push(String::new());
        lines.push("## Scope hints".to_string());
        lines.push(String::new());
        lines.extend(
            direction
                .scope_hints
                .iter()
                .map(|hint| format!("- {}", hint.trim())),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) fn default_validated_direction_detail_doc_path(direction_id: &str) -> Result<String> {
    let fallback_path = default_direction_detail_doc_path(direction_id);
    if is_valid_planning_markdown_path(&fallback_path, PLANNING_DIRECTION_DOCS_DIRECTORY) {
        Ok(fallback_path)
    } else {
        Err(anyhow!(
            "direction {} does not produce a safe default detail_doc_path",
            direction_id.trim()
        ))
    }
}

pub(super) fn set_direction_detail_doc_path(
    directions: &mut DirectionCatalogDocument,
    direction_id: &str,
    detail_doc_path: &str,
) -> Result<()> {
    let Some(direction) = directions
        .directions
        .iter_mut()
        .find(|direction| direction.id.trim() == direction_id.trim())
    else {
        return Err(anyhow!("unknown direction id: {}", direction_id.trim()));
    };
    direction.detail_doc_path = detail_doc_path.to_string();
    Ok(())
}

pub(super) fn set_queue_idle_prompt_path(
    directions: &mut DirectionCatalogDocument,
    prompt_path: &str,
) {
    directions.queue_idle.prompt_path = prompt_path.to_string();
}

#[cfg(test)]
mod tests {
    use super::normalize_queue_idle_review_prompt_markdown;
    use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;

    #[test]
    fn queue_idle_review_prompt_normalizes_legacy_file_authority_copy() {
        let legacy_prompt = r#"# Queue Idle Review Prompt

- `directions.toml`의 direction 목표, success criteria, detail doc를 기준으로 현재 task-ledger work list를 다시 점검하세요.
- When the latest answer clearly implies a next step, derive it.
"#;

        let normalized = normalize_queue_idle_review_prompt_markdown(legacy_prompt);

        assert_eq!(normalized, DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN);
        assert!(normalized.contains("[accepted-db-direction-authority]"));
        assert!(normalized.contains("[accepted-db-task-authority]"));
        assert!(normalized.contains("[db-queue-projection]"));
        assert!(!normalized.contains("directions.toml"));
        assert!(!normalized.contains("task-ledger"));
        assert!(!normalized.contains("latest answer clearly implies"));
    }

    #[test]
    fn queue_idle_review_prompt_keeps_db_authority_copy() {
        let prompt = "# Queue Idle Review Prompt\n\n- Use accepted DB authority.";

        assert_eq!(normalize_queue_idle_review_prompt_markdown(prompt), prompt);
    }

    #[test]
    fn queue_idle_review_prompt_keeps_custom_copy_that_mentions_legacy_terms() {
        let prompt = "# Queue Idle Review Prompt\n\n- Explain why directions.toml and task-ledger are legacy terms, but keep accepted DB authority as the source of truth.";

        assert_eq!(normalize_queue_idle_review_prompt_markdown(prompt), prompt);
    }
}
