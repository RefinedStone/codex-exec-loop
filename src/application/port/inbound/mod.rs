/*
 * Inbound ports are the application entry contracts used by driving adapters.
 * Implementations live under application/service; CLI, HTTP, Telegram, and TUI
 * adapters depend on these traits and request/response values instead.
 */
pub mod planning_control_port;
pub mod planning_task_tool_port;
pub mod planning_workspace_maintenance_port;
