// 학습 주석: skill path는 repository root와 repo-relative SKILL.md 위치를 합쳐 만들어집니다.
// Path/PathBuf를 쓰면 운영 환경과 테스트 fixture 모두에서 플랫폼별 경로 표현을 표준 library에 맡길 수 있습니다.
use std::path::{Path, PathBuf};

// 학습 주석: TurnInputItem은 app-server turn payload에 들어가는 입력 조각입니다.
// 이 adapter는 plain text prompt 앞에 `skill` item을 추가해 worker가 repo-local 지침을 먼저 받게 합니다.
use super::protocol::TurnInputItem;

// 학습 주석: skill name은 app-server protocol payload에서 이 skill을 식별하는 안정적인 이름입니다.
// 문서 path와 분리해 두면 경로가 바뀌더라도 worker-facing skill identity를 별도로 관리할 수 있습니다.
const PLANNING_QUEUE_MUTATION_SKILL_NAME: &str = "akra-planning-queue-mutation";
// 학습 주석: repo-relative path는 실제 skill 문서 위치입니다. planning worker turn마다 이 파일을 skill input으로
// 넘겨 post-turn evaluator가 task mutation JSON 계약을 따르도록 합니다.
const PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH: &str =
    "docs/agent/skills/akra-planning-queue-mutation/SKILL.md";

// 학습 주석: adapter는 repository root만 들고 있습니다. app-server connection이나 filesystem read를 직접 수행하지 않고,
// turn input에 넣을 skill path string을 만드는 작은 outbound boundary입니다.
#[derive(Debug, Clone)]
pub(super) struct PlanningWorkerSkillAdapter {
    // 학습 주석: repository_root는 CARGO_MANIFEST_DIR 또는 test fixture root입니다. skill path는 항상 이 root 아래에서 계산됩니다.
    repository_root: PathBuf,
}

// 학습 주석: PlanningWorkerSkillAdapter는 runtime worker orchestration에서 "skill input을 어떻게 만들지"만 책임집니다.
// 실제 turn 생성은 AppServerAdapter::planning_worker_turn_input이 이 adapter 결과와 prompt text를 조합합니다.
impl PlanningWorkerSkillAdapter {
    // 학습 주석: production constructor는 compile-time manifest dir을 repository root로 사용합니다.
    // binary가 어느 working directory에서 실행되어도 repo-local skill 문서를 찾게 하기 위한 선택입니다.
    pub(super) fn new() -> Self {
        Self {
            // 학습 주석: env!는 빌드 시점에 CARGO_MANIFEST_DIR 값을 박아 넣어 runtime env var 의존성을 없앱니다.
            repository_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        }
    }

    // 학습 주석: tests는 fake root를 넣어 serialized skill path가 안정적인지 확인합니다.
    #[cfg(test)]
    pub(super) fn from_repository_root(repository_root: impl Into<PathBuf>) -> Self {
        Self {
            // 학습 주석: Into<PathBuf>를 받아 test가 &str이나 PathBuf를 편하게 넘길 수 있습니다.
            repository_root: repository_root.into(),
        }
    }

    // 학습 주석: queue_mutation_skill_input은 app-server turn input에 들어갈 `type=skill` item을 만듭니다.
    // planning worker prompt 앞에 이 item이 들어가야 evaluator가 task queue mutation contract를 읽고 응답합니다.
    pub(super) fn queue_mutation_skill_input(&self) -> TurnInputItem {
        // 학습 주석: protocol helper를 써서 serde tag/name/path shape를 중앙 TurnInputItem 정의와 맞춥니다.
        TurnInputItem::skill(
            PLANNING_QUEUE_MUTATION_SKILL_NAME,
            skill_path(&self.repository_root),
        )
    }
}

// 학습 주석: skill_path는 repository root와 repo-relative skill location을 결합해 app-server가 읽을 path string을 만듭니다.
// 반환 타입이 String인 이유는 TurnInputItem::Skill payload가 JSON string field로 serialize되기 때문입니다.
fn skill_path(repository_root: &Path) -> String {
    repository_root
        // 학습 주석: join은 root가 `/repo`이면 `/repo/docs/.../SKILL.md` 형태의 절대/상대 path를 만듭니다.
        .join(PLANNING_QUEUE_MUTATION_SKILL_RELATIVE_PATH)
        // 학습 주석: to_string_lossy는 non-UTF8 path도 JSON payload로 보낼 수 있는 lossy string으로 변환합니다.
        .to_string_lossy()
        // 학습 주석: Cow<str>을 owned String으로 만들어 TurnInputItem이 repository_root borrow와 독립되게 합니다.
        .into_owned()
}

// 학습 주석: tests는 세 가지 계약을 고정합니다.
// 1. turn input JSON이 app-server protocol shape와 맞는지, 2. path가 안정적인지, 3. 실제 skill 문서가 evaluator contract를 담는지.
#[cfg(test)]
mod tests {
    // 학습 주석: tests는 private skill_path까지 직접 확인해 adapter output과 path helper가 같은 기준을 쓰는지 봅니다.
    use super::{PLANNING_QUEUE_MUTATION_SKILL_NAME, PlanningWorkerSkillAdapter, skill_path};

    // 학습 주석: serialized payload test는 TurnInputItem::Skill이 app-server가 기대하는 `type/name/path` JSON을 만드는지 보장합니다.
    #[test]
    fn queue_mutation_skill_uses_repo_local_skill_path() {
        // 학습 주석: fake root를 쓰면 test가 현재 checkout 위치에 묶이지 않고 expected path를 명확히 비교할 수 있습니다.
        let adapter = PlanningWorkerSkillAdapter::from_repository_root("/repo");
        // 학습 주석: adapter가 production과 같은 public method로 skill input을 만듭니다.
        let input = adapter.queue_mutation_skill_input();
        // 학습 주석: serde_json value로 내려 실제 wire payload field를 검사합니다.
        let serialized = serde_json::to_value(input).expect("skill input should serialize");

        assert_eq!(serialized["type"], "skill");
        assert_eq!(serialized["name"], PLANNING_QUEUE_MUTATION_SKILL_NAME);
        assert_eq!(
            serialized["path"],
            "/repo/docs/agent/skills/akra-planning-queue-mutation/SKILL.md"
        );
    }

    // 학습 주석: path helper test는 adapter와 별도로 repo-relative path 상수가 실수로 바뀌는지 감시합니다.
    #[test]
    fn skill_path_is_stable() {
        assert_eq!(
            skill_path(std::path::Path::new("/repo")),
            "/repo/docs/agent/skills/akra-planning-queue-mutation/SKILL.md"
        );
    }

    // 학습 주석: 이 test는 path가 존재할 뿐 아니라 skill 문서가 planning evaluator 역할을 제대로 설명하는지 확인합니다.
    // worker prompt가 이 skill에 의존하므로 문서가 TODO extractor나 completion authority로 변질되지 않는 것이 중요합니다.
    #[test]
    fn queue_mutation_skill_documents_evaluator_contract() {
        // 학습 주석: 실제 checkout의 SKILL.md를 읽어 repo-local file이 빌드/테스트 환경에서 접근 가능한지 검증합니다.
        let skill =
            std::fs::read_to_string(skill_path(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))))
                // 학습 주석: skill이 없어지면 planning worker turn input이 깨지므로 test가 즉시 실패해야 합니다.
                .expect("repo-local planning queue mutation skill should be readable");

        // 학습 주석: 아래 문자열들은 worker role과 output contract의 핵심 guardrail입니다.
        assert!(skill.contains("post-turn planning evaluator"));
        assert!(skill.contains("not as a TODO extractor"));
        assert!(skill.contains("not completion authority"));
        assert!(skill.contains("PlanningTaskMutationService"));
        assert!(skill.contains("planning_task_commands"));
    }
}
