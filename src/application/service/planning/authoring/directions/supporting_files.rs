/*
 * directions supporting_files helper는 direction authority 자체가 아니라 그 authority가 참조하는 보조
 * markdown 파일을 다룬다. direction detail doc과 queue-idle prompt는 worker prompt와 validation에서
 * 함께 읽히므로, authoring/admin draft 흐름이 같은 path mutation 규칙을 쓰도록 이 작은 service helper에 모아 둔다.
 */
use anyhow::{Result, anyhow};

use crate::domain::planning::DirectionCatalogDocument;

// TOML/string field에서 공백뿐인 값을 "없음"으로 다루는 공통 guard다.
pub(super) fn trimmed_non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
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
 * reset, directions maintenance가 이 setter를 거치면 queue-idle review prompt 위치가 validation과
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
    use super::{set_direction_detail_doc_path, set_queue_idle_prompt_path, trimmed_non_empty};
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
        QueueIdleConfig,
    };

    fn directions() -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General".to_string(),
                summary: "General planning work.".to_string(),
                success_criteria: vec!["done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        }
    }

    #[test]
    fn supporting_file_helpers_trim_update_and_report_unknown_direction() {
        assert_eq!(trimmed_non_empty(" detail.md "), Some("detail.md"));
        assert_eq!(trimmed_non_empty(" \n\t "), None);

        let mut directions = directions();
        set_queue_idle_prompt_path(&mut directions, "prompts/queue-idle.md");
        assert_eq!(
            directions.queue_idle.prompt_path,
            "prompts/queue-idle.md".to_string()
        );

        let error = set_direction_detail_doc_path(
            &mut directions,
            "missing-workstream",
            "directions/missing.md",
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "unknown direction id: missing-workstream"
        );

        set_direction_detail_doc_path(
            &mut directions,
            " general-workstream ",
            "directions/general.md",
        )
        .expect("known direction should update");
        assert_eq!(
            directions.directions[0].detail_doc_path,
            "directions/general.md".to_string()
        );
    }
}
