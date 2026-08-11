use crate::application::port::inbound::parallel_mode_control_port::ParallelModeOrchestratorTickResult;
use crate::application::port::inbound::planning_control_port::{
    PlanningControlCommand, PlanningControlRequest,
};
use crate::application::port::inbound::planning_task_tool_port::{
    PlanningTaskToolPort, PlanningTaskToolRequest, PlanningTaskToolResponse,
    planning_task_tool_contract_json,
};
use crate::application::port::inbound::pr_validation_query_port::PrValidationStatusRequest;
use crate::application::port::planning_task_tool_contract::{
    PLANNING_TOOL_PARENT_THREAD_ID_ENV, PLANNING_TOOL_PARENT_TURN_ID_ENV,
};
use crate::composition::production;
use crate::configuration::{
    ConfigDoctorReport, ConfigOverride, ConfigScope, ConfigurationService, ParsedInvocation,
    SettingKey,
};
use crate::domain::planning::PlanningResetTarget;
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
    ADMIN_SERVER_ALIAS_USAGE, ADMIN_SERVER_USAGE, CONFIG_USAGE, DOCTOR_USAGE, PARALLEL_TICK_USAGE,
    PLANNING_TOOL_USAGE, QUEUE_USAGE, RESET_USAGE, STATUS_USAGE, TELEGRAM_BOT_ALIAS_USAGE,
    TELEGRAM_BOT_USAGE,
};

pub fn run_with_env_args(stdout: &mut impl Write) -> Result<Option<i32>> {
    let invocation = ParsedInvocation::parse(std::env::args_os().skip(1))?;
    if invocation.overrides.is_empty() {
        return run_with_args(invocation.command_args, stdout);
    }
    run_with_args_with_config_overrides(invocation.command_args, &invocation.overrides, stdout)
}

pub(crate) fn run_with_args<I, T>(args: I, stdout: &mut impl Write) -> Result<Option<i32>>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    run_with_args_with_config_overrides(args, &[], stdout)
}

pub(crate) fn run_with_args_with_config_overrides<I, T>(
    args: I,
    overrides: &[ConfigOverride],
    stdout: &mut impl Write,
) -> Result<Option<i32>>
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
            writeln!(stdout, "{CONFIG_USAGE}")?;
            Ok(Some(0))
        }
        [command, rest @ ..] if command == OsStr::new("config") => {
            Ok(Some(run_config(rest, overrides, stdout)?))
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
        [command, flag, pull_request]
            if command == OsStr::new("status") && flag == OsStr::new("--pr") =>
        {
            Ok(Some(run_pr_validation_status(
                pull_request.as_os_str(),
                None,
                stdout,
            )?))
        }
        [command, flag, pull_request, workspace]
            if command == OsStr::new("status") && flag == OsStr::new("--pr") =>
        {
            Ok(Some(run_pr_validation_status(
                pull_request.as_os_str(),
                Some(workspace.as_os_str()),
                stdout,
            )?))
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
        [command, _, ..] if command == OsStr::new("config") => {
            bail!("{CONFIG_USAGE}");
        }
        [command, ..] => {
            bail!("unsupported command: {}", command.to_string_lossy());
        }
    }
}

fn run_config(
    args: &[OsString],
    overrides: &[ConfigOverride],
    stdout: &mut impl Write,
) -> Result<i32> {
    let cwd = std::env::current_dir().context("failed to resolve current directory for config")?;
    let (scope, positional) = parse_config_scope(args)?;
    let Some(command) = positional.first().copied() else {
        bail!("{CONFIG_USAGE}");
    };
    let command = command
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("config command must be valid UTF-8"))?;
    match command {
        "path" if positional.len() == 1 => {
            let path =
                ConfigurationService::path_for_scope(&cwd, scope.unwrap_or(ConfigScope::Global))?;
            writeln!(stdout, "{}", path.display())?;
            Ok(0)
        }
        "list" if positional.len() == 1 => {
            match scope {
                Some(scope) => {
                    let path = ConfigurationService::path_for_scope(&cwd, scope)?;
                    writeln!(stdout, "scope: {}", scope.label())?;
                    writeln!(stdout, "path: {}", path.display())?;
                    match ConfigurationService::layer_for_scope(&cwd, scope)? {
                        Some(layer) => {
                            for key in SettingKey::ALL {
                                if let Some(value) = layer.configured_value(key) {
                                    writeln!(stdout, "{} = {} ({})", key, value, scope.label())?;
                                }
                            }
                        }
                        None => writeln!(stdout, "status: unset")?,
                    }
                }
                None => {
                    let resolved = ConfigurationService::resolve_read_only(&cwd, overrides)?;
                    for setting in resolved.effective_settings() {
                        writeln!(
                            stdout,
                            "{} = {} ({})",
                            setting.key, setting.value, setting.origin
                        )?;
                    }
                }
            }
            Ok(0)
        }
        "get" if positional.len() == 2 => {
            let key = parse_config_key(positional[1])?;
            match scope {
                Some(scope) => {
                    let path = ConfigurationService::path_for_scope(&cwd, scope)?;
                    match ConfigurationService::layer_for_scope(&cwd, scope)?
                        .and_then(|layer| layer.configured_value(key))
                    {
                        Some(value) => writeln!(stdout, "{} = {} ({})", key, value, scope.label())?,
                        None => writeln!(stdout, "{} = unset ({})", key, scope.label())?,
                    }
                    writeln!(stdout, "path: {}", path.display())?;
                }
                None => {
                    let resolved = ConfigurationService::resolve_read_only(&cwd, overrides)?;
                    let setting = resolved
                        .effective_settings()
                        .into_iter()
                        .find(|setting| setting.key == key)
                        .expect("known setting must be resolved");
                    writeln!(
                        stdout,
                        "{} = {} ({})",
                        setting.key, setting.value, setting.origin
                    )?;
                }
            }
            Ok(0)
        }
        "set" if positional.len() == 3 => {
            let key = parse_config_key(positional[1])?;
            let value = positional[2]
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("configuration value must be valid UTF-8"))?;
            let scope = scope.unwrap_or(ConfigScope::Global);
            let path = ConfigurationService::set(&cwd, scope, key, value)?;
            writeln!(stdout, "updated: {}", path.display())?;
            writeln!(stdout, "{} = {} ({})", key, value, scope.label())?;
            Ok(0)
        }
        "unset" if positional.len() == 2 => {
            let key = parse_config_key(positional[1])?;
            let scope = scope.unwrap_or(ConfigScope::Global);
            let path = ConfigurationService::unset(&cwd, scope, key)?;
            writeln!(stdout, "updated: {}", path.display())?;
            writeln!(stdout, "{} = unset ({})", key, scope.label())?;
            Ok(0)
        }
        "doctor" if positional.len() == 1 => {
            let report = ConfigurationService::doctor(&cwd, overrides);
            render_config_doctor(stdout, &report)?;
            Ok(if report.is_healthy() { 0 } else { 1 })
        }
        _ => bail!("{CONFIG_USAGE}"),
    }
}

fn parse_config_scope(args: &[OsString]) -> Result<(Option<ConfigScope>, Vec<&OsString>)> {
    let mut scope = None;
    let mut positional = Vec::new();
    for argument in args {
        match argument.to_str() {
            Some("--global") => set_config_scope(&mut scope, ConfigScope::Global)?,
            Some("--project") => set_config_scope(&mut scope, ConfigScope::Project)?,
            _ => positional.push(argument),
        }
    }
    Ok((scope, positional))
}

fn set_config_scope(slot: &mut Option<ConfigScope>, value: ConfigScope) -> Result<()> {
    if slot.replace(value).is_some() {
        bail!("choose at most one of --global and --project")
    }
    Ok(())
}

fn parse_config_key(argument: &OsString) -> Result<SettingKey> {
    let key = argument
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("configuration key must be valid UTF-8"))?;
    SettingKey::parse(key)
}

fn render_config_doctor(stdout: &mut impl Write, report: &ConfigDoctorReport) -> Result<()> {
    let Some(paths) = report.paths.as_ref() else {
        writeln!(stdout, "configuration: {}", report.global)?;
        return Ok(());
    };
    writeln!(stdout, "global path: {}", paths.global.display())?;
    writeln!(stdout, "global config: {}", report.global)?;
    match (&paths.project, &report.project) {
        (Some(path), Some(inspection)) => {
            writeln!(stdout, "project path: {}", path.display())?;
            writeln!(stdout, "project config: {inspection}")?;
        }
        _ => writeln!(stdout, "project config: unavailable (not a Git worktree)")?,
    }
    for shadow in &report.environment {
        writeln!(stdout, "environment shadow: {shadow}")?;
    }
    for legacy in &report.legacy {
        writeln!(stdout, "legacy Git config: {legacy}")?;
    }
    if let Some(error) = &report.resolution_error {
        writeln!(stdout, "resolution: error: {error}")?;
    } else {
        writeln!(stdout, "resolution: ok")?;
    }
    Ok(())
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

fn run_pr_validation_status(
    pull_request_arg: &OsStr,
    workspace_arg: Option<&OsStr>,
    stdout: &mut impl Write,
) -> Result<i32> {
    let pull_request_number = pull_request_arg
        .to_str()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| anyhow::anyhow!("invalid pull request number; {STATUS_USAGE}"))?;
    let workspace_path = resolve_workspace_path(workspace_arg)?;
    validate_workspace_path(&workspace_path).map_err(anyhow::Error::msg)?;
    let workspace_label = workspace_path.display().to_string();
    let query = production::build_pr_validation_query_port(&workspace_label);
    let Some(summary) = query.status_for_pr(PrValidationStatusRequest {
        pull_request_number,
    })?
    else {
        writeln!(stdout, "PR validation")?;
        writeln!(stdout, "pr: {pull_request_number}")?;
        writeln!(stdout, "state: unavailable")?;
        writeln!(stdout, "next: no durable validation record was found")?;
        return Ok(1);
    };

    writeln!(stdout, "PR validation")?;
    writeln!(stdout, "akra_id: {}", summary.akra_id)?;
    writeln!(stdout, "pr: {}", summary.pull_request_number)?;
    writeln!(stdout, "url: {}", summary.canonical_pr_url)?;
    writeln!(stdout, "state: {}", summary.state.label())?;
    writeln!(stdout, "phase: {}", summary.phase_label())?;
    writeln!(stdout, "reason: {}", summary.reason)?;
    writeln!(
        stdout,
        "blocker: {}",
        summary.blocker.as_deref().unwrap_or("none")
    )?;
    writeln!(stdout, "target: {}", summary.target_short_sha)?;
    writeln!(
        stdout,
        "integration: {}",
        summary
            .integration_method
            .map(|method| method.label())
            .unwrap_or("pending")
    )?;
    writeln!(
        stdout,
        "evidence: {}",
        summary.evidence_short_sha.as_deref().unwrap_or("pending")
    )?;
    writeln!(
        stdout,
        "merge: {}",
        summary.merge_short_sha.as_deref().unwrap_or("pending")
    )?;
    writeln!(stdout, "findings: {}", summary.finding_count)?;
    writeln!(stdout, "remediations: {}", summary.remediation_count)?;
    for correlation in &summary.correlations {
        writeln!(
            stdout,
            "correlation: {} -> {}",
            correlation.finding, correlation.remediation_akra_id
        )?;
    }
    writeln!(
        stdout,
        "observation_revision: {}",
        summary.observation_revision
    )?;
    writeln!(
        stdout,
        "post_merge_checkpoint: {}",
        if summary.post_merge_checkpoint_observed {
            "observed"
        } else {
            "pending"
        }
    )?;
    writeln!(stdout, "recovery: {}", summary.next_action())?;
    Ok(0)
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
    let control = production::build_planning_control_port(workspace_label);
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
    match subcommand.to_str() {
        Some("contract") => {
            writeln!(stdout, "{}", planning_task_tool_contract_json())?;
            Ok(0)
        }
        Some("run") => {
            let workspace_path = resolve_workspace_path(workspace_arg)?;
            let workspace_label = workspace_path.display().to_string();
            let task_tool = production::build_planning_task_tool_port(&workspace_label);
            let result = run_planning_tool_request(task_tool.as_ref(), &workspace_path);
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
    let workspace_label = workspace_path.display().to_string();
    let control_plane = production::build_parallel_mode_control_port(&workspace_label);

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
    task_tool: &dyn PlanningTaskToolPort,
    workspace_path: &Path,
) -> Result<PlanningTaskToolResponse> {
    validate_workspace_path(workspace_path).map_err(anyhow::Error::msg)?;
    let mut request_json = String::new();
    // stdin을 쓰면 request 크기와 quoting이 shell argument parsing에서 독립된다.
    std::io::stdin()
        .read_to_string(&mut request_json)
        .context("failed to read planning-tool JSON request from stdin")?;
    let mut request = serde_json::from_str::<PlanningTaskToolRequest>(&request_json)
        .context("failed to parse planning-tool JSON request")?;
    request.apply_cli_host_context(
        optional_utf8_environment(PLANNING_TOOL_PARENT_THREAD_ID_ENV)?,
        optional_utf8_environment(PLANNING_TOOL_PARENT_TURN_ID_ENV)?,
    );
    task_tool.run(workspace_path.to_string_lossy().as_ref(), request)
}

fn optional_utf8_environment(name: &str) -> Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(anyhow::anyhow!("{name} must contain valid UTF-8"))
        }
    }
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
    let workspace = production::build_planning_workspace_maintenance_port(&workspace_label);
    let report = workspace.inspect_workspace(workspace_path.to_string_lossy().as_ref());
    // report shaping은 CLI adapter에 남긴다. application service가 UI-neutral하게 유지되게 하기 위해서다.
    DoctorReport::from_service_report(workspace_label, report)
}

fn reset_workspace(workspace_path: &Path, target: PlanningResetTarget) -> ResetReport {
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(workspace_path) {
        return ResetReport::path_issue(workspace_label, issue);
    }
    let workspace = production::build_planning_workspace_maintenance_port(&workspace_label);
    match workspace.reset_workspace(workspace_path.to_string_lossy().as_ref(), target) {
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
        run_planning_tool, run_reset, run_with_args, run_with_args_with_config_overrides,
        validate_workspace_path,
    };
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::application::service::parallel_mode::{
        ParallelModeOrchestratorTickResult, ParallelModeOrchestratorTrigger,
    };
    use crate::application::service::planning::{PlanningControlCommand, PlanningResetTarget};
    use crate::configuration::ConfigOverride;
    use crate::domain::parallel_mode::{
        ParallelModeOrchestratorStateMachine, PrValidationCommitSha, PrValidationEvent,
        PrValidationFinding, PrValidationFindingKey, PrValidationFindingSource, PrValidationRecord,
        PrValidationRecordKey, PrValidationRemediationCorrelation, PrValidationTarget,
        PrValidationTargetShaSnapshot,
    };
    use std::ffi::OsStr;
    use std::path::PathBuf;

    struct EnvironmentGuard {
        previous_akra_home: Option<std::ffi::OsString>,
    }

    impl EnvironmentGuard {
        fn with_akra_home(path: &std::path::Path) -> Self {
            let previous_akra_home = std::env::var_os("AKRA_HOME");
            unsafe { std::env::set_var("AKRA_HOME", path) };
            Self { previous_akra_home }
        }
    }

    impl Drop for EnvironmentGuard {
        fn drop(&mut self) {
            match self.previous_akra_home.take() {
                Some(value) => unsafe { std::env::set_var("AKRA_HOME", value) },
                None => unsafe { std::env::remove_var("AKRA_HOME") },
            }
        }
    }

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
    fn config_commands_keep_reads_non_mutating_and_render_effective_origins() {
        let _lock = crate::test_utils::process_environment_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let home = unique_temp_path("cli-config-home");
        let _environment = EnvironmentGuard::with_akra_home(&home);
        let config_path = home.join("config.toml");

        for command in [
            vec!["config", "path"],
            vec!["config", "list"],
            vec!["config", "get", "conversation.model"],
            vec!["config", "doctor"],
        ] {
            let mut output = Vec::new();
            let exit = run_with_args(command.iter().copied(), &mut output)
                .expect("read-only config command should succeed");
            assert_eq!(exit, Some(0));
            assert!(
                !config_path.exists(),
                "{command:?} must not create a missing global configuration"
            );
        }

        let mut set_output = Vec::new();
        let set_exit = run_with_args(
            ["config", "set", "conversation.model", "gpt-5.6-terra"],
            &mut set_output,
        )
        .expect("global config set should succeed");
        assert_eq!(set_exit, Some(0));
        assert!(config_path.exists());

        let mut get_output = Vec::new();
        run_with_args(["config", "get", "conversation.model"], &mut get_output)
            .expect("effective config get should succeed");
        let rendered_get = String::from_utf8(get_output).expect("config output should be UTF-8");
        assert!(rendered_get.contains("gpt-5.6-terra (global)"));

        let override_value = ConfigOverride::parse("conversation.model=gpt-5.6-luna")
            .expect("command-line override should parse");
        let mut override_output = Vec::new();
        run_with_args_with_config_overrides(
            ["config", "get", "conversation.model"],
            &[override_value],
            &mut override_output,
        )
        .expect("effective config get with override should succeed");
        let rendered_override =
            String::from_utf8(override_output).expect("override output should be UTF-8");
        assert!(rendered_override.contains("gpt-5.6-luna (command line)"));

        let mut unset_output = Vec::new();
        run_with_args(["config", "unset", "conversation.model"], &mut unset_output)
            .expect("global config unset should succeed");
        let contents =
            std::fs::read_to_string(&config_path).expect("config should remain readable");
        assert!(!contents.contains("conversation.model"));
        let _ = std::fs::remove_dir_all(home);
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
        assert!(rendered.contains("akra status --pr <number>"));
        assert!(!rendered.contains("akra init"));
    }

    #[test]
    fn status_pr_reads_bounded_durable_validation_projection_and_rejects_malformed_input() {
        let workspace = create_temp_workspace("cli-pr-validation-status");
        let key = PrValidationRecordKey::new("queue-pr-42").unwrap();
        let source_sha =
            PrValidationCommitSha::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let record = PrValidationRecord::register(
            key.clone(),
            PrValidationTarget::new("acme/widgets", 42).unwrap(),
            PrValidationTargetShaSnapshot::new(
                source_sha.clone(),
                PrValidationCommitSha::new("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
            ),
        )
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::FindingObserved(
            PrValidationFinding::new(
                PrValidationFindingKey::new(
                    PrValidationFindingSource::new("check_run").unwrap(),
                    "ghp_provider_secret_canary",
                )
                .unwrap(),
                source_sha,
                "Authorization: Bearer ghp_payload_secret_canary ".to_string()
                    + &"raw".repeat(10_000),
            )
            .unwrap(),
        ))
        .unwrap()
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                PrValidationFindingKey::new(
                    PrValidationFindingSource::new("check_run").unwrap(),
                    "ghp_provider_secret_canary",
                )
                .unwrap(),
                PrValidationRecordKey::new("remediation-task-42").unwrap(),
            ),
        ))
        .unwrap();
        assert!(
            SqlitePlanningAuthorityAdapter::compare_and_swap_runtime_pr_validation_record(
                &workspace,
                &key,
                None,
                Some(&record),
            )
            .unwrap()
        );

        let mut output = Vec::new();
        let exit = run_with_args(["status", "--pr", "42", workspace.as_str()], &mut output)
            .expect("PR status should render")
            .expect("PR status should exit");
        let rendered = String::from_utf8(output).unwrap();

        assert_eq!(exit, 0);
        assert!(rendered.contains("akra_id: queue-pr-42"), "{rendered}");
        assert!(
            rendered.contains("url: https://github.com/acme/widgets/pull/42"),
            "{rendered}"
        );
        assert!(rendered.contains("state: remediation"), "{rendered}");
        assert!(rendered.contains("phase: remediation_queued"));
        assert!(rendered.contains("reason: check_run finding"));
        assert!(rendered.contains("blocker: check_run finding"));
        assert!(rendered.contains("target: aaaaaaaaaaaa"));
        assert!(rendered.contains("findings: 1"));
        assert!(rendered.contains("correlation: check_run:"));
        assert!(rendered.contains("-> remediation-task-42"));
        assert!(rendered.contains("recovery: complete the correlated remediation task"));
        assert!(rendered.len() < 768);
        assert!(!rendered.contains("ghp_provider_secret_canary"));
        assert!(!rendered.contains("ghp_payload_secret_canary"));

        for malformed in ["0", "not-a-number", "-1"] {
            let error = dispatch_error(&["status", "--pr", malformed]);
            assert!(error.contains("invalid pull request number"), "{error}");
            assert!(error.contains("akra status --pr <number>"), "{error}");
        }

        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn operator_surface_shows_pr_remediation_merge_and_late_event() {
        let workspace = create_temp_workspace("cli-pr-validation-post-merge-status");
        let key = PrValidationRecordKey::new("queue-pr-42").unwrap();
        let source_sha =
            PrValidationCommitSha::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let merge_sha =
            PrValidationCommitSha::new("cccccccccccccccccccccccccccccccccccccccc").unwrap();
        let late_finding = PrValidationFinding::new(
            PrValidationFindingKey::new(
                PrValidationFindingSource::new("review").unwrap(),
                "late-review-42",
            )
            .unwrap(),
            source_sha.clone(),
            "post-merge late review requires remediation",
        )
        .unwrap();
        let record = PrValidationRecord::register(
            key.clone(),
            PrValidationTarget::new("acme/widgets", 42).unwrap(),
            PrValidationTargetShaSnapshot::new(
                source_sha,
                PrValidationCommitSha::new("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap(),
            ),
        )
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::MergeObserved(merge_sha))
        .unwrap()
        .transition(PrValidationEvent::BeginPostMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::ObservationCheckpointed {
            delivery_revision: 1,
            cursor: None,
            evidence_fingerprint: "merge-evidence".to_string(),
        })
        .unwrap()
        .transition(PrValidationEvent::FindingObserved(late_finding.clone()))
        .unwrap()
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                late_finding.key().clone(),
                PrValidationRecordKey::new("remediation-task-42").unwrap(),
            ),
        ))
        .unwrap();
        assert!(
            SqlitePlanningAuthorityAdapter::compare_and_swap_runtime_pr_validation_record(
                &workspace,
                &key,
                None,
                Some(&record),
            )
            .unwrap()
        );

        let mut output = Vec::new();
        let exit = run_with_args(["status", "--pr", "42", workspace.as_str()], &mut output)
            .unwrap()
            .unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert_eq!(exit, 0);
        assert!(rendered.contains("akra_id: queue-pr-42"), "{rendered}");
        assert!(
            rendered.contains("url: https://github.com/acme/widgets/pull/42"),
            "{rendered}"
        );
        assert!(rendered.contains("phase: remediation_queued"), "{rendered}");
        assert!(rendered.contains("merge: cccccccccccc"), "{rendered}");
        assert!(
            rendered.contains("integration: github_rebase_merge"),
            "{rendered}"
        );
        assert!(rendered.contains("evidence: cccccccccccc"), "{rendered}");
        assert!(
            rendered.contains("post_merge_checkpoint: observed"),
            "{rendered}"
        );
        assert!(
            rendered.contains("recovery: complete the correlated remediation task"),
            "{rendered}"
        );

        std::fs::remove_dir_all(workspace).unwrap();
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
