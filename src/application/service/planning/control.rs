use super::PlanningResetTarget;
use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningApplicationQueueTask, PlanningDoctorReport,
    PlanningServices, PlanningWorkspaceResetResult,
};
use anyhow::Result;
use std::sync::Arc;

/*
 * PlanningControlService는 operator-facing entrypoint가 쓰는 compact command surface다.
 * PlanningControlFacadeService가 제공하는 planning facts를 TUI/CLI/Telegram control flow가 바로 표시할 수
 * 있는 stable text reply로 낮춘다. inbound adapter는 command enum과 text reply만 다루고, queue/proposal
 * 판단은 application projection 뒤에 둔다.
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
    // reset은 target enum을 통해서만 들어온다. caller가 free-form 파괴 명령 문자열을 reset use case로 넘기지 못하게 한다.
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
pub struct PlanningControlRequest {
    pub command: PlanningControlCommand,
}
impl PlanningControlRequest {
    pub fn new(command: PlanningControlCommand) -> Self {
        Self { command }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlResponse {
    pub workspace_dir: String,
    pub reply: PlanningControlReply,
}
impl PlanningControlResponse {
    fn new(workspace_dir: impl Into<String>, reply: PlanningControlReply) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            reply,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlQueueEntry {
    // queue entry는 operator text에 필요한 field만 남긴다. rich admin view가 UI 전용 metadata를 늘려도
    // command API의 compact line 계약은 바뀌지 않는다.
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: String,
    pub combined_priority: i32,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningControlStatusSnapshot {
    // snapshot data는 rendering을 위해 denormalize되어 있다. /status와 /queue를 format하는 동안 application
    // facade를 반복 호출하지 않고, 같은 관측 시점의 health/queue/proposal 상태를 함께 보여 주기 위해서다.
    pub workspace_dir: String,
    pub planning_state: String,
    pub task_authority_signature: Option<u64>,
    pub queue_head_task_signature: Option<u64>,
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
    // reset output은 reset 결과를 반영하되 doctor state를 납작하게 합친다. command caller가 reset 효과와
    // post-reset health를 한 reply 안에서 보여 줄 수 있게 하는 shape다.
    pub target: String,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub planning_state: String,
    pub health: Option<String>,
    pub issue: Option<String>,
}
pub trait PlanningControlSurface: Send + Sync {
    // 좁은 trait은 command executor를 testable하게 만들고, text layer가 full planning facade API에 의존하지 않게 한다.
    fn workspace_dir(&self) -> &str;
    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot>;
    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome>;
}

#[derive(Clone)]
pub struct PlanningControlFacadeService {
    workspace_dir: String,
    planning: PlanningServices,
}
impl PlanningControlFacadeService {
    pub fn new(workspace_dir: impl Into<String>, planning: PlanningServices) -> Self {
        Self {
            workspace_dir: workspace_dir.into(),
            planning,
        }
    }
}
impl PlanningControlSurface for PlanningControlFacadeService {
    fn workspace_dir(&self) -> &str {
        self.workspace_dir.as_str()
    }

    fn load_status_snapshot(&self) -> Result<PlanningControlStatusSnapshot> {
        let doctor = self
            .planning
            .workspace
            .inspect_workspace(self.workspace_dir.as_str());
        let runtime = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(self.workspace_dir.as_str());
        Ok(map_control_status_snapshot(
            self.workspace_dir.clone(),
            doctor,
            PlanningApplicationProjection::from_runtime_snapshot(&runtime),
        ))
    }

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningControlResetOutcome> {
        let result = self
            .planning
            .workspace
            .reset_workspace(self.workspace_dir.as_str(), target)?;
        Ok(map_workspace_reset_result(
            self.workspace_dir.as_str(),
            result,
            &self.planning,
        ))
    }
}

fn map_workspace_reset_result(
    workspace_dir: &str,
    result: PlanningWorkspaceResetResult,
    planning: &PlanningServices,
) -> PlanningControlResetOutcome {
    // reset 직후 doctor를 다시 읽어 command reply가 실제 post-reset health를 보여 주게 한다.
    let doctor = planning.workspace.inspect_workspace(workspace_dir);
    PlanningControlResetOutcome {
        target: result.target.label().to_string(),
        rewritten_paths: result.rewritten_paths,
        removed_paths: result.removed_paths,
        planning_state: doctor.planning_state().label().to_string(),
        health: doctor.health().map(str::to_string),
        issue: doctor.issue().map(str::to_string),
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
    pub fn execute_request(
        &self,
        request: PlanningControlRequest,
    ) -> Result<PlanningControlResponse> {
        let reply = self.execute(request.command)?;
        Ok(PlanningControlResponse::new(
            self.surface.workspace_dir(),
            reply,
        ))
    }
    pub fn execute(&self, command: PlanningControlCommand) -> Result<PlanningControlReply> {
        // execute는 의도적으로 dispatch와 formatting만 담당한다. 모든 read/write는 PlanningControlSurface 경계를 지난다.
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
fn map_control_status_snapshot(
    workspace_dir: String,
    doctor: PlanningDoctorReport,
    projection: PlanningApplicationProjection,
) -> PlanningControlStatusSnapshot {
    /*
     * control snapshot은 짧은 operator response에 들어갈 health, preview, queue 사실만 추려 낸다.
     * queue/proposal lane은 admin DTO가 아니라 PlanningApplicationProjection에서 직접 받기 때문에
     * /status, /queue와 admin overview가 같은 application read model을 공유한다.
     */
    PlanningControlStatusSnapshot {
        workspace_dir,
        planning_state: doctor.planning_state().label().to_string(),
        task_authority_signature: projection.task_authority_signature,
        queue_head_task_signature: projection.queue_head_task_signature,
        queue_summary: doctor.queue_summary().map(str::to_string),
        proposal_summary: doctor.proposal_summary().map(str::to_string),
        health: doctor.health().map(str::to_string),
        issue: doctor.issue().map(str::to_string),
        note: doctor.note().map(str::to_string),
        preview_status_label: projection.status_label,
        preview_detail: projection.status_detail,
        queue_head: projection.queue_head.map(map_application_queue_task),
        visible_tasks: projection
            .visible_tasks
            .into_iter()
            .map(map_application_queue_task)
            .collect(),
        proposed_tasks: projection
            .proposed_tasks
            .into_iter()
            .map(map_application_queue_task)
            .collect(),
    }
}
fn map_application_queue_task(task: PlanningApplicationQueueTask) -> PlanningControlQueueEntry {
    // application queue task에는 rank/update metadata도 있지만 compact control reply는 operator line에 필요한 값만 남긴다.
    PlanningControlQueueEntry {
        task_id: task.task_id,
        task_title: task.task_title,
        direction_id: task.direction_id,
        status: task.status_label,
        combined_priority: task.combined_priority,
    }
}
fn format_status(snapshot: &PlanningControlStatusSnapshot) -> String {
    // /status는 breadth를 우선한다. workspace health, runtime preview, queue summary, count를 한 번에 보여 주되
    // admin doctor가 추가 context를 주지 않은 optional field는 생략한다.
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
    // /queue는 다음에 무엇을 실행할지 판단하는 operator용 detail view다. executable queue와 proposed follow-up을 함께 싣는다.
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
    // chat-style control surface가 읽기 쉽게 각 section은 5개로 제한하되, 숨겨진 backlog 크기는 함께 표시한다.
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
    // 다음 action을 고를 때 먼저 훑는 값이 priority이므로 앞에 둔다. id/direction/status는 같은 title을 구분하는 보조 정보다.
    format!(
        "[{}] {} ({}, {}, {})",
        entry.combined_priority, entry.task_title, entry.task_id, entry.direction_id, entry.status
    )
}
fn format_reset(outcome: &PlanningControlResetOutcome) -> String {
    // reset reply는 count와 concrete path를 모두 보여 줘야 한다. count는 결과를 훑기 쉽게 하고,
    // path는 어떤 authority가 움직였는지 operator가 검증할 수 있게 한다.
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
        PlanningControlCommand, PlanningControlQueueEntry, PlanningControlRequest,
        PlanningControlResetOutcome, PlanningControlService, PlanningControlStatusSnapshot,
        PlanningControlSurface, map_control_status_snapshot,
    };
    use crate::application::service::planning::{
        PlanningApplicationProjection, PlanningApplicationQueueTask, PlanningDoctorReport,
        PlanningResetTarget,
    };
    use crate::domain::planning::{QueueIdlePolicy, TaskStatus};
    use anyhow::Result;
    use std::sync::Arc;

    /*
     * fake surface는 control boundary에서 admin data를 고정한다.
     * 이 test들은 실제 workspace file이나 planning authority store 없이 command dispatch와 rendering 계약을 검증한다.
     */
    struct FakePlanningControlSurface {
        status: PlanningControlStatusSnapshot,
        reset_outcome: PlanningControlResetOutcome,
    }
    impl PlanningControlSurface for FakePlanningControlSurface {
        fn workspace_dir(&self) -> &str {
            self.status.workspace_dir.as_str()
        }

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
        // fixture는 health, queue head, visible task, proposed task, reset path를 모두 포함한다.
        // 각 command가 optional section까지 렌더링하는지 한 번에 확인하기 위해서다.
        PlanningControlService::new(Arc::new(FakePlanningControlSurface {
            status: PlanningControlStatusSnapshot {
                workspace_dir: "/tmp/repo".to_string(),
                planning_state: "ready".to_string(),
                task_authority_signature: Some(42),
                queue_head_task_signature: Some(7),
                queue_summary: Some("queue head ready".to_string()),
                proposal_summary: Some("1 proposal".to_string()),
                health: Some("planning workspace ready".to_string()),
                issue: None,
                note: Some("queue head available".to_string()),
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
        // help text는 공개 command 계약이다. 지원 action을 추가/삭제하면 이 작은 snapshot-style assertion에 드러나야 한다.
        let service = build_service();
        let reply = service
            .execute(PlanningControlCommand::Help)
            .expect("help should execute");

        assert!(reply.text.contains("/status"));
        assert!(reply.text.contains("/reset all"));
    }
    #[test]
    fn status_command_includes_queue_head_and_health() {
        // /status는 planning이 load됐다는 사실만이 아니라 doctor health와 queue head context를 함께 보여 줘야 한다.
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
        // reset 확인 문구는 rewrite된 authority file/store 이름을 포함해야 operator가 실제 효과를 확인할 수 있다.
        let service = build_service();
        let reply = service
            .execute(PlanningControlCommand::Reset(PlanningResetTarget::Queue))
            .expect("reset should execute");

        assert!(reply.text.contains("reset queue 완료"));
        assert!(reply.text.contains("DB task authority"));
    }

    #[test]
    fn execute_request_returns_shared_response_context() {
        /*
         * CLI and Telegram call the same request/response path. The workspace
         * context comes from the control surface, so adapters do not need their
         * own response envelope for the same planning command.
         */
        let service = build_service();
        let response = service
            .execute_request(PlanningControlRequest::new(PlanningControlCommand::Status))
            .expect("status request should execute");

        assert_eq!(response.workspace_dir, "/tmp/repo");
        assert!(response.reply.text.contains("상태 요약"));
    }

    #[test]
    fn control_status_maps_queue_lanes_from_application_projection() {
        /*
         * control snapshot은 admin queue DTO를 거치지 않고 application projection lane을 직접 낮춘다.
         * doctor가 invalid issue를 갖고 있어도 runtime projection의 queue facts는 같은 관측 결과로 보존되어야 한다.
         */
        let projection = PlanningApplicationProjection {
            workspace_present: true,
            workspace_status:
                crate::application::service::planning::PlanningRuntimeWorkspaceStatus::ReadyWithTask,
            task_authority_signature: Some(42),
            queue_head_task_signature: Some(7),
            auto_follow_paused: false,
            status_label: "ready".to_string(),
            status_detail: Some("queue head ready".to_string()),
            queue_summary: Some("projection queue summary".to_string()),
            proposal_summary: Some("projection proposal summary".to_string()),
            queue_idle_policy: QueueIdlePolicy::ReviewAndEnqueue,
            queue_idle_prompt_path: Some(
                ".codex-exec-loop/planning/prompts/queue-idle-review.md".to_string(),
            ),
            has_structured_queue_projection: true,
            queue_head: Some(queue_task(1, "task-1", "Current task", TaskStatus::Ready)),
            visible_tasks: vec![
                queue_task(1, "task-1", "Current task", TaskStatus::Ready),
                queue_task(2, "task-2", "Next task", TaskStatus::Ready),
            ],
            proposed_tasks: vec![queue_task(
                1,
                "proposal-1",
                "Candidate task",
                TaskStatus::Proposed,
            )],
            skipped_tasks: Vec::new(),
        };

        let snapshot = map_control_status_snapshot(
            "/tmp/repo".to_string(),
            PlanningDoctorReport::path_issue("planning invalid".to_string()),
            projection,
        );

        assert_eq!(snapshot.preview_status_label, "ready");
        assert_eq!(snapshot.task_authority_signature, Some(42));
        assert_eq!(snapshot.queue_head_task_signature, Some(7));
        assert_eq!(
            snapshot
                .queue_head
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("task-1")
        );
        assert_eq!(
            snapshot
                .visible_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["task-1", "task-2"]
        );
        assert_eq!(snapshot.proposed_tasks[0].status, "proposed");
    }

    fn queue_task(
        rank: usize,
        task_id: &str,
        task_title: &str,
        status: TaskStatus,
    ) -> PlanningApplicationQueueTask {
        PlanningApplicationQueueTask {
            rank,
            task_id: task_id.to_string(),
            task_title: task_title.to_string(),
            direction_id: "direction-a".to_string(),
            direction_title: "Direction A".to_string(),
            status,
            status_label: status.label().to_string(),
            combined_priority: 100 - rank as i32,
            updated_at: "2026-05-08T00:00:00Z".to_string(),
            rank_reasons: vec![format!("domain-rank={rank}")],
        }
    }
}
