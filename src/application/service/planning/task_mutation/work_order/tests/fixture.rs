use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};

use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
    PlanningDirectionAuthoritySnapshot, PlanningTaskAuthorityCommit,
    PlanningTaskAuthorityCommitResult, PlanningTaskAuthorityMutationAudit,
    PlanningTaskAuthoritySnapshot, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkLinkReference, PlanningWorkOrderLinkResult, PlanningWorkOrderRecord,
    PlanningWorkOrderRegistration, PlanningWorkOrderRegistrationResult,
    PlanningWorkOrderRepositoryPort,
};
use crate::application::service::planning::task_mutation::PlanningTaskCreateInput;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    PriorityQueueProjection, QueueIdleConfig, TaskAuthorityDocument, TaskStatus,
};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub(super) struct Attempt {
    pub task_id: String,
    pub updated_at: String,
    pub observed_revision: i64,
}

#[derive(Default)]
pub(super) struct Script {
    pub orders: BTreeMap<String, PlanningWorkOrderRecord>,
    pub attempts: Vec<Attempt>,
    pub revision_conflicts: usize,
    pub hide_existing_reads: usize,
    pub atomic_existing_returns: usize,
}

pub(super) struct FixtureRepository {
    pub workspace: String,
    pub script: Mutex<Script>,
    inner: NoopPlanningTaskRepositoryPort,
}

impl FixtureRepository {
    pub fn new() -> Arc<Self> {
        let repository = Arc::new(Self {
            workspace: format!(
                "/tmp/akra-work-order-service-{}-{}",
                std::process::id(),
                NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed)
            ),
            script: Mutex::new(Script::default()),
            inner: NoopPlanningTaskRepositoryPort,
        });
        repository.seed_empty_authority();
        repository
    }

    pub fn seed_empty_authority(&self) {
        let directions = DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General".to_string(),
                summary: "Handle ordinary planning work.".to_string(),
                success_criteria: vec!["done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        };
        self.commit_direction_authority_snapshot(
            &self.workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions,
                authority_mutation_owner_token: None,
            },
        )
        .unwrap();
        self.commit_task_authority_snapshot(
            &self.workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &TaskAuthorityDocument {
                    version: PLANNING_FORMAT_VERSION,
                    tasks: Vec::new(),
                },
                queue_projection: &PriorityQueueProjection {
                    next_task: None,
                    active_tasks: Vec::new(),
                    proposed_tasks: Vec::new(),
                    skipped_tasks: Vec::new(),
                },
            },
        )
        .unwrap();
    }

    pub fn snapshot(&self) -> PlanningTaskAuthoritySnapshot {
        self.load_task_authority_snapshot(&self.workspace)
            .unwrap()
            .unwrap()
    }
}

pub(super) fn input() -> PlanningTaskCreateInput {
    PlanningTaskCreateInput {
        acceptance: None,
        direction_id: None,
        direction_relation_note: Some("supports the active direction".to_string()),
        title: "Implement ordinary work".to_string(),
        description: Some("Complete the existing planning lifecycle.".to_string()),
        status: Some(TaskStatus::Ready),
        base_priority: None,
        dynamic_priority_delta: None,
        priority_reason: None,
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
    }
}

impl PlanningTaskRepositoryPort for FixtureRepository {
    fn load_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningDirectionAuthoritySnapshot>> {
        self.inner.load_direction_authority_snapshot(workspace_dir)
    }

    fn commit_direction_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningDirectionAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        self.inner
            .commit_direction_authority_snapshot(workspace_dir, commit)
    }

    fn clear_direction_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        self.inner.clear_direction_authority_snapshot(workspace_dir)
    }

    fn load_task_authority_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
        self.inner.load_task_authority_snapshot(workspace_dir)
    }

    fn commit_task_authority_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        self.inner
            .commit_task_authority_snapshot(workspace_dir, commit)
    }

    fn commit_task_authority_mutation_snapshot(
        &self,
        workspace_dir: &str,
        commit: PlanningTaskAuthorityCommit<'_>,
        audit: PlanningTaskAuthorityMutationAudit<'_>,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        self.inner
            .commit_task_authority_mutation_snapshot(workspace_dir, commit, audit)
    }

    fn clear_task_authority_snapshot(&self, workspace_dir: &str) -> Result<()> {
        self.inner.clear_task_authority_snapshot(workspace_dir)
    }
}

/// This fake scripts commit outcomes to exercise service retries. Atomicity and concurrent
/// connections are tested against the real SQLite adapter, rather than inferred from this fake.
impl PlanningWorkOrderRepositoryPort for FixtureRepository {
    fn replay_work_order(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
        origin: &str,
        link_reference: Option<&PlanningWorkLinkReference>,
        acceptance_receipt: Option<&str>,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        assert_eq!(workspace_dir, self.workspace);
        let mut script = self.script.lock().unwrap();
        if script.orders.contains_key(work_order_id) && script.hide_existing_reads > 0 {
            script.hide_existing_reads -= 1;
            return Ok(None);
        }
        let Some(existing) = script.orders.get_mut(work_order_id) else {
            return Ok(None);
        };
        existing.validate()?;
        existing.ensure_origin(origin)?;
        if existing.acceptance_receipt.as_deref() != acceptance_receipt {
            bail!("work order was admitted with a different acceptance approval");
        }
        if existing.link_reference.as_ref() != link_reference {
            match (&existing.link_reference, link_reference) {
                (None, Some(reference)) if !reference.close_on_merge => {
                    reference.validate()?;
                    existing.link_reference = Some(reference.clone());
                }
                _ => bail!("work order publication reference changed"),
            }
        }
        Ok(Some(existing.clone()))
    }

    fn load_work_order(
        &self,
        workspace_dir: &str,
        work_order_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        assert_eq!(workspace_dir, self.workspace);
        let mut script = self.script.lock().unwrap();
        if script.orders.contains_key(work_order_id) && script.hide_existing_reads > 0 {
            script.hide_existing_reads -= 1;
            return Ok(None);
        }
        Ok(script.orders.get(work_order_id).cloned())
    }

    fn load_work_order_for_task(
        &self,
        workspace_dir: &str,
        task_id: &str,
    ) -> Result<Option<PlanningWorkOrderRecord>> {
        assert_eq!(workspace_dir, self.workspace);
        Ok(self
            .script
            .lock()
            .unwrap()
            .orders
            .values()
            .find(|order| order.task_ids.iter().any(|id| id == task_id))
            .cloned())
    }

    fn register_work_order(
        &self,
        workspace_dir: &str,
        registration: PlanningWorkOrderRegistration<'_>,
    ) -> Result<PlanningWorkOrderRegistrationResult> {
        assert_eq!(workspace_dir, self.workspace);
        let task = registration
            .task_authority
            .tasks
            .iter()
            .find(|task| task.id == registration.initial_task_id)
            .unwrap();
        let mut script = self.script.lock().unwrap();
        script.attempts.push(Attempt {
            task_id: task.id.clone(),
            updated_at: task.updated_at.clone(),
            observed_revision: registration.observed_planning_revision,
        });
        if let Some(existing) = script.orders.get(registration.work_order_id).cloned() {
            existing.ensure_origin(registration.origin)?;
            script.atomic_existing_returns += 1;
            return Ok(PlanningWorkOrderRegistrationResult::Existing {
                work_order: existing,
            });
        }
        if script.revision_conflicts > 0 {
            script.revision_conflicts -= 1;
            let mut directions = self
                .inner
                .load_direction_authority_snapshot(workspace_dir)?
                .unwrap()
                .directions;
            directions.directions[0].summary.push('!');
            let PlanningTaskAuthorityCommitResult::Committed {
                planning_revision, ..
            } = self.inner.commit_direction_authority_snapshot(
                workspace_dir,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions: &directions,
                    authority_mutation_owner_token: None,
                },
            )?
            else {
                panic!("unconditional fixture direction commit must succeed");
            };
            return Ok(PlanningWorkOrderRegistrationResult::RevisionConflict {
                observed_planning_revision: registration.observed_planning_revision,
                current_planning_revision: planning_revision,
            });
        }
        if script.orders.values().any(|order| {
            order
                .task_ids
                .iter()
                .any(|id| id == registration.initial_task_id)
        }) {
            return Ok(PlanningWorkOrderRegistrationResult::TaskIdConflict {
                task_id: registration.initial_task_id.to_string(),
            });
        }
        let result = self.inner.commit_task_authority_mutation_snapshot(
            workspace_dir,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(registration.observed_planning_revision),
                task_authority: registration.task_authority,
                queue_projection: registration.queue_projection,
            },
            registration.audit,
        )?;
        match result {
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => Ok(PlanningWorkOrderRegistrationResult::RevisionConflict {
                observed_planning_revision,
                current_planning_revision,
            }),
            PlanningTaskAuthorityCommitResult::Committed {
                planning_revision, ..
            } => {
                let work_order = PlanningWorkOrderRecord {
                    acceptance_receipt: registration.acceptance_receipt.map(str::to_owned),
                    link_reference: registration.link_reference.cloned(),
                    work_order_id: registration.work_order_id.to_string(),
                    origin: registration.origin.to_string(),
                    task_ids: vec![registration.initial_task_id.to_string()],
                    permits: Vec::new(),
                };
                script
                    .orders
                    .insert(work_order.work_order_id.clone(), work_order.clone());
                Ok(PlanningWorkOrderRegistrationResult::Registered {
                    work_order,
                    planning_revision,
                })
            }
        }
    }

    fn link_supplemental_task(
        &self,
        _workspace_dir: &str,
        _work_order_id: &str,
        _task_id: &str,
    ) -> Result<PlanningWorkOrderLinkResult> {
        bail!("supplemental lineage is exercised against the real SQLite adapter")
    }
}
