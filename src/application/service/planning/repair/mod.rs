// doctor는 planning workspace의 현재 파일/authority 상태를 진단해 admin overview와 init 화면에 설명 가능한 상태를 제공한다.
// 복구를 실행하지 않고 관찰 결과만 만들기 때문에 inbound 화면들이 안전하게 호출할 수 있다.
pub(crate) mod doctor;
// ledger_recovery는 queue projection 복구가 어떤 경로로 이뤄졌는지 표시하는 작은 감사 타입을 담는다.
// reconciliation 결과가 단순 성공 여부를 넘어 어떤 source에서 회복됐는지 보고하게 하는 보조 계약이다.
pub(crate) mod ledger_recovery;
// prompt는 repair/reconciliation 흐름이 worker에게 넘길 복구 지시 문구를 조립하는 영역이다.
// 손상된 authority state를 직접 고치기보다 worker에게 필요한 context와 출력 계약을 제공하는 역할을 맡는다.
pub(crate) mod prompt;
// protected_restore는 turn 실행 중 보호해야 하는 planning 파일이 손상되었을 때 복원하는 파일 중심 repair helper다.
// runtime turn 경계에서 operator 파일과 generated authority 파일의 복구 책임을 좁게 유지한다.
pub(crate) mod protected_restore;
// reconciliation은 turn 이후 workspace, task authority, queue projection을 다시 맞추는 repair의 핵심 service다.
// worker output, DB authority, file workspace가 어긋난 뒤에도 다음 turn이 읽을 수 있는 일관된 snapshot을 만든다.
pub(crate) mod reconciliation;
// reset은 admin/TUI 요청에 따라 planning workspace의 일부 또는 전체를 초기 상태로 되돌리는 명시적 복구 경로다.
// 자동 reconciliation과 달리 operator 의도를 전제로 destructive rewrite를 수행하는 별도 use case다.
pub(crate) mod reset;
