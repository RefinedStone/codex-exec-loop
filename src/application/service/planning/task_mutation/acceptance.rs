//! 완료 기준의 사용자 소유권과 호스트 revision을 모든 typed mutation 경로에서 지킨다.
use super::{
    PlanningTaskCreateInput, PlanningTaskCreatePreview, PlanningTaskMutationCommand,
    PlanningTaskMutationRequest, PlanningTaskMutationSource, PlanningTaskUpdateInput,
};
use crate::domain::planning::{
    PlanningMutationRefusal, TaskAcceptanceCriterion, TaskAuthorityDocument, TaskDefinition,
    TaskStatus, normalize_task_acceptance,
};
use anyhow::{Result, bail};

pub(super) fn create_fields(
    input: &PlanningTaskCreateInput,
    source: PlanningTaskMutationSource,
) -> Result<(Vec<TaskAcceptanceCriterion>, u64)> {
    ensure_user_supplied(input.acceptance.is_some(), source)?;
    let acceptance = normalize_task_acceptance(input.acceptance.as_deref().unwrap_or_default())?;
    let revision = u64::from(!acceptance.is_empty());
    Ok((acceptance, revision))
}

fn ensure_user_supplied(supplied: bool, source: PlanningTaskMutationSource) -> Result<()> {
    if supplied && source != PlanningTaskMutationSource::User {
        bail!("only user task mutations may supply acceptance criteria");
    }
    Ok(())
}

pub(super) fn validate_request(request: &PlanningTaskMutationRequest) -> Result<()> {
    for command in &request.commands {
        match command {
            PlanningTaskMutationCommand::CreateTask(input) => {
                ensure_user_supplied(input.acceptance.is_some(), request.source)?;
            }
            PlanningTaskMutationCommand::UpdateTask(input) => {
                ensure_user_supplied(input.acceptance.is_some(), request.source)?;
                if input.acceptance.is_some() && input.expected_planning_revision.is_none() {
                    bail!("task acceptance changes require expected_planning_revision");
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_original_targets(
    request: &PlanningTaskMutationRequest,
    task_authority: &TaskAuthorityDocument,
) -> Result<()> {
    // 같은 batch의 선행 status patch가 원본 snapshot의 실행 상태를 지울 수 없다.
    for command in &request.commands {
        let PlanningTaskMutationCommand::UpdateTask(input) = command else {
            continue;
        };
        if input.acceptance.is_none() {
            continue;
        }
        let task_id = input.task_id.trim();
        if task_authority
            .tasks
            .iter()
            .any(|task| task.id.trim() == task_id && task.status == TaskStatus::InProgress)
        {
            return Err(PlanningMutationRefusal::locked(
                "task acceptance cannot change while the task is in progress",
            )
            .into());
        }
    }
    Ok(())
}

pub(super) fn validate_preview(preview: &PlanningTaskCreatePreview) -> Result<()> {
    let (acceptance, revision) = create_fields(&preview.request.input, preview.request.source)?;
    // preview.task는 public typed DTO이므로 최초 revision이 같아도 호스트 계산을 생략하지 않는다.
    if preview.task.acceptance != acceptance || preview.task.acceptance_revision != revision {
        bail!("task create preview acceptance no longer matches its request");
    }
    Ok(())
}

pub(super) fn apply_update(
    input: &PlanningTaskUpdateInput,
    source: PlanningTaskMutationSource,
    task: &mut TaskDefinition,
) -> Result<()> {
    let Some(supplied) = input.acceptance.as_ref() else {
        return Ok(());
    };
    ensure_user_supplied(true, source)?;
    if input.expected_planning_revision.is_none() {
        bail!("task acceptance changes require expected_planning_revision");
    }
    if task.status == TaskStatus::InProgress {
        return Err(PlanningMutationRefusal::locked(
            "task acceptance cannot change while the task is in progress",
        )
        .into());
    }
    let acceptance = normalize_task_acceptance(supplied)?;
    let current = normalize_task_acceptance(&task.acceptance)?;
    if acceptance == current {
        // 순서와 주변 공백만 달라진 no-op은 저장된 기준, revision, 감사 정보를 그대로 보존한다.
        return Ok(());
    }
    let revision = task
        .acceptance_revision
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("task acceptance revision overflow"))?;
    task.acceptance = acceptance;
    task.acceptance_revision = revision;
    Ok(())
}
