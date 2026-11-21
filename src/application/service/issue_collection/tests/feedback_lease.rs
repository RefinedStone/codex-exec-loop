use super::*;
use crate::application::port::outbound::issue_publish_intent_journal_port::{
    IssuePublicationLease, IssuePublishIntentJournalPort,
};
use crate::domain::issue_publish_intent::IssuePublishIntentRecord;

#[derive(Default)]
struct LeaseState {
    held: AtomicBool,
    valid: AtomicBool,
    reads: AtomicUsize,
}
struct Journal(Arc<LeaseState>);
struct Lease(Arc<LeaseState>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.held.store(false, Ordering::SeqCst);
    }
}
impl IssuePublicationLease for Lease {
    fn verify_identity(&self) -> Result<()> {
        if !self.0.valid.load(Ordering::SeqCst) {
            bail!("synthetic publication lease replacement");
        }
        Ok(())
    }
}
impl IssuePublishIntentJournalPort for Journal {
    fn acquire_publication_lease(&self, _: &str) -> Result<Box<dyn IssuePublicationLease + '_>> {
        assert!(!self.0.held.swap(true, Ordering::SeqCst));
        Ok(Box::new(Lease(self.0.clone())))
    }
    fn save(&self, _: &str, _: &IssuePublishIntentRecord) -> Result<()> {
        panic!("collection must not write a publication intent")
    }
    fn load(&self, _: &str) -> Result<Vec<IssuePublishIntentRecord>> {
        panic!("collection must not query publication intents")
    }
}
fn state() -> Arc<LeaseState> {
    Arc::new(LeaseState {
        held: AtomicBool::new(false),
        valid: AtomicBool::new(true),
        reads: AtomicUsize::new(0),
    })
}

#[test]
fn provider_reads_and_observation_commits_share_the_feedback_publication_lease() {
    let fixture = Fixture::new();
    let state = state();
    let discover = state.clone();
    let resync = state.clone();
    let source = FakeSource::new(
        move |request| {
            assert!(discover.held.load(Ordering::SeqCst));
            discover.reads.fetch_add(1, Ordering::SeqCst);
            Ok(page(&request, &["I_1"], None))
        },
        move |request| {
            assert!(resync.held.load(Ordering::SeqCst));
            resync.reads.fetch_add(1, Ordering::SeqCst);
            open_read(request)
        },
    );
    let service = fixture
        .service(source, Arc::default(), Arc::default())
        .with_publication_journal(Arc::new(Journal(state.clone())));
    let committed = AtomicUsize::new(0);
    service
        .collect_once_with_committed(&IssueCollectionCancellation::new(), 100, &|facts| {
            assert!(state.held.load(Ordering::SeqCst));
            committed.fetch_add(facts.len(), Ordering::SeqCst);
        })
        .unwrap();
    assert!(!state.held.load(Ordering::SeqCst));
    assert!(state.reads.load(Ordering::SeqCst) >= 2);
    assert_eq!(committed.load(Ordering::SeqCst), 1);
    assert_eq!(query(&service).items.len(), 1);
}

#[test]
fn replaced_publication_lease_discards_a_read_before_it_can_change_the_basis() {
    let fixture = Fixture::new();
    let state = state();
    let invalidate = state.clone();
    let source = FakeSource::new(
        move |request| {
            assert!(invalidate.held.load(Ordering::SeqCst));
            invalidate.valid.store(false, Ordering::SeqCst);
            Ok(page(&request, &["I_1"], None))
        },
        |_| panic!("an invalid publication lease must stop resync"),
    );
    let service = fixture
        .service(source, Arc::default(), Arc::default())
        .with_publication_journal(Arc::new(Journal(state.clone())));
    assert!(
        service
            .collect_once(&IssueCollectionCancellation::new(), 100)
            .is_err()
    );
    assert!(!state.held.load(Ordering::SeqCst));
    assert!(query(&service).items.is_empty());
}
