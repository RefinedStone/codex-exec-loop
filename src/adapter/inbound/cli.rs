use crate::application::service::parallel_mode::ParallelModeOrchestratorTickResult;
use crate::application::service::planning::{
    PlanningControlCommand, PlanningControlRequest, PlanningResetTarget, PlanningServices,
    PlanningTaskToolRequest, PlanningTaskToolResponse,
};
use crate::composition::production;
use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/*
 * CLI adapter는 operational command를 위한 non-TUI entrypoint다.
 * argument parsing과 process exit code는 edge에 남기고, 실제 작업은 application service와 outbound adapter로 위임한다.
 * 그래서 TUI, admin API, automation tool이 같은 planning/parallel-mode 계약을 공유한다.
 */
mod reports;
mod usage;

use self::reports::{
    DoctorReport, PlanningToolErrorReport, ResetReport, render_doctor_report, render_json_line,
    render_reset_report,
};
use self::usage::{
    ADMIN_SERVER_ALIAS_USAGE, ADMIN_SERVER_USAGE, DOCTOR_USAGE, PARALLEL_TICK_USAGE,
    PLANNING_TOOL_USAGE, QUEUE_USAGE, RESET_USAGE, STATUS_USAGE, TELEGRAM_BOT_ALIAS_USAGE,
    TELEGRAM_BOT_USAGE,
};

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
            writeln!(stdout, "{STATUS_USAGE}")?;
            writeln!(stdout, "{QUEUE_USAGE}")?;
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
        [command] if command == OsStr::new("status") => Ok(Some(run_planning_control_command(
            PlanningControlCommand::Status,
            None,
            stdout,
        )?)),
        [command, workspace] if command == OsStr::new("status") => {
            Ok(Some(run_planning_control_command(
                PlanningControlCommand::Status,
                Some(workspace.as_os_str()),
                stdout,
            )?))
        }
        [command] if command == OsStr::new("queue") => Ok(Some(run_planning_control_command(
            PlanningControlCommand::Queue,
            None,
            stdout,
        )?)),
        [command, workspace] if command == OsStr::new("queue") => {
            Ok(Some(run_planning_control_command(
                PlanningControlCommand::Queue,
                Some(workspace.as_os_str()),
                stdout,
            )?))
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
        [command, _, ..] if command == OsStr::new("status") => {
            bail!("{STATUS_USAGE}");
        }
        [command, _, ..] if command == OsStr::new("queue") => {
            bail!("{QUEUE_USAGE}");
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

fn run_planning_control_command(
    command: PlanningControlCommand,
    workspace_arg: Option<&OsStr>,
    stdout: &mut impl Write,
) -> Result<i32> {
    let workspace_path = resolve_workspace_path(workspace_arg)?;
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(&workspace_path) {
        writeln!(stdout, "workspace: {workspace_label}")?;
        writeln!(stdout, "issue: {issue}")?;
        return Ok(1);
    }
    let control = production::build_planning_control_service(workspace_label);
    let response = control.execute_request(PlanningControlRequest::new(command))?;
    writeln!(stdout, "{}", response.reply.text)?;
    Ok(0)
}

fn run_planning_tool(
    subcommand: &OsStr,
    workspace_arg: Option<&OsStr>,
    stdout: &mut impl Write,
) -> Result<i32> {
    // planning tool은 의도적으로 script/worker 지향이다. contract는 schema를 출력하고 run은 stdin payload를 소비한다.
    let planning = production::build_planning_services();
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
    let control_plane = production::build_parallel_mode_control_plane_composition();
    let workspace_label = workspace_path.display().to_string();

    writeln!(stdout, "workspace: {workspace_label}")?;
    // 이 command는 TUI가 supervise하는 같은 distributor queue를 수동/cron 환경에서 tick하는 driver다.
    match control_plane.run_manual_orchestrator_tick(&workspace_label) {
        Ok(result) => render_parallel_tick_result(stdout, &result),
        Err(error) => {
            writeln!(stdout, "parallel distributor tick failed: {error}")?;
            Ok(1)
        }
    }
}

fn render_parallel_tick_result(
    stdout: &mut impl Write,
    result: &ParallelModeOrchestratorTickResult,
) -> Result<i32> {
    if result.notices.is_empty() {
        writeln!(stdout, "parallel distributor queue-idle")?;
    } else {
        for notice in &result.notices {
            writeln!(stdout, "{notice}")?;
        }
    }

    if result.blocked { Ok(1) } else { Ok(0) }
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

fn inspect_workspace(workspace_path: &Path) -> DoctorReport {
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(workspace_path) {
        return DoctorReport::path_issue(workspace_label, issue);
    }
    let planning = production::build_planning_services();
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
    let planning = production::build_planning_services();
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
    use super::{
        DOCTOR_USAGE, PARALLEL_TICK_USAGE, PLANNING_TOOL_USAGE, QUEUE_USAGE, RESET_USAGE,
        STATUS_USAGE, is_admin_command, is_help_flag, is_planning_tool_command,
        is_telegram_command, parse_reset_target, render_parallel_tick_result,
        resolve_workspace_path, run_doctor, run_parallel_tick, run_planning_control_command,
        run_planning_tool, run_reset, run_with_args, validate_workspace_path,
    };
    use crate::application::service::parallel_mode::{
        ParallelModeOrchestratorTickResult, ParallelModeOrchestratorTrigger,
    };
    use crate::application::service::planning::{PlanningControlCommand, PlanningResetTarget};
    use crate::domain::parallel_mode::ParallelModeOrchestratorStateMachine;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    fn unique_temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{}-{}",
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        ))
    }

    fn create_temp_workspace(label: &str) -> String {
        let path = unique_temp_path(label);
        std::fs::create_dir_all(&path).expect("temp workspace should be created");
        path.to_string_lossy().into_owned()
    }

    fn dispatch_error(args: &[&str]) -> String {
        let mut output = Vec::new();
        run_with_args(args.iter().copied(), &mut output)
            .expect_err("dispatcher should reject invalid command shape")
            .to_string()
    }

    #[test]
    fn command_dispatcher_handles_empty_and_arity_errors_at_the_edge() {
        let mut output = Vec::new();
        let exit = run_with_args(std::iter::empty::<&str>(), &mut output)
            .expect("empty args should fall through to TUI");

        assert_eq!(exit, None);
        assert!(output.is_empty());

        for (args, usage) in [
            (vec!["doctor", "one", "two"], DOCTOR_USAGE),
            (vec!["status", "one", "two"], STATUS_USAGE),
            (vec!["queue", "one", "two"], QUEUE_USAGE),
            (vec!["reset", "queue", "one", "two"], RESET_USAGE),
            (
                vec!["planning-task-tool", "run", "one", "two"],
                PLANNING_TOOL_USAGE,
            ),
            (vec!["parallel-tick", "one", "two"], PARALLEL_TICK_USAGE),
        ] {
            assert_eq!(dispatch_error(&args), usage, "{args:?}");
        }

        assert_eq!(
            dispatch_error(&["unsupported-command"]),
            "unsupported command: unsupported-command"
        );
    }

    #[test]
    fn command_classifier_accepts_documented_aliases_only() {
        assert!(is_help_flag(OsStr::new("-h")));
        assert!(is_help_flag(OsStr::new("--help")));
        assert!(!is_help_flag(OsStr::new("help")));

        assert!(is_admin_command(OsStr::new("admin")));
        assert!(is_admin_command(OsStr::new("admin-server")));
        assert!(!is_admin_command(OsStr::new("admin-api")));

        assert!(is_telegram_command(OsStr::new("telegram")));
        assert!(is_telegram_command(OsStr::new("telegram-bot")));
        assert!(!is_telegram_command(OsStr::new("bot")));

        assert!(is_planning_tool_command(OsStr::new("planning-tool")));
        assert!(is_planning_tool_command(OsStr::new("planning-task-tool")));
        assert!(!is_planning_tool_command(OsStr::new("task-tool")));
    }

    #[test]
    fn workspace_path_helpers_keep_missing_file_and_directory_diagnostics_stable() {
        let current_dir = std::env::current_dir().expect("current dir should resolve");
        let resolved_cwd =
            resolve_workspace_path(None).expect("missing workspace arg should use cwd");
        assert_eq!(
            resolved_cwd,
            current_dir
                .canonicalize()
                .expect("current dir should canonicalize")
        );

        let missing = unique_temp_path("cli-missing-workspace");
        let missing_issue =
            validate_workspace_path(&missing).expect_err("missing path should fail");
        assert!(missing_issue.contains("workspace path does not exist"));
        assert!(missing_issue.contains(&missing.display().to_string()));

        let file_path = unique_temp_path("cli-file-workspace");
        std::fs::write(&file_path, "not a directory").expect("temp file should be written");
        let file_issue = validate_workspace_path(&file_path).expect_err("file path should fail");
        assert!(file_issue.contains("workspace path is not a directory"));
        std::fs::remove_file(&file_path).expect("temp file should be removed");

        let workspace = unique_temp_path("cli-existing-workspace");
        std::fs::create_dir_all(&workspace).expect("temp workspace should be created");
        let resolved_existing = resolve_workspace_path(Some(workspace.as_os_str()))
            .expect("existing workspace should canonicalize");
        assert_eq!(
            resolved_existing,
            workspace
                .canonicalize()
                .expect("temp workspace should canonicalize")
        );

        let relative_name = format!(
            "target/cli-relative-workspace-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after epoch")
                .as_nanos()
        );
        let relative_path = current_dir.join(&relative_name);
        std::fs::create_dir_all(&relative_path).expect("relative workspace should be created");
        let resolved_relative = resolve_workspace_path(Some(OsStr::new(&relative_name)))
            .expect("relative workspace should resolve");
        assert_eq!(
            resolved_relative,
            relative_path
                .canonicalize()
                .expect("relative workspace should canonicalize")
        );

        let future_path = unique_temp_path("cli-future-workspace");
        let resolved_missing =
            resolve_workspace_path(Some(future_path.as_os_str())).expect("missing path is allowed");
        assert_eq!(resolved_missing, future_path);

        std::fs::remove_dir_all(relative_path).expect("relative workspace should be removed");
        std::fs::remove_dir_all(workspace).expect("temp workspace should be removed");
    }

    #[test]
    fn path_issue_commands_return_edge_failures_without_entering_services() {
        let missing = unique_temp_path("cli-command-missing");
        let missing_arg = missing.as_os_str();

        let mut doctor_output = Vec::new();
        let doctor_exit =
            run_doctor(Some(missing_arg), &mut doctor_output).expect("doctor should render issue");
        let doctor_rendered = String::from_utf8(doctor_output).expect("doctor output is utf8");
        assert_eq!(doctor_exit, 1);
        assert!(doctor_rendered.contains("planning state: invalid"));
        assert!(doctor_rendered.contains("workspace path does not exist"));

        let mut reset_output = Vec::new();
        let reset_exit = run_reset(OsStr::new("queue"), Some(missing_arg), &mut reset_output)
            .expect("reset should render issue");
        let reset_rendered = String::from_utf8(reset_output).expect("reset output is utf8");
        assert_eq!(reset_exit, 1);
        assert!(reset_rendered.contains("command: reset"));
        assert!(reset_rendered.contains("workspace path does not exist"));

        let mut status_output = Vec::new();
        let status_exit = run_planning_control_command(
            PlanningControlCommand::Status,
            Some(missing_arg),
            &mut status_output,
        )
        .expect("status should render path issue");
        let status_rendered = String::from_utf8(status_output).expect("status output is utf8");
        assert_eq!(status_exit, 1);
        assert!(status_rendered.contains("workspace:"));
        assert!(status_rendered.contains("issue: workspace path does not exist"));

        let mut tick_output = Vec::new();
        let tick_error = run_parallel_tick(Some(missing_arg), &mut tick_output)
            .expect_err("parallel tick should reject missing workspace before rendering")
            .to_string();
        assert!(tick_error.contains("workspace path does not exist"));
        assert!(tick_output.is_empty());
    }

    #[test]
    fn dispatcher_routes_workspace_path_failures_through_subcommand_branches() {
        let missing = unique_temp_path("cli-dispatcher-missing");
        let missing_arg = missing.to_string_lossy().into_owned();

        for (args, expected) in [
            (
                vec!["doctor", missing_arg.as_str()],
                "planning state: invalid",
            ),
            (
                vec!["status", missing_arg.as_str()],
                "issue: workspace path does not exist",
            ),
            (
                vec!["queue", missing_arg.as_str()],
                "issue: workspace path does not exist",
            ),
            (
                vec!["reset", "queue", missing_arg.as_str()],
                "command: reset",
            ),
            (
                vec!["planning-tool", "run", missing_arg.as_str()],
                "\"operation\":\"planning-tool\"",
            ),
        ] {
            let mut output = Vec::new();
            let exit = run_with_args(args, &mut output)
                .expect("path issue branch should render")
                .expect("subcommand should exit");
            let rendered = String::from_utf8(output).expect("output should be utf8");

            assert_eq!(exit, 1);
            assert!(rendered.contains(expected), "{rendered}");
        }

        let mut tick_output = Vec::new();
        let tick_error = run_with_args(["parallel-tick", missing_arg.as_str()], &mut tick_output)
            .expect_err("parallel-tick should reject missing workspace through dispatcher")
            .to_string();
        assert!(tick_error.contains("workspace path does not exist"));
        assert!(tick_output.is_empty());
    }

    #[test]
    fn doctor_and_reset_existing_workspace_paths_enter_application_facades() {
        let workspace = create_temp_workspace("cli-existing-planning");
        let workspace_arg = OsStr::new(&workspace);

        let mut doctor_output = Vec::new();
        let doctor_exit =
            run_doctor(Some(workspace_arg), &mut doctor_output).expect("doctor should render");
        let doctor_rendered = String::from_utf8(doctor_output).expect("doctor output is utf8");

        assert_eq!(doctor_exit, 0);
        assert!(doctor_rendered.contains("workspace:"));
        assert!(doctor_rendered.contains("planning state: ready"));

        let mut reset_output = Vec::new();
        let reset_exit = run_reset(OsStr::new("queue"), Some(workspace_arg), &mut reset_output)
            .expect("reset should render");
        let reset_rendered = String::from_utf8(reset_output).expect("reset output is utf8");

        assert_eq!(reset_exit, 0);
        assert!(reset_rendered.contains("target: queue"));
        assert!(reset_rendered.contains("status: planning workspace reset"));

        std::fs::remove_dir_all(workspace).expect("temp workspace should be removed");
    }

    #[test]
    fn planning_tool_run_reports_json_failure_before_stdin_for_missing_workspace() {
        let missing = unique_temp_path("cli-planning-tool-missing");
        let mut output = Vec::new();
        let exit = run_planning_tool(OsStr::new("run"), Some(missing.as_os_str()), &mut output)
            .expect("planning-tool run should render JSON error");
        let rendered = String::from_utf8(output).expect("planning-tool output is utf8");
        let payload: serde_json::Value =
            serde_json::from_str(rendered.trim_end()).expect("error output should be JSONL");

        assert_eq!(exit, 1);
        assert_eq!(payload["ok"], false);
        assert_eq!(payload["operation"], "planning-tool");
        assert!(
            payload["error"]
                .as_str()
                .expect("error should be a string")
                .contains("workspace path does not exist")
        );
        assert!(
            payload["guidance"]
                .as_array()
                .expect("guidance should be an array")
                .iter()
                .any(|value| value
                    .as_str()
                    .expect("guidance entry should be a string")
                    .contains("Run `akra planning-tool contract`"))
        );

        let mut bad_subcommand_output = Vec::new();
        let bad_subcommand = run_planning_tool(
            OsStr::new("unknown"),
            Some(missing.as_os_str()),
            &mut bad_subcommand_output,
        )
        .expect_err("unknown planning-tool subcommand should report usage")
        .to_string();
        assert_eq!(bad_subcommand, PLANNING_TOOL_USAGE);
        assert!(bad_subcommand_output.is_empty());
    }

    #[test]
    fn help_lists_planning_tool_command() {
        let mut output = Vec::new();
        let exit_code = run_with_args(["--help"], &mut output)
            .expect("help should render")
            .expect("help should exit");
        let rendered = String::from_utf8(output).expect("help should be utf8");

        assert_eq!(exit_code, 0);
        assert!(rendered.contains("akra status [workspace_dir]"));
        assert!(rendered.contains("akra queue [workspace_dir]"));
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

    #[test]
    fn status_and_queue_commands_use_planning_control_surface() {
        let workspace = create_temp_workspace("cli-planning-control");
        let mut status_output = Vec::new();
        let status_exit = run_with_args(
            vec!["status".to_string(), workspace.clone()],
            &mut status_output,
        )
        .expect("status should render")
        .expect("status should exit");
        let status_rendered = String::from_utf8(status_output).expect("status should be utf8");

        let mut queue_output = Vec::new();
        let queue_exit = run_with_args(
            vec!["queue".to_string(), workspace.clone()],
            &mut queue_output,
        )
        .expect("queue should render")
        .expect("queue should exit");
        let queue_rendered = String::from_utf8(queue_output).expect("queue should be utf8");

        assert_eq!(status_exit, 0);
        assert!(status_rendered.contains("상태 요약"));
        assert!(status_rendered.contains("planning_state:"));
        assert_eq!(queue_exit, 0);
        assert!(queue_rendered.contains("큐 요약"));

        std::fs::remove_dir_all(workspace).expect("temp workspace should be removed");
    }

    #[test]
    fn reset_command_spelling_maps_to_shared_application_target() {
        /*
         * CLI spelling is an inbound grammar detail. The application reset path
         * should receive PlanningResetTarget, not a CLI-only target enum or
         * free-form destructive string.
         */
        for (raw, expected) in [
            ("queue", PlanningResetTarget::Queue),
            ("directions", PlanningResetTarget::Directions),
            ("all", PlanningResetTarget::All),
        ] {
            assert_eq!(parse_reset_target(OsStr::new(raw)).unwrap(), expected);
        }
        assert!(parse_reset_target(OsStr::new("tasks")).is_err());
    }

    #[test]
    fn parallel_tick_result_renderer_uses_application_tick_state() {
        /*
         * `akra parallel-tick` should render the application tick result instead
         * of calling distributor internals directly. Blocked is an application
         * result state and must affect the process exit code.
         */
        let mut idle_output = Vec::new();
        let idle_exit = render_parallel_tick_result(
            &mut idle_output,
            &ParallelModeOrchestratorTickResult {
                trigger: ParallelModeOrchestratorTrigger::ManualDispatch,
                state: ParallelModeOrchestratorStateMachine::tick_state(false),
                blocked: false,
                notices: Vec::new(),
            },
        )
        .expect("idle tick result should render");
        assert_eq!(idle_exit, 0);
        assert_eq!(
            String::from_utf8(idle_output).expect("idle output should be utf8"),
            "parallel distributor queue-idle\n"
        );

        let mut blocked_output = Vec::new();
        let blocked_exit = render_parallel_tick_result(
            &mut blocked_output,
            &ParallelModeOrchestratorTickResult {
                trigger: ParallelModeOrchestratorTrigger::ManualDispatch,
                state: ParallelModeOrchestratorStateMachine::tick_state(true),
                blocked: true,
                notices: vec!["integration worktree is blocked".to_string()],
            },
        )
        .expect("blocked tick result should render");
        assert_eq!(blocked_exit, 1);
        assert_eq!(
            String::from_utf8(blocked_output).expect("blocked output should be utf8"),
            "integration worktree is blocked\n"
        );
    }

    #[test]
    fn parallel_tick_enters_through_control_plane_composition() {
        /*
         * CLI must not become a second direct ParallelModeService caller. It can
         * render a synchronous result, but the service graph should be the same
         * control-plane composition used by TUI/admin surfaces.
         */
        let source = include_str!("cli.rs");
        let production_source = source
            .split("#[cfg(test)]")
            .next()
            .expect("CLI source should contain production section");

        assert!(production_source.contains("run_manual_orchestrator_tick"));
        assert!(
            !production_source.contains(".run_orchestrator_tick("),
            "CLI parallel-tick should call the control-plane composition facade"
        );
    }
}
