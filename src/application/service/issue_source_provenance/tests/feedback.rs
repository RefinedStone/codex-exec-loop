use super::*;
use std::sync::Mutex;

use crate::application::service::issue_feedback::tests::support::{
    FakePolicy, FakeStore, pending_record,
};
use crate::domain::issue_automation::feedback::{
    ISSUE_FEEDBACK_PROGRESS_LABEL, IssueFeedbackCommentOwner, IssueFeedbackIntentState,
    feedback_body_hash,
};
use crate::domain::issue_automation::source::{IssueDiscoveryCursor, IssueSourceLabel};

#[derive(Clone, Copy)]
enum PendingKind {
    Add,
    Remove,
    Create,
    Update,
}

fn record(kind: PendingKind, state: IssueFeedbackIntentState) -> IssueFeedbackIntentRecord {
    // A historical workspace alias shares the same remote issue blocker.
    let mut owner = issue("I_created");
    owner.project_scope.workspace_dir = "/historical-workspace".into();
    let mut record = pending_record(owner);
    record.state = state;
    record.revision = if state == IssueFeedbackIntentState::Recorded {
        1
    } else {
        2
    };
    record.intent.operation = match kind {
        PendingKind::Add => IssueFeedbackOperation::AddLabel {
            label: progress_label(),
            previous_event_id: None,
        },
        PendingKind::Remove => IssueFeedbackOperation::RemoveLabel {
            label: progress_label(),
            owned_event_id: "LE_owned".into(),
        },
        PendingKind::Create => record.intent.operation,
        PendingKind::Update => {
            let IssueFeedbackOperation::CreateComment {
                body,
                body_sha256,
                content_sha256,
            } = record.intent.operation
            else {
                unreachable!();
            };
            IssueFeedbackOperation::UpdateComment {
                owner: Box::new(IssueFeedbackCommentOwner {
                    issue: record.intent.issue.clone(),
                    comment_id: "IC_existing".into(),
                    actor_id: record.intent.actor_id.clone(),
                    latest_intent_id: "previous-intent".into(),
                    body_sha256: feedback_body_hash("previous masked body"),
                    content_sha256: feedback_body_hash("previous shared status"),
                }),
                body,
                body_sha256,
                content_sha256,
            }
        }
    };
    record.validate().unwrap();
    record
}

fn progress_label() -> IssueSourceLabel {
    IssueSourceLabel {
        id: "LA_running".into(),
        name: ISSUE_FEEDBACK_PROGRESS_LABEL.into(),
    }
}

fn observed(present: bool) -> IssueSourceObservation {
    let mut observation = observation("I_created");
    if present {
        observation.labels.items.push(progress_label());
    }
    observation
}

/// The intent becomes visible only when the provider answers. An earlier journal
/// read therefore cannot satisfy these assertions.
struct SourceWithPendingFeedback {
    source: Source,
    feedback: Arc<FakeStore>,
    record: IssueFeedbackIntentRecord,
}

impl SourceWithPendingFeedback {
    fn reveal_pending(&self) {
        self.feedback.data.lock().unwrap().records = vec![self.record.clone()];
    }
}

impl IssueSourcePort for SourceWithPendingFeedback {
    fn capabilities(&self) -> IssueSourceCapabilities {
        self.source.capabilities()
    }

    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        let page = self.source.discover(request);
        self.reveal_pending();
        page
    }

    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        let read = self.source.resync(request);
        self.reveal_pending();
        read
    }
}

fn setup_feedback(
    record: IssueFeedbackIntentRecord,
    observation: IssueSourceObservation,
    read: IssueSourceRead,
) -> IssueSourceProvenanceService {
    let mut page = page();
    page.observations[0] = observation;
    setup_feedback_page(record, page, read)
}

fn setup_feedback_page(
    record: IssueFeedbackIntentRecord,
    page: IssueDiscoveryPage,
    read: IssueSourceRead,
) -> IssueSourceProvenanceService {
    let responded = Arc::new(AtomicBool::new(false));
    let feedback = Arc::new(FakeStore::new(issue("I_created")));
    let source = Arc::new(SourceWithPendingFeedback {
        source: Source {
            page: Ok(page),
            read,
            responded: responded.clone(),
        },
        feedback: feedback.clone(),
        record,
    });
    let provenance = Arc::new(Provenance {
        snapshot: Some(snapshot(false)),
        responded,
        reads: AtomicUsize::new(0),
    });
    let policy = Arc::new(FakePolicy {
        user: Mutex::new("[feedback]\ncomment=true\nlabels=true\n".into()),
        project: Mutex::new(None),
        fail: AtomicBool::new(false),
    });
    IssueSourceProvenanceService::new(source, provenance)
        .with_feedback(feedback, IssueAutomationPolicyService::new(policy))
}

fn resync_request() -> IssueResyncRequest {
    IssueResyncRequest {
        issue: issue("I_created"),
        disclosure: disclosure(),
    }
}

#[test]
fn pending_label_add_and_remove_exclude_only_their_discovery_observation_after_provider() {
    for kind in [PendingKind::Add, PendingKind::Remove] {
        for state in [
            IssueFeedbackIntentState::Recorded,
            IssueFeedbackIntentState::Sent,
            IssueFeedbackIntentState::NeedsConfirmation,
        ] {
            for present in [false, true] {
                let observation = observed(present);
                let service = setup_feedback(
                    record(kind, state),
                    observation.clone(),
                    IssueSourceRead::Observed(Box::new(observation)),
                );
                let page = service.discover(request()).unwrap();
                assert_eq!(page.observations.len(), 1);
                assert_eq!(page.observations[0].issue, issue("I_other"));
                assert_eq!(page.scanned_count, 2);
                assert!(page.next_cursor.is_none());
            }
        }
    }
}

#[test]
fn pending_label_add_and_remove_defer_resync_even_when_provider_is_unknown() {
    for kind in [PendingKind::Add, PendingKind::Remove] {
        for state in [
            IssueFeedbackIntentState::Recorded,
            IssueFeedbackIntentState::Sent,
            IssueFeedbackIntentState::NeedsConfirmation,
        ] {
            for present in [false, true] {
                let observation = observed(present);
                for read in [
                    IssueSourceRead::Observed(Box::new(observation.clone())),
                    IssueSourceRead::Unknown {
                        issue: issue("I_created"),
                        reason: IssueSourceFailure::ProviderUnavailable,
                    },
                ] {
                    let service = setup_feedback(record(kind, state), observation.clone(), read);
                    assert_eq!(
                        service.resync(resync_request()),
                        IssueSourceRead::Unknown {
                            issue: issue("I_created"),
                            reason: IssueSourceFailure::FeedbackUnconfirmed,
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn comment_pending_and_other_remote_label_intents_do_not_defer_observations() {
    let mut foreign = record(PendingKind::Add, IssueFeedbackIntentState::Sent);
    foreign.intent.issue.repository_id = "R_other".into();
    for pending in [
        record(
            PendingKind::Create,
            IssueFeedbackIntentState::NeedsConfirmation,
        ),
        record(PendingKind::Update, IssueFeedbackIntentState::Sent),
        record(PendingKind::Add, IssueFeedbackIntentState::Abandoned),
        foreign,
    ] {
        for present in [false, true] {
            let observation = observed(present);
            let service = setup_feedback(
                pending.clone(),
                observation.clone(),
                IssueSourceRead::Observed(Box::new(observation.clone())),
            );
            let page = service.discover(request()).unwrap();
            assert_eq!(page.observations[0].labels, observation.labels);
            assert_eq!(page.observations[0].content, observation.content);
            let IssueSourceRead::Observed(actual) = service.resync(resync_request()) else {
                panic!("unrelated or comment feedback must not defer the source");
            };
            assert_eq!(actual.labels, observation.labels);
            assert_eq!(actual.content, observation.content);
            assert_eq!(
                actual.creation_provenance,
                IssueCreationProvenance::AkraCreated
            );
        }
    }

    let unknown = IssueSourceRead::Unknown {
        issue: issue("I_created"),
        reason: IssueSourceFailure::ProviderUnavailable,
    };
    let service = setup_feedback(
        record(
            PendingKind::Create,
            IssueFeedbackIntentState::NeedsConfirmation,
        ),
        observed(false),
        unknown.clone(),
    );
    assert_eq!(service.resync(resync_request()), unknown);
}

#[test]
fn sticky_label_feedback_preserves_other_discoveries_scanned_count_and_next_cursor() {
    let mut pending = record(
        PendingKind::Remove,
        IssueFeedbackIntentState::NeedsConfirmation,
    );
    pending.contradictory_response = true;
    pending.validate().unwrap();

    let cursor = IssueDiscoveryCursor::new(
        repository(),
        IssueDiscoverySelection::AllOpen,
        25,
        &disclosure(),
        "cursor-after-two",
    )
    .unwrap();
    for only_pending in [false, true] {
        let mut expected = page();
        expected.next_cursor = Some(cursor.clone());
        if only_pending {
            expected
                .observations
                .retain(|item| item.issue == issue("I_created"));
        }
        let service = setup_feedback_page(
            pending.clone(),
            expected.clone(),
            IssueSourceRead::Observed(Box::new(observed(false))),
        );
        let actual = service.discover(request()).unwrap();
        assert_eq!(actual.repository, expected.repository);
        assert_eq!(actual.scanned_count, expected.scanned_count);
        assert_eq!(actual.next_cursor, expected.next_cursor);
        if only_pending {
            assert!(actual.observations.is_empty());
        } else {
            assert_eq!(actual.observations.len(), 1);
            assert_eq!(actual.observations[0].issue, issue("I_other"));
            assert_eq!(
                actual.observations[0].content,
                expected.observations[1].content
            );
            assert_eq!(
                actual.observations[0].creation_provenance,
                IssueCreationProvenance::Unattributed
            );
        }
        let next_request = IssueDiscoveryRequest {
            cursor: actual.next_cursor,
            ..request()
        };
        next_request.validate().unwrap();
    }
}
