/*
 * directions supporting_files helper는 direction authority 자체가 아니라 그 authority가 참조하는 보조
 * markdown 파일을 다룬다. direction detail doc과 queue-idle prompt는 worker prompt와 validation에서
 * 함께 읽히므로, authoring/doctor/admin draft 흐름이 같은 생성/정규화 규칙을 쓰도록 이 작은 service
 * helper에 모아 둔다.
 */
use anyhow::{Result, anyhow};

use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
#[cfg(test)]
use crate::application::service::planning::shared::contract::{
    PLANNING_DIRECTION_DOCS_DIRECTORY, default_direction_detail_doc_path,
};
#[cfg(test)]
use crate::application::service::planning::shared::planning_paths::is_valid_planning_markdown_path;
use crate::domain::planning::DirectionCatalogDocument;

// TOML/string field에서 공백뿐인 값을 "없음"으로 다루는 공통 guard다.
pub(super) fn trimmed_non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

/*
 * queue-idle review prompt는 예전 파일 authority(`directions.toml`, `task-ledger`)를 참조하던 시절의
 * 문구가 workspace에 남아 있을 수 있다. runtime은 이제 accepted DB direction/task authority와 DB queue
 * projection을 기준으로 자동 후속 작업을 평가하므로, legacy prompt만 기본 DB authority copy로 교체한다.
 */
pub(super) fn normalize_queue_idle_review_prompt_markdown(prompt_markdown: &str) -> String {
    if is_legacy_queue_idle_review_prompt(prompt_markdown) {
        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string()
    } else {
        prompt_markdown.to_string()
    }
}

/*
 * legacy 판별은 source marker와 behavior marker가 함께 있을 때만 참으로 본다. 단순히 migration
 * 문서에서 `directions.toml`을 언급하는 custom prompt까지 덮어쓰면 operator가 의도적으로 작성한
 * 안내문을 잃기 때문이다.
 */
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
/*
 * 기본 detail doc 본문은 direction metadata를 사람이 편집할 수 있는 markdown scaffold로 투영한다.
 * 실제 production 생성은 doctor/admin 흐름에서 direction 정보를 읽어 같은 구조의 문서를 만들고,
 * 테스트는 이 helper로 제목/goal/success criteria/scope hints 배치 계약을 검증한다.
 */
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
    /*
     * success criteria와 scope hints는 비어 있을 때 section 자체를 생략한다. 빈 heading을 만들지 않아
     * generated detail doc이 operator가 바로 채울 수 있는 짧은 scaffold로 유지된다.
     */
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
/*
 * default detail doc path는 direction id에서 만들어지지만, path traversal이나 잘못된 확장자를 막는
 * validation contract도 함께 고정되어야 한다. 테스트 helper가 safe path 여부를 바로 검증해 direction
 * scaffold가 planning direction docs directory 밖으로 나가지 않게 한다.
 */
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

/*
 * detail doc 생성/repair는 markdown 파일만 만드는 것으로 끝나지 않는다. direction catalog의 해당
 * direction에도 새 path를 기록해야 validation, runtime prompt fragment, admin overview가 같은 supporting
 * file을 찾을 수 있다.
 */
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

/*
 * queue-idle prompt path는 direction별 값이 아니라 catalog-level queue_idle 설정이다. authoring init,
 * reset, directions maintenance가 이 setter를 거치면 queue idle review prompt 위치가 validation과
 * runtime prompt assembly에서 같은 source of truth로 읽힌다.
 */
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
        /*
         * source marker와 behavior marker가 함께 있는 오래된 prompt는 DB authority runtime과 맞지 않는다.
         * migration fallback이 기본 prompt로 교체하는지 확인한다.
         */
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
        // 이미 DB authority를 기준으로 쓰인 prompt는 operator custom copy로 보고 보존한다.
        let prompt = "# Queue Idle Review Prompt\n\n- Use accepted DB authority.";

        assert_eq!(normalize_queue_idle_review_prompt_markdown(prompt), prompt);
    }

    #[test]
    fn queue_idle_review_prompt_keeps_custom_copy_that_mentions_legacy_terms() {
        /*
         * legacy 용어를 설명 목적으로 언급하는 prompt까지 덮어쓰면 operator가 작성한 migration guidance가
         * 사라진다. behavior marker가 없으면 그대로 둔다.
         */
        let prompt = "# Queue Idle Review Prompt\n\n- Explain why directions.toml and task-ledger are legacy terms, but keep accepted DB authority as the source of truth.";

        assert_eq!(normalize_queue_idle_review_prompt_markdown(prompt), prompt);
    }
}
