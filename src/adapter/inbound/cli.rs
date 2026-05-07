use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::GithubAutomationAdapter;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::planning::{
    PlanningResetTarget, PlanningServices, PlanningTaskToolRequest, PlanningTaskToolResponse,
};
use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/*
 * CLI adapter는 operational command를 위한 non-TUI entrypoint다.
 * argument parsing과 process exit code는 edge에 남기고, 실제 작업은 application service와 outbound adapter로 위임한다.
 * 그래서 TUI, admin API, automation tool이 같은 planning/parallel-mode 계약을 공유한다.
 */
mod reports;

use self::reports::{
    DoctorReport, PlanningToolErrorReport, ResetReport, render_doctor_report, render_json_line,
    render_reset_report,
};

// usage string은 help copy이면서 arity mistake에 대한 정확한 error message다. dispatcher 옆에 두어 route와 copy가 함께 바뀌게 한다.
const ADMIN_SERVER_USAGE: &str = "Usage: akra admin [--port <port>]";
const ADMIN_SERVER_ALIAS_USAGE: &str = "Alias: akra admin-server [--port <port>]";
const DOCTOR_USAGE: &str = "Usage: akra doctor [workspace_dir]";
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
    // None 반환은 native TUI가 계속 booting해야 한다는 뜻이다. 처리된 subcommand는 모두 explicit exit code를 돌린다.
    match args.as_slice() {
        [] => Ok(None),
        [flag] if is_help_flag(flag) => {
            writeln!(stdout, "{ADMIN_SERVER_USAGE}")?;
            writeln!(stdout, "{ADMIN_SERVER_ALIAS_USAGE}")?;
            writeln!(stdout, "{TELEGRAM_BOT_USAGE}")?;
            writeln!(stdout, "{TELEGRAM_BOT_ALIAS_USAGE}")?;
            writeln!(stdout, "{DOCTOR_USAGE}")?;
            writeln!(stdout, "{RESET_USAGE}")?;
            writeln!(stdout, "{PLANNING_TOOL_USAGE}")?;
            writeln!(stdout, "{PARALLEL_TICK_USAGE}")?;
            Ok(Some(0))
        }
        // long-running async service는 첫 command token 뒤의 parsing을 각자 소유한다.
        [command] if is_admin_command(command) => Ok(Some(run_admin_server(&[])?)),
        [command, rest @ ..] if is_admin_command(command) => Ok(Some(run_admin_server(rest)?)),
        [command] if is_telegram_command(command) => Ok(Some(run_telegram_bot(&[])?)),
        [command, rest @ ..] if is_telegram_command(command) => Ok(Some(run_telegram_bot(rest)?)),
        [command] if command == OsStr::new("doctor") => Ok(Some(run_doctor(None, stdout)?)),
        [command, workspace] if command == OsStr::new("doctor") => {
            Ok(Some(run_doctor(Some(workspace.as_os_str()), stdout)?))
        }
        // planning maintenance command는 optional workspace를 받고, 없으면 cwd를 사용한다.
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
        // arity-specific branch를 먼저 두어 unsupported-command error가 정말 unknown command에만 쓰이게 한다.
        [command, _, ..] if command == OsStr::new("doctor") => {
            bail!("{DOCTOR_USAGE}");
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
    // admin API는 async이고 CLI dispatch는 테스트하기 쉬운 synchronous 표면이다. 여기서 runtime을 만들어 경계를 맞춘다.
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
    // planning tool은 의도적으로 script/worker 지향이다. contract는 schema를 출력하고 run은 stdin payload를 소비한다.
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
            // tool caller는 anyhow backtrace보다 structured failure output을 기대한다.
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
    // 이 command는 TUI가 supervise하는 같은 distributor queue를 수동/cron 환경에서 tick하는 driver다.
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
    // stdin을 쓰면 request 크기와 quoting이 shell argument parsing에서 독립된다.
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
    // existing path는 stable report를 위해 canonicalize하고, 아직 없는 future path는 diagnostic용 absolute path로 유지한다.
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
    // 모든 planning CLI command는 native client가 쓰는 repo-scoped authority store를 공유한다.
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
    // parallel tick은 GitHub automation과 local git/worktree runtime port를 모두 필요로 한다.
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
    // report shaping은 CLI adapter에 남긴다. application service가 UI-neutral하게 유지되게 하기 위해서다.
    DoctorReport::from_service_report(workspace_label, report)
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
    // 사람이 입력하는 CLI spelling을 받되, boundary에서 application reset contract로 매핑한다.
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
        assert!(!rendered.contains("akra init"));
    }
    #[test]
    fn planning_tool_contract_is_json_and_worker_oriented() {
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
