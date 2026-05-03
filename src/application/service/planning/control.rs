use super::{PlanningAdminFacadeService, PlanningAdminOverview, PlanningResetTarget};
use crate::application::service::planning::admin::{
    PlanningAdminQueueHeadView, PlanningAdminQueueTaskView,
};
use anyhow::Result;
use std::sync::Arc;

/*
 * PlanningControlService is the compact command surface used by operator-facing
 * entry points. The admin facade owns rich management data; this layer narrows
 * it into text replies that are stable enough for TUI/CLI/Telegram control
 * flows without exposing admin view structs outside the planning boundary.
 */
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
    // Reset is explicit about its target so callers cannot pass free-form
    // destructive strings into the admin reset use case.
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
    // Queue entries keep only the fields needed for operator text. Rich admin
    // views may grow UI-specific metadata without changing this command API.
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlStatusSnapshot {
    // Snapshot data is denormalized for rendering. This avoids repeated admin
    // facade calls while formatting /status and /queue.
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
    // Reset output mirrors the admin reset result but flattens doctor state so
    // command callers can render the post-reset health in one reply.
    pub target: String,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub planning_state: String,
    pub health: Option<String>,
    pub issue: Option<String>,
}
pub trait PlanningControlSurface: Send + Sync {
    // A narrow trait keeps the command executor testable and prevents the text
    // layer from depending on the full admin facade API.
    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot>;
    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome>;
}
impl PlanningControlSurface for PlanningAdminFacadeService {
    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        let overview = self.load_overview()?;
        Ok(map_overview(overview))
    }
    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        // Admin reset returns file changes plus a doctor summary. Control
        // callers need both because a reset can succeed while still reporting a
        // planning health issue that needs operator attention.
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
        // Execute is deliberately just dispatch plus formatting; all reads and
        // writes cross the PlanningControlSurface boundary.
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
    /*
     * The admin overview is optimized for management screens. The control
     * snapshot strips it down to the health, preview, and queue facts that can
     * fit in a short operator response.
     */
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
    // queue_head and visible/proposed tasks originate from different admin view
    // structs, but the control renderer treats them as the same compact line.
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
    // /status favors breadth: workspace health, runtime preview, queue summary,
    // and counts, with optional fields omitted when the admin doctor has no
    // extra context.
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
    // /queue is the detail view for operators deciding what to run next. It
    // includes both executable queue entries and proposed follow-up tasks.
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
    // Limit sections to five entries so chat-style control surfaces remain
    // readable while still disclosing hidden backlog size.
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
    // Priority is first because this text is scanned when deciding the next
    // action; id/direction/status follow as disambiguators.
    format!(
        "[{}] {} ({}, {}, {})",
        entry.combined_priority, entry.task_title, entry.task_id, entry.direction_id, entry.status
    )
}
fn format_reset(outcome: &PlanningControlResetOutcome) -> String {
    // Reset replies must show both counts and concrete paths. Counts make the
    // result skimmable; paths let the operator verify which authorities moved.
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
    use super::{
        PlanningControlCommand, PlanningControlQueueEntry, PlanningControlResetOutcome,
        PlanningControlService, PlanningControlStatusSnapshot, PlanningControlSurface,
    };
    use crate::application::service::planning::PlanningResetTarget;
    use anyhow::Result;
    use std::sync::Arc;

    /*
     * The fake surface freezes admin data at the control boundary. These tests
     * verify command dispatch and rendering contracts without real workspace
     * files or planning authority stores.
     */
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
        // The fixture includes health, queue head, visible tasks, proposed
        // tasks, and reset paths so each command exercises optional sections.
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
                rewritten_paths: vec!["DB task authority".to_string()],
                removed_paths: Vec::new(),
                planning_state: "ready".to_string(),
                health: Some("queue reset complete".to_string()),
                issue: None,
            },
        }))
    }
    #[test]
    fn help_command_lists_supported_actions() {
        // Help text is a public command contract: adding/removing supported
        // actions should be visible in this small snapshot-style assertion.
        let service = build_service();
        let reply = service
            .execute(PlanningControlCommand::Help)
            .expect("help should execute");

        assert!(reply.text.contains("/status"));
        assert!(reply.text.contains("/reset all"));
    }
    #[test]
    fn status_command_includes_queue_head_and_health() {
        // /status should combine doctor health with queue head context, not only
        // report that planning loaded successfully.
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
        // Reset confirmation is actionable only if it names the authority files
        // or stores that were rewritten.
        let service = build_service();
        let reply = service
            .execute(PlanningControlCommand::Reset(PlanningResetTarget::Queue))
            .expect("reset should execute");

        assert!(reply.text.contains("reset queue 완료"));
        assert!(reply.text.contains("DB task authority"));
    }
}
