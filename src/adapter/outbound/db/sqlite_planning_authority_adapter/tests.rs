/*
학습 주석: 이 테스트 모듈은 SQLite planning authority adapter가 application port의 snapshot 계약을 실제
DB 저장소에 맞게 지키는지 확인합니다. service 계층은 `PlanningTaskRepositoryPort`만 보므로, 여기서는
adapter concrete type을 포트 메서드로 호출해 task authority 문서와 queue projection이 함께 round-trip되는지를 고정합니다.
*/
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::domain::planning::{PriorityQueueProjection, TaskAuthorityDocument};

// 학습 주석: temp_workspace는 테스트마다 SQLite namespace를 분리하는 workspace directory를 만듭니다.
// adapter가 workspace path를 DB 파일/row scope의 기준으로 쓰기 때문에, 프로세스 id와 nanos를 섞어 병렬 테스트 충돌을 피합니다.
fn temp_workspace(prefix: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "codex-exec-loop-db-{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be valid")
            .as_nanos()
    ));
    // 학습 주석: SQLite adapter가 workspace 아래에 database 파일을 열 수 있도록 directory를 먼저 만들어 둡니다.
    // 실패하면 테스트 환경 자체가 깨진 것이므로 expect로 즉시 드러냅니다.
    std::fs::create_dir_all(&path).expect("workspace should create");
    path.display().to_string()
}

#[test]
fn task_authority_snapshot_is_committed_to_db_tables() {
    // 학습 주석: 빈 workspace로 시작해야 commit path가 schema bootstrap, initial revision 생성, table insert를
    // 모두 지나갑니다. 기존 DB를 재사용하면 load-only 또는 update-only 경로만 검증할 위험이 있습니다.
    let workspace_dir = temp_workspace("workspace");
    // 학습 주석: concrete adapter를 만들지만 아래 호출은 PlanningTaskRepositoryPort 메서드입니다. 이 테스트는
    // application boundary에서 기대하는 포트 계약이 실제 SQLite 구현에서도 유지되는지 확인합니다.
    let adapter = SqlitePlanningAuthorityAdapter::new();
    // 학습 주석: 최소 task authority 문서로 round-trip을 검증합니다. 내용이 비어 있어도 version과 tasks 배열이
    // DB 직렬화/역직렬화 후 같은 domain value로 돌아와야 합니다.
    let task_authority = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    // 학습 주석: queue_projection은 task_authority와 같은 revision으로 저장되어야 하는 실행 관점 투영입니다.
    // 빈 projection도 next/active/proposed/skipped 필드가 누락 없이 DB snapshot에 남는지 확인합니다.
    let queue_projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: Vec::new(),
        proposed_tasks: Vec::new(),
        skipped_tasks: Vec::new(),
    };

    // 학습 주석: observed_planning_revision이 None인 첫 commit은 lost-update 검사를 건너뛰고 새 snapshot을 씁니다.
    // 이 호출이 성공하면 adapter는 schema 준비, transaction, JSON 저장, revision 발급까지 완료해야 합니다.
    adapter
        .commit_task_authority_snapshot(
            &workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &queue_projection,
            },
        )
        .expect("task authority should commit");

    // 학습 주석: 같은 adapter/workspace에서 다시 읽어야 persistence boundary를 통과합니다. 반환값이 None이면
    // commit이 table에 snapshot을 남기지 못한 것이고, Some이어도 아래 equality가 직렬화 손실을 잡습니다.
    let snapshot = adapter
        .load_task_authority_snapshot(&workspace_dir)
        .expect("task authority should load")
        .expect("snapshot should exist");

    // 학습 주석: task_authority와 queue_projection을 따로 비교해 "문서만 저장됨" 또는 "큐 투영만 저장됨" 같은
    // 반쪽 성공을 막습니다. 두 값이 같은 snapshot으로 돌아와야 planning runtime과 repair flow가 같은 authority를 봅니다.
    assert_eq!(snapshot.task_authority, task_authority);
    assert_eq!(snapshot.queue_projection, queue_projection);
}
