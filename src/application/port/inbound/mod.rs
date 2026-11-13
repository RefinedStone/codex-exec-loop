/*
 * Inbound ports are the application entry contracts used by driving adapters.
 * Implementations live under application/service; CLI, HTTP, Telegram, and TUI
 * adapters depend on these traits and request/response values instead.
 */
pub mod admin_debug_port;
pub mod app_server_prompt_log_query_port;
pub mod kernel_command_port;
// stored command port 는 저장된 planning 데이터만 바꾸는 커널 명령을 실행한다 (plan 18 K4-9d).
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
pub mod review_center_query_port;
pub mod stored_command_port;
pub mod work_context_port;
