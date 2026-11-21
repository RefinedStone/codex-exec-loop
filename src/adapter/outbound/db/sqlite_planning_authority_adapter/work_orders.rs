use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthoritySnapshot,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderError, PlanningWorkOrderLinkResult, PlanningWorkOrderRecord,
    PlanningWorkOrderRegistration, PlanningWorkOrderRegistrationResult,
    PlanningWorkOrderRepositoryPort, validate_work_order_id, validate_work_order_identity,
    validate_work_order_task_id,
};

use super::{
    SqlitePlanningAuthorityAdapter, bump_planning_revision,
    load_task_authority_snapshot_from_connection, open_authority_connection, read_metadata_i64,
    runtime_projection, table_exists,
};

mod permits;
pub(super) use permits::check_work_order_permit_in_transaction;

// Keep schema version 19: older binaries ignore these additive tables. There is deliberately no
// foreign key to planning_tasks, whose rows can be removed or replaced independently of lineage.
fn ensure_work_order_tables(connection: &Connection) -> Result<()> {
    if table_exists(connection, "planning_work_orders")?
        && table_exists(connection, "planning_work_order_tasks")?
        && table_exists(connection, "planning_work_order_permit_basis")?
        && table_exists(connection, "planning_work_order_permit_revocations")?
    {
        return Ok(());
    }
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS planning_work_orders (
                work_order_id TEXT PRIMARY KEY NOT NULL,
                origin TEXT NOT NULL,
                permits_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_tasks (
                task_id TEXT PRIMARY KEY NOT NULL,
                work_order_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
                UNIQUE (work_order_id, ordinal),
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_permit_basis (
                work_order_id TEXT PRIMARY KEY NOT NULL,
                basis_json TEXT NOT NULL,
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );
            CREATE TABLE IF NOT EXISTS planning_work_order_permit_revocations (
                work_order_id TEXT NOT NULL,
                stage TEXT NOT NULL,
                gate_revision TEXT NOT NULL,
                PRIMARY KEY (work_order_id, stage),
                FOREIGN KEY (work_order_id) REFERENCES planning_work_orders(work_order_id)
            );",
        )
        .context("failed to ensure planning work-order tables")
}

impl PlanningWorkOrderRepositoryPort for SqlitePlanningAuthorityAdapter {
    fn load_work_order(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        validate_work_order_id(work_order_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction()
            .context("failed to open work-order read transaction")?;
        load_work_order_record(&transaction, work_order_id)
    }

    fn load_work_order_for_task(
        &self,
        workspace_dir: &str,
        task_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        validate_work_order_task_id(task_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction()
            .context("failed to open work-order task read transaction")?;
        let Some(work_order_id) = load_task_membership(&transaction, task_id)? else {
            return Ok(None);
        };
        let record = load_work_order_record(&transaction, &work_order_id)?
            .context("planning task membership has no work-order record")?;
        Ok(Some(record))
    }

    fn register_work_order(
        &self,
        workspace_dir: &str,
        registration: PlanningWorkOrderRegistration<'_>,
    ) -> Result<PlanningWorkOrderRegistrationResult> {
        validate_work_order_identity(registration.work_order_id, registration.origin)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open work-order registration transaction")?;

        // The durable identity wins even after task removal, receipt pruning, or authority reset.
        // No candidate task, guard, or stale revision can turn a replay into another creation.
        if let Some(work_order) = load_work_order_record(&transaction, registration.work_order_id)?
        {
            work_order.ensure_origin(registration.origin)?;
            return Ok(PlanningWorkOrderRegistrationResult::Existing { work_order });
        }

        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "work-order registration",
            None,
        )?;
        let current_planning_revision =
            read_metadata_i64(&transaction, "planning_revision")?.unwrap_or(0);
        if registration.observed_planning_revision != current_planning_revision {
            return Ok(PlanningWorkOrderRegistrationResult::RevisionConflict {
                observed_planning_revision: registration.observed_planning_revision,
                current_planning_revision,
            });
        }
        validate_work_order_task_id(registration.initial_task_id)?;
        let existing = load_task_authority_snapshot_from_connection(&transaction)?
            .context("planning task authority is unavailable")?;
        if load_task_membership(&transaction, registration.initial_task_id)?.is_some()
            || existing
                .task_authority
                .tasks
                .iter()
                .any(|task| task.id.trim() == registration.initial_task_id)
        {
            return Ok(PlanningWorkOrderRegistrationResult::TaskIdConflict {
                task_id: registration.initial_task_id.to_string(),
            });
        }
        validate_new_task_candidate(&existing, registration)?;

        let planning_revision = Self::write_task_authority_change_in_transaction(
            &transaction,
            &location,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(registration.observed_planning_revision),
                task_authority: registration.task_authority,
                queue_projection: registration.queue_projection,
            },
            Some(&existing.task_authority),
            Some(registration.audit),
        )?;
        transaction
            .execute(
                "INSERT INTO planning_work_orders (work_order_id, origin, permits_json)
                 VALUES (?1, ?2, '[]')",
                params![registration.work_order_id, registration.origin],
            )
            .context("failed to register planning work order")?;
        insert_task_membership(
            &transaction,
            registration.work_order_id,
            registration.initial_task_id,
            0,
        )?;
        let work_order = PlanningWorkOrderRecord {
            work_order_id: registration.work_order_id.to_string(),
            origin: registration.origin.to_string(),
            task_ids: vec![registration.initial_task_id.to_string()],
            permits: Vec::new(),
        };
        transaction
            .commit()
            .context("failed to commit planning work order and task")?;
        Ok(PlanningWorkOrderRegistrationResult::Registered {
            work_order,
            planning_revision,
        })
    }

    fn link_supplemental_task(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        task_id: &str,
    ) -> Result<PlanningWorkOrderLinkResult> {
        validate_work_order_id(work_order_id)?;
        validate_work_order_task_id(task_id)?;
        let location = Self::resolve_authority_location_from_workspace(workspace_dir)?;
        let mut connection = open_authority_connection(&location)?;
        ensure_work_order_tables(&connection)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open supplemental task transaction")?;
        let mut work_order =
            load_work_order_record(&transaction, work_order_id)?.ok_or_else(|| {
                PlanningWorkOrderError::WorkOrderNotFound {
                    work_order_id: work_order_id.to_string(),
                }
            })?;
        if let Some(owner) = load_task_membership(&transaction, task_id)? {
            if owner != work_order_id {
                return Err(PlanningWorkOrderError::TaskAlreadyLinked {
                    task_id: task_id.to_string(),
                    work_order_id: owner,
                }
                .into());
            }
            return Ok(PlanningWorkOrderLinkResult {
                work_order,
                linked: false,
                planning_revision: read_metadata_i64(&transaction, "planning_revision")?
                    .unwrap_or(0),
            });
        }
        runtime_projection::ensure_no_admin_authority_mutation_guard(
            &transaction,
            "work-order supplemental task link",
            None,
        )?;
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM planning_tasks WHERE task_id = ?1)",
                params![task_id],
                |row| row.get(0),
            )
            .context("failed to inspect supplemental planning task")?;
        if !exists {
            return Err(PlanningWorkOrderError::TaskNotFound {
                task_id: task_id.to_string(),
            }
            .into());
        }
        let ordinal = i64::try_from(work_order.task_ids.len())
            .context("planning work-order lineage is too large")?;
        insert_task_membership(&transaction, work_order_id, task_id, ordinal)?;
        work_order.task_ids.push(task_id.to_string());
        let planning_revision = bump_planning_revision(&transaction)?;
        transaction
            .commit()
            .context("failed to commit supplemental task link")?;
        Ok(PlanningWorkOrderLinkResult {
            work_order,
            linked: true,
            planning_revision,
        })
    }
}

fn validate_new_task_candidate(
    existing: &PlanningTaskAuthoritySnapshot,
    registration: PlanningWorkOrderRegistration<'_>,
) -> Result<()> {
    let next = registration.task_authority;
    let previous = &existing.task_authority;
    if next.version != previous.version || next.tasks.len() != previous.tasks.len() + 1 {
        bail!("work-order registration must add exactly one task");
    }
    if next
        .tasks
        .iter()
        .filter(|task| task.id == registration.initial_task_id)
        .count()
        != 1
    {
        bail!("work-order registration must contain its initial task exactly once");
    }
    let retained = next
        .tasks
        .iter()
        .filter(|task| task.id != registration.initial_task_id);
    if !retained.eq(previous.tasks.iter()) {
        bail!("work-order registration must preserve existing tasks");
    }
    if registration.audit.task_ids.len() != 1
        || registration.audit.task_ids[0] != registration.initial_task_id
    {
        bail!("work-order registration audit must identify exactly its initial task");
    }
    Ok(())
}

fn load_task_membership(connection: &Connection, task_id: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT work_order_id FROM planning_work_order_tasks WHERE task_id = ?1",
            params![task_id],
            |row| row.get(0),
        )
        .optional()
        .context("failed to read planning task membership")
}

fn load_work_order_record(
    connection: &Connection,
    work_order_id: &str,
) -> Result<Option<PlanningWorkOrderRecord>> {
    let Some((origin, permits_json)) = connection
        .query_row(
            "SELECT origin, permits_json FROM planning_work_orders WHERE work_order_id = ?1",
            params![work_order_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .context("failed to read planning work-order record")?
    else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare(
            "SELECT task_id, ordinal FROM planning_work_order_tasks
             WHERE work_order_id = ?1 ORDER BY ordinal",
        )
        .context("failed to prepare planning work-order lineage")?;
    let rows = statement
        .query_map(params![work_order_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .context("failed to read planning work-order lineage")?;
    let mut task_ids = Vec::new();
    for row in rows {
        let (task_id, ordinal) = row.context("failed to decode planning work-order lineage")?;
        if ordinal != i64::try_from(task_ids.len()).context("work-order lineage is too large")? {
            bail!("planning work-order lineage is not contiguous");
        }
        task_ids.push(task_id);
    }
    let record = PlanningWorkOrderRecord {
        work_order_id: work_order_id.to_string(),
        origin,
        task_ids,
        permits: serde_json::from_str(&permits_json)
            .context("failed to decode planning work-order permits")?,
    };
    record.validate()?;
    Ok(Some(record))
}

fn insert_task_membership(
    connection: &Connection,
    work_order_id: &str,
    task_id: &str,
    ordinal: i64,
) -> Result<()> {
    connection
        .execute(
            "INSERT INTO planning_work_order_tasks (task_id, work_order_id, ordinal)
             VALUES (?1, ?2, ?3)",
            params![task_id, work_order_id, ordinal],
        )
        .context("failed to insert planning work-order task membership")?;
    Ok(())
}

#[cfg(test)]
mod tests;
