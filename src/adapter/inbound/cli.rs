use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus, PlanningServices,
};
use crate::application::service::planning_contract::PLAN_OFF_FILE_PATH;

const DOCTOR_USAGE: &str = "Usage: akra doctor [workspace_dir]";
const INCOMPLETE_PREFIX: &str = "planning files incomplete:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanningDoctorState {
    Absent,
    Incomplete,
    Invalid,
    ReadyWithoutTask,
    ReadyWithTask,
}

impl PlanningDoctorState {
    fn label(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Incomplete => "incomplete",
            Self::Invalid => "invalid",
            Self::ReadyWithoutTask => "ready_without_task",
            Self::ReadyWithTask => "ready_with_task",
        }
    }

    fn exit_code(self) -> i32 {
        match self {
            Self::Absent | Self::ReadyWithoutTask | Self::ReadyWithTask => 0,
            Self::Incomplete | Self::Invalid => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorReport {
    workspace_path: String,
    planning_state: PlanningDoctorState,
    queue_idle_policy: Option<String>,
    queue_summary: Option<String>,
    proposal_summary: Option<String>,
    health: Option<String>,
    issue: Option<String>,
    note: Option<String>,
}

impl DoctorReport {
    fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            planning_state: PlanningDoctorState::Invalid,
            queue_idle_policy: None,
            queue_summary: None,
            proposal_summary: None,
            health: None,
            issue: Some(issue),
            note: None,
        }
    }

    fn from_snapshot(workspace_path: String, snapshot: &PlanningRuntimeSnapshot) -> Self {
        let planning_state = classify_doctor_state(snapshot);
        let is_ready = matches!(
            planning_state,
            PlanningDoctorState::ReadyWithoutTask | PlanningDoctorState::ReadyWithTask
        );
        let note = if snapshot.workspace_present() && !snapshot.plan_enabled() {
            Some(format!(
                "queue-driven continuation is disabled by {PLAN_OFF_FILE_PATH}"
            ))
        } else {
            None
        };
        let health = match planning_state {
            PlanningDoctorState::Absent => {
                Some("planning workspace is not initialized".to_string())
            }
            PlanningDoctorState::ReadyWithoutTask | PlanningDoctorState::ReadyWithTask => {
                Some("planning workspace is healthy".to_string())
            }
            PlanningDoctorState::Incomplete | PlanningDoctorState::Invalid => None,
        };

        Self {
            workspace_path,
            planning_state,
            queue_idle_policy: is_ready.then(|| snapshot.queue_idle_policy().label().to_string()),
            queue_summary: is_ready
                .then(|| snapshot.queue_summary().map(str::to_string))
                .flatten(),
            proposal_summary: is_ready
                .then(|| snapshot.proposal_summary().map(str::to_string))
                .flatten(),
            health,
            issue: matches!(
                planning_state,
                PlanningDoctorState::Incomplete | PlanningDoctorState::Invalid
            )
            .then(|| snapshot.failure_reason().map(str::to_string))
            .flatten(),
            note,
        }
    }

    fn exit_code(&self) -> i32 {
        self.planning_state.exit_code()
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
            writeln!(stdout, "{DOCTOR_USAGE}")?;
            Ok(Some(0))
        }
        [command] if command == OsStr::new("doctor") => Ok(Some(run_doctor(None, stdout)?)),
        [command, workspace] if command == OsStr::new("doctor") => {
            Ok(Some(run_doctor(Some(workspace.as_os_str()), stdout)?))
        }
        [command, _, ..] if command == OsStr::new("doctor") => {
            bail!("{DOCTOR_USAGE}");
        }
        [command, ..] => {
            bail!("unsupported command: {}", command.to_string_lossy());
        }
    }
}

fn is_help_flag(flag: &OsStr) -> bool {
    matches!(flag.to_str(), Some("-h" | "--help"))
}

fn run_doctor(workspace_arg: Option<&OsStr>, stdout: &mut impl Write) -> Result<i32> {
    let workspace_path = resolve_workspace_path(workspace_arg)?;
    let report = inspect_workspace(&workspace_path);
    render_doctor_report(stdout, &report)?;
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

fn inspect_workspace(workspace_path: &Path) -> DoctorReport {
    let workspace_label = workspace_path.display().to_string();
    if !workspace_path.exists() {
        return DoctorReport::path_issue(
            workspace_label,
            format!(
                "workspace path does not exist: {}",
                workspace_path.display()
            ),
        );
    }
    if !workspace_path.is_dir() {
        return DoctorReport::path_issue(
            workspace_label,
            format!(
                "workspace path is not a directory: {}",
                workspace_path.display()
            ),
        );
    }

    let planning =
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
    let snapshot = planning
        .runtime
        .load_runtime_snapshot_or_invalid(workspace_path.to_string_lossy().as_ref());
    DoctorReport::from_snapshot(workspace_label, &snapshot)
}

fn classify_doctor_state(snapshot: &PlanningRuntimeSnapshot) -> PlanningDoctorState {
    match snapshot.workspace_status() {
        PlanningRuntimeWorkspaceStatus::Uninitialized => PlanningDoctorState::Absent,
        PlanningRuntimeWorkspaceStatus::Invalid => {
            if snapshot
                .failure_reason()
                .is_some_and(|reason| reason.starts_with(INCOMPLETE_PREFIX))
            {
                PlanningDoctorState::Incomplete
            } else {
                PlanningDoctorState::Invalid
            }
        }
        PlanningRuntimeWorkspaceStatus::ReadyNoTask => PlanningDoctorState::ReadyWithoutTask,
        PlanningRuntimeWorkspaceStatus::ReadyWithTask => PlanningDoctorState::ReadyWithTask,
    }
}

fn render_doctor_report(stdout: &mut impl Write, report: &DoctorReport) -> Result<()> {
    writeln!(stdout, "workspace: {}", report.workspace_path)?;
    writeln!(stdout, "planning state: {}", report.planning_state.label())?;

    if let Some(queue_idle_policy) = &report.queue_idle_policy {
        writeln!(stdout, "queue-idle policy: {queue_idle_policy}")?;
    }
    if let Some(queue_summary) = &report.queue_summary {
        writeln!(stdout, "queue summary: {queue_summary}")?;
    }
    if let Some(proposal_summary) = &report.proposal_summary {
        writeln!(stdout, "proposal summary: {proposal_summary}")?;
    }
    if let Some(health) = &report.health {
        writeln!(stdout, "health: {health}")?;
    }
    if let Some(issue) = &report.issue {
        writeln!(stdout, "issue: {issue}")?;
    }
    if let Some(note) = &report.note {
        writeln!(stdout, "note: {note}")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::{Context, Result};

    use super::run_with_args;
    use crate::application::service::planning_bootstrap_service::{
        PlanningBootstrapArtifacts, PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning_contract::{
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
