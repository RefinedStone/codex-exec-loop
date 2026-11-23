//! Final worker admission reads the existing launch seal; it never claims a second launch.
use super::super::{
    SqlitePlanningAuthorityAdapter, load_task_authority_snapshot_from_connection,
    open_authority_connection,
};
use super::{
    control, ensure_work_order_tables, load_task_membership, load_work_order_record, permits,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitStage, PlanningWorkerActionIdentity, validate_work_order_task_id,
};
use crate::domain::parallel_mode::{ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState};
use crate::domain::planning::TaskStatus;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

fn identity_in_connection(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<PlanningWorkerActionIdentity>> {
    validate_work_order_task_id(task_id)?;
    let Some(id) = load_task_membership(connection, task_id)? else {
        return Ok(None);
    };
    let order = load_work_order_record(connection, &id)?.context("worker lineage disappeared")?;
    order.validate()?;
    if !order.task_ids.iter().any(|member| member == task_id) {
        bail!("worker lineage does not contain its task");
    }
    let control = control::load_control(connection, &id)?;
    Ok(Some(PlanningWorkerActionIdentity {
        work_order_id: id,
        origin: order.origin,
        authorization_epoch: control.authorization_epoch,
    }))
}

pub(in super::super) fn identity(
    workspace: &str,
    task_id: &str,
) -> Result<Option<PlanningWorkerActionIdentity>> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let transaction = connection.transaction()?;
    identity_in_connection(&transaction, task_id)
}

pub(in super::super) fn validate_launch(
    workspace: &str,
    expected: &ParallelModeSlotLeaseSnapshot,
    identity: Option<&PlanningWorkerActionIdentity>,
) -> Result<()> {
    let location =
        SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(workspace)?;
    let mut connection = open_authority_connection(&location)?;
    ensure_work_order_tables(&connection)?;
    let transaction = connection.transaction()?;
    let current_identity = identity_in_connection(&transaction, &expected.task_id)?;
    if current_identity.as_ref() != identity {
        bail!("worker action lineage or authorization epoch changed");
    }
    let Some(identity) = identity else {
        return Ok(());
    };
    control::load_control(&transaction, &identity.work_order_id)?
        .require_epoch(identity.authorization_epoch)?;
    let tasks = load_task_authority_snapshot_from_connection(&transaction)?
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
    let lease_json: Option<String> = transaction
        .query_row(
            "SELECT content FROM runtime_slot_leases WHERE slot_id=?1",
            params![expected.slot_id],
            |row| row.get(0),
        )
        .optional()?;
    let current: ParallelModeSlotLeaseSnapshot =
        serde_json::from_str(&lease_json.context("worker launch lease disappeared")?)?;
    if !current.same_generation_as(expected)
        || current.state != ParallelModeSlotLeaseState::Leased
        || current.delivery_target != expected.delivery_target
    {
        bail!("worker launch lease changed before turn/start");
    }
    let sealed: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM planning_work_order_launch_seals WHERE work_order_id=?1 AND slot_id=?2 AND session_key=?3)",
        params![identity.work_order_id, current.slot_id, current.session_key()], |row| row.get(0),
    )?;
    if !sealed {
        bail!("worker launch has no matching durable seal");
    }
    let source = current
        .delivery_target
        .as_ref()
        .map(|target| target.integration_base_commit_sha.as_str());
    let permit = permits::check_work_order_permit_in_transaction(
        &transaction,
        &current.task_id,
        PlanningWorkPermitStage::Dispatch,
        source,
    )?;
    if let Some(reason) = permit.blocked_reason() {
        bail!("{reason}: worker launch is no longer authorized");
    }
    Ok(())
}
