// 학습 주석: queue projection 복구는 "accepted planning 문서에서 다시 만든 것"과 "turn 실행 snapshot에서 되살린 것"을 구분해야 합니다.
// 이 enum은 reconciliation 결과가 어떤 복구 경로를 탔는지 admin/runtime projection에 전달하는 작은 감사 신호입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueProjectionAction {
    // 학습 주석: accepted planning authority를 기준으로 queue projection을 재생성했다는 뜻입니다. 파일/DB authority가 원천이고,
    // 이전 실행 snapshot은 신뢰하지 않는 경로입니다.
    RebuiltFromAcceptedPlanning,
    // 학습 주석: 최근 execution snapshot에 남아 있던 queue projection을 복원했다는 뜻입니다. turn 실행 중 보호 파일이 흔들렸을 때
    // 사용자 작업 맥락을 잃지 않기 위한 복구 경로입니다.
    RestoredFromExecutionSnapshot,
}
