/*
 * outbound port들은 application service가 필요로 하는 외부 능력의 목록이다.
 * 이 파일은 service 관점의 의존성 지도를 보여 주며, adapter/outbound가 각 port를 실제 I/O로 구현한다.
 */
pub mod codex_app_server_port;
// GitHub automation port는 PR 생성, merge, comment 같은 쓰기 작업을 추상화한다.
pub mod github_automation_port;
// GitHub review poller port는 review thread 조회와 상태 수집 계약을 정의한다.
pub mod github_review_poller_port;
// interactive turn runtime port는 app-server와 대화 turn을 실행하는 능력을 분리한다.
pub mod interactive_turn_runtime_port;
// parallel agent worker port는 병렬 lane에서 실제 agent 작업을 시작하는 경계다.
pub mod parallel_agent_worker_port;
// parallel mode runtime port는 worktree와 branch 준비 같은 로컬 런타임 조작을 추상화한다.
pub mod parallel_mode_runtime_port;
// planning authority port는 현재 planning 상태를 읽고 reset하는 권한 있는 저장소 경계다.
pub mod planning_authority_port;
// planning task repository port는 task queue의 영속 조회와 변경 계약을 제공한다.
pub mod planning_task_repository_port;
// planning worker port는 queued task를 실행자로 넘기는 호출 지점을 정의한다.
pub mod planning_worker_port;
// planning workspace port는 directions와 supporting files가 놓인 작업공간 접근을 추상화한다.
pub mod planning_workspace_port;
// session catalog port는 app-server 세션 목록과 상세 조회를 service에서 사용할 계약으로 둔다.
pub mod session_catalog_port;
// startup probe port는 실행 전 환경 점검을 외부 명령이나 파일 접근에서 분리한다.
pub mod startup_probe_port;
// telegram bot port는 Telegram HTTP 호출을 bot runner의 테스트 가능한 경계로 만든다.
pub mod telegram_bot_port;
