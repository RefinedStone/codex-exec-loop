//! admin 의 planning 변경을 저장소 커널의 명령으로 보낸다 (plan 18 K4-9d,
//! 설계 `docs/plan/18-k4-9d-planning-mutations.md` §3.5).

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::kernel_command_execution;
use crate::adapter::inbound::kernel_protocol::{CommandResponse, CommandResult, KernelCommand};
use crate::application::port::inbound::planning_admin_port::{
    PlanningAdminCrudOutcome, PlanningAdminDirectionDeleteRequest,
    PlanningAdminDirectionMutationRequest, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminDraftPromotionOutcome,
    PlanningAdminFileSyncOutcome, PlanningAdminManagementView, PlanningAdminMutationRefusal,
    PlanningAdminOverview, PlanningAdminPort, PlanningAdminResetOutcome,
    PlanningAdminRuntimeSummary, PlanningAdminSessionView, PlanningAdminTaskDeleteRequest,
    PlanningAdminTaskMutationRequest,
};
use crate::application::port::inbound::stored_command_port::StoredCommand;
use crate::application::service::planning::PlanningAdminFacadeService;
use crate::domain::planning::PlanningResetRequest;

/*
 * 입구(admin)의 `PlanningAdminPort` 이다. 읽기와 초안의 편집은 facade 가 직접 한다. 작업의 변경은 저장 명령으로
 * 바꿔서 저장소 커널에 보낸다. 명령은 Gateway 의 커널, 명령 전용 조립, 또는 TUI 가 커널을 쥔 저장소의 직접 경로 중
 * 하나로 간다. 어느 길이든 답은 실행이 끝난 뒤에 온다. 그 뒤에 화면의 값을 저장소에서 다시 읽는다.
 *
 * 커널 자신은 이 구현을 쓰지 않는다. 쓰면 명령이 자기에게 돌아온다 (`production::stored_command_port`).
 */
pub(crate) struct KernelRoutedPlanningAdmin {
    direct: PlanningAdminFacadeService,
    caller: &'static str,
}

impl KernelRoutedPlanningAdmin {
    pub(crate) fn new(direct: PlanningAdminFacadeService, caller: &'static str) -> Self {
        Self { direct, caller }
    }

    // 커널이 있을 수 없는 폴더(대표 checkout 이 없는 저장소, 병렬 slot worktree)는 이전처럼 직접 바꾼다 (K4-9a 와 같다).
    fn has_repository_kernel(&self) -> bool {
        super::production::repository_kernel_target(self.direct.workspace_dir()).is_ok()
    }

    /*
     * 저장 명령 하나를 보내고 그 결과를 돌려준다. 거절은 `PlanningAdminMutationRefusal` 이다: 아무것도 바뀌지
     * 않았고, 입구가 409 와 그 글로 답한다. 실패는 커널이 댄 원인의 글이다.
     */
    fn send(&self, command: StoredCommand) -> Result<CommandResult> {
        let port =
            kernel_command_execution::build(Path::new(self.direct.workspace_dir()), self.caller)?;
        let command = KernelCommand::from_stored(command);
        let answer: CommandResponse = serde_json::from_value(port.command(
            command.kind(),
            serde_json::to_value(&command)?,
            None,
        )?)
        .context("the repository kernel answered the command with an unknown shape")?;
        if let Some(rejection) = answer.rejection {
            return Err(PlanningAdminMutationRefusal {
                reason: rejection.reason,
                message: rejection.message,
            }
            .into());
        }
        if let Some(failure) = answer.failure {
            bail!("{failure}");
        }
        if !answer.processed {
            bail!("the repository kernel stopped before it took the command; nothing was changed");
        }
        answer
            .result
            .context("the repository kernel did not answer the command with a result")
    }
}

fn changed_task_id(result: CommandResult) -> String {
    match result {
        CommandResult::TaskCreated { task_id, .. }
        | CommandResult::TaskUpdated { task_id, .. }
        | CommandResult::TaskDeleted { task_id, .. } => task_id,
    }
}

impl PlanningAdminPort for KernelRoutedPlanningAdmin {
    fn workspace_dir(&self) -> &str {
        self.direct.workspace_dir()
    }

    fn load_overview(&self) -> Result<PlanningAdminOverview> {
        self.direct.load_overview()
    }

    fn load_runtime_summary(&self) -> Result<PlanningAdminRuntimeSummary> {
        self.direct.load_runtime_summary()
    }

    fn load_management_view(&self) -> Result<PlanningAdminManagementView> {
        self.direct.load_management_view()
    }

    fn upsert_direction(
        &self,
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        self.direct.upsert_direction(request)
    }

    fn delete_direction(
        &self,
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        self.direct.delete_direction(request)
    }

    fn upsert_task(
        &self,
        request: PlanningAdminTaskMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        if !self.has_repository_kernel() {
            return self.direct.upsert_task(request);
        }
        let command = PlanningAdminFacadeService::stored_task_command(request)?;
        let updated = matches!(command, StoredCommand::UpdateTask { .. });
        let task_id = changed_task_id(self.send(command)?);
        self.direct.task_upsert_outcome(&task_id, updated)
    }

    fn delete_task(
        &self,
        request: PlanningAdminTaskDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        if !self.has_repository_kernel() {
            return self.direct.delete_task(request);
        }
        let command = PlanningAdminFacadeService::stored_task_delete_command(&request)?;
        let task_id = changed_task_id(self.send(command)?);
        self.direct.task_delete_outcome(&task_id)
    }

    fn create_draft_session(
        &self,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
    ) -> Result<PlanningAdminSessionView> {
        PlanningAdminPort::create_draft_session(&self.direct, kind, direction_id)
    }

    fn load_draft_session(
        &self,
        request: PlanningAdminDraftLoadRequest,
    ) -> Result<PlanningAdminSessionView> {
        PlanningAdminPort::load_draft_session(&self.direct, request)
    }

    fn save_draft_session(
        &self,
        request: PlanningAdminDraftMutationRequest,
    ) -> Result<PlanningAdminSessionView> {
        PlanningAdminPort::save_draft_session(&self.direct, request)
    }

    fn promote_draft_session(
        &self,
        request: PlanningAdminDraftMutationRequest,
    ) -> Result<PlanningAdminDraftPromotionOutcome> {
        PlanningAdminPort::promote_draft_session(&self.direct, request)
    }

    fn export_active_files_for_edit(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.direct.export_active_files_for_edit()
    }

    fn apply_exported_files(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.direct.apply_exported_files()
    }

    fn reset_workspace(&self, request: PlanningResetRequest) -> Result<PlanningAdminResetOutcome> {
        self.direct.reset_workspace(request)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::domain::kernel_event::{KernelEvent, MAX_KERNEL_EVENT_PAGE};
    use crate::domain::kernel_spec::ReasonCode;

    struct Workspace(String);

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // The real admin assembly. No Gateway runs in a test build, so the command goes to the command-only kernel.
    fn admin(name: &str) -> (Workspace, std::sync::Arc<dyn PlanningAdminPort>) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "kernel-routed-admin-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let workspace = path.canonicalize().unwrap().to_string_lossy().into_owned();
        let facade = super::super::production::build_admin_application(workspace.clone()).facade;
        (Workspace(workspace), facade)
    }

    fn task(id: &str, title: &str, revision: Option<i64>) -> PlanningAdminTaskMutationRequest {
        PlanningAdminTaskMutationRequest {
            id: id.to_string(),
            direction_id: String::new(),
            title: title.to_string(),
            description: String::new(),
            status: String::new(),
            base_priority: String::new(),
            dynamic_priority_delta: String::new(),
            priority_reason: String::new(),
            depends_on_text: String::new(),
            blocked_by_text: String::new(),
            expected_planning_revision: revision.map(|value| value.to_string()).unwrap_or_default(),
        }
    }

    fn delete(id: &str, revision: Option<i64>) -> PlanningAdminTaskDeleteRequest {
        PlanningAdminTaskDeleteRequest {
            id: id.to_string(),
            expected_planning_revision: revision.map(|value| value.to_string()).unwrap_or_default(),
        }
    }

    fn task_id(outcome: &PlanningAdminCrudOutcome, title: &str) -> String {
        outcome
            .management
            .tasks
            .iter()
            .find(|task| task.title == title)
            .map(|task| task.id.clone())
            .expect("the task is in the management view")
    }

    fn command_records(workspace: &Workspace) -> Vec<String> {
        super::super::production::kernel_event_log_port()
            .load_kernel_events_after(&workspace.0, 0, MAX_KERNEL_EVENT_PAGE)
            .unwrap()
            .records
            .into_iter()
            .map(|record| match record.event {
                KernelEvent::CommandReceived {
                    command, caller, ..
                } => format!("received {command} by {}", caller.unwrap_or_default()),
                KernelEvent::CommandCompleted { result, .. } => format!("completed {result:?}"),
                KernelEvent::CommandRejected { reason, .. } => {
                    format!("rejected {}", reason.name())
                }
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn admin_task_changes_are_kernel_commands_with_the_admin_caller() {
        // plan 18 K4-9d D1: every task change of the admin leaves `command_received` with `caller: akra-admin`.
        let (workspace, admin) = admin("records");
        let created = admin.upsert_task(task("", "Write the test", None)).unwrap();
        assert!(created.notice.ends_with("added"), "{}", created.notice);
        let id = task_id(&created, "Write the test");

        let updated = admin
            .upsert_task(task(
                &id,
                "Write the test first",
                Some(created.management.planning_revision),
            ))
            .unwrap();
        assert_eq!(updated.notice, format!("task `{id}` updated"));
        assert_eq!(task_id(&updated, "Write the test first"), id);

        let deleted = admin
            .delete_task(delete(&id, Some(updated.management.planning_revision)))
            .unwrap();
        assert_eq!(deleted.notice, format!("task `{id}` deleted"));
        assert!(deleted.management.tasks.iter().all(|task| task.id != id));

        assert_eq!(
            command_records(&workspace),
            [
                "received create_task by akra-admin",
                "completed Done",
                "received update_task by akra-admin",
                "completed Done",
                "received delete_task by akra-admin",
                "completed Done",
            ]
        );
    }

    #[test]
    fn a_stale_admin_form_is_refused_and_changes_nothing() {
        // plan 18 K4-9d D2, D3: the refusal carries the reason code and the text that the user reads.
        let (workspace, admin) = admin("stale");
        let created = admin.upsert_task(task("", "Keep me", None)).unwrap();
        let id = task_id(&created, "Keep me");
        let stale = created.management.planning_revision - 1;

        for error in [
            admin
                .upsert_task(task(&id, "Changed", Some(stale)))
                .unwrap_err(),
            admin.delete_task(delete(&id, Some(stale))).unwrap_err(),
        ] {
            let refusal = PlanningAdminMutationRefusal::find_in(&error).expect("a refusal");
            assert_eq!(refusal.reason, ReasonCode::PlanningRevisionStale);
            assert!(refusal.message.contains("reload"), "{}", refusal.message);
        }

        // A delete form without the revision is not sent to the kernel.
        let missing = admin.delete_task(delete(&id, None)).unwrap_err();
        assert!(PlanningAdminMutationRefusal::find_in(&missing).is_none());
        assert!(
            missing
                .to_string()
                .contains("expected_planning_revision is required"),
            "{missing:#}"
        );

        let management = admin.load_management_view().unwrap();
        assert_eq!(
            management.planning_revision,
            created.management.planning_revision
        );
        assert_eq!(task_id(&created, "Keep me"), id);
        assert_eq!(
            command_records(&workspace)[2..],
            [
                "received update_task by akra-admin",
                "rejected planning_revision_stale",
                "received delete_task by akra-admin",
                "rejected planning_revision_stale",
            ]
        );
    }

    #[test]
    fn a_failed_admin_task_change_reports_the_cause() {
        let (_workspace, admin) = admin("failure");
        let created = admin.upsert_task(task("", "Only task", None)).unwrap();
        let error = admin
            .delete_task(delete(
                "missing-task",
                Some(created.management.planning_revision),
            ))
            .unwrap_err();
        assert!(PlanningAdminMutationRefusal::find_in(&error).is_none());
        assert!(error.to_string().contains("was not found"), "{error:#}");
    }
}
