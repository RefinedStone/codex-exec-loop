//! admin 의 planning 변경을 저장소 커널의 명령으로 보낸다 (plan 18 K4-9d,
//! 설계 `docs/plan/18-k4-9d-planning-mutations.md` §3.5).

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use super::kernel_command_execution;
use crate::adapter::inbound::kernel_protocol::{CommandResponse, CommandResult, KernelCommand};
use crate::application::port::inbound::parallel_agent_profile_port::{
    ParallelAgentProfileConfig, ParallelAgentProfilePort,
};
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
use crate::application::port::outbound::kernel_gateway_port::KernelGatewayPort;
use crate::application::service::planning::PlanningAdminFacadeService;
use crate::domain::planning::PlanningResetRequest;

/*
 * 입구(admin)의 `PlanningAdminPort` 이다. 읽기와 초안의 편집은 facade 가 직접 한다. 작업, 방향, 파일의 변경과 초안의
 * 승격은 저장 명령으로 바꿔서 저장소 커널에 보낸다. 명령은 Gateway 의 커널, 명령 전용 조립, 또는 TUI 가 커널을 쥔 저장소의 직접 경로 중
 * 하나로 간다. 어느 길이든 답은 실행이 끝난 뒤에 온다. 그 뒤에 화면의 값을 저장소에서 다시 읽는다.
 *
 * 커널 자신은 이 구현을 쓰지 않는다. 쓰면 명령이 자기에게 돌아온다 (`production::stored_command_port`).
 */
pub(crate) struct KernelRoutedPlanningAdmin {
    direct: PlanningAdminFacadeService,
    sender: StoredCommandSender,
}

impl KernelRoutedPlanningAdmin {
    pub(crate) fn new(direct: PlanningAdminFacadeService, caller: &'static str) -> Self {
        Self {
            direct,
            sender: StoredCommandSender {
                caller,
                gateway: None,
            },
        }
    }

    /// 시험이 실제 Gateway 대신 프로세스 안의 Gateway 로 보낸다.
    #[cfg(test)]
    pub(crate) fn through_gateway(mut self, gateway: Arc<dyn KernelGatewayPort>) -> Self {
        self.sender.gateway = Some(gateway);
        self
    }

    fn has_repository_kernel(&self) -> bool {
        has_repository_kernel(self.direct.workspace_dir())
    }

    fn send(&self, command: StoredCommand) -> Result<CommandResult> {
        self.sender.send(self.direct.workspace_dir(), command)
    }
}

// 커널이 있을 수 없는 폴더(대표 checkout 이 없는 저장소, 병렬 slot worktree)는 이전처럼 직접 바꾼다 (K4-9a 와 같다).
fn has_repository_kernel(workspace_dir: &str) -> bool {
    super::production::repository_kernel_target(workspace_dir).is_ok()
}

/// 입구의 저장 명령을 저장소 커널에 보낸다. `caller` 는 그 명령의 기록에 남는 입구의 이름이다.
struct StoredCommandSender {
    caller: &'static str,
    /// `None` 이면 이 컴퓨터의 Gateway 이다.
    gateway: Option<Arc<dyn KernelGatewayPort>>,
}

impl StoredCommandSender {
    /*
     * 저장 명령 하나를 보내고 그 결과를 돌려준다. 거절은 `PlanningAdminMutationRefusal` 이다: 아무것도 바뀌지
     * 않았고, 입구가 409 와 그 글로 답한다. 실패는 커널이 댄 원인의 글이다.
     */
    fn send(&self, workspace_dir: &str, command: StoredCommand) -> Result<CommandResult> {
        let port = kernel_command_execution::build_with_gateway(
            Path::new(workspace_dir),
            self.caller,
            self.gateway.clone(),
        )?;
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

fn changed_task_id(result: CommandResult) -> Result<String> {
    match result {
        CommandResult::TaskCreated { task_id, .. }
        | CommandResult::TaskUpdated { task_id, .. }
        | CommandResult::TaskDeleted { task_id, .. } => Ok(task_id),
        other => bail!("the repository kernel answered a task command with {other:?}"),
    }
}

fn changed_direction_id(result: CommandResult) -> Result<String> {
    match result {
        CommandResult::DirectionCreated { direction_id, .. }
        | CommandResult::DirectionUpdated { direction_id, .. } => Ok(direction_id),
        other => bail!("the repository kernel answered a direction command with {other:?}"),
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
        if !self.has_repository_kernel() {
            return self.direct.upsert_direction(request);
        }
        let command = PlanningAdminFacadeService::stored_direction_command(request)?;
        let updated = matches!(command, StoredCommand::UpdateDirection { .. });
        let direction_id = changed_direction_id(self.send(command)?)?;
        self.direct.direction_upsert_outcome(&direction_id, updated)
    }

    fn delete_direction(
        &self,
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        if !self.has_repository_kernel() {
            return self.direct.delete_direction(request);
        }
        let command = PlanningAdminFacadeService::stored_direction_delete_command(&request)?;
        match self.send(command)? {
            CommandResult::DirectionDeleted {
                direction_id,
                removed_task_count,
                default_retained,
                ..
            } => self.direct.direction_delete_outcome(
                &direction_id,
                removed_task_count,
                default_retained,
            ),
            other => bail!("the repository kernel answered delete_direction with {other:?}"),
        }
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
        let task_id = changed_task_id(self.send(command)?)?;
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
        let task_id = changed_task_id(self.send(command)?)?;
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
        if !self.has_repository_kernel() {
            return PlanningAdminPort::promote_draft_session(&self.direct, request);
        }
        let command = self.direct.stored_draft_promote_command(&request)?;
        let StoredCommand::PromoteDraft {
            expected_planning_revision,
            ..
        } = command
        else {
            bail!("the draft promotion did not become a promote_draft command");
        };
        match self.send(command)? {
            CommandResult::DraftPromoted {
                promoted_file_count,
                is_valid,
                planning_revision,
            } => self.direct.draft_promotion_outcome(
                request,
                expected_planning_revision,
                promoted_file_count,
                is_valid,
                planning_revision,
            ),
            other => bail!("the repository kernel answered promote_draft with {other:?}"),
        }
    }

    fn export_active_files_for_edit(&self) -> Result<PlanningAdminFileSyncOutcome> {
        if !self.has_repository_kernel() {
            return self.direct.export_active_files_for_edit();
        }
        match self.send(StoredCommand::ExportPlanningFiles)? {
            CommandResult::PlanningFilesExported { directory, paths } => {
                Ok(self.direct.file_export_outcome(&directory, paths))
            }
            other => bail!("the repository kernel answered export_planning_files with {other:?}"),
        }
    }

    fn apply_exported_files(&self) -> Result<PlanningAdminFileSyncOutcome> {
        if !self.has_repository_kernel() {
            return self.direct.apply_exported_files();
        }
        match self.send(StoredCommand::ApplyPlanningFiles)? {
            CommandResult::PlanningFilesApplied {
                directory, paths, ..
            } => Ok(self.direct.file_apply_outcome(&directory, paths)),
            other => bail!("the repository kernel answered apply_planning_files with {other:?}"),
        }
    }

    fn reset_workspace(&self, request: PlanningResetRequest) -> Result<PlanningAdminResetOutcome> {
        self.direct.reset_workspace(request)
    }
}

/*
 * 입구(admin)의 `ParallelAgentProfilePort` 이다 (plan 18 K4-9b 의 9b-A). 읽기는 직접 하고, 저장은 저장 명령
 * `save_parallel_agent_profiles` 로 저장소 커널에 보낸다. 프로필은 커널의 대표 checkout 에 저장된다.
 */
pub(crate) struct KernelRoutedParallelAgentProfiles {
    direct: Arc<dyn ParallelAgentProfilePort>,
    sender: StoredCommandSender,
}

impl KernelRoutedParallelAgentProfiles {
    pub(crate) fn new(direct: Arc<dyn ParallelAgentProfilePort>, caller: &'static str) -> Self {
        Self {
            direct,
            sender: StoredCommandSender {
                caller,
                gateway: None,
            },
        }
    }
}

impl ParallelAgentProfilePort for KernelRoutedParallelAgentProfiles {
    fn load_config(&self, workspace_dir: &str) -> Result<ParallelAgentProfileConfig, String> {
        self.direct.load_config(workspace_dir)
    }

    fn save_config(
        &self,
        workspace_dir: &str,
        config: &ParallelAgentProfileConfig,
    ) -> Result<(), String> {
        if !has_repository_kernel(workspace_dir) {
            return self.direct.save_config(workspace_dir, config);
        }
        let command = StoredCommand::SaveParallelAgentProfiles {
            config: config.clone(),
        };
        match self.sender.send(workspace_dir, command) {
            Ok(CommandResult::ParallelAgentProfilesSaved { .. }) => Ok(()),
            Ok(other) => Err(format!(
                "the repository kernel answered save_parallel_agent_profiles with {other:?}"
            )),
            Err(error) => Err(format!("{error:#}")),
        }
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
            acceptance: None,
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

    fn direction(
        id: &str,
        title: &str,
        revision: Option<i64>,
    ) -> PlanningAdminDirectionMutationRequest {
        PlanningAdminDirectionMutationRequest {
            id: id.to_string(),
            title: title.to_string(),
            summary: String::new(),
            success_criteria_text: "the release is out".to_string(),
            scope_hints_text: String::new(),
            detail_doc_path: String::new(),
            state: "active".to_string(),
            expected_planning_revision: revision.map(|value| value.to_string()).unwrap_or_default(),
        }
    }

    fn delete_direction(id: &str, revision: Option<i64>) -> PlanningAdminDirectionDeleteRequest {
        PlanningAdminDirectionDeleteRequest {
            id: id.to_string(),
            expected_planning_revision: revision.map(|value| value.to_string()).unwrap_or_default(),
        }
    }

    #[test]
    fn admin_direction_changes_are_kernel_commands_with_the_admin_caller() {
        // plan 18 K4-9d-3: every direction change of the admin leaves `command_received` with `caller: akra-admin`.
        let (workspace, admin) = admin("direction-records");
        let created = admin
            .upsert_direction(direction("", "Release", None))
            .unwrap();
        assert_eq!(created.notice, "direction `dir-release` added");

        let mut child = task("", "Ship it", None);
        child.direction_id = "dir-release".to_string();
        let with_task = admin.upsert_task(child).unwrap();

        let updated = admin
            .upsert_direction(direction(
                "dir-release",
                "Release 2",
                Some(with_task.management.planning_revision),
            ))
            .unwrap();
        assert_eq!(updated.notice, "direction `dir-release` updated");
        assert!(
            updated
                .management
                .directions
                .iter()
                .any(|direction| direction.id == "dir-release" && direction.title == "Release 2")
        );

        let retained = admin
            .delete_direction(delete_direction(
                "general-workstream",
                Some(updated.management.planning_revision),
            ))
            .unwrap();
        assert_eq!(
            retained.notice,
            "default direction `general-workstream` is retained"
        );

        let deleted = admin
            .delete_direction(delete_direction(
                "dir-release",
                Some(retained.management.planning_revision),
            ))
            .unwrap();
        assert_eq!(
            deleted.notice,
            "direction `dir-release` deleted with 1 child tasks"
        );
        assert!(deleted.management.tasks.is_empty());

        assert_eq!(
            command_records(&workspace),
            [
                "received create_direction by akra-admin",
                "completed Done",
                "received create_task by akra-admin",
                "completed Done",
                "received update_direction by akra-admin",
                "completed Done",
                "received delete_direction by akra-admin",
                "completed Done",
                "received delete_direction by akra-admin",
                "completed Done",
            ]
        );
    }

    #[test]
    fn a_stale_admin_direction_form_is_refused_and_changes_nothing() {
        let (workspace, admin) = admin("direction-stale");
        let created = admin.upsert_direction(direction("", "Keep", None)).unwrap();
        let stale = created.management.planning_revision - 1;

        for error in [
            admin
                .upsert_direction(direction("dir-keep", "Changed", Some(stale)))
                .unwrap_err(),
            admin
                .delete_direction(delete_direction("dir-keep", Some(stale)))
                .unwrap_err(),
        ] {
            let refusal = PlanningAdminMutationRefusal::find_in(&error).expect("a refusal");
            assert_eq!(refusal.reason, ReasonCode::PlanningRevisionStale);
            assert!(refusal.message.contains("reload"), "{}", refusal.message);
        }

        // A form without the revision is not sent to the kernel.
        for missing in [
            admin
                .upsert_direction(direction("dir-keep", "Changed", None))
                .unwrap_err(),
            admin
                .delete_direction(delete_direction("dir-keep", None))
                .unwrap_err(),
        ] {
            assert!(PlanningAdminMutationRefusal::find_in(&missing).is_none());
            assert!(
                missing
                    .to_string()
                    .contains("expected_planning_revision is required"),
                "{missing:#}"
            );
        }

        // An unknown id is a failure with its cause. It does not become a new direction.
        let unknown = admin
            .upsert_direction(direction(
                "dir-typo",
                "Typo",
                Some(created.management.planning_revision),
            ))
            .unwrap_err();
        assert!(PlanningAdminMutationRefusal::find_in(&unknown).is_none());
        assert!(unknown.to_string().contains("was not found"), "{unknown:#}");

        let management = admin.load_management_view().unwrap();
        assert_eq!(
            management.planning_revision,
            created.management.planning_revision
        );
        assert!(
            management
                .directions
                .iter()
                .any(|direction| direction.id == "dir-keep" && direction.title == "Keep")
        );
        assert_eq!(
            command_records(&workspace)[2..6],
            [
                "received update_direction by akra-admin",
                "rejected planning_revision_stale",
                "received delete_direction by akra-admin",
                "rejected planning_revision_stale",
            ]
        );
    }

    // File sync writes files beside the repository. It is unavailable on Windows.
    #[cfg(not(windows))]
    #[test]
    fn admin_file_sync_runs_as_kernel_commands_and_a_stale_export_is_refused() {
        // plan 18 K4-9d-4.
        let (workspace, admin) = admin("file-sync");
        let exported = admin.export_active_files_for_edit().unwrap();
        assert_eq!(
            exported.notice,
            "exported 1 planning support files for editing"
        );
        let file = std::path::Path::new(&workspace.0).join(&exported.paths[0]);
        let body = std::fs::read_to_string(&file).unwrap();
        std::fs::write(&file, format!("{body}\nAn edit by the operator.\n")).unwrap();

        let applied = admin.apply_exported_files().unwrap();
        assert_eq!(applied.notice, "applied 1 exported planning paths");
        assert_eq!(applied.paths, exported.paths);

        // Planning changes after the next export. The export recorded the older revision.
        admin.export_active_files_for_edit().unwrap();
        admin.upsert_task(task("", "A newer change", None)).unwrap();
        let revision = admin.load_management_view().unwrap().planning_revision;
        let stale = admin.apply_exported_files().unwrap_err();
        let refusal = PlanningAdminMutationRefusal::find_in(&stale).expect("a refusal");
        assert_eq!(refusal.reason, ReasonCode::PlanningRevisionStale);
        assert_eq!(
            admin.load_management_view().unwrap().planning_revision,
            revision
        );

        assert_eq!(
            command_records(&workspace),
            [
                "received export_planning_files by akra-admin",
                "completed Done",
                "received apply_planning_files by akra-admin",
                "completed Done",
                "received export_planning_files by akra-admin",
                "completed Done",
                "received create_task by akra-admin",
                "completed Done",
                "received apply_planning_files by akra-admin",
                "rejected planning_revision_stale",
            ]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn an_apply_without_an_export_fails_with_its_cause() {
        let (_workspace, admin) = admin("file-sync-missing");
        let error = admin.apply_exported_files().unwrap_err();
        assert!(PlanningAdminMutationRefusal::find_in(&error).is_none());
        assert!(
            error.to_string().contains("export again before applying"),
            "{error:#}"
        );
    }

    fn run_git(directory: &std::path::Path, args: &[&str]) {
        let output = crate::test_utils::git_command()
            .current_dir(directory)
            .args(args)
            .output()
            .expect("git should run");
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    #[cfg(not(windows))]
    #[test]
    fn an_export_from_another_worktree_writes_into_the_primary_checkout() {
        // plan 18 K4-9d-4: the executor uses the primary checkout of the kernel, not the folder of the admin.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "kernel-routed-admin-linked-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("primary")).unwrap();
        let root = Workspace(root.canonicalize().unwrap().to_string_lossy().into_owned());
        let primary = std::path::Path::new(&root.0).join("primary");
        let linked = std::path::Path::new(&root.0).join("linked");
        run_git(&primary, &["init", "-q"]);
        run_git(
            &primary,
            &[
                "-c",
                "user.name=akra",
                "-c",
                "user.email=akra@localhost.invalid",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        );
        run_git(
            &primary,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "linked",
                &linked.display().to_string(),
            ],
        );
        let admin =
            super::super::production::build_admin_application(linked.display().to_string()).facade;

        let exported = admin.export_active_files_for_edit().unwrap();

        assert_eq!(
            exported.notice,
            format!(
                "exported 1 planning support files for editing in `{}`",
                primary.display()
            )
        );
        assert!(primary.join(&exported.paths[0]).is_file());
        assert!(!linked.join(&exported.paths[0]).exists());
        let applied = admin.apply_exported_files().unwrap();
        assert!(
            applied
                .notice
                .ends_with(&format!("in `{}`", primary.display())),
            "{}",
            applied.notice
        );
    }

    #[test]
    fn admin_draft_promotion_is_a_kernel_command_and_a_stale_draft_is_refused() {
        // plan 18 K4-9d-5: the admin sends the revision of the draft from its memory (Q-9d-3).
        let (workspace, admin) = admin("promote");
        // The draft editors need the repository-scoped store, so the workspace is a git repository.
        run_git(std::path::Path::new(&workspace.0), &["init", "-q"]);
        let draft = admin
            .create_draft_session(PlanningAdminDraftKind::FullPlanning, None)
            .unwrap();
        let request = |draft_name: &str| PlanningAdminDraftMutationRequest {
            draft_name: draft_name.to_string(),
            kind: PlanningAdminDraftKind::FullPlanning,
            direction_id: None,
            files: Vec::new(),
        };
        let promoted = admin
            .promote_draft_session(request(&draft.draft_name))
            .unwrap();
        assert!(promoted.is_valid);
        assert_eq!(promoted.promoted_file_count, 1);

        // The admin moved its memory to the new revision, so the same draft promotes again.
        admin
            .promote_draft_session(request(&draft.draft_name))
            .unwrap();

        let stale_draft = admin
            .create_draft_session(PlanningAdminDraftKind::FullPlanning, None)
            .unwrap();
        admin.upsert_task(task("", "A newer change", None)).unwrap();
        let revision = admin.load_management_view().unwrap().planning_revision;
        let stale = admin
            .promote_draft_session(request(&stale_draft.draft_name))
            .unwrap_err();
        let refusal = PlanningAdminMutationRefusal::find_in(&stale).expect("a refusal");
        assert_eq!(refusal.reason, ReasonCode::PlanningRevisionStale);
        assert!(refusal.message.contains("reload"), "{}", refusal.message);
        assert_eq!(
            admin.load_management_view().unwrap().planning_revision,
            revision
        );

        assert_eq!(
            command_records(&workspace),
            [
                "received promote_draft by akra-admin",
                "completed Done",
                "received promote_draft by akra-admin",
                "completed Done",
                "received create_task by akra-admin",
                "completed Done",
                "received promote_draft by akra-admin",
                "rejected planning_revision_stale",
            ]
        );
    }

    // The profile file is written beside the repository. It is unavailable on Windows.
    #[cfg(not(windows))]
    #[test]
    fn admin_profile_save_is_a_kernel_command() {
        // plan 18 K4-9b 9b-A.
        let (workspace, _admin) = admin("profiles");
        let profiles = super::super::production::build_admin_application(workspace.0.clone())
            .parallel_agent_profile_port;
        let mut config = profiles.load_config(&workspace.0).unwrap();
        config.profiles.truncate(1);
        config.profiles[0].display_name = "Only agent".to_string();

        profiles.save_config(&workspace.0, &config).unwrap();

        let saved = profiles.load_config(&workspace.0).unwrap();
        assert_eq!(saved.profiles.len(), 1);
        assert_eq!(saved.profiles[0].display_name, "Only agent");
        assert_eq!(
            command_records(&workspace),
            [
                "received save_parallel_agent_profiles by akra-admin",
                "completed Done",
            ]
        );
    }
}
