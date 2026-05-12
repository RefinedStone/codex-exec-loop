use crate::application::service::planning::{
    PlanningDoctorReport, PlanningResetTarget, PlanningWorkspaceResetResult,
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;

/*
 * planning CLI report는 shell-facing command의 presentation boundary다.
 * application service가 workspace state와 mutation 여부를 결정하고, 이 모듈은 script/test가 소비하는
 * lowercase line protocol과 JSONL envelope를 고정한다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DoctorReport {
    workspace_path: String,
    report: PlanningDoctorReport,
}

impl DoctorReport {
    // path resolution은 doctor service가 inspect하기 전에 실패할 수 있다. adapter는 그 error도 같은 report shape와
    // exit-code path로 접는다.
    pub(super) fn path_issue(workspace_path: String, issue: String) -> Self {
        Self {
            workspace_path,
            report: PlanningDoctorReport::path_issue(issue),
        }
    }

    // health semantics는 service report가 소유한다. CLI는 terminal에 다시 echo할 정확한 workspace label만 더한다.
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

// reset output은 rewrite/remove된 path를 의도적으로 모두 명시한다.
// downstream automation이 file을 다시 읽지 않고도 이 line들을 parse할 수 있어야 하기 때문이다.
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
    // adapter가 workspace path를 resolve하지 못하면 reset target도 아직 validate되지 않았다.
    // 그래서 report에서 target과 mutation line을 생략한다.
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

    // path list를 report로 clone해 render step을 completed service result의 순수 dump로 만든다.
    // workspace state를 두 번째로 읽지 않게 하는 선택이다.
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

    // reset dispatch가 시작된 뒤에는 service가 file mutation 보고 전에 실패해도 selected target이 유용한 diagnostic context다.
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

// machine-facing planning-tool command는 JSONL을 사용한다. caller가 human text를 scraping하지 않고도
// transport failure와 operation-level error를 구분할 수 있게 하기 위해서다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct PlanningToolErrorReport {
    pub(super) ok: bool,
    pub(super) operation: String,
    pub(super) error: String,
    pub(super) guidance: Vec<String>,
}

// doctor rendering은 optional diagnostic을 empty key가 아니라 omitted line으로 유지한다.
// 사람과 단순 shell assertion 모두에게 안정적인 text가 된다.
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

// reset은 mutation path를 repeated key로 출력한다. service가 준 순서를 보존하면서도 human-readable protocol에
// 별도 list syntax를 만들지 않기 위한 형식이다.
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

// JSON escaping은 interpolated string이 아니라 serde에 맡긴다.
// 각 호출은 streaming consumer를 위해 newline-terminated object 하나만 emit한다.
pub(super) fn render_json_line<T: Serialize>(stdout: &mut impl Write, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *stdout, value).context("failed to serialize JSON response")?;
    writeln!(stdout)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DoctorReport, PlanningToolErrorReport, ResetReport, render_doctor_report, render_json_line,
        render_reset_report,
    };
    use crate::application::service::planning::{
        PlanningResetTarget, PlanningWorkspaceResetResult,
    };

    fn rendered_utf8(output: Vec<u8>) -> String {
        String::from_utf8(output).expect("report output should be UTF-8")
    }

    #[test]
    fn doctor_report_renders_path_issue_with_exit_code() {
        let report = DoctorReport::path_issue(
            "/tmp/missing-workspace".to_string(),
            "workspace path does not exist".to_string(),
        );
        let mut output = Vec::new();

        render_doctor_report(&mut output, &report).expect("doctor report should render");
        let rendered = rendered_utf8(output);

        assert_eq!(report.exit_code(), 1);
        assert!(rendered.contains("workspace: /tmp/missing-workspace\n"));
        assert!(rendered.contains("planning state: invalid\n"));
        assert!(rendered.contains("issue: workspace path does not exist\n"));
        assert!(!rendered.contains("queue summary:"));
        assert!(!rendered.contains("health:"));
    }

    #[test]
    fn reset_reports_render_success_failure_and_path_issue_shapes() {
        let success = ResetReport::success(
            "/tmp/workspace".to_string(),
            &PlanningWorkspaceResetResult {
                target: PlanningResetTarget::All,
                rewritten_paths: vec![
                    "planning/result_output.md".to_string(),
                    "planning/task_authority.json".to_string(),
                ],
                removed_paths: vec!["planning/generated/old.json".to_string()],
            },
        );
        let mut success_output = Vec::new();
        render_reset_report(&mut success_output, &success).expect("success should render");
        let success_rendered = rendered_utf8(success_output);

        assert_eq!(success.exit_code(), 0);
        assert!(success_rendered.contains("workspace: /tmp/workspace\n"));
        assert!(success_rendered.contains("command: reset\n"));
        assert!(success_rendered.contains("target: all\n"));
        assert!(success_rendered.contains("rewritten: planning/result_output.md\n"));
        assert!(success_rendered.contains("rewritten: planning/task_authority.json\n"));
        assert!(success_rendered.contains("removed: planning/generated/old.json\n"));
        assert!(success_rendered.contains("status: planning workspace reset\n"));

        let failure = ResetReport::failure(
            "/tmp/workspace".to_string(),
            PlanningResetTarget::Queue,
            "planning workspace is busy".to_string(),
        );
        let mut failure_output = Vec::new();
        render_reset_report(&mut failure_output, &failure).expect("failure should render");
        let failure_rendered = rendered_utf8(failure_output);

        assert_eq!(failure.exit_code(), 1);
        assert!(failure_rendered.contains("target: queue\n"));
        assert!(failure_rendered.contains("issue: planning workspace is busy\n"));
        assert!(!failure_rendered.contains("status: planning workspace reset"));

        let path_issue = ResetReport::path_issue(
            "/tmp/not-a-workspace".to_string(),
            "workspace path is not a directory".to_string(),
        );
        let mut path_issue_output = Vec::new();
        render_reset_report(&mut path_issue_output, &path_issue).expect("path issue should render");
        let path_issue_rendered = rendered_utf8(path_issue_output);

        assert_eq!(path_issue.exit_code(), 1);
        assert!(path_issue_rendered.contains("workspace: /tmp/not-a-workspace\n"));
        assert!(path_issue_rendered.contains("command: reset\n"));
        assert!(path_issue_rendered.contains("issue: workspace path is not a directory\n"));
        assert!(!path_issue_rendered.contains("target:"));
        assert!(!path_issue_rendered.contains("rewritten:"));
    }

    #[test]
    fn json_line_report_uses_serde_escaping_and_newline_delimiter() {
        let report = PlanningToolErrorReport {
            ok: false,
            operation: "planning-tool".to_string(),
            error: "bad \"json\" payload".to_string(),
            guidance: vec![
                "Run `akra planning-tool contract`".to_string(),
                "Keep each request narrow\nand explicit".to_string(),
            ],
        };
        let mut output = Vec::new();

        render_json_line(&mut output, &report).expect("JSON line should render");
        let rendered = rendered_utf8(output);
        let parsed: serde_json::Value =
            serde_json::from_str(rendered.trim_end()).expect("JSONL payload should parse");

        assert!(rendered.ends_with('\n'));
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["operation"], "planning-tool");
        assert_eq!(parsed["error"], "bad \"json\" payload");
        assert_eq!(
            parsed["guidance"][1],
            "Keep each request narrow\nand explicit"
        );
    }
}
