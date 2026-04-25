use std::sync::Arc;

use anyhow::Result;

use super::{PlanningAdminFacadeService, PlanningAdminOverview, PlanningResetTarget};
use crate::application::service::planning::admin::{
    PlanningAdminQueueHeadView, PlanningAdminQueueTaskView,
};

const CONTROL_HELP_TEXT: &str = "지원 명령어\n\
/help\n\
/status\n\
/queue\n\
/reset queue\n\
/reset directions\n\
/reset all";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningControlCommand {
    Help,
    Status,
    Queue,
    Reset(PlanningResetTarget),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlReply {
    pub text: String,
}

impl PlanningControlReply {
    fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlQueueEntry {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlStatusSnapshot {
    pub workspace_dir: String,
    pub planning_state: String,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub health: Option<String>,
    pub issue: Option<String>,
    pub note: Option<String>,
    pub preview_status_label: String,
    pub preview_detail: Option<String>,
    pub queue_head: Option<PlanningControlQueueEntry>,
    pub visible_tasks: Vec<PlanningControlQueueEntry>,
    pub proposed_tasks: Vec<PlanningControlQueueEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlResetOutcome {
    pub target: String,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub planning_state: String,
    pub health: Option<String>,
    pub issue: Option<String>,
}

pub trait PlanningControlSurface: Send + Sync {
    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot>;

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome>;
}

impl PlanningControlSurface for PlanningAdminFacadeService {
    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        let overview = self.load_overview()?;
        Ok(map_overview(overview))
    }

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        let outcome = self.reset_workspace(target)?;
        Ok(PlanningControlResetOutcome {
            target: outcome.target,
            rewritten_paths: outcome.rewritten_paths,
            removed_paths: outcome.removed_paths,
            planning_state: outcome.doctor.planning_state,
            health: outcome.doctor.health,
            issue: outcome.doctor.issue,
        })
    }
}

#[derive(Clone)]
pub struct PlanningControlService {
    surface: Arc<dyn PlanningControlSurface>,
}

impl PlanningControlService {
    pub fn new(surface: Arc<dyn PlanningControlSurface>) -> Self {
        Self { surface }
    }

    pub fn execute(&self, command: PlanningControlCommand) -> Result<PlanningControlReply> {
        match command {
            PlanningControlCommand::Help => Ok(PlanningControlReply::new(CONTROL_HELP_TEXT)),
            PlanningControlCommand::Status => {
                let snapshot = self.surface.load_status_snapshot()?;
                Ok(PlanningControlReply::new(format_status(&snapshot)))
            }
            PlanningControlCommand::Queue => {
                let snapshot = self.surface.load_status_snapshot()?;
                Ok(PlanningControlReply::new(format_queue(&snapshot)))
            }
            PlanningControlCommand::Reset(target) => {
                let outcome = self.surface.reset_workspace(target)?;
                Ok(PlanningControlReply::new(format_reset(&outcome)))
            }
        }
    }

    pub fn help_text(&self) -> &'static str {
        CONTROL_HELP_TEXT
    }
}

fn map_overview(overview: PlanningAdminOverview) -> PlanningControlStatusSnapshot {
    PlanningControlStatusSnapshot {
        workspace_dir: overview.workspace_dir,
        planning_state: overview.doctor.planning_state,
        queue_summary: overview.doctor.queue_summary,
        proposal_summary: overview.doctor.proposal_summary,
        health: overview.doctor.health,
        issue: overview.doctor.issue,
        note: overview.doctor.note,
        preview_status_label: overview.runtime.preview_status_label,
        preview_detail: overview.runtime.preview_detail,
        queue_head: overview.runtime.queue_head.map(map_queue_head),
        visible_tasks: overview
            .runtime
            .visible_tasks
            .into_iter()
            .map(map_queue_task)
            .collect(),
        proposed_tasks: overview
            .runtime
            .proposed_tasks
            .into_iter()
            .map(map_queue_task)
            .collect(),
    }
}

fn map_queue_head(view: PlanningAdminQueueHeadView) -> PlanningControlQueueEntry {
    PlanningControlQueueEntry {
        task_id: view.task_id,
        task_title: view.task_title,
        direction_id: view.direction_id,
        status: view.status,
        combined_priority: view.combined_priority,
    }
}

fn map_queue_task(view: PlanningAdminQueueTaskView) -> PlanningControlQueueEntry {
    PlanningControlQueueEntry {
        task_id: view.task_id,
        task_title: view.task_title,
        direction_id: view.direction_id,
        status: view.status,
        combined_priority: view.combined_priority,
    }
}

fn format_status(snapshot: &PlanningControlStatusSnapshot) -> String {
    let mut lines = vec![
        "상태 요약".to_string(),
        format!("workspace: {}", snapshot.workspace_dir),
        format!("planning_state: {}", snapshot.planning_state),
        format!("preview: {}", snapshot.preview_status_label),
    ];
    if let Some(detail) = snapshot.preview_detail.as_ref() {
        lines.push(format!("preview_detail: {detail}"));
    }
    if let Some(health) = snapshot.health.as_ref() {
        lines.push(format!("health: {health}"));
    }
    if let Some(issue) = snapshot.issue.as_ref() {
        lines.push(format!("issue: {issue}"));
    }
    if let Some(note) = snapshot.note.as_ref() {
        lines.push(format!("note: {note}"));
    }
    if let Some(queue_summary) = snapshot.queue_summary.as_ref() {
        lines.push(format!("queue_summary: {queue_summary}"));
    }
    if let Some(proposal_summary) = snapshot.proposal_summary.as_ref() {
        lines.push(format!("proposal_summary: {proposal_summary}"));
    }
    lines.push(format!(
        "queue_head: {}",
        snapshot
            .queue_head
            .as_ref()
            .map(queue_entry_label)
            .unwrap_or_else(|| "없음".to_string())
    ));
    lines.push(format!("visible_tasks: {}", snapshot.visible_tasks.len()));
    lines.push(format!("proposed_tasks: {}", snapshot.proposed_tasks.len()));
    lines.join("\n")
}

fn format_queue(snapshot: &PlanningControlStatusSnapshot) -> String {
    let mut lines = vec!["큐 요약".to_string()];
    if let Some(queue_summary) = snapshot.queue_summary.as_ref() {
        lines.push(format!("queue_summary: {queue_summary}"));
    }
    if let Some(proposal_summary) = snapshot.proposal_summary.as_ref() {
        lines.push(format!("proposal_summary: {proposal_summary}"));
    }
    lines.push(format!(
        "queue_head: {}",
        snapshot
            .queue_head
            .as_ref()
            .map(queue_entry_label)
            .unwrap_or_else(|| "없음".to_string())
    ));
    lines.push(render_queue_section("queued", &snapshot.visible_tasks));
    lines.push(render_queue_section("proposed", &snapshot.proposed_tasks));
    lines.join("\n")
}

fn render_queue_section(label: &str, entries: &[PlanningControlQueueEntry]) -> String {
    if entries.is_empty() {
        return format!("{label}: 없음");
    }

    let mut lines = vec![format!("{label}:")];
    for (index, entry) in entries.iter().take(5).enumerate() {
        lines.push(format!("{}. {}", index + 1, queue_entry_label(entry)));
    }
    if entries.len() > 5 {
        lines.push(format!("... and {} more", entries.len() - 5));
    }
    lines.join("\n")
}

fn queue_entry_label(entry: &PlanningControlQueueEntry) -> String {
    format!(
        "[{}] {} ({}, {}, {})",
        entry.combined_priority, entry.task_title, entry.task_id, entry.direction_id, entry.status
    )
}

fn format_reset(outcome: &PlanningControlResetOutcome) -> String {
    let mut lines = vec![
        format!("reset {} 완료", outcome.target),
        format!("planning_state: {}", outcome.planning_state),
        format!("rewritten_paths: {}", outcome.rewritten_paths.len()),
        format!("removed_paths: {}", outcome.removed_paths.len()),
    ];
    if !outcome.rewritten_paths.is_empty() {
        lines.push(format!("rewritten: {}", outcome.rewritten_paths.join(", ")));
    }
    if !outcome.removed_paths.is_empty() {
        lines.push(format!("removed: {}", outcome.removed_paths.join(", ")));
    }
    if let Some(health) = outcome.health.as_ref() {
        lines.push(format!("health: {health}"));
    }
    if let Some(issue) = outcome.issue.as_ref() {
        lines.push(format!("issue: {issue}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;

    use super::{
        PlanningControlCommand, PlanningControlQueueEntry, PlanningControlResetOutcome,
        PlanningControlService, PlanningControlStatusSnapshot, PlanningControlSurface,
    };
    use crate::application::service::planning::PlanningResetTarget;

    struct FakePlanningControlSurface {
        status: PlanningControlStatusSnapshot,
        reset_outcome: PlanningControlResetOutcome,
    }

    impl PlanningControlSurface for FakePlanningControlSurface {
        fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
            Ok(self.status.clone())
        }

        fn reset_workspace(
            &self,
            _target: PlanningResetTarget,
        ) -> Result<PlanningControlResetOutcome> {
            Ok(self.reset_outcome.clone())
        }
    }

    fn build_service() -> PlanningControlService {
        PlanningControlService::new(Arc::new(FakePlanningControlSurface {
            status: PlanningControlStatusSnapshot {
                workspace_dir: "/tmp/repo".to_string(),
                planning_state: "ready".to_string(),
                queue_summary: Some("queue head ready".to_string()),
                proposal_summary: Some("1 proposal".to_string()),
                health: Some("planning workspace ready".to_string()),
                issue: None,
                note: Some("next task available".to_string()),
                preview_status_label: "queue ready".to_string(),
                preview_detail: Some("head task is executable".to_string()),
                queue_head: Some(PlanningControlQueueEntry {
                    task_id: "task-1".to_string(),
                    task_title: "Ship Telegram control".to_string(),
                    direction_id: "general-workstream".to_string(),
                    status: "ready".to_string(),
                    combined_priority: 95,
                }),
                visible_tasks: vec![PlanningControlQueueEntry {
                    task_id: "task-1".to_string(),
                    task_title: "Ship Telegram control".to_string(),
                    direction_id: "general-workstream".to_string(),
                    status: "ready".to_string(),
                    combined_priority: 95,
                }],
                proposed_tasks: vec![PlanningControlQueueEntry {
                    task_id: "task-2".to_string(),
                    task_title: "Add webhook delivery".to_string(),
                    direction_id: "general-workstream".to_string(),
                    status: "proposed".to_string(),
                    combined_priority: 60,
                }],
            },
            reset_outcome: PlanningControlResetOutcome {
                target: "queue".to_string(),
                rewritten_paths: vec![".codex-exec-loop/planning/task-ledger.json".to_string()],
                removed_paths: Vec::new(),
                planning_state: "ready".to_string(),
                health: Some("queue reset complete".to_string()),
                issue: None,
            },
        }))
    }

    #[test]
    fn help_command_lists_supported_actions() {
        let service = build_service();

        let reply = service
            .execute(PlanningControlCommand::Help)
            .expect("help should execute");

        assert!(reply.text.contains("/status"));
        assert!(reply.text.contains("/reset all"));
    }

    #[test]
    fn status_command_includes_queue_head_and_health() {
        let service = build_service();

        let reply = service
            .execute(PlanningControlCommand::Status)
            .expect("status should execute");

        assert!(reply.text.contains("상태 요약"));
        assert!(reply.text.contains("planning workspace ready"));
        assert!(reply.text.contains("Ship Telegram control"));
    }

    #[test]
    fn reset_command_reports_rewritten_paths() {
        let service = build_service();

        let reply = service
            .execute(PlanningControlCommand::Reset(PlanningResetTarget::Queue))
            .expect("reset should execute");

        assert!(reply.text.contains("reset queue 완료"));
        assert!(reply.text.contains("task-ledger.json"));
    }
}
