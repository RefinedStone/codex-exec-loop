// File sync는 admin facade가 accepted planning 문서를 workspace의 편집 가능한 파일로 내보내고 다시 읽어오는 경로다.
// 실제 쓰기/읽기는 workspace port를 통해 수행하고, planning 저장소 갱신은 facade helper에 맡긴다.
// Admin API는 실패 원인을 operator에게 그대로 보여 주므로 `Context`로 어느 파일 작업이 실패했는지 붙이고,
// parallel busy guard는 `bail!`로 즉시 중단한다.
use anyhow::{Context, Result, anyhow};
use rand::RngCore;

// 이 파일은 `PlanningAdminFacadeService`에 export/apply 동작을 붙인다. 반환 outcome은 admin page/API가
// notice와 영향을 받은 path 목록을 표시하는 데 쓰는 얇은 DTO다.
use super::{PlanningAdminFacadeService, PlanningAdminFileSyncOutcome};
// Runtime projection snapshot에는 parallel slot lease와 distributor queue 상태가 함께 들어 있다. file sync는
// accepted 파일을 직접 덮어쓸 수 있으므로 이 snapshot으로 병렬 작업 중 여부를 먼저 검사한다.
#[cfg(test)]
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot;
// 현재 admin file sync의 대상은 operator planning documents 중 result-output markdown이다. 상수를 써서 export
// 경로, apply 경로, notice path가 planning service 전체의 canonical path와 일치하게 한다.
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;

// 이 impl은 admin facade의 파일 기반 편집 워크플로우를 담당한다. draft staging API와 달리
// "현재 accepted support file을 workspace에 꺼내 고친 뒤 다시 accepted documents에 반영"하는 운영자용 우회로다.
impl PlanningAdminFacadeService {
    // Accepted planning support file을 workspace 파일로 export한다. parallel worker가 같은 planning authority를
    // 수정 중이면 stale file을 내보낼 수 있으므로 guard를 먼저 통과해야 한다.
    pub fn export_active_files_for_edit(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.ensure_default_authority()?;
        self.with_file_sync_guard("export planning support files", |_owner_token| {
            let documents = self.load_operator_planning_documents()?;
            self.planning_workspace_port
                .export_planning_file_sync_candidate(
                    self.workspace_dir.as_str(),
                    RESULT_OUTPUT_FILE_PATH,
                    &documents.result_output_markdown,
                    documents.observed_planning_revision,
                )
                .with_context(|| format!("failed to export {RESULT_OUTPUT_FILE_PATH}"))?;
            let paths = vec![RESULT_OUTPUT_FILE_PATH.to_string()];
            Ok(PlanningAdminFileSyncOutcome {
                notice: format!(
                    "exported {} planning support files for editing",
                    paths.len()
                ),
                paths,
            })
        })
    }

    // workspace에 export된 파일을 다시 accepted operator documents로 적용한다. 이 경로는 draft validation/promotion이
    // 아니라 admin이 직접 support file을 동기화하는 명령이므로 missing file을 오류로 본다.
    pub fn apply_exported_files(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.ensure_default_authority()?;
        self.with_file_sync_guard("apply exported planning support files", |owner_token| {
            let exported = self
                .planning_workspace_port
                .load_planning_file_sync_candidate(
                    self.workspace_dir.as_str(),
                    RESULT_OUTPUT_FILE_PATH,
                )?
                .ok_or_else(|| anyhow!("missing exported file: {RESULT_OUTPUT_FILE_PATH}"))?;
            let mut documents = self.load_operator_planning_documents()?;
            documents.result_output_markdown = exported.body;
            documents.observed_planning_revision = exported.observed_planning_revision;
            self.commit_operator_planning_documents_with_guard(documents, owner_token)?;
            let paths = vec![RESULT_OUTPUT_FILE_PATH.to_string()];
            Ok(PlanningAdminFileSyncOutcome {
                notice: format!("applied {} exported planning paths", paths.len()),
                paths,
            })
        })
    }

    fn with_file_sync_guard<T>(
        &self,
        action: &str,
        operation: impl FnOnce(&str) -> Result<T>,
    ) -> Result<T> {
        let mut token = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut token);
        let owner_token = token
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.planning_authority_port
            .acquire_admin_authority_mutation_guard(
                self.workspace_dir.as_str(),
                &owner_token,
                action,
            )?;
        let result = operation(&owner_token);
        let release = self
            .planning_authority_port
            .release_admin_authority_mutation_guard(self.workspace_dir.as_str(), &owner_token);
        match (result, release) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(error)) => Err(error.context(
                "planning file sync completed but its runtime exclusion guard could not be released",
            )),
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(release_error)) => Err(anyhow!(
                "planning file sync failed: {error:#}; runtime exclusion guard release also failed: {release_error:#}"
            )),
        }
    }
}

// Parallel busy 설명은 guard의 정책을 문자열로 낮추는 helper다. lease를 먼저 검사하는 이유는 이미 실행/정리
// 중인 slot이 queue record보다 accepted 파일 충돌 위험을 더 직접적으로 나타내기 때문이다.
#[cfg(test)]
fn describe_parallel_busy(runtime: &PlanningAuthorityRuntimeProjectionSnapshot) -> Option<String> {
    // Leased/Running/CleanupPending은 모두 file sync가 끼어들면 안 되는 상태다. cleanup도 아직 authority state를
    // 정리하는 중일 수 있어 완료된 슬롯으로 취급하지 않는다.
    if let Some(lease) = runtime.slot_leases.values().find(|lease| {
        matches!(
            lease.state,
            crate::domain::parallel_mode::ParallelModeSlotLeaseState::Leased
                | crate::domain::parallel_mode::ParallelModeSlotLeaseState::Running
                | crate::domain::parallel_mode::ParallelModeSlotLeaseState::CleanupPending
        )
    }) {
        // slot id, state label, task id를 모두 넣어 operator가 어떤 병렬 lane을 기다리거나 정리해야 하는지 알 수 있게 한다.
        return Some(format!(
            "slot {} is {} for task {}",
            lease.slot_id,
            lease.state.label(),
            lease.task_id
        ));
    }
    // lease가 없더라도 distributor queue에 active record가 있으면 곧 slot 작업으로 이어질 수 있다. 이 경우에도
    // export/apply가 stale authority state를 기준으로 움직일 수 있어 차단한다.
    if let Some(record) = runtime
        .distributor_queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
    {
        // queue item id와 task id를 노출해 아직 lease로 승격되지 않은 작업도 operator가 추적할 수 있게 한다.
        return Some(format!(
            "distributor item {} is {} for task {}",
            record.queue_item_id,
            record.queue_state.label(),
            record.task_id
        ));
    }
    None
}

#[cfg(test)]
fn write_candidate_file(
    workspace_dir: &str,
    relative_path: &str,
    body: &str,
    written_paths: &mut Vec<String>,
) -> Result<()> {
    let path = std::path::Path::new(workspace_dir).join(relative_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    written_paths.push(relative_path.to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    #[cfg(not(windows))]
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_authority_port::{
        PlanningAuthorityDistributorQueueRecord, PlanningAuthorityPort,
        PlanningAuthorityRuntimeProjectionSnapshot,
    };
    use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningFileSyncCandidateRecord, PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning::PlanningServices;
    use crate::domain::parallel_mode::{
        ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
    };

    #[cfg(not(windows))]
    #[test]
    fn export_and_apply_round_trip_result_output_through_workspace_file() {
        let fixture = TestAdminFixture::new("admin-file-sync-round-trip");
        let accepted_body = "# Result Output\n\nAccepted admin copy.".to_string();
        let edited_body = "# Result Output\n\nEdited from exported workspace file.".to_string();
        let mut documents = fixture
            .facade
            .load_operator_planning_documents()
            .expect("seeded documents should load");
        documents.result_output_markdown = accepted_body.clone();
        fixture
            .facade
            .commit_operator_planning_documents(documents)
            .expect("accepted result output should commit");

        let exported = fixture
            .facade
            .export_active_files_for_edit()
            .expect("active support files should export");
        let exported_path = Path::new(&fixture.workspace.path).join(RESULT_OUTPUT_FILE_PATH);

        assert_eq!(
            exported.notice,
            "exported 1 planning support files for editing"
        );
        assert_eq!(exported.paths, vec![RESULT_OUTPUT_FILE_PATH.to_string()]);
        assert_eq!(
            fs::read_to_string(&exported_path).expect("exported file should be readable"),
            accepted_body
        );

        fs::write(&exported_path, &edited_body).expect("operator edit should write");
        let applied = fixture
            .facade
            .apply_exported_files()
            .expect("exported support file should apply");
        let reloaded = fixture
            .facade
            .load_operator_planning_documents()
            .expect("documents should reload after apply");

        assert_eq!(applied.notice, "applied 1 exported planning paths");
        assert_eq!(applied.paths, vec![RESULT_OUTPUT_FILE_PATH.to_string()]);
        assert_eq!(reloaded.result_output_markdown, edited_body);
    }

    #[cfg(not(windows))]
    #[test]
    fn git_backed_file_sync_uses_candidate_file_without_mutating_active_db_on_export() {
        let workspace = TempPlanningWorkspace::new("admin-file-sync-git-backed");
        let output = Command::new("git")
            .args(["init", "-q", &workspace.path])
            .output()
            .expect("git init should run");
        assert!(output.status.success(), "git init should succeed");
        let sqlite = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let workspace_port: Arc<dyn PlanningWorkspacePort> = Arc::new(
            FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(sqlite.clone()),
        );
        let facade = build_facade_with_sqlite(workspace.path.clone(), workspace_port, sqlite);
        let accepted_body = "# Result Output\n\nAccepted DB body.";
        let edited_body = "# Result Output\n\nEdited candidate body.";
        let mut documents = facade
            .load_operator_planning_documents()
            .expect("documents should load");
        documents.result_output_markdown = accepted_body.to_string();
        facade
            .commit_operator_planning_documents(documents)
            .expect("accepted DB body should commit");

        facade
            .export_active_files_for_edit()
            .expect("git-backed candidate should export");
        assert_eq!(
            facade
                .load_operator_planning_documents()
                .expect("active DB body should reload")
                .result_output_markdown,
            accepted_body,
            "export must not write through to active_documents"
        );
        let candidate = Path::new(&workspace.path).join(RESULT_OUTPUT_FILE_PATH);
        assert_eq!(fs::read_to_string(&candidate).unwrap(), accepted_body);
        let manifest = Path::new(&workspace.path)
            .join(".codex-exec-loop/planning/.akra-file-sync-result-output.json");
        assert!(
            !manifest.exists(),
            "git-backed export baseline must stay in the repo-scoped DB"
        );
        let status = Command::new("git")
            .args([
                "-C",
                &workspace.path,
                "status",
                "--porcelain",
                "--untracked-files=all",
            ])
            .output()
            .expect("git status should run");
        assert!(status.status.success());
        assert!(
            !String::from_utf8_lossy(&status.stdout).contains(".akra-file-sync-result-output.json")
        );

        fs::write(&candidate, edited_body).expect("candidate edit should write");
        facade
            .apply_exported_files()
            .expect("candidate should apply to DB authority");
        assert_eq!(
            facade
                .load_operator_planning_documents()
                .expect("applied DB body should reload")
                .result_output_markdown,
            edited_body
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn stale_export_cannot_overwrite_newer_planning_authority() {
        let fixture = TestAdminFixture::new("admin-file-sync-stale-export");
        let mut documents = fixture
            .facade
            .load_operator_planning_documents()
            .expect("documents should load");
        documents.result_output_markdown = "# Result Output\n\nExport baseline.".to_string();
        fixture
            .facade
            .commit_operator_planning_documents(documents)
            .expect("baseline should commit");
        fixture
            .facade
            .export_active_files_for_edit()
            .expect("baseline should export");

        let mut newer = fixture
            .facade
            .load_operator_planning_documents()
            .expect("newer documents should load");
        newer.result_output_markdown = "# Result Output\n\nNewer accepted body.".to_string();
        fixture
            .facade
            .commit_operator_planning_documents(newer)
            .expect("newer authority should commit");
        fs::write(
            Path::new(&fixture.workspace.path).join(RESULT_OUTPUT_FILE_PATH),
            "# Result Output\n\nStale operator edit.",
        )
        .expect("stale candidate edit should write");

        let error = fixture
            .facade
            .apply_exported_files()
            .expect_err("stale export must lose the revision CAS");
        assert!(
            error
                .to_string()
                .contains("planning db changed while editing")
        );
        assert_eq!(
            fixture
                .facade
                .load_operator_planning_documents()
                .expect("newer authority should remain")
                .result_output_markdown,
            "# Result Output\n\nNewer accepted body."
        );
    }

    #[test]
    fn export_and_apply_round_trip_result_output_through_workspace_port() {
        let workspace = TempPlanningWorkspace::new("admin-file-sync-port-round-trip");
        let workspace_port = Arc::new(PortBackedResultOutputWorkspacePort::new(
            "# Result Output\n\nSeeded workspace copy.",
        ));
        let (facade, _) = build_facade(workspace.path.clone(), workspace_port.clone());
        let accepted_body = "# Result Output\n\nAccepted admin copy.".to_string();
        let edited_body = "# Result Output\n\nEdited through workspace port.".to_string();

        let mut documents = facade
            .load_operator_planning_documents()
            .expect("seeded documents should load");
        documents.result_output_markdown = accepted_body.clone();
        facade
            .commit_operator_planning_documents(documents)
            .expect("accepted result output should commit through workspace port");
        let replace_count_before_export = workspace_port.replace_call_count();

        let exported = facade
            .export_active_files_for_edit()
            .expect("active support files should export through workspace port");
        assert_eq!(
            exported.notice,
            "exported 1 planning support files for editing"
        );
        assert_eq!(exported.paths, vec![RESULT_OUTPUT_FILE_PATH.to_string()]);
        assert_eq!(
            workspace_port.current_candidate_result_output().as_deref(),
            Some(accepted_body.as_str())
        );
        assert_eq!(
            workspace_port.replace_call_count(),
            replace_count_before_export + 1,
            "export should materialize the active file through the workspace port"
        );

        workspace_port.set_result_output(&edited_body);
        let applied = facade
            .apply_exported_files()
            .expect("workspace-port export should apply back into authority");
        let reloaded = facade
            .load_operator_planning_documents()
            .expect("documents should reload after workspace-port apply");

        assert_eq!(applied.notice, "applied 1 exported planning paths");
        assert_eq!(applied.paths, vec![RESULT_OUTPUT_FILE_PATH.to_string()]);
        assert_eq!(reloaded.result_output_markdown, edited_body);
    }

    #[test]
    fn apply_exported_files_rejects_missing_workspace_file() {
        let workspace = TempPlanningWorkspace::new("admin-file-sync-missing-export");
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(MissingResultOutputWorkspacePort);
        let (facade, _) = build_facade(workspace.path.clone(), workspace_port);

        let error = facade
            .apply_exported_files()
            .expect_err("missing exported file should fail");

        assert_eq!(
            error.to_string(),
            format!("missing exported file: {RESULT_OUTPUT_FILE_PATH}")
        );
    }

    #[test]
    fn export_is_blocked_by_active_slot_lease() {
        let fixture = TestAdminFixture::new("admin-file-sync-slot-busy");
        fixture
            .authority_port
            .upsert_runtime_slot_lease(
                &fixture.workspace.path,
                &slot_lease("slot-1", "task-busy", ParallelModeSlotLeaseState::Running),
            )
            .expect("busy slot lease should persist");

        let error = fixture
            .facade
            .export_active_files_for_edit()
            .expect_err("active lease should block export");

        assert_eq!(
            error.to_string(),
            "export planning support files is blocked while parallel work is active: slot slot-1 is running for task task-busy"
        );
    }

    #[test]
    fn apply_is_blocked_by_active_distributor_queue_record() {
        let fixture = TestAdminFixture::new("admin-file-sync-queue-busy");
        fixture
            .authority_port
            .upsert_runtime_distributor_queue_record(
                &fixture.workspace.path,
                &queue_record(
                    "queue-1",
                    "task-queued",
                    ParallelModeQueueItemState::MergePending,
                ),
            )
            .expect("active distributor record should persist");

        let error = fixture
            .facade
            .apply_exported_files()
            .expect_err("active distributor queue should block apply");

        assert_eq!(
            error.to_string(),
            "apply exported planning support files is blocked while parallel work is active: distributor item queue-1 is merge pending for task task-queued"
        );
    }

    #[test]
    fn file_sync_guard_blocks_late_runtime_lease_and_queue_projection() {
        let fixture = TestAdminFixture::new("admin-file-sync-guard-first");
        fixture
            .authority_port
            .acquire_admin_file_sync_guard(
                &fixture.workspace.path,
                "file-sync-owner",
                "export planning support files",
            )
            .expect("idle runtime should admit file sync guard");

        let lease_error = fixture
            .authority_port
            .upsert_runtime_slot_lease(
                &fixture.workspace.path,
                &slot_lease("slot-late", "task-late", ParallelModeSlotLeaseState::Leased),
            )
            .expect_err("late lease must lose to file sync guard");
        assert!(
            lease_error
                .to_string()
                .contains("admin authority mutation guard")
        );
        let queue_error = fixture
            .authority_port
            .upsert_runtime_distributor_queue_record(
                &fixture.workspace.path,
                &queue_record(
                    "queue-late",
                    "task-late",
                    ParallelModeQueueItemState::Queued,
                ),
            )
            .expect_err("late queue projection must lose to file sync guard");
        assert!(
            queue_error
                .to_string()
                .contains("admin authority mutation guard")
        );

        fixture
            .authority_port
            .release_admin_file_sync_guard(&fixture.workspace.path, "file-sync-owner")
            .expect("file sync guard should release");
        fixture
            .authority_port
            .upsert_runtime_slot_lease(
                &fixture.workspace.path,
                &slot_lease("slot-late", "task-late", ParallelModeSlotLeaseState::Leased),
            )
            .expect("runtime projection should resume after release");
    }

    #[test]
    fn describe_parallel_busy_ignores_empty_and_terminal_runtime_state() {
        let runtime = PlanningAuthorityRuntimeProjectionSnapshot {
            distributor_queue_records: vec![
                queue_record("queue-idle", "task-idle", ParallelModeQueueItemState::Idle),
                queue_record("queue-done", "task-done", ParallelModeQueueItemState::Done),
                queue_record(
                    "queue-failed",
                    "task-failed",
                    ParallelModeQueueItemState::Failed,
                ),
            ],
            ..PlanningAuthorityRuntimeProjectionSnapshot::default()
        };

        assert_eq!(describe_parallel_busy(&runtime), None);
    }

    #[test]
    fn describe_parallel_busy_reports_cleanup_pending_lease_before_queue() {
        let runtime = PlanningAuthorityRuntimeProjectionSnapshot {
            slot_leases: BTreeMap::from([(
                "slot-cleanup".to_string(),
                slot_lease(
                    "slot-cleanup",
                    "task-cleanup",
                    ParallelModeSlotLeaseState::CleanupPending,
                ),
            )]),
            distributor_queue_records: vec![queue_record(
                "queue-active",
                "task-active",
                ParallelModeQueueItemState::Queued,
            )],
            ..PlanningAuthorityRuntimeProjectionSnapshot::default()
        };

        assert_eq!(
            describe_parallel_busy(&runtime),
            Some("slot slot-cleanup is cleanup_pending for task task-cleanup".to_string())
        );
    }

    #[test]
    fn write_candidate_file_reports_directory_creation_failures() {
        let blocking_path = unique_temp_path("admin-file-sync-blocking-file");
        fs::write(&blocking_path, "not a directory").expect("blocking file should write");
        let workspace_dir = blocking_path.display().to_string();
        let mut written_paths = Vec::new();

        let error = write_candidate_file(
            &workspace_dir,
            RESULT_OUTPUT_FILE_PATH,
            "# Result Output\n\nBody.",
            &mut written_paths,
        )
        .expect_err("file workspace root should block parent directory creation");

        assert!(error.to_string().contains("failed to create"));
        assert!(written_paths.is_empty());
        let _ = fs::remove_file(blocking_path);
    }

    #[test]
    fn write_candidate_file_reports_file_write_failures() {
        let workspace = TempPlanningWorkspace::new("admin-file-sync-write-failure");
        let blocking_relative_path = "already-a-directory";
        let blocking_path = Path::new(&workspace.path).join(blocking_relative_path);
        fs::create_dir_all(&blocking_path).expect("blocking directory should be created");
        let mut written_paths = Vec::new();

        let error = write_candidate_file(
            &workspace.path,
            blocking_relative_path,
            "# Result Output\n\nBody.",
            &mut written_paths,
        )
        .expect_err("directory target should block file write");

        assert!(error.to_string().contains("failed to write"));
        assert!(written_paths.is_empty());
    }

    fn build_facade(
        workspace_dir: String,
        workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> (PlanningAdminFacadeService, Arc<dyn PlanningAuthorityPort>) {
        let sqlite = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let facade = build_facade_with_sqlite(workspace_dir, workspace_port, sqlite.clone());
        let authority_port: Arc<dyn PlanningAuthorityPort> = sqlite;
        (facade, authority_port)
    }

    fn build_facade_with_sqlite(
        workspace_dir: String,
        workspace_port: Arc<dyn PlanningWorkspacePort>,
        sqlite: Arc<SqlitePlanningAuthorityAdapter>,
    ) -> PlanningAdminFacadeService {
        let authority_port: Arc<dyn PlanningAuthorityPort> = sqlite.clone();
        let task_repository_port: Arc<dyn PlanningTaskRepositoryPort> = sqlite;
        let planning = PlanningServices::from_ports(
            workspace_port.clone(),
            authority_port.clone(),
            task_repository_port.clone(),
            Arc::new(NoopPlanningWorkerPort),
        );
        PlanningAdminFacadeService::from_planning_with_authority(
            workspace_dir,
            planning,
            workspace_port,
            authority_port,
            task_repository_port,
        )
    }

    fn slot_lease(
        slot_id: &str,
        task_id: &str,
        state: ParallelModeSlotLeaseState,
    ) -> ParallelModeSlotLeaseSnapshot {
        ParallelModeSlotLeaseSnapshot::new(
            slot_id,
            task_id,
            format!("Task {task_id}"),
            "agent-1",
            format!("akra-agent/{slot_id}/{task_id}"),
            "/tmp/worktree",
            state,
            "2026-05-12T00:00:00+00:00",
            Some("2026-05-12T00:01:00+00:00".to_string()),
        )
    }

    fn queue_record(
        queue_item_id: &str,
        task_id: &str,
        queue_state: ParallelModeQueueItemState,
    ) -> PlanningAuthorityDistributorQueueRecord {
        PlanningAuthorityDistributorQueueRecord {
            queue_item_id: queue_item_id.to_string(),
            queue_order_key: 1,
            session_key: "slot-1@2026-05-12T00:00:00+00:00".to_string(),
            slot_id: "slot-1".to_string(),
            agent_id: "agent-1".to_string(),
            task_id: task_id.to_string(),
            task_title: format!("Task {task_id}"),
            delivery_target: None,
            source_branch: "prerelease".to_string(),
            source_base_commit_sha: "base".to_string(),
            source_commit_sha: "source".to_string(),
            branch_name: format!("akra-agent/slot-1/{task_id}"),
            worktree_path: "/tmp/worktree".to_string(),
            commit_sha: "commit".to_string(),
            original_commit_sha: None,
            planning_refresh_state: "complete".to_string(),
            integration_state: "queued".to_string(),
            integration_base_commit_sha: None,
            integration_commit_sha: None,
            conflict_files: Vec::new(),
            recovery_note: None,
            validation_summary: "validation unavailable".to_string(),
            authority_refresh_outcome: "not refreshed".to_string(),
            github_capabilities: None,
            pull_request_number: None,
            pull_request_url: None,
            queue_state,
            integration_note: "queued".to_string(),
            enqueued_at: "2026-05-12T00:00:00+00:00".to_string(),
            updated_at: "2026-05-12T00:00:00+00:00".to_string(),
            retry_attempts: 0,
            retry_not_before: None,
        }
    }

    struct TestAdminFixture {
        workspace: TempPlanningWorkspace,
        facade: PlanningAdminFacadeService,
        authority_port: Arc<dyn PlanningAuthorityPort>,
    }

    impl TestAdminFixture {
        fn new(prefix: &str) -> Self {
            let workspace = TempPlanningWorkspace::new(prefix);
            let workspace_port: Arc<dyn PlanningWorkspacePort> =
                Arc::new(FilesystemPlanningWorkspaceAdapter::new());
            let (facade, authority_port) = build_facade(workspace.path.clone(), workspace_port);
            Self {
                workspace,
                facade,
                authority_port,
            }
        }
    }

    struct TempPlanningWorkspace {
        path: String,
    }

    impl TempPlanningWorkspace {
        fn new(prefix: &str) -> Self {
            let path = unique_temp_path(prefix);
            fs::create_dir_all(&path).expect("temp planning workspace should be created");
            Self {
                path: path.display().to_string(),
            }
        }
    }

    impl Drop for TempPlanningWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn unique_temp_path(prefix: &str) -> std::path::PathBuf {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"))
    }

    struct PortBackedResultOutputWorkspacePort {
        accepted_result_output_markdown: Mutex<Option<String>>,
        candidate_result_output_markdown: Mutex<Option<String>>,
        candidate_planning_revision: Mutex<Option<i64>>,
        replace_calls: Mutex<usize>,
    }

    impl PortBackedResultOutputWorkspacePort {
        fn new(initial_body: &str) -> Self {
            Self {
                accepted_result_output_markdown: Mutex::new(Some(initial_body.to_string())),
                candidate_result_output_markdown: Mutex::new(None),
                candidate_planning_revision: Mutex::new(None),
                replace_calls: Mutex::new(0),
            }
        }

        fn current_result_output(&self) -> Option<String> {
            self.accepted_result_output_markdown
                .lock()
                .expect("workspace port state should not be poisoned")
                .clone()
        }

        fn current_candidate_result_output(&self) -> Option<String> {
            self.candidate_result_output_markdown
                .lock()
                .expect("workspace port candidate state should not be poisoned")
                .clone()
        }

        fn replace_call_count(&self) -> usize {
            *self
                .replace_calls
                .lock()
                .expect("workspace port state should not be poisoned")
        }

        fn set_result_output(&self, body: &str) {
            *self
                .candidate_result_output_markdown
                .lock()
                .expect("workspace port candidate state should not be poisoned") =
                Some(body.to_string());
        }
    }

    impl PlanningWorkspacePort for PortBackedResultOutputWorkspacePort {
        fn export_planning_file_sync_candidate(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
            body: &str,
            observed_planning_revision: Option<i64>,
        ) -> Result<String> {
            if relative_path != RESULT_OUTPUT_FILE_PATH {
                return Err(anyhow!("unexpected file-sync path"));
            }
            *self
                .candidate_result_output_markdown
                .lock()
                .expect("workspace port candidate state should not be poisoned") =
                Some(body.to_string());
            *self
                .candidate_planning_revision
                .lock()
                .expect("workspace port candidate revision should not be poisoned") =
                observed_planning_revision;
            *self
                .replace_calls
                .lock()
                .expect("workspace port state should not be poisoned") += 1;
            Ok(relative_path.to_string())
        }

        fn load_planning_file_sync_candidate(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<PlanningFileSyncCandidateRecord>> {
            if relative_path != RESULT_OUTPUT_FILE_PATH {
                return Err(anyhow!("unexpected file-sync path"));
            }
            Ok(self
                .current_candidate_result_output()
                .map(|body| PlanningFileSyncCandidateRecord {
                    body,
                    observed_planning_revision: *self
                        .candidate_planning_revision
                        .lock()
                        .expect("workspace port candidate revision should not be poisoned"),
                }))
        }

        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            Err(anyhow::anyhow!(
                "stage_planning_draft_files should not be called"
            ))
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            Err(anyhow::anyhow!(
                "load_planning_draft_files should not be called"
            ))
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow::anyhow!(
                "replace_planning_draft_file should not be called"
            ))
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(PlanningWorkspaceLoadRecord {
                result_output_markdown: self.current_result_output(),
            })
        }

        fn load_planning_workspace_candidate_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Err(anyhow::anyhow!(
                "load_planning_workspace_candidate_files should not be called"
            ))
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            *self
                .accepted_result_output_markdown
                .lock()
                .expect("workspace port state should not be poisoned") =
                record.result_output_markdown.clone();
            Ok(())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            if relative_path == RESULT_OUTPUT_FILE_PATH {
                return Ok(self.current_result_output());
            }
            Ok(Some("# Supplemental Prompt\n\nExisting.".to_string()))
        }

        fn load_optional_planning_candidate_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            if relative_path == RESULT_OUTPUT_FILE_PATH {
                return Ok(self.current_candidate_result_output());
            }
            Ok(Some("# Supplemental Prompt\n\nExisting.".to_string()))
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
            body: Option<&str>,
        ) -> Result<()> {
            if relative_path == RESULT_OUTPUT_FILE_PATH {
                *self
                    .candidate_result_output_markdown
                    .lock()
                    .expect("workspace port candidate state should not be poisoned") =
                    body.map(str::to_string);
                *self
                    .replace_calls
                    .lock()
                    .expect("workspace port state should not be poisoned") += 1;
                return Ok(());
            }
            Err(anyhow::anyhow!(
                "replace_planning_workspace_file should only be called for result-output"
            ))
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<()> {
            Err(anyhow::anyhow!(
                "remove_planning_workspace_entry should not be called"
            ))
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow::anyhow!(
                "archive_rejected_planning_file should not be called"
            ))
        }
    }

    struct MissingResultOutputWorkspacePort;

    impl PlanningWorkspacePort for MissingResultOutputWorkspacePort {
        fn load_planning_file_sync_candidate(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<PlanningFileSyncCandidateRecord>> {
            Ok(None)
        }

        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            Err(anyhow::anyhow!(
                "stage_planning_draft_files should not be called"
            ))
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            Err(anyhow::anyhow!(
                "load_planning_draft_files should not be called"
            ))
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow::anyhow!(
                "replace_planning_draft_file should not be called"
            ))
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(PlanningWorkspaceLoadRecord {
                result_output_markdown: Some(
                    "# Result Output\n\nExisting authority body.".to_string(),
                ),
            })
        }

        fn load_planning_workspace_candidate_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Err(anyhow::anyhow!(
                "load_planning_workspace_candidate_files should not be called"
            ))
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            _record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            Ok(())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            if relative_path == RESULT_OUTPUT_FILE_PATH {
                return Ok(None);
            }
            Ok(Some("# Supplemental Prompt\n\nExisting.".to_string()))
        }

        fn load_optional_planning_candidate_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            if relative_path == RESULT_OUTPUT_FILE_PATH {
                return Ok(None);
            }
            Ok(Some("# Supplemental Prompt\n\nExisting.".to_string()))
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
            _body: Option<&str>,
        ) -> Result<()> {
            Ok(())
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<()> {
            Err(anyhow::anyhow!(
                "remove_planning_workspace_entry should not be called"
            ))
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow::anyhow!(
                "archive_rejected_planning_file should not be called"
            ))
        }
    }
}
