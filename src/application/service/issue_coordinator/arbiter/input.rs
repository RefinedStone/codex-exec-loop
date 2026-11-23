//! The model receives disclosed content and a bounded immutable repository selection.
//! Routing paths, confirmation capabilities and raw provider responses never enter input.
use super::*;
use crate::application::port::outbound::issue_assessment_evidence_port::{
    IssueAssessmentEvidenceRequest, MAX_ISSUE_ASSESSMENT_EVIDENCE_PATHS,
};
use crate::application::port::outbound::issue_assessment_model_port::{
    ISSUE_ASSESSMENT_PROFILE_VERSION, IssueAssessmentModelConfiguration,
    MAX_ISSUE_ASSESSMENT_INPUT_BYTES,
};
use crate::domain::issue_masking::{IssueTextField, plan_issue_masking};
use crate::domain::issue_secret_spans::secret_spans;
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

pub(super) struct CurrentArbiter {
    pub record: StoredIssueDecision,
    pub fence: IssueDecisionFence,
    pub item: IssueCollectionItem,
    pub assessment: IssueTriageAssessment,
    pub basis: IssueDecisionBasis,
}

impl CurrentArbiter {
    pub fn context(&self, mut input_basis: IssueArbiterInputBasis) -> IssueArbiterCurrentContext {
        input_basis.decision_basis = self.basis.clone();
        input_basis.decision_revision = self.record.revision;
        IssueArbiterCurrentContext {
            fence: self.fence,
            current: self.assessment.clone(),
            input_basis,
        }
    }
}

pub(super) struct PreparedArbiter {
    pub basis: IssueArbiterInputBasis,
    pub settings: IssueArbiterSettings,
    pub configuration: IssueAssessmentModelConfiguration,
    pub json: String,
}

impl IssueCoordinatorService {
    pub(super) fn arbiter_current(
        &self,
        id: &IssueDecisionId,
        committed: &Reporter<'_>,
    ) -> Result<CurrentArbiter> {
        let record = self
            .store
            .get_issue_decision(&self.workspace, id)?
            .ok_or_else(|| anyhow!("the arbiter decision is unavailable"))?;
        let (triage, item, assessment) = self.current(record.request.issue(), committed)?;
        let definition = record.request.definition();
        let mut basis = IssueDecisionBasis::from_assessment(
            &assessment,
            definition.gate,
            definition.basis.work_order_id.clone(),
        )?;
        basis.authorization_epoch = self
            .store
            .issue_authorization_epoch(&self.workspace, record.request.issue())?;
        let control = self.execution.control(
            &self.workspace,
            &issue_work_order_id(record.request.issue()),
        )?;
        if !control.allows_automation()
            || control.authorization_epoch != basis.authorization_epoch
            || self
                .store
                .pending_issue_interventions(&self.workspace)?
                .iter()
                .any(|pending| pending.request.issue == *record.request.issue())
        {
            bail!("the issue arbiter is fenced by a current intervention");
        }
        Ok(CurrentArbiter {
            fence: IssueDecisionFence {
                triage,
                decision_revision: record.revision,
            },
            record,
            item,
            assessment,
            basis,
        })
    }

    pub(super) fn prepare_arbiter_input(
        &self,
        runtime: &ArbiterRuntime,
        current: &CurrentArbiter,
    ) -> std::result::Result<PreparedArbiter, IssueArbiterWaitReason> {
        let automation = self
            .triage
            .automation_policy_for_version(&current.basis.policy_version)
            .map_err(|_| IssueArbiterWaitReason::ContextUnavailable)?;
        let privacy = self
            .policy
            .resolve(&self.workspace)
            .map_err(|_| IssueArbiterWaitReason::ContextUnavailable)?;
        let privacy_version = issue_policy_version(&privacy);
        if privacy_version != current.assessment.basis().validation_policy_version {
            return Err(IssueArbiterWaitReason::ContextUnavailable);
        }
        let configuration = runtime
            .model
            .configuration()
            .map_err(|_| IssueArbiterWaitReason::ModelUnavailable)?;
        if configuration.profile_version != ISSUE_ASSESSMENT_PROFILE_VERSION
            || configuration.route_revision.is_empty()
            || configuration.route_revision.len() > 128
            || configuration
                .route_revision
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
            || configuration.timeout_seconds == 0
            || configuration.model.as_ref().is_some_and(|model| {
                model.is_empty() || model.len() > 128 || model.chars().any(char::is_control)
            })
        {
            return Err(IssueArbiterWaitReason::ModelUnavailable);
        }
        let settings = IssueArbiterSettings {
            min_confidence: automation.arbiter.min_confidence,
            max_running: automation
                .arbiter
                .max_running
                .min(configuration.max_running),
            max_started_per_day: automation
                .arbiter
                .max_started_per_day
                .min(configuration.max_started_per_day),
            timeout_seconds: automation
                .arbiter
                .timeout_seconds
                .min(configuration.timeout_seconds),
        };
        settings
            .validate()
            .map_err(|_| IssueArbiterWaitReason::ContextUnavailable)?;
        if settings.max_running == 0 || settings.max_started_per_day == 0 {
            return Err(IssueArbiterWaitReason::BudgetExhausted);
        }
        let Some(IssueSourceRead::Observed(observation)) = &current.item.current else {
            return Err(IssueArbiterWaitReason::ContextUnavailable);
        };
        let IssueSourceContent::Shared(snapshot) = &observation.content else {
            return Err(IssueArbiterWaitReason::ContextUnavailable);
        };
        if snapshot.policy_version != privacy_version
            || current.basis.field_hash.as_ref() != Some(&snapshot.basis.field_hash)
        {
            return Err(IssueArbiterWaitReason::ContextUnavailable);
        }
        let paths = selected_paths(&snapshot.shared_copy.body)?;
        let source = runtime
            .evidence
            .current_commit_sha(&self.workspace)
            .map_err(|_| IssueArbiterWaitReason::EvidenceUnavailable)?;
        let evidence = runtime
            .evidence
            .read(IssueAssessmentEvidenceRequest {
                workspace_directory: &self.workspace,
                commit_sha: &source,
                selected_paths: &paths,
                policy: &automation,
            })
            .map_err(|_| IssueArbiterWaitReason::EvidenceUnavailable)?;
        if evidence.commit_sha != source {
            return Err(IssueArbiterWaitReason::EvidenceChanged);
        }
        let definition = current.record.request.definition();
        let issue = current.record.request.issue();
        let input = serde_json::json!({
            "schema_version": 1,
            "profile_version": configuration.profile_version,
            "issue": {
                "provider": issue.provider, "server_id": issue.server_id,
                "repository_id": issue.repository_id, "issue_id": issue.issue_id,
                "title": snapshot.shared_copy.title, "body": snapshot.shared_copy.body,
            },
            "decision": {
                "id": current.record.request.id().as_str(), "gate": definition.gate,
                "kind": definition.kind, "question": definition.question.as_str(),
                "options": definition.options, "recommended": definition.recommended,
                "basis": current.basis, "revision": current.record.revision,
                "due_at": definition.due_at,
            },
            "assessment": {
                "eligibility": current.assessment.eligibility(), "readiness": current.assessment.readiness(),
                "risk_class": current.assessment.risk_class(), "effective_mode": current.assessment.effective_mode(),
                "reasons": current.assessment.reasons(), "gates": current.assessment.gates(),
            },
            "policy": automation,
            "repository": evidence,
        });
        validate_disclosed_input(&input, &privacy, &self.workspace)?;
        let json = serde_json::to_string(&input)
            .map_err(|_| IssueArbiterWaitReason::ContextUnavailable)?;
        if json.len() > MAX_ISSUE_ASSESSMENT_INPUT_BYTES {
            return Err(IssueArbiterWaitReason::EvidenceUnavailable);
        }
        // A complete response cannot apply a configuration observed before slow reads.
        self.triage
            .automation_policy_for_version(&current.basis.policy_version)
            .map_err(|_| IssueArbiterWaitReason::ContextUnavailable)?;
        let latest_privacy = self
            .policy
            .resolve(&self.workspace)
            .map_err(|_| IssueArbiterWaitReason::ContextUnavailable)?;
        if issue_policy_version(&latest_privacy) != privacy_version {
            return Err(IssueArbiterWaitReason::ContextUnavailable);
        }
        if runtime.model.configuration().ok().as_ref() != Some(&configuration) {
            return Err(IssueArbiterWaitReason::EvidenceChanged);
        }
        if runtime
            .evidence
            .current_commit_sha(&self.workspace)
            .ok()
            .as_ref()
            != Some(&source)
        {
            return Err(IssueArbiterWaitReason::EvidenceChanged);
        }
        let basis = IssueArbiterInputBasis {
            decision_basis: current.basis.clone(),
            decision_revision: current.record.revision,
            binding_run_revision: current.fence.triage.binding_run_revision,
            repository_commit_sha: source,
            evidence_sha256: evidence.snapshot_sha256,
            model_route_revision: configuration.route_revision.clone(),
            input_sha256: format!("{:x}", Sha256::digest(json.as_bytes())),
        };
        basis
            .validate()
            .map_err(|_| IssueArbiterWaitReason::EvidenceUnavailable)?;
        Ok(PreparedArbiter {
            basis,
            settings,
            configuration,
            json,
        })
    }

    pub(super) fn sanitize_arbiter_reply(
        &self,
        current: &CurrentArbiter,
        raw: &str,
    ) -> std::result::Result<IssueArbiterProposal, IssueArbiterWaitReason> {
        let reply = IssueArbiterModelReply::parse_json(raw, &current.record.request)
            .map_err(|_| IssueArbiterWaitReason::InvalidResponse)?;
        let privacy = self
            .policy
            .resolve(&self.workspace)
            .map_err(|_| IssueArbiterWaitReason::ContextUnavailable)?;
        if issue_policy_version(&privacy) != current.assessment.basis().validation_policy_version {
            return Err(IssueArbiterWaitReason::ContextUnavailable);
        }
        let has_blockers = !reply.blockers.is_empty();
        let note = if reply.reasons.is_empty() || has_blockers {
            None
        } else {
            let text = reply.reasons.join("\n");
            if text.len() > MAX_ISSUE_DECISION_NOTE_BYTES {
                return Err(IssueArbiterWaitReason::InvalidResponse);
            }
            Some(
                super::super::super::issue_decision::shared_text(&text, &privacy)
                    .map_err(|_| IssueArbiterWaitReason::InvalidResponse)?,
            )
        };
        Ok(IssueArbiterProposal {
            decision: reply.decision,
            confidence: reply.confidence,
            has_blockers,
            note,
        })
    }
}

fn selected_paths(body: &str) -> std::result::Result<Vec<String>, IssueArbiterWaitReason> {
    let mentioned = crate::domain::issue_automation::triage::mentioned_paths(body);
    if mentioned.incomplete {
        return Err(IssueArbiterWaitReason::EvidenceUnavailable);
    }
    let mut paths: BTreeSet<String> = ["README.md", "Cargo.toml", "package.json"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    // Triage also considers bare names for conservative protection matching. Evidence
    // selects explicit file-like paths only; state/field identifiers are not files.
    paths.extend(
        mentioned
            .paths
            .into_iter()
            .filter(|path| path.contains('/') || path.contains('.')),
    );
    if paths.len() > MAX_ISSUE_ASSESSMENT_EVIDENCE_PATHS {
        return Err(IssueArbiterWaitReason::EvidenceUnavailable);
    }
    Ok(paths.into_iter().collect())
}

// Validate the unescaped values, including policy strings and evidence path names.
// Masking a value here would invalidate the immutable input/evidence fingerprints.
fn validate_disclosed_input(
    value: &serde_json::Value,
    privacy: &crate::domain::issue_policy::IssuePolicy,
    workspace: &str,
) -> std::result::Result<(), IssueArbiterWaitReason> {
    match value {
        serde_json::Value::String(text) => {
            let spans = secret_spans(IssueTextField::Body, text);
            if text.contains(workspace) || !spans.is_empty() {
                return Err(IssueArbiterWaitReason::EvidenceUnavailable);
            }
            let disclosed = plan_issue_masking(
                "",
                text,
                &spans,
                privacy.disclosure_for(IssueDisclosureTarget::AiHandoff),
            )
            .map_err(|_| IssueArbiterWaitReason::EvidenceUnavailable)?;
            if disclosed.shared_copy.body != *text {
                return Err(IssueArbiterWaitReason::EvidenceUnavailable);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                validate_disclosed_input(value, privacy, workspace)?;
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                validate_disclosed_input(value, privacy, workspace)?;
            }
        }
        _ => {}
    }
    Ok(())
}
