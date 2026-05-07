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
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningWorkspaceFiles, TaskActor,
    TaskAuthorityDocument, TaskStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * promotion request는 하나의 workspace와 하나의 root turn에 묶인다. workspace는 어떤 planning authority set을
 * rewrite할지 결정하고, root_turn_id는 현재 구현이 repository state에서만 promote하더라도 "queue가 idle이라고
 * 판단한 turn"을 audit/debug에서 추적할 수 있게 request contract에 남긴다.
 */
pub struct PlanningProposalPromotionRequest<'a> {
    // filesystem adapter와 repository adapter가 모두 이 값을 planning workspace boundary로 사용한다.
    pub workspace_directory: &'a str,
    // worker/runtime layer가 내린 queue-idle decision을 나중에 추적하기 위해 보존되는 turn id다.
    pub root_turn_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * caller는 proposal이 실제 promote됐는지와 무관하게 fresh runtime snapshot을 받는다. worker orchestration은 service
 * 반환 뒤 같은 snapshot shape로 render/submit을 이어가고, promoted/title/notices는 authority 변경 여부를 설명하는
 * 보조 정보로만 사용하면 된다.
 */
pub struct PlanningProposalPromotionOutcome {
    // no-op check 뒤 또는 commit 뒤 다시 읽은 snapshot이다. queue state가 authoritative store를 반영하게 한다.
    pub runtime_snapshot: PlanningRuntimeSnapshot,
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
    // runtime snapshot reload는 일반 worker turn에서도 쓰는 같은 prompt service에 위임한다.
    planning_prompt_service: PlanningPromptService,
    // promotion write는 mutation 전 workspace가 valid할 때만 허용된다.
    planning_validation_service: PlanningValidationService,
    // domain projection은 이미 ready task가 있는지와 어떤 proposal이 1순위인지 판단한다.
    priority_queue_service: PriorityQueueService,
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
            planning_task_repository_port,
        }
    }

    /*
     * executable queue가 비어 있을 때 proposed task 하나만 promote한다. 이 operation은 의도적으로 보수적이다.
     * ready task가 이미 있거나 promotable proposal이 없으면 authority를 쓰지 않고 current runtime snapshot만 돌려준다.
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
        // domain queue projection이 "이미 실행 가능한 work가 있는가"와 "top proposal은 무엇인가"를 동시에 정의한다.
        let queue_projection = self
            .priority_queue_service
            .build_projection(&directions, &task_authority)?;
        // ready next task가 있으면 worker orchestration에는 이미 실행 가능한 work가 있다. 이때 promote하면 operator/model이
        // 정한 실행 순서를 임의로 바꾸게 된다.
        if queue_projection.next_task.is_some() || queue_projection.proposed_tasks.is_empty() {
            return Ok(PlanningProposalPromotionOutcome {
                runtime_snapshot: self
                    .planning_prompt_service
                    .load_runtime_snapshot(request.workspace_directory)?,
                notices: Vec::new(),
                promoted_task_title: None,
                promoted: false,
            });
        }

        // 방금 평가한 ordering에서 selected proposal이 drift하지 않도록 projection에서 바로 consume한다.
        let top_proposal = queue_projection
            .proposed_tasks
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("proposal promotion requested without a promotable proposal"))?;
        // authority rewrite는 title이 아니라 task id로 수행한다. title은 presentation text라 중복되거나 변경될 수 있다.
        let promoted_task = task_authority
            .tasks
            .iter_mut()
            .find(|task| task.id.trim() == top_proposal.task_id.trim())
            .ok_or_else(|| {
                anyhow!(
                    "top promotable proposal {} was not found in task authority",
                    top_proposal.task_id
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
            // 성공한 commit 뒤에는 refreshed runtime snapshot이 다음 source of truth가 된다.
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
        let runtime_snapshot = self
            .planning_prompt_service
            .load_runtime_snapshot(request.workspace_directory)?;

        // notice는 projection title을 사용한다. queue가 operator/worker에게 보여 준 task 표현과 같은 wording을 유지한다.
        let promoted_task_title = top_proposal.task_title.trim().to_string();
        let mut notices = Vec::new();
        notices.push(format!(
            "host promoted top follow-up proposal into the executable queue: {}",
            promoted_task_title
        ));

        Ok(PlanningProposalPromotionOutcome {
            runtime_snapshot,
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
