use std::io::Write;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::application::service::planning::{
    PlanningBootstrapMode, PlanningDoctorReport, PlanningResetTarget, PlanningRuntimeSnapshot,
    PlanningWorkspaceInitResult, PlanningWorkspaceResetResult,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DoctorReport {
    workspace_path: String,
    report: PlanningDoctorReport,
}

impl DoctorReport {
    pub(super) fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            report: PlanningDoctorReport::path_issue(issue),
        }
    }

    pub(super) fn from_service_report(
        workspace_path: String,
        report: PlanningDoctorReport,
    ) -> Self {
        Self {
            workspace_path,
            report,
        }
    }

    pub(super) fn exit_code(&self) -> i32 {
        self.report.exit_code()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InitReport {
    workspace_path: String,
    mode: &'static str,
    created_file_count: Option<usize>,
    queue_idle_policy: Option<String>,
    status: Option<String>,
    issue: Option<String>,
}

impl InitReport {
    pub(super) fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            mode: "simple",
            created_file_count: None,
            queue_idle_policy: None,
            status: None,
            issue: Some(issue),
        }
    }

    pub(super) fn success(
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

    pub(super) fn failure(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            mode: "simple",
            created_file_count: None,
            queue_idle_policy: None,
            status: None,
            issue: Some(issue),
        }
    }

    pub(super) fn exit_code(&self) -> i32 {
        if self.issue.is_some() { 1 } else { 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResetReport {
    workspace_path: String,
    target: Option<&'static str>,
    rewritten_paths: Vec<String>,
    removed_paths: Vec<String>,
    status: Option<String>,
    issue: Option<String>,
}

impl ResetReport {
    pub(super) fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            target: None,
            rewritten_paths: Vec::new(),
            removed_paths: Vec::new(),
            status: None,
            issue: Some(issue),
        }
    }

    pub(super) fn success(workspace_path: String, result: &PlanningWorkspaceResetResult) -> Self {
        Self {
            workspace_path,
            target: Some(result.target.label()),
            rewritten_paths: result.rewritten_paths.clone(),
            removed_paths: result.removed_paths.clone(),
            status: Some("planning workspace reset".to_string()),
            issue: None,
        }
    }

    pub(super) fn failure(
        workspace_path: String,
        target: PlanningResetTarget,
        issue: String,
    ) -> Self {
        Self {
            workspace_path,
            target: Some(target.label()),
            rewritten_paths: Vec::new(),
            removed_paths: Vec::new(),
            status: None,
            issue: Some(issue),
        }
    }

    pub(super) fn exit_code(&self) -> i32 {
        if self.issue.is_some() { 1 } else { 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct PlanningToolErrorReport {
    pub(super) ok: bool,
    pub(super) operation: String,
    pub(super) error: String,
    pub(super) guidance: Vec<String>,
}

pub(super) fn render_doctor_report(stdout: &mut impl Write, report: &DoctorReport) -> Result<()> {
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

pub(super) fn render_init_report(stdout: &mut impl Write, report: &InitReport) -> Result<()> {
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

pub(super) fn render_reset_report(stdout: &mut impl Write, report: &ResetReport) -> Result<()> {
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

pub(super) fn render_json_line<T: Serialize>(stdout: &mut impl Write, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *stdout, value).context("failed to serialize JSON response")?;
    writeln!(stdout)?;
    Ok(())
}

fn bootstrap_mode_label(mode: PlanningBootstrapMode) -> &'static str {
    match mode {
        PlanningBootstrapMode::Detail => "detail",
        PlanningBootstrapMode::Simple => "simple",
    }
}
