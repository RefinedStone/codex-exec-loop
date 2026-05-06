use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PriorityQueueProjection,
    PriorityQueueService, PriorityQueueTask, TaskActor, TaskAuthorityDocument, TaskDefinition,
    TaskStatus,
};
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use std::sync::Arc;
const DEFAULT_TASK_PRIORITY: i32 = 80;
const MAX_REVISION_CONFLICT_RETRIES: usize = 3;
const MAX_COLLISION_SUFFIX_ATTEMPTS: u32 = 20;
const TASK_ID_HASH_CHARS: usize = 12;
const MAX_TASK_MUTATION_COMMANDS: usize = 16;
mod commands;
mod helpers;
mod validation;
pub use self::commands::{
    PlanningTaskCommandExtraction, PlanningTaskCreateInput, PlanningTaskMutationCommand,
    PlanningTaskUpdateInput, extract_planning_task_commands,
};
use self::helpers::{
    build_task_id, default_relation_note, direction_title, find_direction, format_timestamp,
    increment_suffix, normalize_references, required_id, required_text, select_direction,
    task_id_exists, terminal_status,
};

/*
 * planning task의 write-side gateway다. TUI user flow, runtime intake, worker/LLM command
 * extraction이 모두 같은 authority-document path를 통과하게 만든다. 이 경계를 통일해야
 * optimistic revision, queue projection rebuild, audit attribution이 entry point별로 갈라지지 않는다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningTaskMutationSource {
    User,
    Llm,
    System,
}
impl PlanningTaskMutationSource {
    // domain audit record는 actor identity를 저장하고, task-id generation은 stable slug가 필요하다.
    // 두 mapping을 source enum 가까이에 두면 helper code가 inbound mutation attribution을 추측하지 않는다.
    fn actor(self) -> TaskActor {
        match self {
            Self::User => TaskActor::User,
            Self::Llm => TaskActor::Llm,
            Self::System => TaskActor::System,
        }
    }
    fn id_slug(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Llm => "llm",
            Self::System => "system",
        }
    }
    fn can_update_existing_description(self) -> bool {
        matches!(self, Self::User)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskMutationRequest {
    pub workspace_directory: String,
    // source/source_turn_id는 command batch 전체와 함께 이동한다. create와 update가 같은
    // actor/provenance 규칙으로 audit field를 채우게 하려는 요청 단위 metadata다.
    pub source: PlanningTaskMutationSource,
    pub source_turn_id: Option<String>,
    pub commands: Vec<PlanningTaskMutationCommand>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskCreatePreviewRequest {
    pub workspace_directory: String,
    pub source: PlanningTaskMutationSource,
    pub source_turn_id: Option<String>,
    pub input: PlanningTaskCreateInput,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskCreatePreview {
    // 원본 request를 보존해 inbound layer가 나중에 approximate command envelope를 재구성하지 않고
    // preview 그대로 commit할 수 있게 한다.
    pub request: PlanningTaskCreatePreviewRequest,
    pub task: TaskDefinition,
    pub direction_title: String,
    pub generated_at: DateTime<Utc>,
    // preview된 id가 commit 중 충돌하면 service는 suffix만 전진시키고 generated_at은 유지한다.
    // 이렇게 해야 retry id가 시간 흐름이 아니라 충돌 횟수에만 반응한다.
    pub collision_suffix: Option<u32>,
    pub observed_planning_revision: i64,
    pub queue_head: Option<PriorityQueueTask>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskMutationCommitResult {
    pub committed_planning_revision: i64,
    // queue_head는 caller cache가 아니라 방금 재계산한 projection에서 온다. commit 직후 TUI의
    // next-task view가 저장된 authority와 같은 상태를 보게 하기 위한 응답 값이다.
    pub queue_head: Option<PriorityQueueTask>,
    pub task_authority_changed: bool,
    pub applied_command_count: usize,
    pub committed_task_ids: Vec<String>,
}
#[derive(Clone)]
pub struct PlanningTaskMutationService {
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    priority_queue_service: PriorityQueueService,
}
impl PlanningTaskMutationService {
    pub fn new(
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_task_repository_port,
            priority_queue_service,
        }
    }
    pub fn preview_create_task(
        &self,
        request: PlanningTaskCreatePreviewRequest,
    ) -> Result<PlanningTaskCreatePreview> {
        // preview는 commit과 같은 builder/validation을 사용하고 authority snapshot write만 생략한다.
        // 사용자가 보는 preview가 commit 가능하지 않은 별도 객체가 되지 않게 하는 구조다.
        let context = self.load_context(&request.workspace_directory)?;
        self.preview_create_task_with_authority(
            request,
            &context.directions,
            &context.task_authority,
            context.task_planning_revision,
        )
    }
    pub(crate) fn preview_create_task_with_authority(
        &self,
        request: PlanningTaskCreatePreviewRequest,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
        task_planning_revision: i64,
    ) -> Result<PlanningTaskCreatePreview> {
        let generated_at = Utc::now();
        let task = self.build_unique_task(
            &request.input,
            request.source,
            request.source_turn_id.as_deref(),
            PlanningTaskAuthorityView {
                directions,
                task_authority,
            },
            generated_at,
            None,
        )?;
        let direction_title = direction_title(directions, &task.direction_id)
            .unwrap_or_else(|| task.direction_id.clone());
        let mut next_task_authority = task_authority.clone();
        next_task_authority.tasks.push(task.clone());
        // preview는 결과 authority document가 실제로 validation을 통과하고 app이 표시할 queue projection을
        // 만들 수 있을 때만 의미가 있다.
        let queue_projection = self.validate_and_project(directions, &next_task_authority)?;
        Ok(PlanningTaskCreatePreview {
            request,
            task,
            direction_title,
            generated_at,
            collision_suffix: None,
            observed_planning_revision: task_planning_revision,
            queue_head: queue_projection.next_task,
        })
    }
    pub fn commit_create_preview(
        &self,
        preview: &PlanningTaskCreatePreview,
    ) -> Result<PlanningTaskMutationCommitResult> {
        let mut observed_revision = preview.observed_planning_revision;
        let mut next_suffix = preview.collision_suffix;
        /*
         * preview는 다른 writer와 race할 수 있다. 첫 시도는 사용자가 확인한 원래 task를 보존하고,
         * revision conflict 이후에는 최신 authority snapshot을 기준으로 task id를 다시 계산한다.
         * 새로 commit된 work와 충돌하지 않게 하면서도 generated_at은 preview 시각에 묶어 둔다.
         */
        for _ in 0..=MAX_REVISION_CONFLICT_RETRIES {
            let context = self.load_context(&preview.request.workspace_directory)?;
            let task = if context.task_planning_revision == preview.observed_planning_revision
                && next_suffix == preview.collision_suffix
            {
                preview.task.clone()
            } else {
                self.build_unique_task(
                    &preview.request.input,
                    preview.request.source,
                    preview.request.source_turn_id.as_deref(),
                    PlanningTaskAuthorityView {
                        directions: &context.directions,
                        task_authority: &context.task_authority,
                    },
                    preview.generated_at,
                    next_suffix,
                )?
            };
            let committed_task_id = task.id.clone();
            let mut next_task_authority = context.task_authority.clone();
            next_task_authority.tasks.push(task);
            let queue_projection =
                self.validate_and_project(&context.directions, &next_task_authority)?;
            match self.commit_authority(
                &preview.request.workspace_directory,
                Some(observed_revision),
                &next_task_authority,
                &queue_projection,
            )? {
                PlanningTaskAuthorityCommitResult::Committed { planning_revision } => {
                    return Ok(PlanningTaskMutationCommitResult {
                        committed_planning_revision: planning_revision,
                        queue_head: queue_projection.next_task,
                        task_authority_changed: true,
                        applied_command_count: 1,
                        committed_task_ids: vec![committed_task_id],
                    });
                }
                PlanningTaskAuthorityCommitResult::Conflict {
                    current_planning_revision,
                    ..
                } => {
                    // revision conflict는 authority set이 바뀌었을 수 있다는 뜻이다. 다음 loop는
                    // 새 revision을 관찰하고 collision suffix도 한 단계 올려 같은 id 재시도를 피한다.
                    observed_revision = current_planning_revision;
                    next_suffix = increment_suffix(next_suffix);
                }
            }
        }

        bail!("planning task mutation could not commit because planning state kept changing")
    }
    pub fn apply_commands(
        &self,
        request: PlanningTaskMutationRequest,
    ) -> Result<PlanningTaskMutationCommitResult> {
        // worker response는 repository를 건드리기 전에 command 수를 제한한다. extractor가 나쁜
        // batch를 넘겨도 oversized authority rewrite로 이어지지 않게 하는 선행 guard다.
        if request.commands.len() > MAX_TASK_MUTATION_COMMANDS {
            bail!(
                "planning task mutation accepts at most {MAX_TASK_MUTATION_COMMANDS} command(s) per worker response"
            );
        }
        if request.commands.is_empty() {
            // empty command batch도 caller가 최신 revision과 queue head를 관찰하게 하지만,
            // no-op commit은 만들지 않는다.
            let context = self.load_context(&request.workspace_directory)?;
            let queue_projection =
                self.validate_and_project(&context.directions, &context.task_authority)?;
            return Ok(PlanningTaskMutationCommitResult {
                committed_planning_revision: context.task_planning_revision,
                queue_head: queue_projection.next_task,
                task_authority_changed: false,
                applied_command_count: 0,
                committed_task_ids: Vec::new(),
            });
        }
        let mut observed_revision = None;
        // command batch는 retry마다 최신 authority에서 다시 적용된다. create id, update guard,
        // queue projection이 optimistic commit 직전 하나의 일관된 snapshot에서 계산되게 한다.
        for _ in 0..=MAX_REVISION_CONFLICT_RETRIES {
            let context = self.load_context(&request.workspace_directory)?;
            observed_revision = Some(context.task_planning_revision);
            let mut next_task_authority = context.task_authority.clone();
            let application = self.apply_commands_to_authority(
                &request,
                &context.directions,
                &mut next_task_authority,
                Utc::now(),
            )?;
            let queue_projection =
                self.validate_and_project(&context.directions, &next_task_authority)?;
            if !application.changed {
                // 현재 task definition과 같게 normalize되는 update는 inspected id를 보고하되
                // authority file은 다시 쓰지 않는다.
                return Ok(PlanningTaskMutationCommitResult {
                    committed_planning_revision: context.task_planning_revision,
                    queue_head: queue_projection.next_task,
                    task_authority_changed: false,
                    applied_command_count: 0,
                    committed_task_ids: application.committed_task_ids,
                });
            }
            match self.commit_authority(
                &request.workspace_directory,
                observed_revision,
                &next_task_authority,
                &queue_projection,
            )? {
                PlanningTaskAuthorityCommitResult::Committed { planning_revision } => {
                    return Ok(PlanningTaskMutationCommitResult {
                        committed_planning_revision: planning_revision,
                        queue_head: queue_projection.next_task,
                        task_authority_changed: true,
                        applied_command_count: request.commands.len(),
                        committed_task_ids: application.committed_task_ids,
                    });
                }
                PlanningTaskAuthorityCommitResult::Conflict { .. } => continue,
            }
        }
        let observed_revision = observed_revision.unwrap_or_default();
        bail!(
            "planning task mutation could not commit because planning state kept changing after observed revision {observed_revision}"
        )
    }
    fn load_context(&self, workspace_directory: &str) -> Result<PlanningTaskMutationContext> {
        // task validation은 direction id와 현재 planning format에 의존하므로 direction/task
        // authority를 같은 context로 읽는다.
        let direction_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_directory)?
            .ok_or_else(|| anyhow!("planning direction authority is unavailable"))?;
        let task_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_directory)?
            .ok_or_else(|| anyhow!("planning task authority is unavailable"))?;
        if direction_snapshot.directions.version != PLANNING_FORMAT_VERSION {
            bail!(
                "unsupported direction authority version {}; expected {}",
                direction_snapshot.directions.version,
                PLANNING_FORMAT_VERSION
            );
        }
        if task_snapshot.task_authority.version != PLANNING_FORMAT_VERSION {
            bail!(
                "unsupported task authority version {}; expected {}",
                task_snapshot.task_authority.version,
                PLANNING_FORMAT_VERSION
            );
        }
        Ok(PlanningTaskMutationContext {
            directions: direction_snapshot.directions,
            task_authority: task_snapshot.task_authority,
            task_planning_revision: task_snapshot.planning_revision,
        })
    }
    fn apply_commands_to_authority(
        &self,
        request: &PlanningTaskMutationRequest,
        directions: &DirectionCatalogDocument,
        task_authority: &mut TaskAuthorityDocument,
        updated_at: DateTime<Utc>,
    ) -> Result<PlanningTaskMutationApplication> {
        let mut committed_task_ids = Vec::new();
        let mut changed = false;
        for command in &request.commands {
            // in-memory authority document가 유일한 mutation target이다. batch 전체를 적용한 뒤
            // validation과 persistence를 실행해 중간 상태가 repository에 보이지 않게 한다.
            match command {
                PlanningTaskMutationCommand::CreateTask(input) => {
                    let task = self.build_unique_task(
                        input,
                        request.source,
                        request.source_turn_id.as_deref(),
                        PlanningTaskAuthorityView {
                            directions,
                            task_authority,
                        },
                        updated_at,
                        None,
                    )?;
                    committed_task_ids.push(task.id.clone());
                    task_authority.tasks.push(task);
                    changed = true;
                }
                PlanningTaskMutationCommand::UpdateTask(input) => {
                    let updated = self.apply_update(
                        input,
                        request.source,
                        request.source_turn_id.as_deref(),
                        directions,
                        task_authority,
                        updated_at,
                    )?;
                    // update가 no-op이어도 addressed id를 포함한다. caller는 어떤 command가
                    // 검사됐는지 correlation할 수 있다.
                    committed_task_ids.push(input.task_id.trim().to_string());
                    changed |= updated;
                }
            }
        }
        Ok(PlanningTaskMutationApplication {
            committed_task_ids,
            changed,
        })
    }
    fn build_unique_task(
        &self,
        input: &PlanningTaskCreateInput,
        source: PlanningTaskMutationSource,
        source_turn_id: Option<&str>,
        authority: PlanningTaskAuthorityView<'_>,
        generated_at: DateTime<Utc>,
        starting_suffix: Option<u32>,
    ) -> Result<TaskDefinition> {
        let mut suffix = starting_suffix;
        // task id는 content/time/source에서 파생된다. bounded suffix loop는 collision escape hatch일 뿐,
        // 무한 allocation 전략이 아니다.
        for _ in 0..MAX_COLLISION_SUFFIX_ATTEMPTS {
            let task = self.build_task(
                input,
                source,
                source_turn_id,
                authority.directions,
                generated_at,
                suffix,
            )?;
            if !task_id_exists(authority.task_authority, &task.id) {
                return Ok(task);
            }
            suffix = increment_suffix(suffix);
        }
        bail!("planning task mutation could not allocate a unique task id")
    }
    fn build_task(
        &self,
        input: &PlanningTaskCreateInput,
        source: PlanningTaskMutationSource,
        source_turn_id: Option<&str>,
        directions: &DirectionCatalogDocument,
        generated_at: DateTime<Utc>,
        collision_suffix: Option<u32>,
    ) -> Result<TaskDefinition> {
        let title = required_text(&input.title, "task title")?.to_string();
        // description이 없으면 의도적으로 title을 fallback으로 쓴다. generated task가 compact queue
        // surface에서도 최소한 표시 가능한 설명을 갖게 하기 위해서다.
        let description = input
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(title.as_str())
            .to_string();
        let direction = select_direction(input.direction_id.as_deref(), directions)?;
        let actor = source.actor();
        let dynamic_priority_delta = input.dynamic_priority_delta.unwrap_or(0);
        let priority_reason = input
            .priority_reason
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if dynamic_priority_delta != 0 && priority_reason.is_empty() {
            bail!(
                "task `{title}` must include priority_reason when dynamic_priority_delta is non-zero"
            );
        }
        // authority document에 들어가기 전 모든 field를 normalize한다. 이후 update path가 structural
        // equality만으로 no-op 여부를 판단할 수 있게 하려는 전처리다.
        Ok(TaskDefinition {
            id: build_task_id(source, generated_at, &title, collision_suffix),
            direction_id: direction.id.trim().to_string(),
            direction_relation_note: default_relation_note(
                input.direction_relation_note.as_deref(),
                direction,
            ),
            title,
            description,
            status: input.status.unwrap_or(TaskStatus::Ready),
            base_priority: input.base_priority.unwrap_or(DEFAULT_TASK_PRIORITY),
            dynamic_priority_delta,
            priority_reason,
            depends_on: normalize_references(&input.depends_on),
            blocked_by: normalize_references(&input.blocked_by),
            created_by: actor,
            last_updated_by: actor,
            source_turn_id: source_turn_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            updated_at: format_timestamp(generated_at),
        })
    }
    fn apply_update(
        &self,
        input: &PlanningTaskUpdateInput,
        source: PlanningTaskMutationSource,
        source_turn_id: Option<&str>,
        directions: &DirectionCatalogDocument,
        task_authority: &mut TaskAuthorityDocument,
        updated_at: DateTime<Utc>,
    ) -> Result<bool> {
        let task_id = required_id(&input.task_id, "task id")?.to_string();
        let task = task_authority
            .tasks
            .iter_mut()
            .find(|task| task.id.trim() == task_id)
            .ok_or_else(|| anyhow!("task `{task_id}` does not exist"))?;
        let previous_task = task.clone();

        /*
         * update command는 partial patch다. absent optional field는 기존 값을 보존하고,
         * present field는 create와 같은 trimming, direction, priority, terminal-status guard를 통과한다.
         */
        if let Some(direction_id) = input.direction_id.as_deref() {
            let direction = find_direction(direction_id, directions)?;
            task.direction_id = direction.id.trim().to_string();
            if input.direction_relation_note.is_none()
                && task.direction_relation_note.trim().is_empty()
            {
                // direction move의 default relation note는 caller가 note를 제공하지 않았고 현재 note가
                // blank일 때만 채운다. 기존 audit 설명을 불필요하게 덮어쓰지 않기 위해서다.
                task.direction_relation_note = default_relation_note(None, direction);
            }
        }
        if let Some(direction_relation_note) = input.direction_relation_note.as_deref() {
            task.direction_relation_note = direction_relation_note.trim().to_string();
        }
        if let Some(title) = input.title.as_deref() {
            task.title = required_text(title, "task title")?.to_string();
        }
        if source.can_update_existing_description() {
            if let Some(description) = input.description.as_deref() {
                task.description = required_text(description, "task description")?.to_string();
            }
        } else if task.description.trim().is_empty()
            && let Some(description) = input.description.as_deref()
        {
            task.description = required_text(description, "task description")?.to_string();
        }
        if let Some(status) = input.status {
            if terminal_status(task.status) && task.status != status {
                bail!(
                    "task `{}` cannot change from terminal status `{}` to `{}`",
                    task.id.trim(),
                    task.status.label(),
                    status.label()
                );
            }
            task.status = status;
        }
        if let Some(base_priority) = input.base_priority {
            task.base_priority = base_priority;
        }
        if let Some(dynamic_priority_delta) = input.dynamic_priority_delta {
            task.dynamic_priority_delta = dynamic_priority_delta;
        }
        if let Some(priority_reason) = input.priority_reason.as_deref() {
            task.priority_reason = priority_reason.trim().to_string();
        }
        if let Some(depends_on) = input.depends_on.as_ref() {
            task.depends_on = normalize_references(depends_on);
        }
        if let Some(blocked_by) = input.blocked_by.as_ref() {
            task.blocked_by = normalize_references(blocked_by);
        }
        if task.dynamic_priority_delta != 0 && task.priority_reason.trim().is_empty() {
            bail!(
                "task `{}` must include priority_reason when dynamic_priority_delta is non-zero",
                task.id.trim()
            );
        }
        if *task == previous_task {
            return Ok(false);
        }
        task.last_updated_by = source.actor();
        if let Some(source_turn_id) = source_turn_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            task.source_turn_id = Some(source_turn_id.to_string());
        }
        task.updated_at = format_timestamp(updated_at);
        Ok(true)
    }
    fn commit_authority(
        &self,
        workspace_directory: &str,
        observed_planning_revision: Option<i64>,
        task_authority: &TaskAuthorityDocument,
        queue_projection: &PriorityQueueProjection,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        // compare-and-swap semantics는 repository port가 소유한다. 이 layer는 이미 검증된
        // task authority와 그에 대응하는 queue projection을 함께 넘긴다.
        self.planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_directory,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision,
                    task_authority,
                    queue_projection,
                },
            )
    }
}
#[derive(Debug, Clone)]
struct PlanningTaskMutationContext {
    // load된 authority document와 task authority를 읽을 때 관찰한 planning revision이다.
    directions: DirectionCatalogDocument,
    task_authority: TaskAuthorityDocument,
    task_planning_revision: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanningTaskMutationApplication {
    // id 목록은 changed 여부와 별도로 보고한다. no-op update도 어떤 task를 검사했는지 보이게 한다.
    committed_task_ids: Vec<String>,
    changed: bool,
}
#[derive(Debug, Clone, Copy)]
struct PlanningTaskAuthorityView<'a> {
    // id allocation과 direction validation에 필요한 read-only authority 묶음이다.
    directions: &'a DirectionCatalogDocument,
    task_authority: &'a TaskAuthorityDocument,
}
#[cfg(test)]
mod tests;
