use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use anyhow::Result;

use crate::domain::planning::{
    DirectionCatalogDocument, PriorityQueueProjection, TaskAuthorityDocument,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: task authority snapshot은 planning task 저장소의 읽기 모델입니다.
 * `TaskAuthorityDocument`는 원본 authority 문서이고 `PriorityQueueProjection`은 실행/선택 흐름에서
 * 바로 쓰기 쉬운 큐 관점 투영입니다. 둘을 같은 revision으로 묶어야 문서와 큐가 서로 어긋나지 않습니다.
 */
pub struct PlanningTaskAuthoritySnapshot {
    // 학습 주석: optimistic commit에서 기준점으로 삼는 단조 증가 revision입니다.
    pub planning_revision: i64,
    // 학습 주석: task authority의 canonical 문서입니다. task 상태, 메타데이터, 정책 판단의 원천입니다.
    pub task_authority: TaskAuthorityDocument,
    // 학습 주석: scheduler/runtime이 우선순위를 계산할 때 다시 파싱하지 않도록 함께 저장한 투영입니다.
    pub queue_projection: PriorityQueueProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: direction authority snapshot은 작업 지시 카탈로그의 저장소 읽기 모델입니다.
 * task authority와 같은 revision 체계를 쓰기 때문에 bootstrap, repair, authoring 서비스가
 * "내가 읽은 지시 문서를 기준으로 아직 최신인가"를 같은 방식으로 확인할 수 있습니다.
 */
pub struct PlanningDirectionAuthoritySnapshot {
    // 학습 주석: direction catalog 저장 흐름의 optimistic 기준 revision입니다.
    pub planning_revision: i64,
    // 학습 주석: 실제 지시 카탈로그 문서입니다. authoring/doctor 흐름이 이 내용을 기준으로 draft를 만듭니다.
    pub directions: DirectionCatalogDocument,
}

#[derive(Debug, Clone, Copy)]
/*
 * 학습 주석: task authority commit 요청은 소유권을 가져오지 않고 문서 참조만 빌립니다.
 * application service가 이미 만든 문서/큐 투영을 저장소 경계에 넘기는 얇은 명령 객체이며,
 * `observed_planning_revision`으로 lost update를 막는 비교 후 저장 계약을 표현합니다.
 */
pub struct PlanningTaskAuthorityCommit<'a> {
    // 학습 주석: 호출자가 마지막으로 본 revision입니다. `None`이면 현재값과 무관하게 새 snapshot을 씁니다.
    pub observed_planning_revision: Option<i64>,
    // 학습 주석: 저장할 task authority 문서 참조입니다. 포트 구현체는 필요할 때 복제하거나 직렬화합니다.
    pub task_authority: &'a TaskAuthorityDocument,
    // 학습 주석: 같은 revision으로 저장할 우선순위 큐 투영입니다.
    pub queue_projection: &'a PriorityQueueProjection,
}

#[derive(Debug, Clone, Copy)]
/*
 * 학습 주석: direction authority commit은 task commit과 같은 revision 충돌 모델을 공유합니다.
 * 별도 타입으로 분리해 task 문서와 direction 카탈로그가 섞여 저장되는 실수를 타입 수준에서 막습니다.
 */
pub struct PlanningDirectionAuthorityCommit<'a> {
    // 학습 주석: 호출자가 읽었던 direction authority revision입니다.
    pub observed_planning_revision: Option<i64>,
    // 학습 주석: 저장할 direction catalog 문서 참조입니다.
    pub directions: &'a DirectionCatalogDocument,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * 학습 주석: commit 결과는 단순 성공/실패가 아니라 충돌 정보를 값으로 돌려줍니다.
 * 이 덕분에 service는 저장소 오류와 동시 수정 충돌을 구분하고, 필요하면 최신 snapshot을 다시 읽어
 * merge, retry, 사용자 안내 같은 후속 정책을 선택할 수 있습니다.
 */
pub enum PlanningTaskAuthorityCommitResult {
    Committed {
        // 학습 주석: 성공적으로 저장된 새 revision입니다. 호출자는 이후 commit의 observed 값으로 사용할 수 있습니다.
        planning_revision: i64,
    },
    Conflict {
        // 학습 주석: 호출자가 기준으로 삼았던 오래된 revision입니다.
        observed_planning_revision: i64,
        // 학습 주석: 저장소에 이미 존재하는 최신 revision입니다.
        current_planning_revision: i64,
    },
}

/*
 * 학습 주석: `PlanningTaskRepositoryPort`는 planning authority 문서를 저장하는 outbound 경계입니다.
 * application 계층은 파일, SQLite, 테스트 double 중 무엇이 뒤에 있는지 모르고 snapshot 단위로 읽고 씁니다.
 * 메서드는 direction authority와 task authority를 같은 패턴으로 나눠, 서로 다른 문서의 생명주기를
 * 독립적으로 관리하면서도 revision 충돌 처리 방식은 동일하게 유지합니다.
 */
pub trait PlanningTaskRepositoryPort: Send + Sync {
    // 학습 주석: workspace에 저장된 direction authority snapshot을 읽습니다. 아직 없으면 `Ok(None)`입니다.
    fn load_direction_authority_snapshot(
        &self,
        // 학습 주석: snapshot을 namespace로 분리하는 workspace 기준 경로/식별자입니다.
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>>;

    // 학습 주석: direction authority snapshot을 revision 조건부로 저장합니다.
    fn commit_direction_authority_snapshot(
        &self,
        // 학습 주석: 저장 대상 workspace namespace입니다.
        workspace_dir: &str,
        // 학습 주석: 저장할 direction catalog와 관측 revision을 담은 명령입니다.
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult>;

    // 학습 주석: workspace의 direction authority snapshot을 제거해 bootstrap/seed가 다시 만들 수 있게 합니다.
    fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> Result<()>;

    // 학습 주석: workspace에 저장된 task authority와 큐 투영을 같은 revision으로 읽습니다.
    fn load_task_authority_snapshot(
        &self,
        // 학습 주석: snapshot을 찾을 workspace namespace입니다.
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>>;

    // 학습 주석: task authority와 queue projection을 한 revision으로 조건부 저장합니다.
    fn commit_task_authority_snapshot(
        &self,
        // 학습 주석: 저장 대상 workspace namespace입니다.
        workspace_dir: &str,
        // 학습 주석: 저장할 task authority, queue projection, observed revision을 담은 명령입니다.
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult>;

    // 학습 주석: workspace의 task authority snapshot을 제거합니다. direction snapshot과 별도로 초기화할 수 있습니다.
    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()>;
}

#[derive(Debug, Default)]
/*
 * 학습 주석: `NoopPlanningTaskRepositoryPort`는 실제 DB adapter가 없을 때도 planning service를 조립하기 위한
 * in-memory fallback입니다. 이름은 noop이지만 commit/load/clear 동작을 전역 map에 보존하므로,
 * 테스트나 경량 실행에서 authority 흐름의 revision semantics를 그대로 연습할 수 있습니다.
 */
pub struct NoopPlanningTaskRepositoryPort;

impl PlanningTaskRepositoryPort for NoopPlanningTaskRepositoryPort {
    // 학습 주석: direction snapshot 전역 map에서 workspace별 값을 복제해 반환합니다.
    fn load_direction_authority_snapshot(
        &self,
        // 학습 주석: map key로 쓰이는 workspace namespace입니다.
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        // 학습 주석: Mutex를 잠근 동안만 map을 보고, 밖으로는 owned snapshot을 돌려 잠금 수명을 짧게 유지합니다.
        Ok(noop_direction_authority_store()
            .lock()
            .expect("noop direction authority store should not be poisoned")
            .get(workspace_dir)
            .cloned())
    }

    // 학습 주석: direction snapshot을 optimistic revision 규칙으로 갱신합니다.
    fn commit_direction_authority_snapshot(
        &self,
        // 학습 주석: 저장할 workspace namespace입니다.
        workspace_dir: &str,
        // 학습 주석: 저장할 문서 참조와 호출자가 관측한 revision입니다.
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        // 학습 주석: 현재 revision 확인과 새 snapshot 삽입을 같은 lock 범위에서 수행해 테스트 환경의 lost update를 막습니다.
        let mut store = noop_direction_authority_store()
            .lock()
            .expect("noop direction authority store should not be poisoned");
        // 학습 주석: 저장된 snapshot이 없으면 revision 0으로 간주해 첫 commit이 revision 1이 되게 합니다.
        let current_revision = store
            .get(workspace_dir)
            .map(|snapshot| snapshot.planning_revision)
            .unwrap_or(0);
        // 학습 주석: 호출자가 읽은 revision과 현재 revision이 다르면 저장하지 않고 충돌 정보를 돌려줍니다.
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        // 학습 주석: 성공 commit은 항상 revision을 하나 올려 후속 읽기/쓰기의 기준점을 바꿉니다.
        let planning_revision = current_revision + 1;
        store.insert(
            workspace_dir.to_string(),
            PlanningDirectionAuthoritySnapshot {
                planning_revision,
                // 학습 주석: commit 객체는 참조만 들고 있으므로 in-memory store에는 owned document로 복제합니다.
                directions: commit.directions.clone(),
            },
        );
        Ok(PlanningTaskAuthorityCommitResult::Committed { planning_revision })
    }

    // 학습 주석: direction snapshot을 workspace 단위로 제거합니다.
    fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        noop_direction_authority_store()
            .lock()
            .expect("noop direction authority store should not be poisoned")
            .remove(workspace_dir);
        Ok(())
    }

    // 학습 주석: task authority snapshot 전역 map에서 workspace별 값을 복제해 반환합니다.
    fn load_task_authority_snapshot(
        &self,
        // 학습 주석: map key로 쓰이는 workspace namespace입니다.
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        // 학습 주석: snapshot은 clone되어 반환되므로 caller가 값을 수정해도 store 내부 상태는 바뀌지 않습니다.
        Ok(noop_task_authority_store()
            .lock()
            .expect("noop task authority store should not be poisoned")
            .get(workspace_dir)
            .cloned())
    }

    // 학습 주석: task authority 문서와 큐 투영을 같은 revision으로 저장합니다.
    fn commit_task_authority_snapshot(
        &self,
        // 학습 주석: 저장할 workspace namespace입니다.
        workspace_dir: &str,
        // 학습 주석: 저장할 task authority, queue projection, observed revision입니다.
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        // 학습 주석: 현재 revision 검사와 새 snapshot 삽입을 같은 Mutex guard 안에서 처리합니다.
        let mut store = noop_task_authority_store()
            .lock()
            .expect("noop task authority store should not be poisoned");
        // 학습 주석: direction store와 같은 규칙으로 빈 저장소의 현재 revision을 0으로 둡니다.
        let current_revision = store
            .get(workspace_dir)
            .map(|snapshot| snapshot.planning_revision)
            .unwrap_or(0);
        // 학습 주석: observed revision이 현재와 다르면 task 문서와 queue projection 모두 저장하지 않습니다.
        if let Some(observed_revision) = commit.observed_planning_revision
            && observed_revision != current_revision
        {
            return Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: observed_revision,
                current_planning_revision: current_revision,
            });
        }
        // 학습 주석: 성공하면 두 문서가 같은 새 revision을 공유합니다.
        let planning_revision = current_revision + 1;
        store.insert(
            workspace_dir.to_string(),
            PlanningTaskAuthoritySnapshot {
                planning_revision,
                // 학습 주석: 참조로 받은 task authority를 store가 소유하도록 복제합니다.
                task_authority: commit.task_authority.clone(),
                // 학습 주석: task authority와 동시에 산출된 큐 투영도 같은 snapshot에 넣습니다.
                queue_projection: commit.queue_projection.clone(),
            },
        );
        Ok(PlanningTaskAuthorityCommitResult::Committed { planning_revision })
    }

    // 학습 주석: task authority snapshot을 workspace 단위로 제거합니다.
    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        noop_task_authority_store()
            .lock()
            .expect("noop task authority store should not be poisoned")
            .remove(workspace_dir);
        Ok(())
    }
}

/*
 * 학습 주석: task authority용 전역 in-memory store입니다.
 * `OnceLock`은 첫 호출 때만 `Mutex<BTreeMap<...>>`을 만들고 이후에는 같은 인스턴스를 공유합니다.
 * 실제 adapter가 없는 경로에서도 여러 service 인스턴스가 같은 process 안에서 snapshot을 다시 읽을 수 있습니다.
 */
fn noop_task_authority_store() -> &'static Mutex<BTreeMap<String, PlanningTaskAuthoritySnapshot>> {
    static STORE: OnceLock<Mutex<BTreeMap<String, PlanningTaskAuthoritySnapshot>>> =
        OnceLock::new();
    STORE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/*
 * 학습 주석: direction authority용 전역 in-memory store입니다.
 * task authority와 저장소를 분리해 clear/commit이 서로 간섭하지 않도록 만들고,
 * 두 문서가 독립적인 revision 흐름을 갖는다는 포트 계약도 테스트 double에서 그대로 보존합니다.
 */
fn noop_direction_authority_store()
-> &'static Mutex<BTreeMap<String, PlanningDirectionAuthoritySnapshot>> {
    static STORE: OnceLock<Mutex<BTreeMap<String, PlanningDirectionAuthoritySnapshot>>> =
        OnceLock::new();
    STORE.get_or_init(|| Mutex::new(BTreeMap::new()))
}
