/*
 * domain 계층은 adapter나 I/O 없이도 설명 가능한 핵심 상태와 규칙을 둔다.
 * service는 이 모델들을 조합하고, adapter는 domain 타입을 화면이나 외부 API 형식으로 변환한다.
 */
pub(crate) mod app_server_prompt_log;
pub(crate) mod clipboard_image;
pub(crate) mod conversation;
// conversation_runtime_envelope preserves requested, applied, and later-observed app-server
// execution state without exposing raw provider configuration or credentials.
pub(crate) mod conversation_item_lifecycle;
pub(crate) mod conversation_progressive_activity;
pub(crate) mod conversation_runtime_envelope;
pub(crate) mod conversation_stream;
pub(crate) mod delegation;
pub(crate) mod runtime_capabilities;
// kernel_event 는 커널의 판단 사건이다. 추가 전용 기록에 쓰고 재시작할 때 판단 상태를 되돌린다 (plan 19 Q3).
pub(crate) mod kernel_event;
// kernel_spec 은 커널 스펙의 domain 쪽 표이다: 명령의 종류와 성질, 기록의 종류 (plan 18 K4-1).
pub(crate) mod kernel_spec;
// git_graph 는 커밋 그래프의 값 타입과 레인 배치 규칙이다 (plan 27 G1).
pub(crate) mod git_graph;
// git_operation 은 git 작업 엔진의 계획, 일지 기록, 되돌리기 규칙이다 (plan 27 G3).
pub(crate) mod git_operation;
// issue_work 는 외부 이슈의 참조와 공유본이다. 복원 평문은 가지지 않는다 (#2415).
pub(crate) mod issue_work;
// 이슈 자동화의 추적 장부 값과 상태 전이이다 (plan 28 IA1-1).
pub(crate) mod issue_automation;
// issue_publish_intent 는 이슈 게시 의도의 기록과 상태 규칙이다 (#2424).
pub(crate) mod issue_publish_intent;
// issue_policy 는 프로젝트의 이슈 규약과 대상별 공개 정책이다 (#2416).
pub(crate) mod issue_policy;
// 공유본과 복원 위치를 계산하는 순수한 가림 규칙이다 (#2537).
pub(crate) mod issue_masking;
// 원문을 보관하지 않고 알려진 비밀값의 위치만 찾는다 (#2537).
pub(crate) mod issue_secret_spans;
// issue_validation 은 이슈 필수 규약의 순수한 검증 규칙과 통과 결과이다 (#2418).
pub(crate) mod issue_validation;
// issue_review 는 LLM 품질 검토의 권고와 그 적용이다. 검증 통과나 공유본을 만들지 않는다 (#2430).
pub(crate) mod issue_review;
// issue_template 은 대상 저장소의 이슈 템플릿에서 읽은 필수 입력과 판이다 (#2417).
pub(crate) mod issue_template;
// github_review는 review thread와 polling 결과를 서비스가 다루기 쉬운 값으로 표현한다.
pub(crate) mod github_review;
pub(crate) mod pr_review;
// MCP hub 는 사용자당 하나인 MCP 서버 등록부와 도구 검색 규칙을 가진다.
pub(crate) mod mcp_hub;
pub(crate) mod mcp_write_policy;
// operator_alert는 TUI/Telegram 같은 delivery adapter가 공유할 수 있는 사용자 알림 이벤트다.
pub(crate) mod operator_alert;
// parallel_mode는 lane, branch, worktree처럼 병렬 작업 자체의 의미를 담는다.
pub(crate) mod parallel_agent_profile;
pub(crate) mod parallel_mode;
// planning은 direction, task, queue, validation 같은 실행 계획의 중심 모델이다.
pub(crate) mod planning;
// recent_sessions는 최근 세션 목록을 화면과 저장소 사이에서 안정적인 값으로 고정한다.
pub(crate) mod recent_sessions;
pub(crate) mod repository_extension;
// review_center는 review thread, pending inbox, history가 공유하는 저장·조회 projection을 둔다.
pub(crate) mod review_center;
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
// turn_terminal은 app-server terminal truth와 application delivery 결과를 분리해 보존한다.
pub(crate) mod turn_terminal;
// 작업 컨텍스트는 스레드에 나온 외부 문서를 코드로 인식하는 규칙을 가진다.
pub(crate) mod work_context;

pub(crate) mod issue_markdown;

pub mod issue_creation;
pub(crate) mod kernel_extension;
pub(crate) mod storage_analysis;

pub(crate) mod provider_credentials;

pub(crate) mod provider_work;

pub mod conversation_workspace;

pub(crate) mod task_usage;
