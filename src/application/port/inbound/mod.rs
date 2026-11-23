/*
 * Inbound ports are the application entry contracts used by driving adapters.
 * Implementations live under application/service; CLI, HTTP, Telegram, and TUI
 * adapters depend on these traits and request/response values instead.
 */
pub mod admin_debug_port;
pub mod app_server_prompt_log_query_port;
pub(crate) mod extension_host_port;
// issue handoff port 는 이슈 인계에 쓸 공유본을 준다 (#2428).
pub mod issue_automation_policy_query_port;
pub mod issue_handoff_port;
pub mod kernel_command_port;
// stored command port 는 저장된 planning 데이터만 바꾸는 커널 명령을 실행한다 (plan 18 K4-9d).
pub mod git_graph_port;
// git operation port 는 git 작업의 계획, 실행, 일지, 되돌리기이다 (plan 27 G3).
pub mod git_operation_port;
// issue draft port 는 Desktop 의 이슈 초안, 미리보기, 게시 명령이다 (#2426).
pub mod issue_draft_port;
pub mod mcp_hub_port;
pub mod parallel_agent_profile_port;
pub mod parallel_mode_admin_port;
pub mod parallel_mode_control_port;
pub mod planning_admin_port;
pub mod planning_control_port;
pub mod planning_projection_port;
pub mod planning_task_tool_port;
pub mod planning_workspace_maintenance_port;
pub mod pr_validation_command_port;
pub mod pr_validation_query_port;
pub mod pr_validation_rollout_evidence_query_port;
pub mod resource_version_provider_port;
pub mod review_center_query_port;
pub mod stored_command_port;
pub mod task_evidence_query_port;
pub mod work_context_port;
// 명시적인 이슈 폼 설치 요청은 서비스 구현이 아닌 이 경계로 들어간다.
pub mod issue_form_install_port;

pub mod issue_intervention_port;
pub(crate) mod vault_restore_session_port;
