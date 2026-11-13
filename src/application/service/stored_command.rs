//! 저장 명령을 planning 서비스로 실행한다 (plan 18 K4-9d). port 는 `StoredCommandPort` 이다.

use anyhow::Result;

use crate::application::port::inbound::stored_command_port::{
    StoredCommand, StoredCommandOutcome, StoredCommandPort, StoredCommandRejection,
    StoredCommandResult, StoredTaskFields,
};
use crate::application::service::planning::PlanningAdminFacadeService;
use crate::application::service::planning::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskMutationCommand, PlanningTaskUpdateInput,
};
use crate::domain::planning::PlanningMutationRefusal;

/*
 * 실행기의 구현이다. 사용자의 planning 변경을 맡는 admin facade 를 쓴다. 이 facade 는 커널의 조립으로 만든 것이어야
 * 한다 (직접 실행). 입구의 조립으로 만든 facade 는 변경을 커널 명령으로 보내므로, 그것을 쓰면 명령이 자기에게 돌아온다.
 */
pub struct StoredCommandService {
    planning: PlanningAdminFacadeService,
}

impl StoredCommandService {
    pub fn new(planning: PlanningAdminFacadeService) -> Self {
        Self { planning }
    }

    fn run(&self, command: StoredCommand) -> Result<StoredCommandResult> {
        match command {
            StoredCommand::CreateTask { task } => {
                let change = self.planning.apply_operator_task_command(
                    PlanningTaskMutationCommand::CreateTask(create_input(task)),
                )?;
                Ok(StoredCommandResult::TaskCreated {
                    task_id: change.task_id,
                    planning_revision: change.planning_revision,
                })
            }
            StoredCommand::UpdateTask {
                task_id,
                task,
                expected_planning_revision,
            } => {
                let change = self.planning.apply_operator_task_command(
                    PlanningTaskMutationCommand::UpdateTask(update_input(
                        task_id,
                        task,
                        expected_planning_revision,
                    )),
                )?;
                Ok(StoredCommandResult::TaskUpdated {
                    task_id: change.task_id,
                    planning_revision: change.planning_revision,
                })
            }
            StoredCommand::DeleteTask {
                task_id,
                expected_planning_revision,
            } => {
                let planning_revision = self
                    .planning
                    .delete_operator_task(&task_id, Some(expected_planning_revision))?;
                Ok(StoredCommandResult::TaskDeleted {
                    task_id: task_id.trim().to_string(),
                    planning_revision,
                })
            }
        }
    }
}

impl StoredCommandPort for StoredCommandService {
    fn execute(&self, command: StoredCommand) -> Result<StoredCommandOutcome> {
        match self.run(command) {
            Ok(result) => Ok(StoredCommandOutcome::Done(result)),
            // 낡은 revision 과 잡힌 guard 는 실패가 아니라 거절이다. 아무것도 바뀌지 않았다.
            Err(error) => match PlanningMutationRefusal::find_in(&error) {
                Some(refusal) => Ok(StoredCommandOutcome::Rejected(
                    StoredCommandRejection::from(refusal),
                )),
                None => Err(error),
            },
        }
    }
}

fn create_input(task: StoredTaskFields) -> PlanningTaskCreateInput {
    PlanningTaskCreateInput {
        direction_id: task.direction_id,
        direction_relation_note: None,
        title: task.title,
        description: task.description,
        status: task.status,
        base_priority: task.base_priority,
        dynamic_priority_delta: task.dynamic_priority_delta,
        priority_reason: task.priority_reason,
        depends_on: task.depends_on,
        blocked_by: task.blocked_by,
    }
}

/*
 * 고치기는 작업의 필드 전체를 바꾼다 (admin 의 폼과 같은 뜻이다). 그래서 이유와 참조는 값이 없어도 "비운다"로
 * 보낸다. 방향, 설명, 상태, 우선순위는 값이 없으면 그대로 둔다. 서비스의 고치기 입력이 그 필드들에서 "없음"을
 * "그대로"로 읽기 때문이다.
 */
fn update_input(
    task_id: String,
    task: StoredTaskFields,
    expected_planning_revision: i64,
) -> PlanningTaskUpdateInput {
    PlanningTaskUpdateInput {
        expected_planning_revision: Some(expected_planning_revision),
        task_id,
        direction_id: task.direction_id,
        direction_relation_note: None,
        title: Some(task.title),
        description: task.description,
        status: task.status,
        base_priority: task.base_priority,
        dynamic_priority_delta: task.dynamic_priority_delta,
        priority_reason: Some(task.priority_reason.unwrap_or_default()),
        depends_on: Some(task.depends_on),
        blocked_by: Some(task.blocked_by),
    }
}
