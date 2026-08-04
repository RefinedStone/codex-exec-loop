/*
 * Host context names are shared by the CLI ingress and app-server egress.
 * They belong to the application boundary contract rather than either adapter
 * or the planning task-tool service implementation.
 */
pub(crate) const PLANNING_TOOL_PARENT_THREAD_ID_ENV: &str = "AKRA_PLANNING_TOOL_PARENT_THREAD_ID";
pub(crate) const PLANNING_TOOL_PARENT_TURN_ID_ENV: &str = "AKRA_PLANNING_TOOL_PARENT_TURN_ID";
