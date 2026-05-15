/*
 * domain 계층은 adapter나 I/O 없이도 설명 가능한 핵심 상태와 규칙을 둔다.
 * service는 이 모델들을 조합하고, adapter는 domain 타입을 화면이나 외부 API 형식으로 변환한다.
 */
pub(crate) mod conversation;
pub(crate) mod conversation_stream;
// github_review는 review thread와 polling 결과를 서비스가 다루기 쉬운 값으로 표현한다.
pub(crate) mod github_review;
// operator_alert는 TUI/Telegram 같은 delivery adapter가 공유할 수 있는 사용자 알림 이벤트다.
pub(crate) mod operator_alert;
// parallel_mode는 lane, branch, worktree처럼 병렬 작업 자체의 의미를 담는다.
pub(crate) mod parallel_mode;
// planning은 direction, task, queue, validation 같은 실행 계획의 중심 모델이다.
pub(crate) mod planning;
// recent_sessions는 최근 세션 목록을 화면과 저장소 사이에서 안정적인 값으로 고정한다.
pub(crate) mod recent_sessions;
// session_browser는 세션 탐색 UI가 필요한 필터링/선택 상태를 domain 값으로 둔다.
pub(crate) mod session_browser;
// session_summary는 app-server 세션을 목록과 상세 화면에 맞는 요약으로 표현한다.
pub(crate) mod session_summary;
// startup_diagnostics는 실행 전 점검 결과를 severity와 message가 있는 진단값으로 담는다.
pub(crate) mod startup_diagnostics;
// terminal_bridge_attachment는 shell/terminal 연결 상태를 UI와 runtime이 공유하는 모델이다.
pub(crate) mod terminal_bridge_attachment;
// text는 여러 계층에서 재사용하는 문자열 정규화와 표시 규칙을 담는다.
pub(crate) mod text;
