use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::adapter::outbound::app_server::{AppServerPlanningWorkerAdapter, CodexAppServerAdapter};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::service::planning::{
    PlanningDoctorReport, PlanningResetTarget, PlanningRuntimeSnapshot, PlanningServices,
    PlanningTrackedDirectionsApplyResult, PlanningWorkspaceInitResult,
    PlanningWorkspaceResetResult,
};

const ADMIN_SERVER_USAGE: &str = "Usage: akra admin [--port <port>]";
const ADMIN_SERVER_ALIAS_USAGE: &str = "Alias: akra admin-server [--port <port>]";
const DOCTOR_USAGE: &str = "Usage: akra doctor [workspace_dir]";
const QUEUE_USAGE: &str = "Usage: akra queue apply [workspace_dir]";
const DIRECTIONS_USAGE: &str = "Usage: akra directions apply [workspace_dir]";
const INIT_USAGE: &str = "Usage: akra init [workspace_dir]";
const RESET_USAGE: &str = "Usage: akra reset <queue|directions|all> [workspace_dir]";
const TELEGRAM_BOT_USAGE: &str = "Usage: akra telegram [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]";
const TELEGRAM_BOT_ALIAS_USAGE: &str = "Alias: akra telegram-bot [--token <token>] [--allow-chat-id <chat_id>]... [--poll-timeout-seconds <seconds>] [--keep-pending]";

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorReport {
    workspace_path: String,
    report: PlanningDoctorReport,
}

impl DoctorReport {
    fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            report: PlanningDoctorReport::path_issue(issue),
        }
    }

    fn from_service_report(workspace_path: String, report: PlanningDoctorReport) -> Self {
        Self {
            workspace_path,
            report,
        }
    }

    fn exit_code(&self) -> i32 {
        self.report.exit_code()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InitReport {
    workspace_path: String,
    mode: &'static str,
    created_file_count: Option<usize>,
    queue_idle_policy: Option<String>,
    status: Option<String>,
    issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResetReport {
    workspace_path: String,
    target: Option<&'static str>,
    rewritten_paths: Vec<String>,
    removed_paths: Vec<String>,
    status: Option<String>,
    issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectionsApplyReport {
    workspace_path: String,
    applied_paths: Vec<String>,
    validation_report: Option<crate::domain::planning::PlanningValidationReport>,
    status: Option<String>,
    issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueApplyReport {
    workspace_path: String,
    applied_paths: Vec<String>,
    validation_report: Option<crate::domain::planning::PlanningValidationReport>,
    status: Option<String>,
    issue: Option<String>,
}

impl InitReport {
    fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            mode: "simple",
            created_file_count: None,
            queue_idle_policy: None,
            status: None,
            issue: Some(issue),
        }
    }

    fn success(
        workspace_path: String,
        result: &PlanningWorkspaceInitResult,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Self {
        Self {
            workspace_path,
            mode: bootstrap_mode_label(result.mode),
            created_file_count: Some(result.created_file_count),
            queue_idle_policy: Some(snapshot.queue_idle_policy().label().to_string()),
            status: Some("planning workspace initialized".to_string()),
            issue: None,
        }
    }

    fn failure(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            mode: "simple",
            created_file_count: None,
            queue_idle_policy: None,
            status: None,
            issue: Some(issue),
        }
    }

    fn exit_code(&self) -> i32 {
        if self.issue.is_some() { 1 } else { 0 }
    }
}

impl ResetReport {
    fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            target: None,
            rewritten_paths: Vec::new(),
            removed_paths: Vec::new(),
            status: None,
            issue: Some(issue),
        }
    }

    fn success(workspace_path: String, result: &PlanningWorkspaceResetResult) -> Self {
        Self {
            workspace_path,
            target: Some(result.target.label()),
            rewritten_paths: result.rewritten_paths.clone(),
            removed_paths: result.removed_paths.clone(),
            status: Some("planning workspace reset".to_string()),
            issue: None,
        }
    }

    fn failure(workspace_path: String, target: PlanningResetTarget, issue: String) -> Self {
        Self {
            workspace_path,
            target: Some(target.label()),
            rewritten_paths: Vec::new(),
            removed_paths: Vec::new(),
            status: None,
            issue: Some(issue),
        }
    }

    fn exit_code(&self) -> i32 {
        if self.issue.is_some() { 1 } else { 0 }
    }
}

impl DirectionsApplyReport {
    fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            applied_paths: Vec::new(),
            validation_report: None,
            status: None,
            issue: Some(issue),
        }
    }

    fn success(workspace_path: String, result: PlanningTrackedDirectionsApplyResult) -> Self {
        let status = if result.applied() {
            Some(format!(
                "tracked directions applied to active planning ({} files)",
                result.applied_paths.len()
            ))
        } else {
            Some("tracked directions apply blocked by validation".to_string())
        };
        let issue = if result.validation_report.is_valid() {
            None
        } else {
            Some(first_validation_issue_message(&result.validation_report))
        };
        Self {
            workspace_path,
            applied_paths: result.applied_paths,
            validation_report: Some(result.validation_report),
            status,
            issue,
        }
    }

    fn failure(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            applied_paths: Vec::new(),
            validation_report: None,
            status: None,
            issue: Some(issue),
        }
    }

    fn exit_code(&self) -> i32 {
        let blocked_by_validation = self
            .validation_report
            .as_ref()
            .is_some_and(|report| !report.is_valid());
        if self.issue.is_some() || blocked_by_validation {
            1
        } else {
            0
        }
    }
}

impl QueueApplyReport {
    fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            applied_paths: Vec::new(),
            validation_report: None,
            status: None,
            issue: Some(issue),
        }
    }

    fn failure(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            applied_paths: Vec::new(),
            validation_report: None,
            status: None,
            issue: Some(issue),
        }
    }

    fn exit_code(&self) -> i32 {
        let blocked_by_validation = self
            .validation_report
            .as_ref()
            .is_some_and(|report| !report.is_valid());
        if self.issue.is_some() || blocked_by_validation {
            1
        } else {
            0
        }
    }
}

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
            writeln!(stdout, "{QUEUE_USAGE}")?;
            writeln!(stdout, "{DIRECTIONS_USAGE}")?;
            writeln!(stdout, "{INIT_USAGE}")?;
            writeln!(stdout, "{RESET_USAGE}")?;
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
        [command] if command == OsStr::new("queue") => {
            bail!("{QUEUE_USAGE}");
        }
        [command, action] if command == OsStr::new("queue") => {
            Ok(Some(run_queue(action.as_os_str(), None, stdout)?))
        }
        [command, action, workspace] if command == OsStr::new("queue") => Ok(Some(run_queue(
            action.as_os_str(),
            Some(workspace.as_os_str()),
            stdout,
        )?)),
        [command] if command == OsStr::new("directions") => {
            bail!("{DIRECTIONS_USAGE}");
        }
        [command, action] if command == OsStr::new("directions") => {
            Ok(Some(run_directions(action.as_os_str(), None, stdout)?))
        }
        [command, action, workspace] if command == OsStr::new("directions") => Ok(Some(
            run_directions(action.as_os_str(), Some(workspace.as_os_str()), stdout)?,
        )),
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
        [command, _, ..] if command == OsStr::new("doctor") => {
            bail!("{DOCTOR_USAGE}");
        }
        [command, _, _, ..] if command == OsStr::new("directions") => {
            bail!("{DIRECTIONS_USAGE}");
        }
        [command, _, ..] if command == OsStr::new("init") => {
            bail!("{INIT_USAGE}");
        }
        [command, _, _, ..] if command == OsStr::new("reset") => {
            bail!("{RESET_USAGE}");
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

fn run_directions(
    action_arg: &OsStr,
    workspace_arg: Option<&OsStr>,
    stdout: &mut impl Write,
) -> Result<i32> {
    match action_arg
        .to_string_lossy()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "apply" => {
            let workspace_path = resolve_workspace_path(workspace_arg)?;
            let report = apply_tracked_directions(&workspace_path);
            render_directions_apply_report(stdout, &report)?;
            Ok(report.exit_code())
        }
        _ => bail!("{DIRECTIONS_USAGE}"),
    }
}

fn run_queue(
    action_arg: &OsStr,
    workspace_arg: Option<&OsStr>,
    stdout: &mut impl Write,
) -> Result<i32> {
    match action_arg
        .to_string_lossy()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "apply" => {
            let workspace_path = resolve_workspace_path(workspace_arg)?;
            let report = apply_tracked_task_authority(&workspace_path);
            render_queue_apply_report(stdout, &report)?;
            Ok(report.exit_code())
        }
        _ => bail!("{QUEUE_USAGE}"),
    }
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
        Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
        planning_authority.clone(),
        planning_authority,
        Arc::new(AppServerPlanningWorkerAdapter::new(app_server_adapter)),
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

fn apply_tracked_directions(workspace_path: &Path) -> DirectionsApplyReport {
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(workspace_path) {
        return DirectionsApplyReport::path_issue(workspace_label, issue);
    }

    let planning = build_production_planning_services();
    match planning
        .workspace
        .apply_tracked_directions(workspace_path.to_string_lossy().as_ref())
    {
        Ok(result) => DirectionsApplyReport::success(workspace_label, result),
        Err(error) => DirectionsApplyReport::failure(workspace_label, error.to_string()),
    }
}

fn apply_tracked_task_authority(workspace_path: &Path) -> QueueApplyReport {
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(workspace_path) {
        return QueueApplyReport::path_issue(workspace_label, issue);
    }

    QueueApplyReport::failure(
        workspace_label,
        "tracked task authority import was removed; use runtime task intake or admin task management".to_string(),
    )
}

fn render_doctor_report(stdout: &mut impl Write, report: &DoctorReport) -> Result<()> {
    writeln!(stdout, "workspace: {}", report.workspace_path)?;
    writeln!(
        stdout,
        "planning state: {}",
        report.report.planning_state().label()
    )?;

    if let Some(queue_idle_policy) = report.report.queue_idle_policy() {
        writeln!(stdout, "queue-idle policy: {queue_idle_policy}")?;
    }
    if let Some(queue_summary) = report.report.queue_summary() {
        writeln!(stdout, "queue summary: {queue_summary}")?;
    }
    if let Some(proposal_summary) = report.report.proposal_summary() {
        writeln!(stdout, "proposal summary: {proposal_summary}")?;
    }
    if let Some(health) = report.report.health() {
        writeln!(stdout, "health: {health}")?;
    }
    if let Some(issue) = report.report.issue() {
        writeln!(stdout, "issue: {issue}")?;
    }
    if let Some(note) = report.report.note() {
        writeln!(stdout, "note: {note}")?;
    }

    Ok(())
}

fn render_directions_apply_report(
    stdout: &mut impl Write,
    report: &DirectionsApplyReport,
) -> Result<()> {
    writeln!(stdout, "workspace: {}", report.workspace_path)?;
    writeln!(stdout, "command: directions apply")?;
    writeln!(stdout, "source: DB direction authority")?;

    for applied_path in &report.applied_paths {
        writeln!(stdout, "applied: {applied_path}")?;
    }
    if let Some(validation_report) = &report.validation_report {
        writeln!(
            stdout,
            "validation: {}",
            validation_label(validation_report)
        )?;
        for issue in validation_report.errors() {
            writeln!(stdout, "validation issue: {}", issue.message)?;
        }
    }
    if let Some(status) = &report.status {
        writeln!(stdout, "status: {status}")?;
    }
    if let Some(issue) = &report.issue {
        writeln!(stdout, "issue: {issue}")?;
    }

    Ok(())
}

fn render_queue_apply_report(stdout: &mut impl Write, report: &QueueApplyReport) -> Result<()> {
    writeln!(stdout, "workspace: {}", report.workspace_path)?;
    writeln!(stdout, "command: queue apply")?;
    writeln!(stdout, "source: DB task authority")?;

    for applied_path in &report.applied_paths {
        writeln!(stdout, "applied: {applied_path}")?;
    }
    if let Some(validation_report) = &report.validation_report {
        writeln!(
            stdout,
            "validation: {}",
            validation_label(validation_report)
        )?;
        for issue in validation_report.errors() {
            writeln!(stdout, "validation issue: {}", issue.message)?;
        }
    }
    if let Some(status) = &report.status {
        writeln!(stdout, "status: {status}")?;
    }
    if let Some(issue) = &report.issue {
        writeln!(stdout, "issue: {issue}")?;
    }

    Ok(())
}

fn render_init_report(stdout: &mut impl Write, report: &InitReport) -> Result<()> {
    writeln!(stdout, "workspace: {}", report.workspace_path)?;
    writeln!(stdout, "command: init")?;
    writeln!(stdout, "mode: {}", report.mode)?;

    if let Some(created_file_count) = report.created_file_count {
        writeln!(stdout, "created files: {created_file_count}")?;
    }
    if let Some(queue_idle_policy) = &report.queue_idle_policy {
        writeln!(stdout, "queue-idle policy: {queue_idle_policy}")?;
    }
    if let Some(status) = &report.status {
        writeln!(stdout, "status: {status}")?;
    }
    if let Some(issue) = &report.issue {
        writeln!(stdout, "issue: {issue}")?;
    }

    Ok(())
}

fn render_reset_report(stdout: &mut impl Write, report: &ResetReport) -> Result<()> {
    writeln!(stdout, "workspace: {}", report.workspace_path)?;
    writeln!(stdout, "command: reset")?;
    if let Some(target) = report.target {
        writeln!(stdout, "target: {target}")?;
    }
    for rewritten_path in &report.rewritten_paths {
        writeln!(stdout, "rewritten: {rewritten_path}")?;
    }
    for removed_path in &report.removed_paths {
        writeln!(stdout, "removed: {removed_path}")?;
    }
    if let Some(status) = &report.status {
        writeln!(stdout, "status: {status}")?;
    }
    if let Some(issue) = &report.issue {
        writeln!(stdout, "issue: {issue}")?;
    }

    Ok(())
}

fn bootstrap_mode_label(
    mode: crate::application::service::planning::PlanningBootstrapMode,
) -> &'static str {
    match mode {
        crate::application::service::planning::PlanningBootstrapMode::Detail => "detail",
        crate::application::service::planning::PlanningBootstrapMode::Simple => "simple",
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

fn validation_label(report: &crate::domain::planning::PlanningValidationReport) -> &'static str {
    if report.is_valid() {
        "valid"
    } else {
        "invalid"
    }
}

fn first_validation_issue_message(
    report: &crate::domain::planning::PlanningValidationReport,
) -> String {
    report
        .errors()
        .first()
        .map(|issue| issue.message.clone())
        .unwrap_or_else(|| "planning validation failed".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_compiles_after_task_authority_file_removal() {
        assert!(std::env::current_dir().is_ok());
    }
}
