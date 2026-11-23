use super::*;
use crate::application::port::outbound::issue_collection_origin_port::{
    IssueCollectionOrigin, IssueCollectionOriginPort,
};
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget, IssueResyncRequest,
    IssueSourceCapabilities, IssueSourcePort,
};
use crate::application::service::issue_policy::issue_policy_version;
use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_policy::IssuePolicy;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueSharedCopy,
};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct FlowSource {
    pub read: Mutex<IssueSourceRead>,
    pub discovery_calls: AtomicUsize,
    pub resync_calls: AtomicUsize,
    issue: IssueReference,
}
fn complete<T>(items: Vec<T>) -> IssueMetadataList<T> {
    IssueMetadataList {
        items,
        completeness: IssueMetadataCompleteness::Complete,
    }
}
impl FlowSource {
    pub fn new(workspace: &str) -> Self {
        let issue = IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_flow".into(),
            issue_id: "I_flow".into(),
            project_scope: IssueProjectScope {
                workspace_dir: workspace.into(),
            },
        };
        let shared_copy = IssueSharedCopy {
            title: "수집한 이슈의 작업 결과를 연결한다".into(),
            body: "## 완료 기준\n- 결과 파일에 approved result를 기록한다.\n".into(),
        };
        let observed = IssueSourceObservation {
            repository_visibility: Default::default(),
            repository_name_with_owner: None,
            issue: issue.clone(),
            web: Some(IssueSourceWeb {
                number: 2555,
                url: ISSUE_URL.into(),
            }),
            remote_state: IssueRemoteState::Open,
            creation_provenance: IssueCreationProvenance::Unattributed,
            labels: complete(Vec::new()),
            assignees: complete(Vec::new()),
            author: Some(IssueSourceActor {
                kind: IssueSourceActorKind::UserAccount,
                id: "U_flow_author".into(),
                login: "flow-author".into(),
            }),
            label_actors: complete(Vec::new()),
            linked_pull_request: IssuePrPresence::Absent,
            content: IssueSourceContent::Shared(IssueAutomationSnapshot {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
                    provider_updated_at: "2026-11-21T00:00:00Z".into(),
                },
                shared_copy,
                policy_version: issue_policy_version(&IssuePolicy::builtin()),
            }),
        };
        Self {
            read: Mutex::new(IssueSourceRead::Observed(Box::new(observed))),
            issue,
            discovery_calls: AtomicUsize::new(0),
            resync_calls: AtomicUsize::new(0),
        }
    }
    pub fn issue(&self) -> IssueReference {
        self.issue.clone()
    }
}
impl IssueCollectionOriginPort for FlowSource {
    fn github_origin(&self, workspace: &str) -> Result<IssueCollectionOrigin> {
        ensure!(
            workspace == self.issue.project_scope.workspace_dir,
            "origin scope changed"
        );
        // Git pushes go only to the fixture's local bare origin; this is the source locator.
        Ok(IssueCollectionOrigin {
            owner: "RefinedStone".into(),
            name: "codex-exec-loop".into(),
        })
    }
}
impl IssueSourcePort for FlowSource {
    fn capabilities(&self) -> IssueSourceCapabilities {
        IssueSourceCapabilities::read_only()
    }
    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> std::result::Result<IssueDiscoveryPage, IssueSourceFailure> {
        request.validate()?;
        self.discovery_calls.fetch_add(1, Ordering::SeqCst);
        let repository = IssueSourceRepository::from_reference(&self.issue);
        match &request.target {
            IssueDiscoveryTarget::GitHubLocator {
                project_scope,
                expected_repository_id,
                ..
            } => {
                if project_scope != &repository.project_scope
                    || expected_repository_id
                        .as_ref()
                        .is_some_and(|id| id != &repository.repository_id)
                {
                    return Err(IssueSourceFailure::IdentityMismatch);
                }
            }
            IssueDiscoveryTarget::Repository(target) if target != &repository => {
                return Err(IssueSourceFailure::IdentityMismatch);
            }
            IssueDiscoveryTarget::Repository(_) => {}
        }
        let read = self.read.lock().unwrap().clone();
        let observations = match read {
            IssueSourceRead::Observed(observed) => {
                if observed.remote_state == IssueRemoteState::Open {
                    vec![*observed]
                } else {
                    Vec::new()
                }
            }
            IssueSourceRead::Unknown { reason, .. } => return Err(reason),
        };
        Ok(IssueDiscoveryPage {
            repository,
            observations,
            next_cursor: None,
            scanned_count: 1,
        })
    }
    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        self.resync_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            request.issue, self.issue,
            "resync must use the discovered stable identity"
        );
        self.read.lock().unwrap().clone()
    }
}
