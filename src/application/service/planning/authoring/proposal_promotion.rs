use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{SecondsFormat, Utc};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::prompt::{
    PlanningPromptService, PlanningRuntimeProjection,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningProposalPromotionDecision,
    PlanningProposalPromotionPolicy, PlanningWorkspaceFiles, TaskActor, TaskAuthorityDocument,
    TaskStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * promotion request는 하나의 workspace에 묶인다. queue-idle 판단 turn 같은 runtime provenance는 이
 * service가 쓰는 authority mutation에는 참여하지 않으므로 request contract에 싣지 않는다.
 */
pub struct PlanningProposalPromotionRequest<'a> {
    // filesystem adapter와 repository adapter가 모두 이 값을 planning workspace boundary로 사용한다.
    pub workspace_directory: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * caller는 proposal이 실제 promote됐는지와 무관하게 fresh runtime projection을 받는다. worker orchestration은 service
 * 반환 뒤 같은 snapshot shape로 render/submit을 이어가고, promoted/title/notices는 authority 변경 여부를 설명하는
 * 보조 정보로만 사용하면 된다.
 */
pub struct PlanningProposalPromotionOutcome {
    // no-op check 뒤 또는 commit 뒤 다시 읽은 snapshot이다. queue state가 authoritative store를 반영하게 한다.
    pub runtime_projection: PlanningRuntimeProjection,
    // adapter가 아니라 application service가 만든 operator-facing message다.
    pub notices: Vec<String>,
    // 성공적인 promotion 때만 채워지며, wording을 안정화하기 위해 proposal projection의 title을 복사한다.
    pub promoted_task_title: Option<String>,
    // valid no-op와 authority rewrite를 구분한다.
    pub promoted: bool,
}

#[derive(Clone)]
/*
 * 이 application service는 runtime queue inspection과 authority mutation 사이의 gap을 닫는다. 현재 planning
 * document를 port로 읽고, domain queue service에 어떤 proposed task가 promote 가능한지 묻고, write 전에 전체
 * workspace를 검증한 다음 observed revision guard와 함께 repository port로 commit한다.
 */
pub struct PlanningProposalPromotionService {
    // workspace file은 whole-workspace validation에 필요한 result-output content를 제공한다.
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    // runtime projection reload는 일반 worker turn에서도 쓰는 같은 prompt service에 위임한다.
    planning_prompt_service: PlanningPromptService,
    // promotion write는 mutation 전 workspace가 valid할 때만 허용된다.
    planning_validation_service: PlanningValidationService,
    // domain projection은 이미 ready task가 있는지와 어떤 proposal이 1순위인지 판단한다.
    priority_queue_service: PriorityQueueService,
    // promotion 가능 여부는 projection 기반 domain decision으로만 판단한다.
    proposal_promotion_policy: PlanningProposalPromotionPolicy,
    // repository port는 accepted authority snapshot과 conflict-aware commit을 소유한다.
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
}

impl PlanningProposalPromotionService {
    /*
     * composition은 모든 collaboration point를 명시적으로 주입한다. repository를 port dependency로 유지하면
     * filesystem-backed/DB-backed authority store가 promotion logic을 바꾸지 않고도 같은 optimistic revision
     * contract를 강제할 수 있다.
     */
    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_prompt_service: PlanningPromptService,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_prompt_service,
            planning_validation_service,
            priority_queue_service,
            proposal_promotion_policy: PlanningProposalPromotionPolicy::new(),
            planning_task_repository_port,
        }
    }

    /*
     * executable queue가 비어 있을 때 proposed task 하나만 promote한다. 이 operation은 의도적으로 보수적이다.
     * ready task가 이미 있거나 promotable proposal이 없으면 authority를 쓰지 않고 current runtime projection만 돌려준다.
     * 실제 promote할 때는 mutation 뒤 queue projection을 다시 만들고, load 시 관찰한 revision으로 commit한다.
     */
    pub fn promote_top_proposal_to_ready_if_needed(
        &self,
        request: PlanningProposalPromotionRequest<'_>,
    ) -> Result<PlanningProposalPromotionOutcome> {
        // validation은 DB authority와 함께 result-output prompt도 필요하므로 workspace file을 먼저 읽는다.
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(request.workspace_directory)?;
        // helper는 complete accepted state를 검증하고, 뒤 commit에 사용할 revision guard를 함께 반환한다.
        let (directions, mut task_authority, observed_planning_revision) =
            self.load_valid_workspace_documents(request.workspace_directory, &workspace_record)?;
        // domain queue projection은 promotion policy가 판단할 active/proposed lane facts를 제공한다.
        let queue_projection = self
            .priority_queue_service
            .build_projection(&directions, &task_authority)?;
        let promotion_decision = self.proposal_promotion_policy.decide(&queue_projection);
        let promotion_candidate = match promotion_decision {
            PlanningProposalPromotionDecision::Promote(candidate) => candidate,
            PlanningProposalPromotionDecision::Noop(_) => {
                return Ok(PlanningProposalPromotionOutcome {
                    runtime_projection: self
                        .planning_prompt_service
                        .load_runtime_projection(request.workspace_directory)?,
                    notices: Vec::new(),
                    promoted_task_title: None,
                    promoted: false,
                });
            }
        };

        // authority rewrite는 title이 아니라 task id로 수행한다. title은 presentation text라 중복되거나 변경될 수 있다.
        let promoted_task = task_authority
            .tasks
            .iter_mut()
            .find(|task| task.id.trim() == promotion_candidate.task_id)
            .ok_or_else(|| {
                anyhow!(
                    "top promotable proposal {} was not found in task authority",
                    promotion_candidate.task_id
                )
            })?;

        // mutation은 의도적으로 최소화한다. status와 system audit metadata만 바꿔 proposal의 본문/priority 판단을 보존한다.
        promoted_task.status = TaskStatus::Ready;
        promoted_task.last_updated_by = TaskActor::System;
        promoted_task.updated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        // commit에는 post-mutation projection을 넘긴다. repository adapter가 denormalized queue state를 같은 transaction
        // 의미로 저장할 수 있게 하기 위해서다.
        let next_queue_projection = self
            .priority_queue_service
            .build_projection(&directions, &task_authority)?;
        // proposal promotion은 queued proposal 하나에 대한 system-owned authority rewrite다. loaded snapshot에서
        // 관찰한 revision으로 보호해 stale worker-side promotion이 operator edit를 덮어쓰지 않게 한다.
        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                request.workspace_directory,
                PlanningTaskAuthorityCommit {
                    // optimistic locking은 오래된 worker-side promotion이 operator edit를 덮어쓰는 것을 막는다.
                    observed_planning_revision: Some(observed_planning_revision),
                    task_authority: &task_authority,
                    queue_projection: &next_queue_projection,
                },
            )? {
            // 성공한 commit 뒤에는 refreshed runtime projection이 다음 source of truth가 된다.
            PlanningTaskAuthorityCommitResult::Committed { .. } => {}
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                anyhow::bail!(
                    "planning db changed while promoting proposal (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                );
            }
        }

        // commit 뒤 reload해서 downstream worker policy가 promoted task를 일반 ready work로 보게 한다.
        let runtime_projection = self
            .planning_prompt_service
            .load_runtime_projection(request.workspace_directory)?;

        // notice는 projection title을 사용한다. queue가 operator/worker에게 보여 준 task 표현과 같은 wording을 유지한다.
        let promoted_task_title = promotion_candidate.task_title;
        let mut notices = Vec::new();
        notices.push(format!(
            "host promoted top follow-up proposal into the executable queue: {}",
            promoted_task_title
        ));

        Ok(PlanningProposalPromotionOutcome {
            runtime_projection,
            notices,
            promoted_task_title: Some(promoted_task_title),
            promoted: true,
        })
    }

    /*
     * promotion에 사용할 validated authority pair를 만든다. repository snapshot은 accepted planning authority이고,
     * result-output은 아직 workspace port에서 온다. validation은 둘을 결합해 partially invalid workspace에서 promotion이
     * write되지 않게 하며, 반환 revision은 나중에 conflict check에 쓰는 정확한 task-authority version이다.
     */
    fn load_valid_workspace_documents(
        &self,
        workspace_dir: &str,
        workspace_record: &PlanningWorkspaceLoadRecord,
    ) -> Result<(DirectionCatalogDocument, TaskAuthorityDocument, i64)> {
        // 이 service가 mutate하는 문서는 task authority뿐이므로 그 revision이 write guard가 된다.
        let snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .ok_or_else(|| anyhow!("planning task authority is unavailable"))?;
        // direction authority는 task direction link 검증과 queue projection rebuild를 위해 읽는다.
        let directions_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .ok_or_else(|| anyhow!("planning direction authority is unavailable"))?;
        // validation은 task document를 workspace file에서 읽을 때와 같은 JSON form으로 기대한다.
        let task_authority_json = serde_json::to_string(&snapshot.task_authority)?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(workspace_record_to_files(
                    workspace_record,
                    &directions_snapshot.directions,
                    &task_authority_json,
                )?);
        // promotion은 queue convenience이지 repair tool이 아니다. invalid planning state는 먼저 별도 흐름에서 고쳐야 한다.
        if !validation_result.is_valid() {
            let first_error = validation_result
                .report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning validation failed".to_string());
            return Err(anyhow!(
                "cannot promote proposal from an invalid planning workspace: {first_error}"
            ));
        }

        // downstream projection이 normalized state를 보도록 validation이 반환한 parsed document를 사용한다.
        let directions = validation_result
            .directions
            .ok_or_else(|| anyhow!("validated planning workspace did not include directions"))?;
        let task_authority = validation_result.task_authority.ok_or_else(|| {
            anyhow!("validated planning workspace did not include task-authority")
        })?;
        // queue projection과 repository commit contract는 planning document schema version과 함께 움직인다.
        if task_authority.version != PLANNING_FORMAT_VERSION {
            return Err(anyhow!(
                "unsupported task-authority version {}; expected {}",
                task_authority.version,
                PLANNING_FORMAT_VERSION
            ));
        }

        Ok((directions, task_authority, snapshot.planning_revision))
    }
}

/*
 * workspace-port load shape를 validation domain input으로 바꾸는 adapter다. direction/task authority는 repository
 * snapshot에서 오고, result-output markdown은 아직 DB authority가 아니라 supporting file이므로 workspace record에서 온다.
 */
fn workspace_record_to_files<'a>(
    workspace_record: &'a PlanningWorkspaceLoadRecord,
    directions: &'a DirectionCatalogDocument,
    task_authority_json: &'a str,
) -> Result<PlanningWorkspaceFiles<'a>> {
    Ok(PlanningWorkspaceFiles {
        directions,
        task_authority_json,
        // result-output이 없으면 runtime prompt contract를 검증할 수 없으므로 promotion을 막는다.
        result_output_markdown: workspace_record
            .result_output_markdown
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing result-output.md"))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
        PlanningDirectionAuthoritySnapshot, PlanningTaskAuthoritySnapshot,
    };
    use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
    use crate::domain::planning::{
        DirectionDefinition, DirectionState, OriginSessionKind, PriorityQueueProjection,
        QueueIdleConfig, QueueIdlePolicy, TaskDefinition, TaskMutationProvenance,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn workspace_record_to_files_requires_result_output() {
        let directions = direction_catalog();
        let task_authority_json = "{}";

        let error = workspace_record_to_files(
            &PlanningWorkspaceLoadRecord {
                result_output_markdown: None,
            },
            &directions,
            task_authority_json,
        )
        .expect_err("missing result output should block validation input");

        assert_eq!(
            error.to_string(),
            "planning workspace is missing result-output.md"
        );
    }

    #[test]
    fn promotes_top_proposal_and_returns_refreshed_projection() {
        let fixture = PromotionFixture::new("proposal-promote");
        let directions = direction_catalog();
        fixture.write_result_output();
        fixture.seed_authority(
            &directions,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![
                    task("proposal-low", "Low proposal", 10, TaskStatus::Proposed),
                    task("proposal-high", "High proposal", 90, TaskStatus::Proposed),
                ],
            },
        );

        let outcome = fixture
            .service
            .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                workspace_directory: fixture.workspace.path_str(),
            })
            .expect("top proposal should promote");

        assert!(outcome.promoted);
        assert_eq!(
            outcome.promoted_task_title.as_deref(),
            Some("High proposal")
        );
        assert_eq!(
            outcome.notices,
            vec![
                "host promoted top follow-up proposal into the executable queue: High proposal"
                    .to_string()
            ]
        );
        assert_eq!(
            outcome
                .runtime_projection
                .queue_head()
                .expect("promoted proposal should become the queue head")
                .task_id,
            "proposal-high"
        );
        assert!(
            outcome
                .runtime_projection
                .proposal_summary()
                .expect("lower proposal should remain visible")
                .contains("Low proposal")
        );

        let snapshot = fixture.task_snapshot();
        let promoted = snapshot
            .task_authority
            .tasks
            .iter()
            .find(|task| task.id == "proposal-high")
            .expect("promoted task should remain in authority");
        assert_eq!(promoted.status, TaskStatus::Ready);
        assert_eq!(promoted.last_updated_by, TaskActor::System);
        assert!(promoted.updated_at.ends_with('Z'));
        let lower_proposal = snapshot
            .task_authority
            .tasks
            .iter()
            .find(|task| task.id == "proposal-low")
            .expect("lower proposal should remain in authority");
        assert_eq!(lower_proposal.status, TaskStatus::Proposed);
    }

    #[test]
    fn ready_queue_head_keeps_promotion_as_noop_without_writing_authority() {
        let fixture = PromotionFixture::new("proposal-noop-ready");
        let directions = direction_catalog();
        fixture.write_result_output();
        fixture.seed_authority(
            &directions,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![
                    task("ready-task", "Ready task", 50, TaskStatus::Ready),
                    task("proposal-task", "Proposal task", 90, TaskStatus::Proposed),
                ],
            },
        );
        let before_revision = fixture.task_snapshot().planning_revision;

        let outcome = fixture
            .service
            .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                workspace_directory: fixture.workspace.path_str(),
            })
            .expect("ready queue head should make promotion a no-op");

        assert!(!outcome.promoted);
        assert!(outcome.notices.is_empty());
        assert!(outcome.promoted_task_title.is_none());
        assert_eq!(
            outcome
                .runtime_projection
                .queue_head()
                .expect("existing ready task should remain queue head")
                .task_id,
            "ready-task"
        );
        let after_snapshot = fixture.task_snapshot();
        assert_eq!(after_snapshot.planning_revision, before_revision);
        assert_eq!(
            after_snapshot
                .task_authority
                .tasks
                .iter()
                .find(|task| task.id == "proposal-task")
                .expect("proposal should remain in authority")
                .status,
            TaskStatus::Proposed
        );
    }

    #[test]
    fn invalid_workspace_blocks_promotion_before_authority_write() {
        let fixture = PromotionFixture::new("proposal-invalid-workspace");
        let directions = direction_catalog();
        fixture.write_result_output_body("not a heading");
        fixture.seed_authority(
            &directions,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![task(
                    "proposal-task",
                    "Proposal task",
                    90,
                    TaskStatus::Proposed,
                )],
            },
        );
        let before_revision = fixture.task_snapshot().planning_revision;

        let error = fixture
            .service
            .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                workspace_directory: fixture.workspace.path_str(),
            })
            .expect_err("invalid result output should block promotion");

        assert!(
            error.to_string().contains(
                "cannot promote proposal from an invalid planning workspace: result-output.md must start with a markdown heading"
            )
        );
        let after_snapshot = fixture.task_snapshot();
        assert_eq!(after_snapshot.planning_revision, before_revision);
        assert_eq!(
            after_snapshot.task_authority.tasks[0].status,
            TaskStatus::Proposed
        );
    }

    #[test]
    fn commit_conflict_reports_observed_and_current_revisions() {
        let directions = direction_catalog();
        let task_authority = TaskAuthorityDocument {
            version: PLANNING_FORMAT_VERSION,
            tasks: vec![task(
                "proposal-task",
                "Proposal task",
                90,
                TaskStatus::Proposed,
            )],
        };
        let queue_projection = PriorityQueueService::new()
            .build_projection(&directions, &task_authority)
            .expect("fixture projection should build");
        let repository: Arc<dyn PlanningTaskRepositoryPort> = Arc::new(ConflictRepository {
            directions,
            task_authority,
            queue_projection,
        });
        let fixture = PromotionFixture::new_with_repository("proposal-conflict", repository);
        fixture.write_result_output();

        let error = fixture
            .service
            .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                workspace_directory: fixture.workspace.path_str(),
            })
            .expect_err("stale promotion should report a commit conflict");

        assert_eq!(
            error.to_string(),
            "planning db changed while promoting proposal (observed revision 1, current revision 2); reload and retry"
        );
    }

    #[test]
    fn missing_authority_snapshot_reports_actionable_error() {
        let fixture = PromotionFixture::new("proposal-missing-authority");
        fixture.write_result_output();

        let error = fixture
            .service
            .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                workspace_directory: fixture.workspace.path_str(),
            })
            .expect_err("missing task authority should block promotion");

        assert_eq!(error.to_string(), "planning task authority is unavailable");
    }

    fn direction_catalog() -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::Stop,
                prompt_path: String::new(),
            },
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General workstream".to_string(),
                summary: "General planning work.".to_string(),
                success_criteria: vec!["done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        }
    }

    fn task(id: &str, title: &str, priority: i32, status: TaskStatus) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: "general-workstream".to_string(),
            direction_relation_note: "relates to the general workstream".to_string(),
            title: title.to_string(),
            description: format!("Do {title}."),
            status,
            base_priority: priority,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::Worker,
            last_updated_by: TaskActor::Worker,
            source_turn_id: None,
            provenance: TaskMutationProvenance::new(OriginSessionKind::System),
            updated_at: "2026-05-12T00:00:00Z".to_string(),
        }
    }

    struct PromotionFixture {
        workspace: TempPlanningWorkspace,
        workspace_port: Arc<dyn PlanningWorkspacePort>,
        repository: Arc<dyn PlanningTaskRepositoryPort>,
        service: PlanningProposalPromotionService,
        priority_queue: PriorityQueueService,
    }

    impl PromotionFixture {
        fn new(prefix: &str) -> Self {
            Self::new_with_repository(prefix, Arc::new(NoopPlanningTaskRepositoryPort))
        }

        fn new_with_repository(
            prefix: &str,
            repository: Arc<dyn PlanningTaskRepositoryPort>,
        ) -> Self {
            let workspace = TempPlanningWorkspace::new(prefix);
            let workspace_port: Arc<dyn PlanningWorkspacePort> =
                Arc::new(FilesystemPlanningWorkspaceAdapter::new());
            let validation = PlanningValidationService::new();
            let priority_queue = PriorityQueueService::new();
            let prompt = PlanningPromptService::with_task_repository(
                workspace_port.clone(),
                validation.clone(),
                priority_queue.clone(),
                repository.clone(),
            );
            let service = PlanningProposalPromotionService::with_task_repository(
                workspace_port.clone(),
                prompt,
                validation,
                priority_queue.clone(),
                repository.clone(),
            );
            Self {
                workspace,
                workspace_port,
                repository,
                service,
                priority_queue,
            }
        }

        fn write_result_output(&self) {
            self.write_result_output_body("# Result Output\n\n- Record completed work.\n");
        }

        fn write_result_output_body(&self, body: &str) {
            self.workspace_port
                .replace_planning_workspace_file(
                    self.workspace.path_str(),
                    RESULT_OUTPUT_FILE_PATH,
                    Some(body),
                )
                .expect("result output should be written");
        }

        fn seed_authority(
            &self,
            directions: &DirectionCatalogDocument,
            task_authority: TaskAuthorityDocument,
        ) {
            self.repository
                .commit_direction_authority_snapshot(
                    self.workspace.path_str(),
                    PlanningDirectionAuthorityCommit {
                        observed_planning_revision: None,
                        directions,
                    },
                )
                .expect("direction authority should be seeded");
            let queue_projection = self
                .priority_queue
                .build_projection(directions, &task_authority)
                .expect("task authority projection should build");
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

        fn task_snapshot(&self) -> PlanningTaskAuthoritySnapshot {
            self.repository
                .load_task_authority_snapshot(self.workspace.path_str())
                .expect("task authority should load")
                .expect("task authority should exist")
        }
    }

    struct ConflictRepository {
        directions: DirectionCatalogDocument,
        task_authority: TaskAuthorityDocument,
        queue_projection: PriorityQueueProjection,
    }

    impl PlanningTaskRepositoryPort for ConflictRepository {
        fn load_direction_authority_snapshot(
            &self,
            _workspace_dir: &str,
        ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
            Ok(Some(PlanningDirectionAuthoritySnapshot {
                planning_revision: 1,
                directions: self.directions.clone(),
            }))
        }

        fn commit_direction_authority_snapshot(
            &self,
            _workspace_dir: &str,
            _commit: PlanningDirectionAuthorityCommit<'_>,
        ) -> Result<PlanningTaskAuthorityCommitResult> {
            Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: 1,
            })
        }

        fn clear_direction_authority_snapshot(&self, _workspace_dir: &str) -> Result<()> {
            Ok(())
        }

        fn load_task_authority_snapshot(
            &self,
            _workspace_dir: &str,
        ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
            Ok(Some(PlanningTaskAuthoritySnapshot {
                planning_revision: 1,
                task_authority: self.task_authority.clone(),
                queue_projection: self.queue_projection.clone(),
            }))
        }

        fn commit_task_authority_snapshot(
            &self,
            _workspace_dir: &str,
            commit: PlanningTaskAuthorityCommit<'_>,
        ) -> Result<PlanningTaskAuthorityCommitResult> {
            Ok(PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision: commit.observed_planning_revision.unwrap_or_default(),
                current_planning_revision: 2,
            })
        }

        fn clear_task_authority_snapshot(&self, _workspace_dir: &str) -> Result<()> {
            Ok(())
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
