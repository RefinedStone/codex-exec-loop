use super::*;
use crate::domain::planning::verification::{
    TaskVerificationExecutionManifest, TaskVerificationRecipe,
};

#[derive(Default)]
pub(super) struct Environments {
    entries: Vec<Entry>,
}

struct Entry {
    recipe: TaskVerificationRecipe,
    oid_length: usize,
    manifest: TaskVerificationExecutionManifest,
    fingerprint: Result<String, TaskDeliveryEvidenceReason>,
    criterion_ids: Vec<String>,
}

impl Environments {
    pub(super) fn observe(
        &mut self,
        runner: &dyn TaskVerificationRunnerPort,
        manifest: &TaskVerificationExecutionManifest,
        criterion_id: &str,
    ) -> Result<String, TaskDeliveryEvidenceReason> {
        manifest
            .validate()
            .map_err(|_| TaskDeliveryEvidenceReason::EvidenceUnavailable)?;
        let index = match self.entries.iter().position(|entry| {
            entry.recipe == manifest.recipe && entry.oid_length == manifest.source.tree_sha.len()
        }) {
            Some(index) => index,
            None => {
                self.entries.push(Entry {
                    recipe: manifest.recipe,
                    oid_length: manifest.source.tree_sha.len(),
                    manifest: manifest.clone(),
                    fingerprint: runner
                        .environment_fingerprint(manifest)
                        .map_err(|_| TaskDeliveryEvidenceReason::EvidenceUnavailable),
                    criterion_ids: Vec::new(),
                });
                self.entries.len() - 1
            }
        };
        if !self.entries[index]
            .criterion_ids
            .iter()
            .any(|id| id == criterion_id)
        {
            self.entries[index]
                .criterion_ids
                .push(criterion_id.to_owned());
        }
        self.entries[index].fingerprint.clone()
    }

    pub(super) fn finish(
        &self,
        runner: &dyn TaskVerificationRunnerPort,
        observation: &mut TaskDeliveryEvidenceObservation,
    ) {
        for entry in &self.entries {
            let current = runner.environment_fingerprint(&entry.manifest);
            let reason = match (&entry.fingerprint, current) {
                (Ok(first), Ok(current)) if first == &current => continue,
                (Ok(_), Ok(_)) => TaskDeliveryEvidenceReason::EvidenceStale,
                _ => TaskDeliveryEvidenceReason::EvidenceUnavailable,
            };
            for row in &mut observation.criteria {
                if row.state == TaskDeliveryCriterionState::HostPassed
                    && entry.criterion_ids.contains(&row.criterion_id)
                {
                    row.state = TaskDeliveryCriterionState::Blocked;
                    row.reason = Some(reason);
                    row.receipt_event_id = None;
                }
            }
            add_reason(observation, reason);
        }
    }
}
