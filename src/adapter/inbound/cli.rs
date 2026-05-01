use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::GithubAutomationAdapter;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::{
    PlanningResetTarget, PlanningServices, PlanningTaskToolRequest, PlanningTaskToolResponse,
};

mod reports;

use self::reports::{
    DoctorReport, InitReport, PlanningToolErrorReport, ResetReport, render_doctor_report,
    render_init_report, render_json_line, render_reset_report,
};

const ADMIN_SERVER_USAGE: &str = "Usage: akra admin [--port <port>]";
const ADMIN_SERVER_ALIAS_USAGE: &str = "Alias: akra admin-server [--port <port>]";
const DOCTOR_USAGE: &str = "Usage: akra doctor [workspace_dir]";
const INIT_USAGE: &str = "Usage: akra init [workspace_dir]";
const RESET_USAGE: &str = "Usage: akra reset <queue|directions|all> [workspace_dir]";
const PLANNING_TOOL_USAGE: &str = "Usage: akra planning-tool <contract|run> [workspace_dir]";
const PARALLEL_TICK_USAGE: &str = "Usage: akra parallel-tick [workspace_dir]";
const TELEGRAM_BOT_USAGE: &str = "Usage: akra telegram [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]";
const TELEGRAM_BOT_ALIAS_USAGE: &str = "Alias: akra telegram-bot [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]";

pub fn run_with_env_args(stdout: &mut impl Write) -> Result<Option<i32>> {
    run_with_args(std::env::args_os().skip(1), stdout)
}

pub(crate) fn run_with_args<I, T>(args: I, stdout: &mut impl Write) -> Result<Option<i32>>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(None),
        [flag] if is_help_flag(flag) => {
            writeln!(stdout, "{ADMIN_SERVER_USAGE}")?;
            writeln!(stdout, "{ADMIN_SERVER_ALIAS_USAGE}")?;
            writeln!(stdout, "{TELEGRAM_BOT_USAGE}")?;
            writeln!(stdout, "{TELEGRAM_BOT_ALIAS_USAGE}")?;
            writeln!(stdout, "{DOCTOR_USAGE}")?;
            writeln!(stdout, "{INIT_USAGE}")?;
            writeln!(stdout, "{RESET_USAGE}")?;
            writeln!(stdout, "{PLANNING_TOOL_USAGE}")?;
            writeln!(stdout, "{PARALLEL_TICK_USAGE}")?;
            Ok(Some(0))
        }
        [command] if is_admin_command(command) => Ok(Some(run_admin_server(&[])?)),
        [command, rest @ ..] if is_admin_command(command) => Ok(Some(run_admin_server(rest)?)),
        [command] if is_telegram_command(command) => Ok(Some(run_telegram_bot(&[])?)),
        [command, rest @ ..] if is_telegram_command(command) => Ok(Some(run_telegram_bot(rest)?)),
        [command] if command == OsStr::new("doctor") => Ok(Some(run_doctor(None, stdout)?)),
        [command, workspace] if command == OsStr::new("doctor") => {
            Ok(Some(run_doctor(Some(workspace.as_os_str()), stdout)?))
        }
        [command] if command == OsStr::new("init") => Ok(Some(run_init(None, stdout)?)),
        [command, workspace] if command == OsStr::new("init") => {
            Ok(Some(run_init(Some(workspace.as_os_str()), stdout)?))
        }
        [command, target] if command == OsStr::new("reset") => {
            Ok(Some(run_reset(target.as_os_str(), None, stdout)?))
        }
        [command, target, workspace] if command == OsStr::new("reset") => Ok(Some(run_reset(
            target.as_os_str(),
            Some(workspace.as_os_str()),
            stdout,
        )?)),
        [command, subcommand] if is_planning_tool_command(command) => Ok(Some(run_planning_tool(
            subcommand.as_os_str(),
            None,
            stdout,
        )?)),
        [command, subcommand, workspace] if is_planning_tool_command(command) => Ok(Some(
            run_planning_tool(subcommand.as_os_str(), Some(workspace.as_os_str()), stdout)?,
        )),
        [command] if command == OsStr::new("parallel-tick") => {
            Ok(Some(run_parallel_tick(None, stdout)?))
        }
        [command, workspace] if command == OsStr::new("parallel-tick") => Ok(Some(
            run_parallel_tick(Some(workspace.as_os_str()), stdout)?,
        )),
        [command, _, ..] if command == OsStr::new("doctor") => {
            bail!("{DOCTOR_USAGE}");
        }
        [command, _, ..] if command == OsStr::new("init") => {
            bail!("{INIT_USAGE}");
        }
        [command, _, _, ..] if command == OsStr::new("reset") => {
            bail!("{RESET_USAGE}");
        }
        [command, _, _, ..] if is_planning_tool_command(command) => {
            bail!("{PLANNING_TOOL_USAGE}");
        }
        [command, _, _, ..] if command == OsStr::new("parallel-tick") => {
            bail!("{PARALLEL_TICK_USAGE}");
        }
        [command, ..] => {
            bail!("unsupported command: {}", command.to_string_lossy());
        }
    }
}

fn is_help_flag(flag: &OsStr) -> bool {
    matches!(flag.to_str(), Some("-h" | "--help"))
}

fn is_admin_command(command: &OsStr) -> bool {
    matches!(command.to_str(), Some("admin" | "admin-server"))
}

fn is_telegram_command(command: &OsStr) -> bool {
    matches!(command.to_str(), Some("telegram" | "telegram-bot"))
}

fn is_planning_tool_command(command: &OsStr) -> bool {
    matches!(
        command.to_str(),
        Some("planning-tool" | "planning-task-tool")
    )
}

fn run_admin_server(args: &[OsString]) -> Result<i32> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start tokio runtime for admin server")?;
    runtime.block_on(crate::adapter::inbound::admin_api::run_with_args(
        args.iter().map(|arg| arg.to_string_lossy().to_string()),
    ))?;
    Ok(0)
}

fn run_telegram_bot(args: &[OsString]) -> Result<i32> {
    crate::adapter::inbound::telegram_bot::run_with_args(
        args.iter().map(|arg| arg.to_string_lossy().to_string()),
    )?;
    Ok(0)
}

fn run_doctor(workspace_arg: Option<&OsStr>, stdout: &mut impl Write) -> Result<i32> {
    let workspace_path = resolve_workspace_path(workspace_arg)?;
    let report = inspect_workspace(&workspace_path);
    render_doctor_report(stdout, &report)?;
    Ok(report.exit_code())
}

fn run_init(workspace_arg: Option<&OsStr>, stdout: &mut impl Write) -> Result<i32> {
    let workspace_path = resolve_workspace_path(workspace_arg)?;
    let report = initialize_workspace(&workspace_path);
    render_init_report(stdout, &report)?;
    Ok(report.exit_code())
}

fn run_reset(
    target_arg: &OsStr,
    workspace_arg: Option<&OsStr>,
    stdout: &mut impl Write,
) -> Result<i32> {
    let target = parse_reset_target(target_arg)?;
    let workspace_path = resolve_workspace_path(workspace_arg)?;
    let report = reset_workspace(&workspace_path, target);
    render_reset_report(stdout, &report)?;
    Ok(report.exit_code())
}

fn run_planning_tool(
    subcommand: &OsStr,
    workspace_arg: Option<&OsStr>,
    stdout: &mut impl Write,
) -> Result<i32> {
    let planning = build_production_planning_services();
    match subcommand.to_str() {
        Some("contract") => {
            writeln!(stdout, "{}", planning.task_tool.contract_json())?;
            Ok(0)
        }
        Some("run") => {
            let workspace_path = resolve_workspace_path(workspace_arg)?;
            let workspace_label = workspace_path.display().to_string();
            let result = run_planning_tool_request(&planning, &workspace_path);
            match result {
                Ok(response) => {
                    render_json_line(stdout, &response)?;
                    Ok(0)
                }
                Err(error) => {
                    render_json_line(
                        stdout,
                        &PlanningToolErrorReport {
                            ok: false,
                            operation: "planning-tool".to_string(),
                            error: error.to_string(),
                            guidance: vec![
                                format!("usage: {PLANNING_TOOL_USAGE}"),
                                format!("workspace: {workspace_label}"),
                                "Run `akra planning-tool contract` for the compact JSON contract."
                                    .to_string(),
                            ],
                        },
                    )?;
                    Ok(1)
                }
            }
        }
        _ => bail!("{PLANNING_TOOL_USAGE}"),
    }
}

fn run_parallel_tick(workspace_arg: Option<&OsStr>, stdout: &mut impl Write) -> Result<i32> {
    let workspace_path = resolve_workspace_path(workspace_arg)?;
    validate_workspace_path(&workspace_path).map_err(anyhow::Error::msg)?;
    let service = build_production_parallel_mode_service();
    let workspace_label = workspace_path.display().to_string();

    writeln!(stdout, "workspace: {workspace_label}")?;
    match service.process_distributor_queue(&workspace_label) {
        Ok(notices) if notices.is_empty() => {
            writeln!(stdout, "parallel distributor queue idle")?;
            Ok(0)
        }
        Ok(notices) => {
            for notice in notices {
                writeln!(stdout, "{notice}")?;
            }
            Ok(0)
        }
        Err(error) => {
            writeln!(stdout, "parallel distributor tick failed: {error}")?;
            Ok(1)
        }
    }
}

fn run_planning_tool_request(
    planning: &PlanningServices,
    workspace_path: &Path,
) -> Result<PlanningTaskToolResponse> {
    validate_workspace_path(workspace_path).map_err(anyhow::Error::msg)?;
    let mut request_json = String::new();
    std::io::stdin()
        .read_to_string(&mut request_json)
        .context("failed to read planning-tool JSON request from stdin")?;
    let request = serde_json::from_str::<PlanningTaskToolRequest>(&request_json)
        .context("failed to parse planning-tool JSON request")?;
    planning
        .task_tool
        .run(workspace_path.to_string_lossy().as_ref(), request)
}

fn resolve_workspace_path(workspace_arg: Option<&OsStr>) -> Result<PathBuf> {
    let current_dir =
        std::env::current_dir().context("failed to resolve the current working directory")?;
    let requested = workspace_arg
        .map(PathBuf::from)
        .unwrap_or(current_dir.clone());
    let absolute = if requested.is_absolute() {
        requested
    } else {
        current_dir.join(requested)
    };

    if absolute.exists() {
        absolute
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", absolute.display()))
    } else {
        Ok(absolute)
    }
}

fn validate_workspace_path(workspace_path: &Path) -> Result<(), String> {
    if !workspace_path.exists() {
        return Err(format!(
            "workspace path does not exist: {}",
            workspace_path.display()
        ));
    }
    if !workspace_path.is_dir() {
        return Err(format!(
            "workspace path is not a directory: {}",
            workspace_path.display()
        ));
    }
    Ok(())
}

fn build_production_planning_services() -> PlanningServices {
    let app_server_adapter = Arc::new(CodexAppServerAdapter::new(
        "codex-exec-loop-native",
        env!("CARGO_PKG_VERSION"),
    ));
    let planning_authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
    PlanningServices::from_ports(
        Arc::new(FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(
            planning_authority.clone(),
        )),
        planning_authority.clone(),
        planning_authority,
        Arc::new(AppServerPlanningWorkerAdapter::new(app_server_adapter)),
    )
}

fn build_production_parallel_mode_service() -> ParallelModeService {
    ParallelModeService::new(
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        Arc::new(GithubAutomationAdapter::new()),
        Arc::new(GitParallelModeRuntimeAdapter::new()),
    )
}

fn inspect_workspace(workspace_path: &Path) -> DoctorReport {
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(workspace_path) {
        return DoctorReport::path_issue(workspace_label, issue);
    }

    let planning = build_production_planning_services();
    let report = planning
        .workspace
        .inspect_workspace(workspace_path.to_string_lossy().as_ref());
    DoctorReport::from_service_report(workspace_label, report)
}

fn initialize_workspace(workspace_path: &Path) -> InitReport {
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(workspace_path) {
        return InitReport::path_issue(workspace_label, issue);
    }

    let planning = build_production_planning_services();
    match planning
        .workspace
        .initialize_simple_workspace(workspace_path.to_string_lossy().as_ref())
    {
        Ok(result) => {
            let snapshot = planning
                .runtime
                .load_runtime_snapshot_or_invalid(workspace_path.to_string_lossy().as_ref());
            InitReport::success(workspace_label, &result, &snapshot)
        }
        Err(error) => InitReport::failure(workspace_label, error.to_string()),
    }
}

fn reset_workspace(workspace_path: &Path, target: PlanningResetTarget) -> ResetReport {
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(workspace_path) {
        return ResetReport::path_issue(workspace_label, issue);
    }

    let planning = build_production_planning_services();
    match planning
        .workspace
        .reset_workspace(workspace_path.to_string_lossy().as_ref(), target)
    {
        Ok(result) => ResetReport::success(workspace_label, &result),
        Err(error) => ResetReport::failure(workspace_label, target, error.to_string()),
    }
}

fn parse_reset_target(target: &OsStr) -> Result<PlanningResetTarget> {
    match target
        .to_string_lossy()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "queue" => Ok(PlanningResetTarget::Queue),
        "directions" => Ok(PlanningResetTarget::Directions),
        "all" => Ok(PlanningResetTarget::All),
        _ => bail!("{RESET_USAGE}"),
    }
}

#[cfg(test)]
mod tests {
    use super::run_with_args;

    #[test]
    fn help_lists_planning_tool_command() {
        let mut output = Vec::new();

        let exit_code = run_with_args(["--help"], &mut output)
            .expect("help should render")
            .expect("help should exit");
        let rendered = String::from_utf8(output).expect("help should be utf8");

        assert_eq!(exit_code, 0);
        assert!(rendered.contains("akra planning-tool <contract|run>"));
        assert!(rendered.contains("akra parallel-tick [workspace_dir]"));
    }

    #[test]
    fn planning_tool_contract_is_json_and_llm_oriented() {
        let mut output = Vec::new();

        let exit_code = run_with_args(["planning-tool", "contract"], &mut output)
            .expect("contract should render")
            .expect("contract should exit");
        let rendered = String::from_utf8(output).expect("contract should be utf8");
        let value: serde_json::Value =
            serde_json::from_str(rendered.trim()).expect("contract should be JSON");

        assert_eq!(exit_code, 0);
        assert_eq!(value["tool"], "akra planning-tool");
        assert!(rendered.contains("akra planning-tool run ."));
        assert!(rendered.contains("do not use payload.worktree_path"));
        assert!(rendered.contains("list_tasks|create_task|update_task"));
    }
}
