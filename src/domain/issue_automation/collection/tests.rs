use super::*;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

fn reference() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_stable".to_owned(),
        issue_id: "I_stable".to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: "/work/project".to_owned(),
        },
    }
}

fn state() -> StoredIssueCollection {
    StoredIssueCollection {
        config: IssueCollectionConfig::default(),
        epoch: 1,
        revision: 1,
        binding_id: None,
        discovery_cursor: None,
        resync_cursor: None,
        status: IssueCollectionStatus::default(),
    }
}

#[test]
fn new_collection_defaults_to_observe_but_does_not_claim_a_remote_identity() {
    let state = state();
    assert_eq!(state.config.mode, IssueCollectionMode::Observe);
    assert!(state.binding_id.is_none());
    assert_eq!(state.validate(), Ok(()));
}

#[test]
fn persisted_collection_fences_reject_unsaved_overflowing_or_reversed_versions() {
    for fence in [
        IssueCollectionFence {
            epoch: 0,
            revision: 1,
        },
        IssueCollectionFence {
            epoch: 1,
            revision: 0,
        },
        IssueCollectionFence {
            epoch: 2,
            revision: 1,
        },
        IssueCollectionFence {
            epoch: 1,
            revision: u64::MAX,
        },
    ] {
        let mut state = state();
        state.epoch = fence.epoch;
        state.revision = fence.revision;
        assert_eq!(state.validate(), Err(IssueAutomationError::InvalidRevision));
    }
}

#[test]
fn failure_status_cannot_describe_a_success_or_an_unattempted_collection() {
    let mut candidate = state();
    candidate.status.last_failure = Some(IssueCollectionFailure::OriginUnavailable);
    assert!(candidate.validate().is_err());
    candidate.status.consecutive_failures = 1;
    assert!(candidate.validate().is_err());
    candidate.status.last_attempt_at_ms = Some(42);
    assert_eq!(candidate.validate(), Ok(()));
    candidate.status.last_failure = None;
    assert!(candidate.validate().is_err());
    candidate.status.consecutive_failures = 0;
    candidate.status.last_success_at_ms = Some(42);
    assert_eq!(candidate.validate(), Ok(()));
}

#[test]
fn collection_query_bounds_and_cursor_identity_are_validated() {
    for limit in [0, MAX_ISSUE_COLLECTION_PAGE_SIZE + 1] {
        let query = IssueCollectionQuery {
            limit,
            ..IssueCollectionQuery::default()
        };
        assert!(query.validate().is_err());
    }
    let mut query = IssueCollectionQuery {
        after: Some(reference()),
        ..IssueCollectionQuery::default()
    };
    assert_eq!(query.validate(), Ok(()));
    query.after.as_mut().unwrap().issue_id.clear();
    assert!(query.validate().is_err());
}

#[test]
fn resync_checkpoint_cannot_cross_project_scopes() {
    let mut candidate = state();
    let mut through = reference();
    through.project_scope.workspace_dir = "/other/project".to_owned();
    candidate.resync_cursor = Some(IssueResyncCursor {
        after: reference(),
        through,
    });
    assert_eq!(
        candidate.validate(),
        Err(IssueAutomationError::IdentityChanged)
    );
}
