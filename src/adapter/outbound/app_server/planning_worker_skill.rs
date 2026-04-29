use std::path::{Path, PathBuf};

use super::protocol::TurnInputItem;

const PLANNING_QUEUE_MUTATION_SKILL_NAME: &str = "akra-planning-queue-mutation";
const PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH: &str =
    "docs/agent/skills/akra-planning-queue-mutation/SKILL.md";

#[derive(Debug, Clone)]
pub(super) struct PlanningWorkerSkillAdapter {
    repository_root: PathBuf,
}

impl PlanningWorkerSkillAdapter {
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

    pub(super) fn queue_mutation_skill_input(&self) -> TurnInputItem {
        TurnInputItem::skill(
            PLANNING_QUEUE_MUTATION_SKILL_NAME,
            skill_path(&self.repository_root),
        )
    }
}

fn skill_path(repository_root: &Path) -> String {
    repository_root
        .join(PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::{PLANNING_QUEUE_MUTATION_SKILL_NAME, PlanningWorkerSkillAdapter, skill_path};

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

    #[test]
    fn skill_path_is_stable() {
        assert_eq!(
            skill_path(std::path::Path::new("/repo")),
            "/repo/docs/agent/skills/akra-planning-queue-mutation/SKILL.md"
        );
    }

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
