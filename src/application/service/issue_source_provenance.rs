//! 공급자 관찰 뒤 생성 장부의 안정 좌표를 붙인다. 판정과 신뢰 결정은 domain이 소유한다.
use std::sync::Arc;

use crate::application::port::outbound::issue_creation_provenance_port::{
    IssueCreationProvenancePort, IssueCreationProvenanceSnapshot,
};
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget, IssueResyncRequest,
    IssueSourceCapabilities, IssueSourcePort,
};
use crate::domain::issue_automation::source::{
    IssueCreationProvenance, IssueSourceFailure, IssueSourceObservation, IssueSourceRead,
    IssueSourceRepository,
};
use crate::domain::issue_work::IssueProvider;

pub struct IssueSourceProvenanceService {
    source: Arc<dyn IssueSourcePort>,
    provenance: Arc<dyn IssueCreationProvenancePort>,
}

impl IssueSourceProvenanceService {
    pub fn new(
        source: Arc<dyn IssueSourcePort>,
        provenance: Arc<dyn IssueCreationProvenancePort>,
    ) -> Self {
        Self { source, provenance }
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
            // Sent는 원격 생성 전에 저장된다. 원격 응답보다 먼저 장부를 읽으면
            // 동시 생성 시도와 새로 발견한 이슈의 관계를 놓칠 수 있다.
            let snapshot = self.provenance.load(&page.repository).ok();
            for observation in &mut page.observations {
                Self::apply(observation, snapshot.as_ref());
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
        if let IssueSourceRead::Observed(observation) = &mut read {
            if observation.issue != expected {
                return IssueSourceRead::Unknown {
                    issue: expected,
                    reason: IssueSourceFailure::IdentityMismatch,
                };
            }
            let repository = IssueSourceRepository::from_reference(&expected);
            let snapshot = self.provenance.load(&repository).ok();
            Self::apply(observation, snapshot.as_ref());
        }
        read
    }
}

#[cfg(test)]
mod tests;
