use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::service::planning::{
    PlanningDoctorReport, PlanningResetTarget, PlanningRuntimeSnapshot, PlanningServices,
    PlanningTrackedDirectionsApplyResult, PlanningWorkspaceInitResult,
    PlanningWorkspaceResetResult,
};

const ADMIN_SERVER_USAGE: &str = "Usage: akra admin [--port <port>]";
const ADMIN_SERVER_ALIAS_USAGE: &str = "Alias: akra admin-server [--port <port>]";
const DOCTOR_USAGE: &str = "Usage: akra doctor [workspace_dir]";
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

fn inspect_workspace(workspace_path: &Path) -> DoctorReport {
    let workspace_label = workspace_path.display().to_string();
    if let Err(issue) = validate_workspace_path(workspace_path) {
        return DoctorReport::path_issue(workspace_label, issue);
    }

    let planning =
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
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

    let planning =
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
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

    let planning =
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
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

    let planning =
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    match planning
        .workspace
        .apply_tracked_directions(workspace_path.to_string_lossy().as_ref())
    {
        Ok(result) => DirectionsApplyReport::success(workspace_label, result),
        Err(error) => DirectionsApplyReport::failure(workspace_label, error.to_string()),
    }
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
    writeln!(stdout, "source: .codex-exec-loop/planning/directions.toml")?;

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
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::{Context, Result};

    use super::run_with_args;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning::PlanningServices;
    use crate::application::service::planning::authoring::bootstrap::{
        PlanningBootstrapArtifacts, PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning::shared::contract::{
        DIRECTIONS_FILE_PATH, PLAN_OFF_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
        TASK_LEDGER_SCHEMA_FILE_PATH,
    };

    struct TestWorkspace {
        path: PathBuf,
    }

    impl TestWorkspace {
        fn new(label: &str) -> Result<Self> {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be after the unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "codex-exec-loop-{label}-{}-{unique_suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&path)
                .with_context(|| format!("failed to create {}", path.display()))?;
            Ok(Self { path })
        }

        fn write_file(&self, relative_path: &str, body: &str) -> Result<()> {
            let path = self.path.join(relative_path);
            let parent = path
                .parent()
                .context("planning workspace file should have a parent directory")?;
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
            fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))
        }

        fn install_artifacts(&self, artifacts: &PlanningBootstrapArtifacts) -> Result<()> {
            self.write_file(&artifacts.directions_path, &artifacts.directions_toml)?;
            self.write_file(&artifacts.task_ledger_path, &artifacts.task_ledger_json)?;
            self.write_file(
                &artifacts.task_ledger_schema_path,
                &artifacts.task_ledger_schema_json,
            )?;
            self.write_file(
                &artifacts.result_output_path,
                &artifacts.result_output_markdown,
            )?;
            for supplemental_file in &artifacts.supplemental_files {
                self.write_file(&supplemental_file.active_path, &supplemental_file.body)?;
            }
            Ok(())
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn doctor_args(workspace: &TestWorkspace) -> Vec<OsString> {
        vec![
            OsString::from("doctor"),
            workspace.path.as_os_str().to_os_string(),
        ]
    }

    fn init_args(workspace: &TestWorkspace) -> Vec<OsString> {
        vec![
            OsString::from("init"),
            workspace.path.as_os_str().to_os_string(),
        ]
    }

    fn directions_apply_args(workspace: &TestWorkspace) -> Vec<OsString> {
        vec![
            OsString::from("directions"),
            OsString::from("apply"),
            workspace.path.as_os_str().to_os_string(),
        ]
    }

    fn reset_args(target: &str, workspace: &TestWorkspace) -> Vec<OsString> {
        vec![
            OsString::from("reset"),
            OsString::from(target),
            workspace.path.as_os_str().to_os_string(),
        ]
    }

    #[test]
    fn doctor_reports_absent_workspace_as_healthy() {
        let workspace =
            TestWorkspace::new("doctor-absent").expect("test workspace should be created");
        let mut stdout = Vec::new();

        let exit_code = run_with_args(doctor_args(&workspace), &mut stdout)
            .expect("doctor should run")
            .expect("doctor should produce an exit code");
        let output = String::from_utf8(stdout).expect("doctor output should be valid utf-8");

        assert_eq!(exit_code, 0);
        assert!(output.contains("planning state: absent"));
        assert!(output.contains("health: planning workspace is not initialized"));
    }

    #[test]
    fn help_output_mentions_admin_server_command() {
        let mut stdout = Vec::new();

        let exit_code = run_with_args([OsString::from("--help")], &mut stdout)
            .expect("help should run")
            .expect("help should produce an exit code");
        let output = String::from_utf8(stdout).expect("help output should be valid utf-8");

        assert_eq!(exit_code, 0);
        assert!(output.contains("Usage: akra admin [--port <port>]"));
        assert!(output.contains("Alias: akra admin-server [--port <port>]"));
        assert!(output.contains("Usage: akra directions apply [workspace_dir]"));
    }

    #[test]
    fn help_output_mentions_telegram_bot_command() {
        let mut stdout = Vec::new();

        let exit_code = run_with_args([OsString::from("--help")], &mut stdout)
            .expect("help should run")
            .expect("help should produce an exit code");
        let output = String::from_utf8(stdout).expect("help output should be valid utf-8");

        assert_eq!(exit_code, 0);
        assert!(output.contains("Usage: akra telegram"));
        assert!(output.contains("Alias: akra telegram-bot"));
    }

    #[test]
    fn init_creates_simple_planning_scaffold_directly() {
        let workspace =
            TestWorkspace::new("init-simple").expect("test workspace should be created");
        let mut stdout = Vec::new();

        let exit_code = run_with_args(init_args(&workspace), &mut stdout)
            .expect("init should run")
            .expect("init should produce an exit code");
        let output = String::from_utf8(stdout).expect("init output should be valid utf-8");

        assert_eq!(exit_code, 0);
        assert!(output.contains("command: init"));
        assert!(output.contains("mode: simple"));
        assert!(output.contains("created files: 5"));
        assert!(output.contains("queue-idle policy: review_and_enqueue"));
        assert!(output.contains("status: planning workspace initialized"));
        assert!(
            Path::new(&workspace.path)
                .join(DIRECTIONS_FILE_PATH)
                .is_file()
        );
        assert!(
            Path::new(&workspace.path)
                .join(TASK_LEDGER_FILE_PATH)
                .is_file()
        );
        assert!(
            Path::new(&workspace.path)
                .join(TASK_LEDGER_SCHEMA_FILE_PATH)
                .is_file()
        );
        assert!(
            Path::new(&workspace.path)
                .join(RESULT_OUTPUT_FILE_PATH)
                .is_file()
        );
        assert!(
            Path::new(&workspace.path)
                .join(".codex-exec-loop/planning/prompts/queue-idle-review.md")
                .is_file()
        );
    }

    #[test]
    fn init_refuses_to_overwrite_existing_planning_workspace() {
        let workspace =
            TestWorkspace::new("init-existing").expect("test workspace should be created");
        workspace
            .write_file(DIRECTIONS_FILE_PATH, "version = 1\n")
            .expect("existing directions should be writable");
        let before = fs::read_to_string(Path::new(&workspace.path).join(DIRECTIONS_FILE_PATH))
            .expect("existing directions should be readable");
        let mut stdout = Vec::new();

        let exit_code = run_with_args(init_args(&workspace), &mut stdout)
            .expect("init should run")
            .expect("init should produce an exit code");
        let output = String::from_utf8(stdout).expect("init output should be valid utf-8");
        let after = fs::read_to_string(Path::new(&workspace.path).join(DIRECTIONS_FILE_PATH))
            .expect("existing directions should remain readable");

        assert_eq!(exit_code, 1);
        assert!(output.contains("command: init"));
        assert!(output.contains("issue: planning workspace already exists"));
        assert_eq!(before, after);
        assert!(
            !Path::new(&workspace.path)
                .join(TASK_LEDGER_FILE_PATH)
                .exists()
        );
    }

    #[test]
    fn directions_apply_reports_success_with_applied_paths() {
        let workspace =
            TestWorkspace::new("directions-apply-success").expect("test workspace should exist");
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        planning
            .workspace
            .initialize_simple_workspace(workspace.path.to_string_lossy().as_ref())
            .expect("planning workspace should initialize");
        workspace
            .write_file(
                DIRECTIONS_FILE_PATH,
                "version = 1\n\n[queue_idle]\npolicy = \"review_and_enqueue\"\nprompt_path = \".codex-exec-loop/planning/prompts/queue-idle-review.md\"\n\n[[directions]]\nid = \"general-workstream\"\ntitle = \"General workstream\"\nsummary = \"summary\"\nsuccess_criteria = [\"one\"]\nscope_hints = [\"two\"]\ndetail_doc_path = \"\"\nstate = \"active\"\n",
            )
            .expect("candidate directions should write");
        let mut stdout = Vec::new();

        let exit_code = run_with_args(directions_apply_args(&workspace), &mut stdout)
            .expect("directions apply should run")
            .expect("directions apply should produce an exit code");
        let output =
            String::from_utf8(stdout).expect("directions apply output should be valid utf-8");

        assert_eq!(exit_code, 0);
        assert!(output.contains("command: directions apply"));
        assert!(output.contains("source: .codex-exec-loop/planning/directions.toml"));
        assert!(output.contains(&format!("applied: {DIRECTIONS_FILE_PATH}")));
        assert!(output.contains("validation: valid"));
        assert!(output.contains("status: tracked directions applied to active planning"));
    }

    #[test]
    fn directions_apply_reports_validation_failure_reason() {
        let workspace =
            TestWorkspace::new("directions-apply-invalid").expect("test workspace should exist");
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        planning
            .workspace
            .initialize_simple_workspace(workspace.path.to_string_lossy().as_ref())
            .expect("planning workspace should initialize");
        workspace
            .write_file(
                DIRECTIONS_FILE_PATH,
                "version = 1\n\n[queue_idle]\npolicy = \"review_and_enqueue\"\nprompt_path = \".codex-exec-loop/planning/prompts/missing.md\"\n\n[[directions]]\nid = \"general-workstream\"\ntitle = \"General workstream\"\nsummary = \"summary\"\nsuccess_criteria = [\"one\"]\nscope_hints = [\"two\"]\ndetail_doc_path = \"\"\nstate = \"active\"\n",
            )
            .expect("invalid candidate directions should write");
        let mut stdout = Vec::new();

        let exit_code = run_with_args(directions_apply_args(&workspace), &mut stdout)
            .expect("directions apply should run")
            .expect("directions apply should produce an exit code");
        let output =
            String::from_utf8(stdout).expect("directions apply output should be valid utf-8");

        assert_eq!(exit_code, 1);
        assert!(output.contains("command: directions apply"));
        assert!(output.contains("validation: invalid"));
        assert!(output.contains("validation issue: queue_idle.prompt_path does not exist"));
        assert!(output.contains("status: tracked directions apply blocked by validation"));
        assert!(output.contains("issue: queue_idle.prompt_path does not exist"));
    }

    #[test]
    fn reset_queue_rewrites_task_ledger_and_clears_queue_side_runtime_state() {
        let workspace =
            TestWorkspace::new("reset-queue").expect("test workspace should be created");
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        planning
            .workspace
            .initialize_simple_workspace(workspace.path.to_string_lossy().as_ref())
            .expect("planning workspace should initialize");
        workspace
            .write_file(
                TASK_LEDGER_FILE_PATH,
                r#"{"version":1,"tasks":[{"id":"task-1","direction_id":"general-workstream","direction_relation_note":"keep working","title":"Do work","description":"reset the queue","status":"ready","base_priority":10,"created_by":"user","last_updated_by":"user","updated_at":"2026-04-16T00:00:00Z"}]}"#,
            )
            .expect("task ledger should write");
        workspace
            .write_file(
                ".codex-exec-loop/planning/queue.snapshot.json",
                "{\"next_task\":null}",
            )
            .expect("queue snapshot should write");
        let mut stdout = Vec::new();

        let exit_code = run_with_args(reset_args("queue", &workspace), &mut stdout)
            .expect("reset should run")
            .expect("reset should produce an exit code");
        let output = String::from_utf8(stdout).expect("reset output should be valid utf-8");

        assert_eq!(exit_code, 0);
        assert!(output.contains("command: reset"));
        assert!(output.contains("target: queue"));
        assert!(output.contains(&format!("rewritten: {TASK_LEDGER_FILE_PATH}")));
        assert!(output.contains("removed: .codex-exec-loop/planning/queue.snapshot.json"));
        assert!(output.contains("status: planning workspace reset"));
        assert_eq!(
            fs::read_to_string(Path::new(&workspace.path).join(TASK_LEDGER_FILE_PATH))
                .expect("task ledger should be readable after reset"),
            "{\n  \"version\": 1,\n  \"tasks\": []\n}"
        );
        assert!(
            !Path::new(&workspace.path)
                .join(".codex-exec-loop/planning/queue.snapshot.json")
                .exists()
        );
    }

    #[test]
    fn reset_directions_refuses_when_live_tasks_exist() {
        let workspace = TestWorkspace::new("reset-directions-blocked")
            .expect("test workspace should be created");
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        planning
            .workspace
            .initialize_simple_workspace(workspace.path.to_string_lossy().as_ref())
            .expect("planning workspace should initialize");
        workspace
            .write_file(
                TASK_LEDGER_FILE_PATH,
                r#"{"version":1,"tasks":[{"id":"task-1","direction_id":"general-workstream","direction_relation_note":"keep working","title":"Do work","description":"reset directions","status":"ready","base_priority":10,"created_by":"user","last_updated_by":"user","updated_at":"2026-04-16T00:00:00Z"}]}"#,
            )
            .expect("task ledger should write");
        let mut stdout = Vec::new();

        let exit_code = run_with_args(reset_args("directions", &workspace), &mut stdout)
            .expect("reset should run")
            .expect("reset should produce an exit code");
        let output = String::from_utf8(stdout).expect("reset output should be valid utf-8");

        assert_eq!(exit_code, 1);
        assert!(output.contains("command: reset"));
        assert!(output.contains("target: directions"));
        assert!(output.contains("issue: planning directions reset is blocked by live tasks"));
    }

    #[test]
    fn doctor_reports_incomplete_workspace_and_blocks_exit_code() {
        let workspace =
            TestWorkspace::new("doctor-incomplete").expect("test workspace should be created");
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        workspace
            .write_file(DIRECTIONS_FILE_PATH, &artifacts.directions_toml)
            .expect("directions.toml should be writable");
        workspace
            .write_file(TASK_LEDGER_FILE_PATH, &artifacts.task_ledger_json)
            .expect("task-ledger.json should be writable");
        workspace
            .write_file(RESULT_OUTPUT_FILE_PATH, &artifacts.result_output_markdown)
            .expect("result-output.md should be writable");

        let mut stdout = Vec::new();
        let exit_code = run_with_args(doctor_args(&workspace), &mut stdout)
            .expect("doctor should run")
            .expect("doctor should produce an exit code");
        let output = String::from_utf8(stdout).expect("doctor output should be valid utf-8");

        assert_eq!(exit_code, 1);
        assert!(output.contains("planning state: incomplete"));
        assert!(output.contains("issue: planning files incomplete: missing"));
        assert!(output.contains(TASK_LEDGER_SCHEMA_FILE_PATH));
    }

    #[test]
    fn doctor_reports_ready_without_task_with_proposal_summary() {
        let workspace =
            TestWorkspace::new("doctor-proposal").expect("test workspace should be created");
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        workspace
            .install_artifacts(&PlanningBootstrapArtifacts {
                task_ledger_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-followup-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "The latest answer suggested a concrete next step.",
      "title": "Draft follow-up checklist",
      "description": "Persist the follow-up as a proposal for review.",
      "status": "proposed",
      "base_priority": 30,
      "dynamic_priority_delta": 0,
      "priority_reason": "Suggested follow-up option from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-16T09:00:00Z"
    }
  ]
}"#
                .to_string(),
                ..artifacts
            })
            .expect("planning artifacts should be written");

        let mut stdout = Vec::new();
        let exit_code = run_with_args(doctor_args(&workspace), &mut stdout)
            .expect("doctor should run")
            .expect("doctor should produce an exit code");
        let output = String::from_utf8(stdout).expect("doctor output should be valid utf-8");

        assert_eq!(exit_code, 0);
        assert!(output.contains("planning state: ready_without_task"));
        assert!(output.contains("queue-idle policy: review_and_enqueue"));
        assert!(output.contains("proposal summary: Draft follow-up checklist"));
        assert!(output.contains("health: planning workspace is healthy"));
    }

    #[test]
    fn doctor_reports_ready_with_task_and_plan_off_note() {
        let workspace =
            TestWorkspace::new("doctor-ready").expect("test workspace should be created");
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        workspace
            .install_artifacts(&PlanningBootstrapArtifacts {
                task_ledger_json: r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "title": "Implement doctor command",
      "description": "Add the external planning doctor command.",
      "status": "ready",
      "base_priority": 12,
      "dynamic_priority_delta": 0,
      "priority_reason": "",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-16T10:00:00Z"
    }
  ]
}"#
                .to_string(),
                ..artifacts
            })
            .expect("planning artifacts should be written");
        workspace
            .write_file(PLAN_OFF_FILE_PATH, "plan off\n")
            .expect("plan.off should be writable");

        let mut stdout = Vec::new();
        let exit_code = run_with_args(doctor_args(&workspace), &mut stdout)
            .expect("doctor should run")
            .expect("doctor should produce an exit code");
        let output = String::from_utf8(stdout).expect("doctor output should be valid utf-8");

        assert_eq!(exit_code, 0);
        assert!(output.contains("planning state: ready_with_task"));
        assert!(output.contains("queue-idle policy: stop"));
        assert!(output.contains("queue summary: now: Implement doctor command"));
        assert!(output.contains("note: queue-driven continuation is disabled by"));
        assert!(output.contains(Path::new(PLAN_OFF_FILE_PATH).display().to_string().as_str()));
    }

    #[test]
    fn doctor_reports_invalid_workspace_validation_failure() {
        let workspace =
            TestWorkspace::new("doctor-invalid").expect("test workspace should be created");
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        workspace
            .write_file(DIRECTIONS_FILE_PATH, &artifacts.directions_toml)
            .expect("directions.toml should be writable");
        workspace
            .write_file(
                TASK_LEDGER_FILE_PATH,
                r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1"
    }
  ]
}"#,
            )
            .expect("task-ledger.json should be writable");
        workspace
            .write_file(
                TASK_LEDGER_SCHEMA_FILE_PATH,
                &artifacts.task_ledger_schema_json,
            )
            .expect("task-ledger.schema.json should be writable");
        workspace
            .write_file(RESULT_OUTPUT_FILE_PATH, &artifacts.result_output_markdown)
            .expect("result-output.md should be writable");

        let mut stdout = Vec::new();
        let exit_code = run_with_args(doctor_args(&workspace), &mut stdout)
            .expect("doctor should run")
            .expect("doctor should produce an exit code");
        let output = String::from_utf8(stdout).expect("doctor output should be valid utf-8");

        assert_eq!(exit_code, 1);
        assert!(output.contains("planning state: invalid"));
        assert!(output.contains("issue: planning validation failed"));
    }
}
