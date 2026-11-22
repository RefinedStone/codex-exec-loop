use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskAuthorityMutationAudit;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderSupplementalAdmission, PlanningWorkOrderSupplementalAdmissionResult,
    validate_admission_key, validate_work_order_id,
};
use crate::domain::planning::{
    OriginSessionKind, TASK_AUTHORITY_FORMAT_VERSION, TaskMutationProvenance,
};

use super::super::{
    MAX_COLLISION_SUFFIX_ATTEMPTS, MAX_REVISION_CONFLICT_RETRIES, PlanningTaskCreateInput,
    PlanningTaskMutationSource, TaskMutationAuditContext, task_id_exists,
};
use super::{PlanningWorkOrderAdmission, PlanningWorkOrderService};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningSupplementalWorkRequest {
    pub workspace_directory: String,
    pub work_order_id: String,
    pub admission_key: String,
    pub input: PlanningTaskCreateInput,
}

impl PlanningWorkOrderService {
    pub fn admit_supplemental_task_once(
        &self,
        request: PlanningSupplementalWorkRequest,
    ) -> Result<PlanningWorkOrderAdmission> {
        super::super::acceptance::create_fields(
            &request.input,
            PlanningTaskMutationSource::System,
        )?;
        validate_work_order_id(&request.work_order_id)?;
        validate_admission_key(&request.admission_key)?;
        let generated_at = Utc::now();
        let provenance = TaskMutationProvenance::new(OriginSessionKind::System)
            .with_thread_turn(None, Some(request.admission_key.clone()));
        let mut conflicts = 0;
        let mut collisions = 0;
        let mut collision_suffix = None;
        loop {
            if let Some(task_id) = self.repository.load_work_order_supplemental_task(
                &request.workspace_directory,
                &request.work_order_id,
                &request.admission_key,
            )? {
                let work_order = self
                    .repository
                    .load_work_order(&request.workspace_directory, &request.work_order_id)?
                    .context("supplemental admission work order is missing")?;
                return Ok(PlanningWorkOrderAdmission {
                    work_order,
                    task_id,
                    created: false,
                });
            }
            let expected = self
                .repository
                .load_work_order_admission_snapshot(
                    &request.workspace_directory,
                    &request.work_order_id,
                )?
                .context("supplemental admission work order is missing")?;
            let context = self
                .task_mutation
                .load_context(&request.workspace_directory)?;
            let mut candidate = context.task_authority.clone();
            let historical: Vec<_> = candidate
                .tasks
                .iter()
                .filter(|task| {
                    task.provenance.origin_session_kind == Some(OriginSessionKind::System)
                        && task.provenance.turn_id.as_deref()
                            == Some(request.admission_key.as_str())
                })
                .map(|task| task.id.clone())
                .collect();
            if historical.len() > 1 {
                bail!("supplemental admission has ambiguous historical task identities");
            }
            let task_id = if let Some(existing) = historical.first() {
                existing.clone()
            } else {
                let task = self.task_mutation.build_task(
                    &request.input,
                    TaskMutationAuditContext {
                        source: PlanningTaskMutationSource::System,
                        legacy_source_turn_id: None,
                        provenance: &provenance,
                    },
                    &context.directions,
                    generated_at,
                    collision_suffix,
                )?;
                if task_id_exists(&candidate, &task.id) {
                    collisions += 1;
                    if collisions >= MAX_COLLISION_SUFFIX_ATTEMPTS {
                        bail!("supplemental task could not allocate a unique planning id");
                    }
                    collision_suffix = self
                        .task_mutation
                        .task_id_policy
                        .next_collision_suffix(collision_suffix);
                    continue;
                }
                let id = task.id.clone();
                candidate.version = TASK_AUTHORITY_FORMAT_VERSION;
                candidate.tasks.push(task);
                id
            };
            let projection = self
                .task_mutation
                .validate_and_project(&context.directions, &candidate)?;
            match self.repository.admit_supplemental_work_order_task(
                &request.workspace_directory,
                PlanningWorkOrderSupplementalAdmission {
                    admission_key: &request.admission_key,
                    expected: &expected,
                    observed_planning_revision: context.task_planning_revision,
                    task_id: &task_id,
                    task_authority: &candidate,
                    queue_projection: &projection,
                    audit: PlanningTaskAuthorityMutationAudit {
                        task_ids: std::slice::from_ref(&task_id),
                        legacy_source_turn_id: None,
                        provenance: &provenance,
                    },
                },
            )? {
                PlanningWorkOrderSupplementalAdmissionResult::Admitted {
                    snapshot,
                    task_id,
                    created,
                    ..
                } => {
                    return Ok(PlanningWorkOrderAdmission {
                        work_order: snapshot.work_order,
                        task_id,
                        created,
                    });
                }
                PlanningWorkOrderSupplementalAdmissionResult::Existing { snapshot, task_id } => {
                    return Ok(PlanningWorkOrderAdmission {
                        work_order: snapshot.work_order,
                        task_id,
                        created: false,
                    });
                }
                PlanningWorkOrderSupplementalAdmissionResult::RevisionConflict { .. }
                | PlanningWorkOrderSupplementalAdmissionResult::WorkOrderConflict { .. } => {
                    conflicts += 1;
                    if conflicts > MAX_REVISION_CONFLICT_RETRIES {
                        bail!(
                            "supplemental task could not commit because planning ownership kept changing"
                        );
                    }
                }
                PlanningWorkOrderSupplementalAdmissionResult::TaskIdConflict {
                    task_id: rejected,
                } => {
                    if rejected != task_id || !historical.is_empty() {
                        bail!(
                            "historical supplemental task cannot be rebound to another work order"
                        );
                    }
                    collisions += 1;
                    if collisions >= MAX_COLLISION_SUFFIX_ATTEMPTS {
                        bail!("supplemental task could not allocate a unique planning id");
                    }
                    collision_suffix = self
                        .task_mutation
                        .task_id_policy
                        .next_collision_suffix(collision_suffix);
                }
            }
        }
    }
}
