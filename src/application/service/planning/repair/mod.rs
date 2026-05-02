// 학습 주석: doctor는 planning workspace의 현재 파일/authority 상태를 진단해 admin overview와 init 화면에 설명 가능한 상태를 제공합니다.
pub(crate) mod doctor;
// 학습 주석: ledger_recovery는 queue projection 복구가 어떤 경로로 이뤄졌는지 표시하는 작은 감사 타입을 담습니다.
pub(crate) mod ledger_recovery;
// 학습 주석: prompt는 repair/reconciliation 흐름이 worker에게 넘길 복구 지시 문구를 조립하는 영역입니다.
pub(crate) mod prompt;
// 학습 주석: protected_restore는 turn 실행 중 보호해야 하는 planning 파일이 손상되었을 때 복원하는 파일 중심 repair helper입니다.
pub(crate) mod protected_restore;
// 학습 주석: reconciliation은 turn 이후 workspace, task authority, queue projection을 다시 맞추는 repair의 핵심 service입니다.
pub(crate) mod reconciliation;
// 학습 주석: reset은 admin/TUI 요청에 따라 planning workspace의 일부 또는 전체를 초기 상태로 되돌리는 명시적 복구 경로입니다.
pub(crate) mod reset;
