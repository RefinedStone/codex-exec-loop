//! 제어 변경과 허가 철회는 같은 planning transaction에서 확정한다.
use super::super::{
    SqlitePlanningAuthorityAdapter, load_direction_catalog_from_connection,
    load_task_authority_snapshot_from_connection, open_authority_connection, runtime_projection,
    table_exists,
};
use super::{ensure_work_order_tables, load_task_membership, load_work_order_record, permits};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityMutationAudit,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControl, PlanningWorkControlAction, PlanningWorkControlRepositoryPort,
    PlanningWorkInstruction, PlanningWorkInstructionAdmission, validate_work_order_id,
    validate_work_order_task_id,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PriorityQueueService, TaskActor, TaskMutationProvenance, TaskStatus,
};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub(super) fn load_control(
    connection: &Connection,
    work_order_id: &str,
) -> Result<PlanningWorkControl> {
    let json: Option<String> = connection
        .query_row(
            "SELECT control_json FROM planning_work_order_controls WHERE work_order_id = ?1",
            params![work_order_id],
            |row| row.get(0),
        )
        .optional()?;
    json.map(|json| serde_json::from_str(&json).context("invalid work control record"))
        .transpose()
        .map(|control| control.unwrap_or_default())
}

fn validate_command_id(command_id: &str) -> Result<()> {
    if command_id.trim().is_empty()
        || command_id.len() > 256
        || command_id.chars().any(char::is_control)
    {
        bail!("work control requires a bounded command_id");
    }
    Ok(())
}

impl PlanningWorkControlRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn set_work_order_execution_limit(
        &self,
        workspace: &str,
        work_order_id: &str,
        max_attempts: u32,
    ) -> Result<()> {
        super::execution::set_limit(workspace, work_order_id, max_attempts)
    }

    fn started_work_order_task_ids(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Vec<String>> {
        super::execution::started_task_ids(workspace, work_order_id)
    }

    fn load_work_order_control(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<PlanningWorkControl> {
        validate_work_order_id(work_order_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace)?;
        let connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        load_control(&connection, work_order_id)
    }

    fn apply_work_order_control(
        &self,
        workspace: &str,
        work_order_id: &str,
        command_id: &str,
        action: PlanningWorkControlAction,
    ) -> Result<PlanningWorkControl> {
        validate_work_order_id(work_order_id)?;
        validate_command_id(command_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let action_json = serde_json::to_string(&action)?;
        let previous: Option<(String,String,String)> = tx.query_row(
            "SELECT work_order_id, action_json, result_json FROM planning_work_order_control_commands WHERE command_id=?1",
            params![command_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?;
        if let Some((owner, recorded_action, result)) = previous {
            if owner != work_order_id || recorded_action != action_json {
                bail!("work control command_id was reused for different content");
            }
            return Ok(serde_json::from_str(&result)?);
        }
        let current = load_control(&tx, work_order_id)?;
        let next = current.apply(action)?;
        if next != current {
            tx.execute(
                "INSERT INTO planning_work_order_controls (work_order_id,control_json) VALUES (?1,?2)
                 ON CONFLICT(work_order_id) DO UPDATE SET control_json=excluded.control_json",
                params![work_order_id,serde_json::to_string(&next)?],
            )?;
            if let Some(order) = load_work_order_record(&tx, work_order_id)? {
                permits::write_permits(&tx, work_order_id, &[])?;
                if action == PlanningWorkControlAction::Cancel {
                    cancel_lineage(&tx, &location, &order.task_ids)?;
                }
            }
            let event = match action {
                PlanningWorkControlAction::Hold => "issue_held",
                PlanningWorkControlAction::Release => "issue_released",
                _ => "issue_work_control_changed",
            };
            let payload = serde_json::json!({
                "work_order_id":work_order_id,"command_id":command_id,
                "action":action,"control":next
            })
            .to_string();
            runtime_projection::append_runtime_event(
                &tx,
                event,
                "work_order_control",
                work_order_id,
                event,
                &payload,
            )?;
            runtime_projection::append_runtime_event(
                &tx,
                "issue_work_permit_revoked",
                "work_order_control",
                work_order_id,
                "work control invalidated earlier execution permissions",
                &payload,
            )?;
        }
        tx.execute(
            "INSERT INTO planning_work_order_control_commands (command_id,work_order_id,action_json,result_json) VALUES (?1,?2,?3,?4)",
            params![command_id,work_order_id,action_json,serde_json::to_string(&next)?],
        )?;
        tx.commit()?;
        Ok(next)
    }

    fn append_work_order_instruction(
        &self,
        workspace: &str,
        work_order_id: &str,
        instruction: &PlanningWorkInstruction,
    ) -> Result<PlanningWorkInstructionAdmission> {
        validate_work_order_id(work_order_id)?;
        validate_command_id(&instruction.command_id)?;
        if instruction.message.trim().is_empty() || instruction.message.len() > 64 * 1024 {
            bail!("work instruction requires bounded nonempty text");
        }
        let location = Self::resolve_authority_location_from_workspace(workspace)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<String>=tx.query_row(
            "SELECT message FROM planning_work_order_instructions WHERE work_order_id=?1 AND command_id=?2",
            params![work_order_id,instruction.command_id], |row|row.get(0),
        ).optional()?;
        if let Some(message) = current {
            if message != instruction.message {
                bail!("work instruction command_id was reused");
            }
            return Ok(PlanningWorkInstructionAdmission::Queued { changed: false });
        }
        let control = load_control(&tx, work_order_id)?;
        if control.cancelled || control.taken_over {
            bail!("the work is no longer automatically owned");
        }
        // 슬롯을 받은 뒤에는 오래된 task description에 붙이지 않고 실제 thread로 전달해야 한다.
        let mut statement=tx.prepare(
            "SELECT lease.content FROM planning_work_order_tasks AS member
             JOIN runtime_slot_leases AS lease ON json_extract(lease.content,'$.task_id')=member.task_id
             WHERE member.work_order_id=?1",
        )?;
        let leases = statement
            .query_map(params![work_order_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        for json in leases {
            let lease: crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot =
                serde_json::from_str(&json)?;
            let sealed:bool=tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM planning_work_order_launch_seals WHERE work_order_id=?1 AND slot_id=?2 AND session_key=?3)",
                params![work_order_id,lease.slot_id,lease.session_key()],|row|row.get(0),
            )?;
            if lease.state != crate::domain::parallel_mode::ParallelModeSlotLeaseState::Leased
                || sealed
            {
                return Ok(PlanningWorkInstructionAdmission::Active);
            }
        }
        let ordinal:i64=tx.query_row(
            "SELECT COALESCE(MAX(ordinal),-1)+1 FROM planning_work_order_instructions WHERE work_order_id=?1",
            params![work_order_id], |row|row.get(0),
        )?;
        tx.execute(
            "INSERT INTO planning_work_order_instructions (work_order_id,command_id,message,ordinal) VALUES (?1,?2,?3,?4)",
            params![work_order_id,instruction.command_id,instruction.message,ordinal],
        )?;
        if let Some(order) = load_work_order_record(&tx, work_order_id)? {
            append_instruction_to_tasks(&tx, &location, &order.task_ids, instruction)?;
        }
        tx.commit()?;
        Ok(PlanningWorkInstructionAdmission::Queued { changed: true })
    }

    fn load_work_order_instructions(
        &self,
        workspace: &str,
        work_order_id: &str,
    ) -> Result<Vec<PlanningWorkInstruction>> {
        validate_work_order_id(work_order_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace)?;
        let connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let mut statement=connection.prepare(
            "SELECT command_id,message FROM planning_work_order_instructions WHERE work_order_id=?1 ORDER BY ordinal",
        )?;
        let rows = statement.query_map(params![work_order_id], |row| {
            Ok(PlanningWorkInstruction {
                command_id: row.get(0)?,
                message: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn work_order_cleanup_allowed_for_task(&self, workspace: &str, task_id: &str) -> Result<bool> {
        validate_work_order_task_id(task_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace)?;
        let connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let Some(order) = load_task_membership(&connection, task_id)? else {
            return Ok(true);
        };
        Ok(!load_control(&connection, &order)?.taken_over)
    }
}

pub(super) fn cancel_lineage(
    tx: &Transaction<'_>,
    location: &PlanningAuthorityLocation,
    task_ids: &[String],
) -> Result<()> {
    mutate_tasks(tx, location, task_ids, |task| {
        if matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled) {
            return false;
        }
        task.status = TaskStatus::Cancelled;
        true
    })
}

fn append_instruction_to_tasks(
    tx: &Transaction<'_>,
    location: &PlanningAuthorityLocation,
    task_ids: &[String],
    instruction: &PlanningWorkInstruction,
) -> Result<()> {
    mutate_tasks(tx, location, task_ids, |task| {
        if !matches!(
            task.status,
            TaskStatus::Ready
                | TaskStatus::Proposed
                | TaskStatus::Blocked
                | TaskStatus::AwaitingUser
        ) {
            return false;
        }
        task.description.push_str(&format!(
            "

Operator instruction [{}]:
{}",
            instruction.command_id, instruction.message
        ));
        true
    })
}

fn mutate_tasks(
    tx: &Transaction<'_>,
    location: &PlanningAuthorityLocation,
    task_ids: &[String],
    mutation: impl Fn(&mut crate::domain::planning::TaskDefinition) -> bool,
) -> Result<()> {
    let Some(snapshot) = load_task_authority_snapshot_from_connection(tx)? else {
        return Ok(());
    };
    let mut candidate = snapshot.task_authority.clone();
    let mut changed = Vec::new();
    for task in &mut candidate.tasks {
        if task_ids.contains(&task.id) && mutation(task) {
            task.last_updated_by = TaskActor::User;
            task.updated_at = chrono::Utc::now().to_rfc3339();
            changed.push(task.id.clone());
        }
    }
    if changed.is_empty() {
        return Ok(());
    }
    runtime_projection::ensure_no_admin_authority_mutation_guard(
        tx,
        "work-order intervention",
        None,
    )?;
    let directions = load_direction_catalog_from_connection(tx)?
        .context("planning directions are unavailable")?;
    let projection = PriorityQueueService::new().build_projection(&directions, &candidate)?;
    let provenance = TaskMutationProvenance::default();
    SqlitePlanningAuthorityAdapter::write_task_authority_change_in_transaction(
        tx,
        location,
        PlanningTaskAuthorityCommit {
            observed_planning_revision: Some(snapshot.planning_revision),
            task_authority: &candidate,
            queue_projection: &projection,
        },
        Some(&snapshot.task_authority),
        Some(PlanningTaskAuthorityMutationAudit {
            task_ids: &changed,
            legacy_source_turn_id: None,
            provenance: &provenance,
        }),
    )?;
    Ok(())
}

pub(super) fn append_pending_instructions(
    connection: &Connection,
    work_order_id: &str,
    task: &mut crate::domain::planning::TaskDefinition,
) -> Result<()> {
    let mut statement=connection.prepare(
        "SELECT command_id,message FROM planning_work_order_instructions WHERE work_order_id=?1 ORDER BY ordinal",
    )?;
    let rows = statement.query_map(params![work_order_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (command_id, message) = row?;
        task.description.push_str(&format!(
            "

Operator instruction [{}]:
{}",
            command_id, message
        ));
    }
    Ok(())
}

pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter) fn validate_cancelled_lineage(
    connection: &Connection,
    candidate: &crate::domain::planning::TaskAuthorityDocument,
) -> Result<()> {
    if !table_exists(connection, "planning_work_order_tasks")? {
        return Ok(());
    }
    ensure_work_order_tables(connection)?;
    let previous = load_task_authority_snapshot_from_connection(connection)?;
    for task in &candidate.tasks {
        if task.status == TaskStatus::Cancelled {
            continue;
        }
        if let Some(work_order_id) = load_task_membership(connection, &task.id)?
            && load_control(connection, &work_order_id)?.cancelled
        {
            let already_done = task.status == TaskStatus::Done
                && previous.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .task_authority
                        .tasks
                        .iter()
                        .any(|old| old.id == task.id && old.status == TaskStatus::Done)
                });
            if !already_done {
                bail!("cancelled work-order tasks cannot be automatically resumed");
            }
        }
    }
    Ok(())
}

/// 현재 lease/제어/허가와 추가 지시를 한 transaction에서 고정한다.
pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter) fn claim_work_order_launch(
    workspace: &str,
    expected: &crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot,
) -> Result<Vec<PlanningWorkInstruction>> {
    use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkPermitStage;
    use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(work_order_id) = load_task_membership(&tx, &expected.task_id)? else {
        return Ok(Vec::new());
    };
    let tasks = load_task_authority_snapshot_from_connection(&tx)?
        .context("worker launch task authority is unavailable")?;
    let task = tasks
        .task_authority
        .tasks
        .iter()
        .find(|task| task.id == expected.task_id)
        .context("worker launch task disappeared")?;
    if task.status == TaskStatus::AwaitingUser {
        bail!("awaiting-user work requires its isolated answer continuation");
    }

    let json: Option<String> = tx
        .query_row(
            "SELECT content FROM runtime_slot_leases WHERE slot_id=?1",
            params![expected.slot_id],
            |row| row.get(0),
        )
        .optional()?;
    let current: ParallelModeSlotLeaseSnapshot =
        serde_json::from_str(&json.context("the worker launch lease disappeared")?)?;
    if !current.same_generation_as(expected)
        || current.state != ParallelModeSlotLeaseState::Leased
        || current.delivery_target != expected.delivery_target
    {
        bail!("the worker launch lease changed");
    }
    let already_claimed:bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM planning_work_order_launch_seals WHERE work_order_id=?1 AND slot_id=?2 AND session_key=?3)",
        params![work_order_id,current.slot_id,current.session_key()],|row|row.get(0),
    )?;
    if already_claimed {
        return Err(crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderLaunchError::AlreadyClaimed.into());
    }
    let source = current
        .delivery_target
        .as_ref()
        .map(|target| target.integration_base_commit_sha.as_str())
        .unwrap_or("");
    let permit = permits::check_work_order_permit_in_transaction(
        &tx,
        &current.task_id,
        PlanningWorkPermitStage::Dispatch,
        Some(source),
    )?;
    if let Some(reason) = permit.blocked_reason() {
        bail!("{reason}: the worker launch is no longer authorized");
    }
    super::execution::claim_task_attempt(&tx, &work_order_id, &current)?;
    let mut statement=tx.prepare(
        "SELECT command_id,message FROM planning_work_order_instructions WHERE work_order_id=?1 ORDER BY ordinal",
    )?;
    let instructions = statement
        .query_map(params![work_order_id], |row| {
            Ok(PlanningWorkInstruction {
                command_id: row.get(0)?,
                message: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    tx.execute(
        "INSERT INTO planning_work_order_launch_seals (work_order_id,slot_id,session_key) VALUES (?1,?2,?3)
         ON CONFLICT(work_order_id,slot_id) DO UPDATE SET session_key=excluded.session_key",
        params![work_order_id,current.slot_id,current.session_key()],
    )?;
    tx.commit()?;
    Ok(instructions)
}
