use std::path::{Path, PathBuf};

use super::protocol::TurnInputItem;

/*
 * 이 adapter는 planning worker turn input에 repo-local skill 문서를 붙이는 작은 app-server
 * outbound boundary다. worker prompt가 plain text만 받으면 task queue mutation JSON 계약을 잊기 쉬우므로,
 * turn payload 앞에 `type=skill` item을 추가해 evaluator 전용 지침을 protocol 레벨에서 고정한다.
 */
const PLANNING_QUEUE_MUTATION_SKILL_NAME: &str = "akra-planning-queue-mutation";
const PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH: &str =
    "docs/agent/skills/akra-planning-queue-mutation/SKILL.md";

#[derive(Debug, Clone)]
pub(super) struct PlanningWorkerSkillAdapter {
    // repository_root만 보관해 app-server connection과 filesystem read 책임을 이 adapter 밖에 둔다.
    repository_root: PathBuf,
}

impl PlanningWorkerSkillAdapter {
    /*
     * production constructor는 compile-time manifest dir을 repository root로 사용한다.
     * binary를 어느 working directory에서 실행하더라도 planning worker가 checkout 안의 SKILL.md를
     * 안정적으로 참조해야 하기 때문이다.
     */
    pub(super) fn new() -> Self {
        Self {
            repository_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        }
    }

    #[cfg(test)]
    pub(super) fn from_repository_root(repository_root: impl Into<PathBuf>) -> Self {
        Self {
            repository_root: repository_root.into(),
        }
    }

    /*
     * planning worker prompt 앞에 이 item이 들어가야 worker가 post-turn evaluator 역할과
     * PlanningTaskMutationService JSON contract를 먼저 읽은 뒤 자연어 prompt를 해석한다.
     */
    pub(super) fn queue_mutation_skill_input(&self) -> TurnInputItem {
        TurnInputItem::skill(
            PLANNING_QUEUE_MUTATION_SKILL_NAME,
            skill_path(&self.repository_root),
        )
    }
}

/*
 * skill_path는 repo-relative skill location을 app-server가 읽을 path string으로 투영한다.
 * 반환 타입은 TurnInputItem::Skill payload가 JSON string field로 serialize되는 wire contract에 맞춘 것이다.
 */
fn skill_path(repository_root: &Path) -> String {
    repository_root
        .join(PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::{PLANNING_QUEUE_MUTATION_SKILL_NAME, PlanningWorkerSkillAdapter, skill_path};

    /*
     * serialized payload test는 app-server가 기대하는 `type/name/path` shape를 고정한다.
     * adapter가 protocol helper를 통하지 않거나 skill name/path 상수가 drift하면 이 테스트가 먼저 잡는다.
     */
    #[test]
    fn queue_mutation_skill_uses_repo_local_skill_path() {
        let adapter = PlanningWorkerSkillAdapter::from_repository_root("/repo");
        let input = adapter.queue_mutation_skill_input();
        let serialized = serde_json::to_value(input).expect("skill input should serialize");

        assert_eq!(serialized["type"], "skill");
        assert_eq!(serialized["name"], PLANNING_QUEUE_MUTATION_SKILL_NAME);
        assert_eq!(
            serialized["path"],
            "/repo/docs/agent/skills/akra-planning-queue-mutation/SKILL.md"
        );
    }

    // path helper를 직접 고정해 repo-relative skill location 변경을 명시적 리뷰 대상으로 만든다.
    #[test]
    fn skill_path_is_stable() {
        assert_eq!(
            skill_path(std::path::Path::new("/repo")),
            "/repo/docs/agent/skills/akra-planning-queue-mutation/SKILL.md"
        );
    }

    /*
     * worker prompt가 이 skill에 의존하므로 문서가 단순 TODO extractor나 completion authority로
     * 변질되면 안 된다. 파일 존재뿐 아니라 evaluator 역할과 mutation service contract의 핵심 문구를 확인한다.
     */
    #[test]
    fn queue_mutation_skill_documents_evaluator_contract() {
        let skill =
            std::fs::read_to_string(skill_path(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))))
                .expect("repo-local planning queue mutation skill should be readable");

        assert!(skill.contains("post-turn planning evaluator"));
        assert!(skill.contains("not as a TODO extractor"));
        assert!(skill.contains("not completion authority"));
        assert!(skill.contains("PlanningTaskMutationService"));
        assert!(skill.contains("planning_task_commands"));
    }
}
