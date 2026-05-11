use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapArtifacts, PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_DRAFTS_DIRECTORY, PLANNING_PROMPTS_DIRECTORY, PLANNING_REJECTED_DIRECTORY,
    RESULT_OUTPUT_FILE_PATH,
};
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{DirectionCatalogDocument, TaskAuthorityDocument, TaskStatus};

/*
 * Reset은 operator가 명시적으로 선택하는 planning authority의 파괴적 복구 경로다.
 * worker mutation prompt를 거치지 않고 bootstrap에서 만든 새 authority를 일반 planning이 쓰는
 * workspace/repository port로 직접 기록한다. 그래야 reset 이후 runtime projection도 동일한
 * 단일 source of truth에서 다시 읽힌다.
 */

// directions reset은 기존 task를 보존하면서 direction authority와 prompt/detail 산출물을 교체한다.
const RESET_DIRECTIONS_REMOVED_PATHS: &[&str] = &[
    PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY,
];

// full reset은 generated draft/rejection도 지워 오래된 planning 상태가 bootstrap 뒤에 남지 않게 한다.
const RESET_ALL_GENERATED_ARTIFACT_PATHS: &[&str] =
    &[PLANNING_DRAFTS_DIRECTORY, PLANNING_REJECTED_DIRECTORY];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// CLI, admin API, Telegram, TUI, control command adapter가 공유하는 공개 reset 대상이다.
pub enum PlanningResetTarget {
    Queue,
    Directions,
    All,
}
impl PlanningResetTarget {
    // label은 외부 command/report 표면에 노출되는 stable 문자열이다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Directions => "directions",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 결과는 외부에 보이는 파일 효과만 보고하고, DB authority rewrite는 target 선택 자체로 표현한다.
pub struct PlanningWorkspaceResetResult {
    pub target: PlanningResetTarget,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
}

#[derive(Clone)]
/*
 * reset service는 두 outbound boundary를 조율한다.
 * `PlanningWorkspacePort`는 active scaffold 파일을 쓰거나 지우고,
 * `PlanningTaskRepositoryPort`는 검증 뒤 accepted DB authority snapshot과 queue projection을 commit한다.
 */
pub struct PlanningResetService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}
impl PlanningResetService {
    // production constructor는 file authority와 DB authority 표면을 모두 다시 쓸 collaborator를 받는다.
    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_bootstrap_service,
            planning_task_repository_port,
            planning_validation_service,
            priority_queue_service,
        }
    }

    /*
     * 선택된 파괴 범위에 맞춰 기존 planning workspace를 reset한다.
     * bootstrap 산출물은 항상 Simple mode로 생성해 queue/directions/all reset이 같은 기준
     * direction catalog, 기본 queue-idle prompt, 빈 task authority를 공유하게 한다.
     */
    pub fn reset_workspace(
        &self,
        workspace_dir: &str,
        target: PlanningResetTarget,
    ) -> Result<PlanningWorkspaceResetResult> {
        let workspace = self.load_existing_workspace(workspace_dir)?;
        let bootstrap = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        match target {
            PlanningResetTarget::Queue => self.reset_queue(workspace_dir, &workspace, &bootstrap),
            PlanningResetTarget::Directions => {
                self.ensure_directions_reset_is_safe(workspace_dir)?;
                self.reset_directions(workspace_dir, &workspace, &bootstrap)
            }
            PlanningResetTarget::All => self.reset_all(workspace_dir, &bootstrap),
        }
    }

    // reset은 완전히 없는 workspace를 암묵적으로 초기화하지 않는다. bootstrap 생성은 init/doctor 책임이다.
    fn load_existing_workspace(&self, workspace_dir: &str) -> Result<PlanningWorkspaceLoadRecord> {
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        if workspace.has_any_files() {
            Ok(workspace)
        } else {
            Err(anyhow!(
                "planning workspace is unavailable; initialize planning first"
            ))
        }
    }

    /*
     * queue reset은 task authority를 bootstrap의 빈 큐로 되돌린다.
     * direction 파일과 prompt는 건드리지 않으므로, commit helper는 교체 task authority를 받기 전에
     * 기존 direction DB snapshot과 result-output markdown을 재사용해 검증해야 한다.
     */
    fn reset_queue(
        &self,
        workspace_dir: &str,
        workspace: &PlanningWorkspaceLoadRecord,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.commit_task_authority_from_document(
            workspace_dir,
            None,
            &bootstrap.task_authority,
            workspace.result_output_markdown.as_deref(),
        )?;
        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::Queue,
            rewritten_paths: Vec::new(),
            removed_paths: Vec::new(),
        })
    }

    /*
     * live task가 있으면 directions reset을 막는다.
     * 진행 중인 작업 아래에서 direction authority만 교체하면 task/direction 관계가 고아가 될 수 있다.
     * direction과 task queue를 함께 버리려는 경우에는 operator가 reset all을 선택해야 한다.
     */
    fn ensure_directions_reset_is_safe(&self, workspace_dir: &str) -> Result<()> {
        let task_authority = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.task_authority)
            .unwrap_or_else(|| TaskAuthorityDocument {
                version: crate::domain::planning::PLANNING_FORMAT_VERSION,
                tasks: Vec::new(),
            });
        let live_tasks = task_authority
            .tasks
            .iter()
            .filter(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled))
            .map(|task| format!("{}({})", task.id.trim(), task.status.label()))
            .collect::<Vec<_>>();
        if live_tasks.is_empty() {
            return Ok(());
        }
        let live_task_summary = live_tasks
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let extra_count = live_tasks.len().saturating_sub(3);
        let suffix = if extra_count == 0 {
            String::new()
        } else {
            format!(" (+{extra_count} more)")
        };
        Err(anyhow!(
            "planning directions reset is blocked by live tasks: {live_task_summary}{suffix}; use reset all to replace the full workspace instead"
        ))
    }

    /*
     * directions reset은 direction catalog와 보조 prompt/detail 파일을 새로 만들고, 기존 task authority를
     * 새 direction 기준으로 다시 commit한다. 이 검증 단계가 reset된 direction catalog와 맞지 않는
     * task를 repository snapshot이 받아들이지 못하게 하는 마지막 가드다.
     */
    fn reset_directions(
        &self,
        workspace_dir: &str,
        workspace: &PlanningWorkspaceLoadRecord,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.reset_directions_side_artifacts(workspace_dir, bootstrap)?;
        let task_authority = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.task_authority)
            .unwrap_or_else(|| bootstrap.task_authority.clone());
        self.commit_task_authority_from_document(
            workspace_dir,
            Some(&bootstrap.directions),
            &task_authority,
            workspace.result_output_markdown.as_deref(),
        )?;
        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::Directions,
            rewritten_paths: vec![DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()],
            removed_paths: removed_path_strings(RESET_DIRECTIONS_REMOVED_PATHS),
        })
    }

    /*
     * full reset은 active scaffold, direction authority, task authority, generated planning cache를 모두 교체한다.
     * `result-output.md`를 다시 쓰는 유일한 target이기도 하다. queue/directions reset은 operator-facing
     * 현재 planning instruction 문서를 지우면 안 되기 때문이다.
     */
    fn reset_all(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.reset_all_generated_artifacts(workspace_dir)?;
        self.reset_directions_side_artifacts(workspace_dir, bootstrap)?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some(&bootstrap.result_output_markdown),
            )?;
        self.commit_task_authority_from_document(
            workspace_dir,
            Some(&bootstrap.directions),
            &bootstrap.task_authority,
            Some(&bootstrap.result_output_markdown),
        )?;
        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::All,
            rewritten_paths: vec![
                RESULT_OUTPUT_FILE_PATH.to_string(),
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            ],
            removed_paths: reset_all_removed_path_strings(),
        })
    }

    // 새 bootstrap 상태를 쓰기 전에 generated 산출물을 지워 오래된 draft/rejection이 되살아나지 않게 한다.
    fn reset_all_generated_artifacts(&self, workspace_dir: &str) -> Result<()> {
        for path in RESET_ALL_GENERATED_ARTIFACT_PATHS {
            self.planning_workspace_port
                .remove_planning_workspace_entry(workspace_dir, path)?;
        }
        Ok(())
    }

    /*
     * direction side 산출물은 direction authority를 보조하는 file-backed 자료다.
     * DB direction snapshot을 supplemental file보다 먼저 commit한다. 뒤쪽 파일 쓰기가 실패해도
     * authority source는 갱신되고, operator는 반환된 error로 실패한 path를 볼 수 있다.
     */
    fn reset_directions_side_artifacts(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<()> {
        for path in RESET_DIRECTIONS_REMOVED_PATHS {
            self.planning_workspace_port
                .remove_planning_workspace_entry(workspace_dir, path)?;
        }
        self.commit_direction_authority_from_bootstrap(workspace_dir, &bootstrap.directions)?;
        for supplemental_file in &bootstrap.supplemental_files {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    &supplemental_file.active_path,
                    Some(&supplemental_file.body),
                )?;
        }
        Ok(())
    }

    /*
     * 전체 planning runtime 계약을 검증할 context가 충분할 때만 task authority를 commit한다.
     * directions나 result-output이 없으면 active workspace authority로 증명할 수 없는 queue projection을
     * commit하기보다 DB task snapshot을 지우는 편이 더 안전한 reset 효과다.
     */
    fn commit_task_authority_from_document(
        &self,
        workspace_dir: &str,
        directions: Option<&DirectionCatalogDocument>,
        task_authority: &TaskAuthorityDocument,
        result_output_markdown: Option<&str>,
    ) -> Result<()> {
        let loaded_directions;
        let directions = match directions {
            Some(directions) => Some(directions),
            None => {
                loaded_directions = self
                    .planning_task_repository_port
                    .load_direction_authority_snapshot(workspace_dir)?
                    .map(|snapshot| snapshot.directions);
                loaded_directions.as_ref()
            }
        };
        let (Some(directions), Some(result_output_markdown)) = (directions, result_output_markdown)
        else {
            return self
                .planning_task_repository_port
                .clear_task_authority_snapshot(workspace_dir);
        };
        let task_authority_json = serde_json::to_string(task_authority)?;
        let validation_result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions,
                task_authority_json: &task_authority_json,
                result_output_markdown,
            },
        );
        if !validation_result.is_valid() {
            return Ok(());
        }

        // validation은 승인된 direction/task 문서를 다시 parse하므로, commit에는 normalized domain 값을 사용한다.
        let directions = validation_result
            .directions
            .as_ref()
            .ok_or_else(|| anyhow!("valid reset workspace did not include directions"))?;
        let task_authority = validation_result
            .task_authority
            .as_ref()
            .ok_or_else(|| anyhow!("valid reset workspace did not include task-authority"))?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| anyhow!("valid reset queue build failed: {error}"))?;
        // reset은 incremental task mutation이 아니라 operator/system authority rewrite 경계다.
        // caller가 파괴적 reset target을 명시적으로 선택했으므로 revision guard 없이 commit한다.
        self.planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority,
                    queue_projection: &queue_projection,
                },
            )
            .map(|_| ())
    }

    // direction authority reset은 queue projection이 필요 없다. task는 검증 뒤 별도로 commit된다.
    fn commit_direction_authority_from_bootstrap(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
    ) -> Result<()> {
        self.planning_task_repository_port
            .commit_direction_authority_snapshot(
                workspace_dir,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions,
                },
            )
            .map(|_| ())
    }
}

// full-reset report에 쓰려고 direction-side 제거 목록과 generated-artifact 제거 목록을 합친다.
fn reset_all_removed_path_strings() -> Vec<String> {
    RESET_DIRECTIONS_REMOVED_PATHS
        .iter()
        .chain(RESET_ALL_GENERATED_ARTIFACT_PATHS.iter())
        .map(|path| (*path).to_string())
        .collect()
}

// static reset path slice를 노출하지 않고 owned report data로 변환한다.
fn removed_path_strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
    };
    use crate::domain::planning::{
        DirectionDefinition, DirectionState, OriginSessionKind, PLANNING_FORMAT_VERSION,
        QueueIdleConfig, QueueIdlePolicy, TaskActor, TaskDefinition, TaskMutationProvenance,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    // reset caller가 공개 enum matching으로 연결된 동안 target variant가 사라지지 않게 고정한다.
    fn reset_target_values_still_exist() {
        assert!(matches!(
            PlanningResetTarget::Queue,
            PlanningResetTarget::Queue
        ));
        assert!(matches!(
            PlanningResetTarget::Directions,
            PlanningResetTarget::Directions
        ));
        assert!(matches!(PlanningResetTarget::All, PlanningResetTarget::All));
    }

    #[test]
    fn reset_target_labels_match_command_vocabulary() {
        assert_eq!(PlanningResetTarget::Queue.label(), "queue");
        assert_eq!(PlanningResetTarget::Directions.label(), "directions");
        assert_eq!(PlanningResetTarget::All.label(), "all");
    }

    #[test]
    fn reset_workspace_rejects_missing_active_workspace() {
        let fixture = ResetFixture::new("missing-workspace");

        let error = fixture
            .service
            .reset_workspace(fixture.workspace.path_str(), PlanningResetTarget::Queue)
            .expect_err("reset should not bootstrap a missing workspace");

        assert_eq!(
            error.to_string(),
            "planning workspace is unavailable; initialize planning first"
        );
    }

    #[test]
    fn queue_reset_clears_task_authority_when_direction_context_is_missing() {
        let fixture = ResetFixture::new("queue-clear-missing-context");
        fixture.write_result_output("# Result Output\n\n- Keep the operator contract.\n");
        let directions =
            direction_catalog(vec![direction("active-direction", DirectionState::Active)]);
        fixture.seed_task_authority(
            &directions,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![task("live-task", "active-direction", TaskStatus::Ready)],
            },
        );

        let result = fixture
            .service
            .reset_workspace(fixture.workspace.path_str(), PlanningResetTarget::Queue)
            .expect("queue reset should clear task authority without direction context");

        assert_eq!(result.target, PlanningResetTarget::Queue);
        assert!(result.rewritten_paths.is_empty());
        assert!(result.removed_paths.is_empty());
        assert!(
            fixture
                .repository
                .load_task_authority_snapshot(fixture.workspace.path_str())
                .expect("task authority snapshot should load")
                .is_none()
        );
    }

    #[test]
    fn queue_reset_replaces_tasks_with_bootstrap_empty_authority_when_context_is_valid() {
        let fixture = ResetFixture::new("queue-reset-empty-authority");
        let bootstrap = fixture.bootstrap_artifacts();
        fixture.write_result_output(&bootstrap.result_output_markdown);
        fixture.seed_direction_authority(&bootstrap.directions);
        fixture.seed_task_authority(
            &bootstrap.directions,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![task("queued-task", "general-workstream", TaskStatus::Ready)],
            },
        );

        fixture
            .service
            .reset_workspace(fixture.workspace.path_str(), PlanningResetTarget::Queue)
            .expect("queue reset should commit the bootstrap empty task authority");

        let snapshot = fixture
            .repository
            .load_task_authority_snapshot(fixture.workspace.path_str())
            .expect("task authority snapshot should load")
            .expect("queue reset should keep a task authority snapshot");
        assert!(snapshot.task_authority.tasks.is_empty());
        assert!(snapshot.queue_projection.next_task.is_none());
    }

    #[test]
    fn directions_reset_is_blocked_by_live_tasks_with_bounded_summary() {
        let fixture = ResetFixture::new("directions-reset-live-tasks");
        let directions =
            direction_catalog(vec![direction("active-direction", DirectionState::Active)]);
        fixture.write_result_output("# Result Output\n\n- Keep the operator contract.\n");
        fixture.seed_direction_authority(&directions);
        fixture.seed_task_authority(
            &directions,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![
                    task("task-a", "active-direction", TaskStatus::Ready),
                    task("task-b", "active-direction", TaskStatus::InProgress),
                    task("task-c", "active-direction", TaskStatus::Blocked),
                    task("task-d", "active-direction", TaskStatus::Proposed),
                ],
            },
        );

        let error = fixture
            .service
            .reset_workspace(
                fixture.workspace.path_str(),
                PlanningResetTarget::Directions,
            )
            .expect_err("directions reset should reject live task authority");

        assert_eq!(
            error.to_string(),
            "planning directions reset is blocked by live tasks: task-a(ready), task-b(in_progress), task-c(blocked) (+1 more); use reset all to replace the full workspace instead"
        );
    }

    #[test]
    fn directions_reset_rewrites_prompt_side_artifacts_and_empty_direction_authority() {
        let fixture = ResetFixture::new("directions-reset-safe");
        let bootstrap = fixture.bootstrap_artifacts();
        fixture.write_result_output(&bootstrap.result_output_markdown);
        fixture.seed_direction_authority(&direction_catalog(vec![direction(
            "old-direction",
            DirectionState::Active,
        )]));
        fixture.seed_task_authority(
            &bootstrap.directions,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: Vec::new(),
            },
        );
        fixture.create_file(
            Path::new(PLANNING_DIRECTION_DOCS_DIRECTORY).join("old.md"),
            "old direction detail",
        );
        fixture.create_file(
            Path::new(PLANNING_PROMPTS_DIRECTORY).join("old.md"),
            "old prompt",
        );

        let result = fixture
            .service
            .reset_workspace(
                fixture.workspace.path_str(),
                PlanningResetTarget::Directions,
            )
            .expect("safe directions reset should succeed");

        assert_eq!(result.target, PlanningResetTarget::Directions);
        assert_eq!(
            result.rewritten_paths,
            vec![DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()]
        );
        assert_eq!(
            result.removed_paths,
            removed_path_strings(RESET_DIRECTIONS_REMOVED_PATHS)
        );
        assert!(
            !fixture
                .workspace
                .path()
                .join(PLANNING_DIRECTION_DOCS_DIRECTORY)
                .exists()
        );
        assert!(
            fixture
                .workspace
                .path()
                .join(DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
                .is_file()
        );
        let direction_snapshot = fixture
            .repository
            .load_direction_authority_snapshot(fixture.workspace.path_str())
            .expect("direction authority should load")
            .expect("directions reset should commit direction authority");
        assert_eq!(direction_snapshot.directions, bootstrap.directions);
    }

    #[test]
    fn all_reset_replaces_result_output_and_removes_generated_artifacts() {
        let fixture = ResetFixture::new("all-reset");
        let bootstrap = fixture.bootstrap_artifacts();
        fixture.write_result_output("# Result Output\n\n- Old instructions.\n");
        fixture.seed_direction_authority(&direction_catalog(vec![direction(
            "old-direction",
            DirectionState::Active,
        )]));
        fixture.seed_task_authority(
            &direction_catalog(vec![direction("old-direction", DirectionState::Active)]),
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![task("old-task", "old-direction", TaskStatus::Ready)],
            },
        );
        fixture.create_file(
            Path::new(PLANNING_DRAFTS_DIRECTORY)
                .join("stale")
                .join("draft.md"),
            "stale draft",
        );
        fixture.create_file(
            Path::new(PLANNING_REJECTED_DIRECTORY)
                .join("stale")
                .join("rejected.md"),
            "stale rejected file",
        );

        let result = fixture
            .service
            .reset_workspace(fixture.workspace.path_str(), PlanningResetTarget::All)
            .expect("all reset should replace the full planning scaffold");

        assert_eq!(result.target, PlanningResetTarget::All);
        assert_eq!(
            result.rewritten_paths,
            vec![
                RESULT_OUTPUT_FILE_PATH.to_string(),
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()
            ]
        );
        assert_eq!(result.removed_paths, reset_all_removed_path_strings());
        assert_eq!(
            fs::read_to_string(fixture.workspace.path().join(RESULT_OUTPUT_FILE_PATH))
                .expect("result output should be readable"),
            bootstrap.result_output_markdown
        );
        assert!(
            !fixture
                .workspace
                .path()
                .join(PLANNING_DRAFTS_DIRECTORY)
                .exists()
        );
        assert!(
            !fixture
                .workspace
                .path()
                .join(PLANNING_REJECTED_DIRECTORY)
                .exists()
        );
        let task_snapshot = fixture
            .repository
            .load_task_authority_snapshot(fixture.workspace.path_str())
            .expect("task authority should load")
            .expect("all reset should commit empty task authority");
        assert!(task_snapshot.task_authority.tasks.is_empty());
    }

    fn direction_catalog(directions: Vec<DirectionDefinition>) -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::ReviewAndEnqueue,
                prompt_path: DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            },
            directions,
        }
    }

    fn direction(id: &str, state: DirectionState) -> DirectionDefinition {
        DirectionDefinition {
            id: id.to_string(),
            title: format!("Direction {id}"),
            summary: format!("Summary for {id}"),
            success_criteria: vec!["done".to_string()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state,
        }
    }

    fn task(id: &str, direction_id: &str, status: TaskStatus) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: direction_id.to_string(),
            direction_relation_note: "relates to the direction".to_string(),
            title: format!("Task {id}"),
            description: "Do the task".to_string(),
            status,
            base_priority: 10,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            provenance: TaskMutationProvenance::new(OriginSessionKind::System),
            updated_at: "2026-05-12T00:00:00Z".to_string(),
        }
    }

    struct ResetFixture {
        workspace: TempPlanningWorkspace,
        service: PlanningResetService,
        repository: Arc<dyn PlanningTaskRepositoryPort>,
        workspace_port: Arc<dyn PlanningWorkspacePort>,
        priority_queue: PriorityQueueService,
        bootstrap: PlanningBootstrapService,
    }

    impl ResetFixture {
        fn new(prefix: &str) -> Self {
            let workspace = TempPlanningWorkspace::new(prefix);
            let workspace_port: Arc<dyn PlanningWorkspacePort> =
                Arc::new(FilesystemPlanningWorkspaceAdapter::new());
            let repository: Arc<dyn PlanningTaskRepositoryPort> =
                Arc::new(NoopPlanningTaskRepositoryPort);
            let bootstrap = PlanningBootstrapService::new();
            let validation = PlanningValidationService::new();
            let priority_queue = PriorityQueueService::new();
            let service = PlanningResetService::with_task_repository(
                workspace_port.clone(),
                bootstrap.clone(),
                repository.clone(),
                validation,
                priority_queue.clone(),
            );
            Self {
                workspace,
                service,
                repository,
                workspace_port,
                priority_queue,
                bootstrap,
            }
        }

        fn bootstrap_artifacts(&self) -> PlanningBootstrapArtifacts {
            self.bootstrap
                .build_artifacts_for_mode(PlanningBootstrapMode::Simple)
        }

        fn write_result_output(&self, body: &str) {
            self.workspace_port
                .replace_planning_workspace_file(
                    self.workspace.path_str(),
                    RESULT_OUTPUT_FILE_PATH,
                    Some(body),
                )
                .expect("result output should be written");
        }

        fn create_file(&self, relative_path: PathBuf, body: &str) {
            let path = self.workspace.path().join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("fixture parent directory should be created");
            }
            fs::write(path, body).expect("fixture file should be written");
        }

        fn seed_direction_authority(&self, directions: &DirectionCatalogDocument) {
            self.repository
                .commit_direction_authority_snapshot(
                    self.workspace.path_str(),
                    PlanningDirectionAuthorityCommit {
                        observed_planning_revision: None,
                        directions,
                    },
                )
                .expect("direction authority should be seeded");
        }

        fn seed_task_authority(
            &self,
            directions: &DirectionCatalogDocument,
            task_authority: TaskAuthorityDocument,
        ) {
            let queue_projection = self
                .priority_queue
                .build_projection(directions, &task_authority)
                .expect("seed task authority should build a queue projection");
            self.repository
                .commit_task_authority_snapshot(
                    self.workspace.path_str(),
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: None,
                        task_authority: &task_authority,
                        queue_projection: &queue_projection,
                    },
                )
                .expect("task authority should be seeded");
        }
    }

    struct TempPlanningWorkspace {
        path: PathBuf,
        path_text: String,
    }

    impl TempPlanningWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&path).expect("temp planning workspace should be created");
            let path_text = path.display().to_string();
            Self { path, path_text }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn path_str(&self) -> &str {
            &self.path_text
        }
    }

    impl Drop for TempPlanningWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
