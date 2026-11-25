//! 공급자 관찰 뒤 생성 장부의 안정 좌표를 붙인다. 판정과 신뢰 결정은 domain이 소유한다.
use super::issue_automation_policy::IssueAutomationPolicyService;
use std::sync::Arc;

use crate::application::port::outbound::issue_board_field_port::IssueBoardFieldPort;
use crate::application::port::outbound::issue_creation_provenance_port::{
    IssueCreationProvenancePort, IssueCreationProvenanceSnapshot,
};
use crate::application::port::outbound::issue_feedback_store_port::IssueFeedbackStorePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget, IssueResyncRequest,
    IssueSourceCapabilities, IssueSourcePort,
};
use crate::domain::issue_automation::feedback::{
    IssueFeedbackIntentRecord, IssueFeedbackOperation,
};
use crate::domain::issue_automation::source::{
    IssueCreationProvenance, IssueSourceFailure, IssueSourceObservation, IssueSourceRead,
    IssueSourceRepository,
};
use crate::domain::issue_work::{IssueProvider, IssueReference};

pub struct IssueSourceProvenanceService {
    source: Arc<dyn IssueSourcePort>,
    provenance: Arc<dyn IssueCreationProvenancePort>,
    feedback: Option<(
        Arc<dyn IssueFeedbackStorePort>,
        IssueAutomationPolicyService,
    )>,
    board: Option<(Arc<dyn IssueBoardFieldPort>, IssueAutomationPolicyService)>,
}

impl IssueSourceProvenanceService {
    pub fn new(
        source: Arc<dyn IssueSourcePort>,
        provenance: Arc<dyn IssueCreationProvenancePort>,
    ) -> Self {
        Self {
            source,
            provenance,
            feedback: None,
            board: None,
        }
    }

    /// 정책에 보드 열 조건이 있을 때만 이슈마다 보드 항목을 읽는다 (#2565).
    pub fn with_board_fields(
        mut self,
        board: Arc<dyn IssueBoardFieldPort>,
        policy: IssueAutomationPolicyService,
    ) -> Self {
        self.board = Some((board, policy));
        self
    }

    pub fn with_feedback(
        mut self,
        feedback: Arc<dyn IssueFeedbackStorePort>,
        policy: IssueAutomationPolicyService,
    ) -> Self {
        self.feedback = Some((feedback, policy));
        self
    }

    fn apply_feedback(
        &self,
        observation: &mut IssueSourceObservation,
    ) -> Result<(), IssueSourceFailure> {
        use crate::domain::issue_automation::feedback::{
            ISSUE_FEEDBACK_PROGRESS_LABEL, projection::suppress_owned_progress_label,
        };
        let Some((feedback, policy)) = &self.feedback else {
            return Ok(());
        };
        if !observation
            .labels
            .items
            .iter()
            .any(|label| label.name == ISSUE_FEEDBACK_PROGRESS_LABEL)
        {
            return Ok(());
        }
        let Some(snapshot) = feedback
            .snapshot(&observation.issue)
            .map_err(|_| IssueSourceFailure::ProviderUnavailable)?
        else {
            return Ok(());
        };
        if snapshot.state.issue != observation.issue {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        if let Some(proof) = snapshot.state.label_proof.as_ref() {
            let policy = policy
                .resolve(&observation.issue.project_scope.workspace_dir)
                .map_err(|_| IssueSourceFailure::ProviderUnavailable)?;
            suppress_owned_progress_label(observation, proof, &policy.policy)?;
        }
        Ok(())
    }

    fn apply(
        observation: &mut IssueSourceObservation,
        snapshot: Option<&IssueCreationProvenanceSnapshot>,
    ) {
        observation.creation_provenance = snapshot
            .map(|snapshot| snapshot.classify(&observation.issue))
            .unwrap_or(IssueCreationProvenance::Unresolved);
    }
}

impl IssueSourcePort for IssueSourceProvenanceService {
    fn capabilities(&self) -> IssueSourceCapabilities {
        self.source.capabilities()
    }

    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        request.validate()?;
        let mut page = self.source.discover(request.clone())?;
        let expected_repository = match &request.target {
            IssueDiscoveryTarget::Repository(expected) => expected == &page.repository,
            IssueDiscoveryTarget::GitHubLocator {
                project_scope,
                expected_repository_id,
                ..
            } => {
                page.repository.provider == IssueProvider::GitHub
                    && page.repository.project_scope == *project_scope
                    && expected_repository_id
                        .as_ref()
                        .is_none_or(|id| id == &page.repository.repository_id)
            }
        };
        if !expected_repository
            || page
                .observations
                .iter()
                .any(|item| !page.repository.matches_reference(&item.issue))
        {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        if !page.observations.is_empty() {
            // provider 응답 뒤, publication lease 안에서 페이지당 한 번만 읽는다.
            // 미확정 Add/Remove는 라벨이 아직 없거나 이미 없어졌어도 판정 입력으로 저장하지 않는다.
            if let Some((feedback, _)) = &self.feedback {
                let pending = feedback
                    .pending(&page.repository.project_scope.workspace_dir)
                    .map_err(|_| IssueSourceFailure::ProviderUnavailable)?;
                // 이미 추적 중인 해당 이슈만 보류한다. 같은 페이지의 다른 이슈와
                // 원격 scanned_count/next_cursor는 그대로 진행한다.
                page.observations.retain(|observation| {
                    !has_pending_observed_feedback(&observation.issue, &pending)
                });
            }
            if page.observations.is_empty() {
                return Ok(page);
            }
            // Sent는 원격 생성 전에 저장된다. 원격 응답보다 먼저 장부를 읽으면
            // 동시 생성 시도와 새로 발견한 이슈의 관계를 놓칠 수 있다.
            let snapshot = self.provenance.load(&page.repository).ok();
            let board = self.board_condition(&page.repository.project_scope.workspace_dir);
            for observation in &mut page.observations {
                Self::apply(observation, snapshot.as_ref());
                self.apply_feedback(observation)?;
                self.apply_board(observation, board.as_ref())?;
            }
        }
        Ok(page)
    }

    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        let expected = request.issue.clone();
        if let Err(reason) = request.validate() {
            return IssueSourceRead::Unknown {
                issue: expected,
                reason,
            };
        }
        let mut read = self.source.resync(request);
        let actual = match &read {
            IssueSourceRead::Observed(observation) => &observation.issue,
            IssueSourceRead::Unknown { issue, .. } => issue,
        };
        if actual != &expected {
            return IssueSourceRead::Unknown {
                issue: expected,
                reason: IssueSourceFailure::IdentityMismatch,
            };
        }
        if let Some((feedback, _)) = &self.feedback {
            let pending = match feedback.unresolved(&expected) {
                Ok(pending) => pending,
                Err(_) => {
                    return IssueSourceRead::Unknown {
                        issue: expected,
                        reason: IssueSourceFailure::ProviderUnavailable,
                    };
                }
            };
            if has_pending_observed_feedback(&expected, &pending) {
                return IssueSourceRead::Unknown {
                    issue: expected,
                    reason: IssueSourceFailure::FeedbackUnconfirmed,
                };
            }
        }
        if let IssueSourceRead::Observed(observation) = &mut read {
            let repository = IssueSourceRepository::from_reference(&expected);
            let snapshot = self.provenance.load(&repository).ok();
            Self::apply(observation, snapshot.as_ref());
            let board = self.board_condition(&expected.project_scope.workspace_dir);
            if let Err(reason) = self
                .apply_feedback(observation)
                .and_then(|()| self.apply_board(observation, board.as_ref()))
            {
                return IssueSourceRead::Unknown {
                    issue: expected,
                    reason,
                };
            }
        }
        read
    }
}

fn has_pending_observed_feedback(
    issue: &IssueReference,
    pending: &[IssueFeedbackIntentRecord],
) -> bool {
    pending.iter().any(|record| {
        record.intent.issue.same_remote_issue(issue)
            && record.state.is_unresolved()
            && matches!(
                &record.intent.operation,
                IssueFeedbackOperation::AddLabel { .. }
                    | IssueFeedbackOperation::RemoveLabel { .. }
                    | IssueFeedbackOperation::SetBoardField { .. }
            )
    })
}

mod board;
#[cfg(test)]
mod tests;
