use super::preview::sanitized_text;
use super::*;
use crate::application::port::inbound::task_evidence_query_port::{
    PreviewKind, PreviewTarget, TaskEvidenceQueryPort,
};
use crate::domain::planning::TaskAcceptanceChannel;
use crate::domain::planning::evidence::{HumanEvidenceDecision, HumanEvidenceReview};
use sha2::{Digest, Sha256};

impl TaskEvidenceService {
    pub(crate) fn review_evidence(
        &self,
        target: &PreviewTarget,
        decision: HumanEvidenceDecision,
        confirm: impl FnOnce(&str) -> bool,
    ) -> Result<()> {
        if !matches!(target.kind, PreviewKind::Worker) {
            return Err(TaskEvidenceError::Invalid.into());
        }
        let current = self.current_criterion(
            &target.task_id,
            &target.criterion_id,
            &target.scope_id,
            target.revision,
        )?;
        let c = current
            .task
            .acceptance
            .iter()
            .find(|c| c.id == target.criterion_id && c.channel != TaskAcceptanceChannel::Command)
            .ok_or(TaskEvidenceError::Invalid)?;
        let e = current
            .evidence
            .iter()
            .find(|r| {
                r.event_id == target.event_id
                    && r.evidence.artifact.sha256 == target.sha256
                    && r.evidence.attempt.scope == current.scope
                    && r.evidence.attempt.criterion_id == target.criterion_id
            })
            .ok_or(TaskEvidenceError::Stale)?;
        let lease = running_lease(&current)?;
        let tree = &e.evidence.recorded_tree.tree_sha;
        let check = || -> Result<()> {
            if snapshot_worker_tree(self.runtime.as_ref(), &lease)?.tree_sha != *tree {
                return Err(TaskEvidenceError::Stale.into());
            }
            Ok(())
        };
        with_running_worker(
            self.authority.as_ref(),
            self.runtime.as_ref(),
            &self.workspace,
            &lease,
            check,
        )?;
        let preview = self.sanitized_preview(target)?;
        let label = match decision {
            HumanEvidenceDecision::Pass => "PASS",
            HumanEvidenceDecision::Fail => "FAIL",
        };
        let metadata = serde_json::to_string(&(
            sanitized_text(target.task_id.as_bytes())?,
            sanitized_text(target.criterion_id.as_bytes())?,
            sanitized_text(c.scenario.as_bytes())?,
            tree,
            &target.scope_id,
            &target.sha256,
            &target.event_id,
        ))?;
        let message = format!(
            "Record human {label} for this evidence only?\nTask / criterion / scenario / tree / scope / artifact / receipt:\n{metadata}\n\nEvidence (untrusted text):\n{preview}\n\nEnd of evidence. Confirm {label}, or cancel."
        );
        if !confirm(&message) {
            return Err(TaskEvidenceError::HumanConfirmationRequired.into());
        }
        let review = HumanEvidenceReview {
            review_id: format!("{:064x}", rand::random::<u128>()),
            confirmation_sha256: format!("{:x}", Sha256::digest(message.as_bytes())),
            evidence_event_id: e.event_id.clone(),
            scope: current.scope.clone(),
            criterion_id: c.id.clone(),
            artifact: e.evidence.artifact.clone(),
            tree_sha: tree.clone(),
            decision,
        };
        with_running_worker(
            self.authority.as_ref(),
            self.runtime.as_ref(),
            &self.workspace,
            &lease,
            || {
                check()?;
                if self.sanitized_preview(target)? != preview {
                    return Err(TaskEvidenceError::Stale.into());
                }
                self.repository
                    .record_human_review(&self.workspace, &review)
            },
        )
    }
}
