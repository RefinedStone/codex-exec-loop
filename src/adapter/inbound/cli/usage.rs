pub(super) use crate::adapter::inbound::telegram_bot::TELEGRAM_BOT_ALIAS_USAGE;
use crate::adapter::inbound::telegram_bot::TELEGRAM_BOT_COMMAND_USAGE;

// CLI usage strings live beside command dispatch because they are edge copy and arity errors.
pub(super) const ADMIN_SERVER_USAGE: &str = "Usage: akra admin [--port <port>]";
pub(super) const ADMIN_SERVER_ALIAS_USAGE: &str = "Alias: akra admin-server [--port <port>]";
pub(super) const DOCTOR_USAGE: &str = "Usage: akra doctor [workspace_dir]";
pub(super) const STATUS_USAGE: &str =
    "Usage: akra status [workspace_dir] | akra status --pr <number> [workspace_dir]";
pub(super) const QUEUE_USAGE: &str = "Usage: akra queue [workspace_dir]";
pub(super) const RESET_USAGE: &str =
    "Usage: akra reset <queue|directions|all> [workspace_dir] [--confirm]";
pub(super) const PLANNING_TOOL_USAGE: &str =
    "Usage: akra planning-tool <contract|run> [workspace_dir]";
pub(super) const MCP_USAGE: &str =
    "Usage: akra mcp <servers|add|remove|login|search|describe|call|skill>
  akra mcp servers
  akra mcp add <name> --url <https-url>
  akra mcp add <name> -- <command> [args...]
  akra mcp remove <name>
  akra mcp login <name>
  akra mcp search [words...]
  akra mcp describe <server> <tool>
  akra mcp call <server> <tool> [<json-object>|-]
  akra mcp skill <status|install|remove>";
pub(super) const PARALLEL_TICK_USAGE: &str = "Usage: akra parallel-tick [workspace_dir]";
pub(super) const CONFIG_USAGE: &str =
    "Usage: akra config <list|get|set|unset|path|doctor> [--global|--project] [key] [value]";
pub(super) const TELEGRAM_BOT_USAGE: &str = TELEGRAM_BOT_COMMAND_USAGE;
