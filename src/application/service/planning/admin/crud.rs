use std::collections::BTreeSet;

use anyhow::{Result, anyhow, bail};

use super::direction_mutation::{
    PlanningAdminDirectionMutationCommand, PlanningAdminDirectionMutationService,
};
use super::documents::{default_direction_id, normalized_required_id, remove_task_references};
use super::projection::map_management_view;
use super::{
    PlanningAdminCrudOutcome, PlanningAdminDirectionDeleteRequest,
    PlanningAdminDirectionMutationRequest, PlanningAdminFacadeService, PlanningAdminManagementView,
    PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
use crate::application::service::planning::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskMutationCommand, PlanningTaskMutationRequest,
    PlanningTaskMutationSource, PlanningTaskUpdateInput,
};
use crate::domain::planning::{OriginSessionKind, TaskMutationProvenance, TaskStatus};

/*
 * admin CRUD는 operator-facing mutation bridge다. direction 변경은 direction catalog와 task cascade를 함께
 * 다루는 전용 document service로 위임하고, task upsert는 shared PlanningTaskMutationService 명령으로 변환한다.
 * 이 구조 덕분에 admin form edit도 runtime/model task command와 같은 default 적용, queue rebuild, validation
 * path를 통과한다. 예외는 task delete뿐인데, 이것은 operator maintenance 전용으로 직접 document를 정리한다.
 */
impl PlanningAdminFacadeService {
    pub fn load_management_view(&self) -> Result<PlanningAdminManagementView> {
        // management view는 매 mutation 후 accepted authority에서 다시 읽는다. 방금 처리한 in-memory command 결과를
        // 재사용하지 않는 이유는 commit boundary의 repair/default/validation이 실제 저장된 모양을 바꿀 수 있기 때문이다.
        let documents = self.load_operator_planning_documents()?;
        Ok(map_management_view(
            &documents.directions,
            &documents.task_authority,
            default_direction_id(&documents.directions)?,
        ))
    }
    pub fn upsert_direction(
        &self,
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        // direction upsert는 catalog document semantics가 강하므로 task mutation service를 거치지 않는다. 전용
        // direction mutation service가 id 생성, default 보장, dependent cleanup 정책을 담당하고 facade는 refreshed
        // management view와 notice만 조립한다.
        let outcome = PlanningAdminDirectionMutationService::new(self)
            .apply(PlanningAdminDirectionMutationCommand::Upsert(request))?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: if outcome.updated {
                format!("direction `{}` updated", outcome.direction_id)
            } else {
                format!("direction `{}` added", outcome.direction_id)
            },
            management,
        })
    }
    pub fn delete_direction(
        &self,
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        // default direction delete는 실제 삭제가 아니라 retained outcome으로 보고한다. blank task creation fallback과
        // bootstrap repair anchor라서 operator가 삭제를 눌러도 service가 다시 보장하고 no-op에 가까운 notice를 돌려준다.
        let outcome = PlanningAdminDirectionMutationService::new(self)
            .apply(PlanningAdminDirectionMutationCommand::Delete(request))?;
        let management = self.load_management_view()?;
        if !outcome.deleted {
            return Ok(PlanningAdminCrudOutcome {
                notice: format!("default direction `{}` is retained", outcome.direction_id),
                management,
            });
        }
        Ok(PlanningAdminCrudOutcome {
            notice: format!(
                "direction `{}` deleted with {} child tasks",
                outcome.direction_id, outcome.removed_task_count
            ),
            management,
        })
    }
    pub fn upsert_task(
        &self,
        request: PlanningAdminTaskMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        // task form submit은 document를 직접 고치지 않고 common mutation service로 들어간다. admin/operator 입력도
        // worker/runtime/model command와 같은 priority 계산과 queue projection commit 규칙을 공유해야 하기 때문이다.
        self.ensure_default_authority()?;
        let updated = !request.id.trim().is_empty();
        let command = task_command_from_request(request)?;
        let commit = self
            .task_mutation_service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: self.workspace_dir.clone(),
                // admin UI에서 온 명시적 operator edit이므로 source는 User다. model-generated command와 구분해야
                // audit/debug에서 사람이 바꾼 task와 자동 추출 task를 나눠 볼 수 있다.
                source: PlanningTaskMutationSource::User,
                source_turn_id: None,
                provenance: TaskMutationProvenance::new(OriginSessionKind::System),
                commands: vec![command],
            })?;
        let task_id = commit.committed_task_ids.first().cloned().ok_or_else(|| {
            anyhow!("planning task mutation completed without returning a task id")
        })?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: if updated {
                format!("task `{task_id}` updated")
            } else {
                format!("task `{task_id}` added")
            },
            management,
        })
    }
    pub fn delete_task(
        &self,
        request: PlanningAdminTaskDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        // admin delete는 명시적 operator maintenance action이다. LLM/runtime task command는 여전히 task를 삭제할 수
        // 없고 `cancelled`로 이동해야 한다. 그래서 delete만 shared mutation service 바깥의 admin-only path로 남긴다.
        let task_id = normalized_required_id(&request.id, "task id")?;
        let mut documents = self.load_operator_planning_documents()?;
        let original_count = documents.task_authority.tasks.len();
        // 직접 삭제는 이 operator path로 제한된다. 삭제 후 dangling dependency/blocker reference를 남기면 queue
        // validation과 rank reason이 없는 task를 가리키게 되므로 commit 전에 graph reference를 함께 정리한다.
        documents
            .task_authority
            .tasks
            .retain(|task| task.id.trim() != task_id);
        if documents.task_authority.tasks.len() == original_count {
            bail!("task `{task_id}` was not found");
        }
        remove_task_references(
            &mut documents.task_authority,
            &BTreeSet::from([task_id.to_string()]),
        );
        self.commit_operator_planning_documents(documents)?;
        let management = self.load_management_view()?;
        Ok(PlanningAdminCrudOutcome {
            notice: format!("task `{task_id}` deleted"),
            management,
        })
    }
}

fn task_command_from_request(
    request: PlanningAdminTaskMutationRequest,
) -> Result<PlanningTaskMutationCommand> {
    // blank id는 create, nonblank id는 update다. admin form은 숫자/status/reference를 모두 text field로 보내므로,
    // 이 mapper가 browser payload를 typed task mutation command로 낮추는 좁은 parser boundary다.
    let task_id = request.id.trim().to_string();
    if task_id.is_empty() {
        return Ok(PlanningTaskMutationCommand::CreateTask(
            PlanningTaskCreateInput {
                direction_id: optional_id(request.direction_id, "direction id")?,
                direction_relation_note: None,
                title: required_text(&request.title, "task title")?.to_string(),
                description: optional_text(request.description),
                status: optional_task_status(&request.status)?,
                base_priority: optional_i32(&request.base_priority, "base priority")?,
                dynamic_priority_delta: optional_i32(
                    &request.dynamic_priority_delta,
                    "dynamic priority delta",
                )?,
                priority_reason: optional_text(request.priority_reason),
                depends_on: split_references(&request.depends_on_text),
                blocked_by: split_references(&request.blocked_by_text),
            },
        ));
    }

    // update는 priority_reason에 Some(empty_string), reference에는 Some(empty_vec)를 허용한다. create에서는 blank가
    // default를 뜻하지만, update에서는 operator가 기존 값을 명시적으로 비우는 행위여야 하기 때문이다.
    Ok(PlanningTaskMutationCommand::UpdateTask(
        PlanningTaskUpdateInput {
            task_id: normalized_required_id(&task_id, "task id")?.to_string(),
            direction_id: optional_id(request.direction_id, "direction id")?,
            direction_relation_note: None,
            title: Some(required_text(&request.title, "task title")?.to_string()),
            description: optional_text(request.description),
            status: optional_task_status(&request.status)?,
            base_priority: optional_i32(&request.base_priority, "base priority")?,
            dynamic_priority_delta: optional_i32(
                &request.dynamic_priority_delta,
                "dynamic priority delta",
            )?,
            priority_reason: Some(request.priority_reason.trim().to_string()),
            depends_on: Some(split_references(&request.depends_on_text)),
            blocked_by: Some(split_references(&request.blocked_by_text)),
        },
    ))
}

fn optional_id(value: String, label: &str) -> Result<Option<String>> {
    // optional id는 blank일 수 있지만, 값이 있으면 graph reference와 route parameter로 안전해야 한다. 이 함수가
    // normalized_required_id를 재사용해 admin task form과 direction document helper가 같은 id 규칙을 공유한다.
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(normalized_required_id(value, label)?.to_string()))
    }
}

fn required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

fn optional_text(value: String) -> Option<String> {
    // create input에서 None은 common mutation default를 호출한다는 뜻이다. blank string을 그대로 넣으면 "비어 있는
    // description"과 "default를 적용할 description 없음"을 구분할 수 없어진다.
    let value = value.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
}

fn optional_task_status(raw: &str) -> Result<Option<TaskStatus>> {
    // status string은 admin management view가 렌더링한 label과 맞물린다. projection과 parser가 같은 label set을
    // 공유해야 form round-trip 중 status가 domain enum으로 안정적으로 돌아온다.
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    Ok(Some(match raw.to_ascii_lowercase().as_str() {
        "ready" => TaskStatus::Ready,
        "blocked" => TaskStatus::Blocked,
        "in_progress" => TaskStatus::InProgress,
        "done" => TaskStatus::Done,
        "cancelled" => TaskStatus::Cancelled,
        "awaiting_user" => TaskStatus::AwaitingUser,
        "proposed" => TaskStatus::Proposed,
        other => bail!("unknown task status `{other}`"),
    }))
}

fn optional_i32(raw: &str, label: &str) -> Result<Option<i32>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse::<i32>()
        .map(Some)
        .map_err(|error| anyhow::anyhow!("{label} must be an integer: {error}"))
}

fn split_references(raw: &str) -> Vec<String> {
    // reference field는 빠른 편집을 위해 comma list와 textarea line을 모두 허용한다. parser는 순서를 보존하면서
    // blank item만 제거해 dependency list가 operator가 적은 순서대로 mutation service에 전달되게 한다.
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{PlanningAdminTaskMutationRequest, task_command_from_request};
    use crate::application::service::planning::task_mutation::PlanningTaskMutationCommand;
    use crate::domain::planning::TaskStatus;

    #[test]
    fn admin_task_create_maps_blank_fields_to_common_defaults() {
        // create에서 blank admin field는 absent로 남아야 한다. direction fallback, priority default 같은 공통
        // 생성 규칙은 admin parser가 아니라 shared mutation service에 중앙화되어 있다.
        let command = task_command_from_request(PlanningAdminTaskMutationRequest {
            id: String::new(),
            direction_id: String::new(),
            title: "Ship admin task bridge".to_string(),
            description: String::new(),
            status: String::new(),
            base_priority: String::new(),
            dynamic_priority_delta: String::new(),
            priority_reason: String::new(),
            depends_on_text: "task-a, task-b\n task-c".to_string(),
            blocked_by_text: String::new(),
        })
        .expect("admin request should map to common create command");
        let PlanningTaskMutationCommand::CreateTask(input) = command else {
            panic!("expected create command");
        };
        assert_eq!(input.direction_id, None);
        assert_eq!(input.description, None);
        assert_eq!(input.status, None);
        assert_eq!(input.base_priority, None);
        assert_eq!(input.dynamic_priority_delta, None);
        assert_eq!(input.depends_on, vec!["task-a", "task-b", "task-c"]);
    }

    #[test]
    fn admin_task_update_maps_to_common_update_command() {
        // update는 model-oriented command extraction path를 거치지 않지만 같은 UpdateTask command로 내려간다.
        // admin edit가 title/status/priority/reference를 명시 값으로 교체할 수 있어야 하기 때문이다.
        let command = task_command_from_request(PlanningAdminTaskMutationRequest {
            id: "task-1".to_string(),
            direction_id: "general-workstream".to_string(),
            title: "Updated task".to_string(),
            description: "Updated description".to_string(),
            status: "blocked".to_string(),
            base_priority: "90".to_string(),
            dynamic_priority_delta: "-5".to_string(),
            priority_reason: "waiting for review".to_string(),
            depends_on_text: "task-a".to_string(),
            blocked_by_text: "task-b".to_string(),
        })
        .expect("admin request should map to common update command");
        let PlanningTaskMutationCommand::UpdateTask(input) = command else {
            panic!("expected update command");
        };
        assert_eq!(input.task_id, "task-1");
        assert_eq!(input.direction_id.as_deref(), Some("general-workstream"));
        assert_eq!(input.title.as_deref(), Some("Updated task"));
        assert_eq!(input.description.as_deref(), Some("Updated description"));
        assert_eq!(input.status, Some(TaskStatus::Blocked));
        assert_eq!(input.base_priority, Some(90));
        assert_eq!(input.dynamic_priority_delta, Some(-5));
        assert_eq!(input.priority_reason.as_deref(), Some("waiting for review"));
        assert_eq!(input.depends_on, Some(vec!["task-a".to_string()]));
        assert_eq!(input.blocked_by, Some(vec!["task-b".to_string()]));
    }
}
