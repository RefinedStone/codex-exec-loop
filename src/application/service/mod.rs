/*
 * service 모듈은 화면이나 저장소 세부사항을 모르는 유스케이스 묶음이다.
 * 각 service는 domain 모델과 outbound port를 조합하고, adapter는 이 공개 진입점만 호출한다.
 */
pub mod conversation_runtime_event;
// conversation_service는 사용자 turn과 app-server 스트림을 대화 단위로 조율한다.
pub mod conversation_service;
// github_review_poller_service는 GitHub review thread 상태를 polling 유스케이스로 묶는다.
pub mod github_review_poller_service;
// manual_prompt_preparation은 TUI가 직접 bootstrap/intake를 조립하지 않게 하는 application preflight다.
pub mod manual_prompt_preparation;
// parallel_agent_persona는 병렬 agent prompt에 선택적으로 주입되는 작은 페르소나 설정을 관리한다.
pub mod parallel_agent_persona;
// parallel_agent_profile은 병렬 agent의 이름, 역할, avatar, persona prompt 설정을 관리한다.
pub mod parallel_agent_profile;
// parallel_mode는 여러 worktree/agent lane을 만들고 배분하는 application 흐름이다.
pub mod parallel_mode;
// post_turn_decision은 턴 완료 후 auto-follow, parallel continuation, operator alert를 분리해 결정한다.
pub(crate) mod post_turn_decision;
// post_turn_evaluation은 완료된 turn 뒤 planning/parallel 후속 평가를 UI 없는 application effect로 실행한다.
pub mod post_turn_evaluation;
// planning은 작업 방향, queue, worker 실행, control 명령을 한 도메인 흐름으로 묶는다.
pub mod planning;
// prompt_component는 사용자에게 보이지 않는 prompt 조각을 service 내부에서 재사용하게 한다.
pub(crate) mod prompt_component;
// session_service는 저장된 세션 목록과 상세 조회를 application 경계로 노출한다.
pub mod session_service;
// startup_service는 TUI 시작 전 환경 점검과 진단 정보를 모은다.
pub mod startup_service;
// turn_prompt_assembly_service는 현재 상태와 사용자 입력을 app-server turn prompt로 조립한다.
pub mod turn_prompt_assembly_service;
