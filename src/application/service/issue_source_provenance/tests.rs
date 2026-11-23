mod feedback;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::domain::issue_automation::source::{
    IssueContentWithheldReason, IssueDiscoverySelection, IssueMetadataCompleteness,
    IssueMetadataList, IssuePrPresence, IssueRemoteState, IssueSourceContent,
    IssueSourceDisclosure,
};
use crate::domain::issue_policy::IssueDisclosurePolicy;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueReference,
};

fn repository() -> IssueSourceRepository {
    IssueSourceRepository {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_repository".into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/work/repository".into(),
        },
    }
}

fn issue(id: &str) -> IssueReference {
    let repository = repository();
    IssueReference {
        provider: repository.provider,
        server_id: repository.server_id,
        repository_id: repository.repository_id,
        issue_id: id.into(),
        project_scope: repository.project_scope,
    }
}

fn list<T>() -> IssueMetadataList<T> {
    IssueMetadataList {
        items: Vec::new(),
        completeness: IssueMetadataCompleteness::Complete,
    }
}

fn observation(id: &str) -> IssueSourceObservation {
    IssueSourceObservation {
        repository_visibility: Default::default(),
        repository_name_with_owner: None,
        issue: issue(id),
        web: None,
        remote_state: IssueRemoteState::Open,
        labels: list(),
        assignees: list(),
        author: None,
        label_actors: list(),
        linked_pull_request: IssuePrPresence::Unknown,
        creation_provenance: IssueCreationProvenance::Unattributed,
        content: IssueSourceContent::Withheld {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields("private", "private"),
                provider_updated_at: "2026-11-20T00:00:00Z".into(),
            },
            policy_version: "current-policy".into(),
            reason: IssueContentWithheldReason::UserConfirmationRequired,
        },
    }
}

fn disclosure() -> IssueSourceDisclosure {
    IssueSourceDisclosure {
        policy: IssueDisclosurePolicy::builtin(),
        policy_version: "current-policy".into(),
    }
}

struct Source {
    page: Result<IssueDiscoveryPage, IssueSourceFailure>,
    read: IssueSourceRead,
    responded: Arc<AtomicBool>,
}

impl IssueSourcePort for Source {
    fn capabilities(&self) -> IssueSourceCapabilities {
        IssueSourceCapabilities::read_only()
    }
    fn discover(&self, _: IssueDiscoveryRequest) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        self.responded.store(true, Ordering::SeqCst);
        self.page.clone()
    }
    fn resync(&self, _: IssueResyncRequest) -> IssueSourceRead {
        self.responded.store(true, Ordering::SeqCst);
        self.read.clone()
    }
}

struct Provenance {
    snapshot: Option<IssueCreationProvenanceSnapshot>,
    responded: Arc<AtomicBool>,
    reads: AtomicUsize,
}

impl IssueCreationProvenancePort for Provenance {
    fn load(&self, _: &IssueSourceRepository) -> anyhow::Result<IssueCreationProvenanceSnapshot> {
        assert!(
            self.responded.load(Ordering::SeqCst),
            "journal must be read after the provider"
        );
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.snapshot
            .clone()
            .ok_or_else(|| anyhow::anyhow!("unavailable private details"))
    }
}

fn snapshot(unresolved: bool) -> IssueCreationProvenanceSnapshot {
    IssueCreationProvenanceSnapshot {
        repository: repository(),
        created_issue_ids: BTreeSet::from(["I_created".into()]),
        unresolved,
    }
}

fn setup(
    snapshot: Option<IssueCreationProvenanceSnapshot>,
    page: Result<IssueDiscoveryPage, IssueSourceFailure>,
    read: IssueSourceRead,
) -> (IssueSourceProvenanceService, Arc<Provenance>) {
    let responded = Arc::new(AtomicBool::new(false));
    let source = Arc::new(Source {
        page,
        read,
        responded: responded.clone(),
    });
    let provenance = Arc::new(Provenance {
        snapshot,
        responded,
        reads: AtomicUsize::new(0),
    });
    (
        IssueSourceProvenanceService::new(source, provenance.clone()),
        provenance,
    )
}

fn page() -> IssueDiscoveryPage {
    IssueDiscoveryPage {
        repository: repository(),
        observations: vec![observation("I_created"), observation("I_other")],
        next_cursor: None,
        scanned_count: 2,
    }
}

fn request() -> IssueDiscoveryRequest {
    IssueDiscoveryRequest {
        target: IssueDiscoveryTarget::Repository(repository()),
        selection: IssueDiscoverySelection::AllOpen,
        cursor: None,
        page_size: 25,
        disclosure: disclosure(),
    }
}

#[test]
fn discovery_reads_one_creation_snapshot_after_the_provider_and_preserves_withheld_content() {
    let expected = page();
    let (service, provenance) = setup(
        Some(snapshot(false)),
        Ok(expected.clone()),
        IssueSourceRead::Observed(Box::new(observation("I_created"))),
    );
    let actual = service.discover(request()).unwrap();
    assert_eq!(provenance.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        actual.observations[0].creation_provenance,
        IssueCreationProvenance::AkraCreated
    );
    assert_eq!(
        actual.observations[1].creation_provenance,
        IssueCreationProvenance::Unattributed
    );
    assert_eq!(
        actual.observations[0].content,
        expected.observations[0].content
    );
    assert_eq!(actual.scanned_count, expected.scanned_count);
    assert_eq!(actual.next_cursor, expected.next_cursor);
    assert_eq!(service.capabilities(), IssueSourceCapabilities::read_only());
}

#[test]
fn unresolved_attempts_do_not_erase_known_stable_ids_or_infer_a_candidate_from_content() {
    let (service, _) = setup(
        Some(snapshot(true)),
        Ok(page()),
        IssueSourceRead::Observed(Box::new(observation("I_created"))),
    );
    let actual = service.discover(request()).unwrap();
    assert_eq!(
        actual.observations[0].creation_provenance,
        IssueCreationProvenance::AkraCreated
    );
    assert_eq!(
        actual.observations[1].creation_provenance,
        IssueCreationProvenance::Unresolved
    );
}

#[test]
fn journal_failure_and_repository_mismatch_are_unresolved_not_unattributed() {
    let mut wrong = snapshot(false);
    wrong.repository.repository_id = "R_other".into();
    for data in [None, Some(wrong)] {
        let (service, _) = setup(
            data,
            Ok(page()),
            IssueSourceRead::Observed(Box::new(observation("I_created"))),
        );
        let actual = service.resync(IssueResyncRequest {
            issue: issue("I_created"),
            disclosure: disclosure(),
        });
        let IssueSourceRead::Observed(actual) = actual else {
            panic!("expected observed content");
        };
        assert_eq!(
            actual.creation_provenance,
            IssueCreationProvenance::Unresolved
        );
    }
}

#[test]
fn unavailable_provider_and_mismatched_scope_do_not_read_local_creation_data() {
    let unknown = IssueSourceRead::Unknown {
        issue: issue("I_created"),
        reason: IssueSourceFailure::ProviderUnavailable,
    };
    let (service, provenance) = setup(
        Some(snapshot(false)),
        Err(IssueSourceFailure::ProviderUnavailable),
        unknown.clone(),
    );
    assert_eq!(
        service.discover(request()),
        Err(IssueSourceFailure::ProviderUnavailable)
    );
    assert_eq!(
        service.resync(IssueResyncRequest {
            issue: issue("I_created"),
            disclosure: disclosure()
        }),
        unknown
    );
    assert_eq!(provenance.reads.load(Ordering::SeqCst), 0);

    let mut wrong_page = page();
    wrong_page.repository.project_scope.workspace_dir = "/somewhere/else".into();
    let mut wrong_observation = observation("I_created");
    wrong_observation.issue.project_scope.workspace_dir = "/somewhere/else".into();
    let (service, provenance) = setup(
        Some(snapshot(false)),
        Ok(wrong_page),
        IssueSourceRead::Observed(Box::new(wrong_observation)),
    );
    assert_eq!(
        service.discover(request()),
        Err(IssueSourceFailure::IdentityMismatch)
    );
    assert_eq!(
        service.resync(IssueResyncRequest {
            issue: issue("I_created"),
            disclosure: disclosure()
        }),
        IssueSourceRead::Unknown {
            issue: issue("I_created"),
            reason: IssueSourceFailure::IdentityMismatch
        }
    );
    assert_eq!(provenance.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn classification_never_crosses_provider_server_repository_or_workspace_identity() {
    let data = snapshot(false);
    let mut variants = vec![issue("I_created"); 4];
    variants[0].provider = IssueProvider::Jira;
    variants[1].server_id = "other.example".into();
    variants[2].repository_id = "R_other".into();
    variants[3].project_scope.workspace_dir = "/work/other".into();
    for issue in variants {
        assert_eq!(data.classify(&issue), IssueCreationProvenance::Unresolved);
    }
}
