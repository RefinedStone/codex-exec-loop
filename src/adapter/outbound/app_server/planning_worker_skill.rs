use std::path::{Path, PathBuf};

use super::protocol::TurnInputItem;

/*
 * 이 adapter는 planning worker turn input에 bundled skill asset을 붙이는 작은 app-server
 * outbound boundary다. worker prompt가 plain text만 받으면 task queue mutation JSON 계약을 잊기 쉬우므로,
 * turn payload 앞에 `type=skill` item을 추가해 evaluator 전용 지침을 protocol 레벨에서 고정한다.
 */
const PLANNING_QUEUE_MUTATION_SKILL_NAME: &str = "akra-planning-queue-mutation";
const PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH: &str =
    "assets/app-server/skills/akra-planning-queue-mutation/SKILL.md";

#[derive(Debug, Clone)]
pub(super) struct PlanningWorkerSkillAdapter {
    // asset_root만 보관해 app-server connection과 filesystem read 책임을 이 adapter 밖에 둔다.
    asset_root: PathBuf,
}

impl PlanningWorkerSkillAdapter {
    /*
     * production constructor는 설치된 binary 옆 asset을 먼저 찾고, 개발 checkout에서는 manifest dir의
     * asset으로 fallback한다. npm/native bundle과 `cargo run` 양쪽에서 같은 relative contract를 유지하기 위해서다.
     */
    pub(super) fn new() -> Self {
        Self {
            asset_root: production_skill_asset_root(),
        }
    }

    #[cfg(test)]
    pub(super) fn from_asset_root(asset_root: impl Into<PathBuf>) -> Self {
        Self {
            asset_root: asset_root.into(),
        }
    }

    /*
     * planning worker prompt 앞에 이 item이 들어가야 worker가 post-turn evaluator 역할과
     * PlanningTaskMutationService JSON contract를 먼저 읽은 뒤 자연어 prompt를 해석한다.
     */
    pub(super) fn queue_mutation_skill_input(&self) -> TurnInputItem {
        TurnInputItem::skill(
            PLANNING_QUEUE_MUTATION_SKILL_NAME,
            skill_path(&self.asset_root),
        )
    }
}

/*
 * skill_path는 asset-root-relative skill location을 app-server가 읽을 path string으로 투영한다.
 * 반환 타입은 TurnInputItem::Skill payload가 JSON string field로 serialize되는 wire contract에 맞춘 것이다.
 */
fn skill_path(asset_root: &Path) -> String {
    asset_root
        .join(PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH)
        .to_string_lossy()
        .into_owned()
}

fn production_skill_asset_root() -> PathBuf {
    installed_skill_asset_root()
        .filter(|asset_root| {
            asset_root
                .join(PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH)
                .is_file()
        })
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn installed_skill_asset_root() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::{PLANNING_QUEUE_MUTATION_SKILL_NAME, PlanningWorkerSkillAdapter, skill_path};

    /*
     * serialized payload test는 app-server가 기대하는 `type/name/path` shape를 고정한다.
     * adapter가 protocol helper를 통하지 않거나 skill name/path 상수가 drift하면 이 테스트가 먼저 잡는다.
     */
    #[test]
    fn queue_mutation_skill_uses_asset_skill_path() {
        let adapter = PlanningWorkerSkillAdapter::from_asset_root("/repo");
        let input = adapter.queue_mutation_skill_input();
        let serialized = serde_json::to_value(input).expect("skill input should serialize");

        assert_eq!(serialized["type"], "skill");
        assert_eq!(serialized["name"], PLANNING_QUEUE_MUTATION_SKILL_NAME);
        assert_eq!(
            serialized["path"],
            "/repo/assets/app-server/skills/akra-planning-queue-mutation/SKILL.md"
        );
    }

    // path helper를 직접 고정해 asset-relative skill location 변경을 명시적 리뷰 대상으로 만든다.
    #[test]
    fn skill_path_is_stable() {
        assert_eq!(
            skill_path(std::path::Path::new("/repo")),
            "/repo/assets/app-server/skills/akra-planning-queue-mutation/SKILL.md"
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
                .expect("planning queue mutation skill asset should be readable");

        assert!(skill.contains("post-turn planning evaluator"));
        assert!(skill.contains("not as a TODO extractor"));
        assert!(skill.contains("not completion authority"));
        assert!(skill.contains("PlanningTaskMutationService"));
        assert!(skill.contains("planning_task_commands"));
    }
}
