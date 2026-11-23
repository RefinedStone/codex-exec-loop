mod external_closure;
mod visibility;

use super::super::tests::{
    Fixture, binding_for, discovered, reference, save_changed, snapshot, transition_and_save,
};
use super::super::{
    APPLICATION_ID, BINDINGS_SCHEMA, ISSUES_SCHEMA, SCHEMA_VERSION, insert_issue_row,
    normalized_schema, private_store,
};
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_source_port::IssueDiscoveryTarget;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest,
};
use crate::domain::issue_automation::source::{
    IssueContentWithheldReason, IssueDiscoverySelection, IssueMetadataCompleteness,
    IssueMetadataList, IssuePrPresence, IssueRemoteState, IssueSourceContent,
    IssueSourceDisclosure, IssueSourceFailure, IssueSourceObservation, IssueSourceRepository,
    IssueSourceWeb,
};
use crate::domain::issue_automation::{
    IssueAutomationAutonomy, IssueAutomationFlag, IssueAutomationState, IssueDecisionResumePhase,
    IssueTransition, IssueTriageResult, TrackedIssue,
};
use crate::domain::issue_policy::IssueDisclosurePolicy;

fn observed(fixture: &Fixture, id: &str, body: &str) -> IssueSourceObservation {
    IssueSourceObservation {
        content_problems: vec![],
        jira: None,
        repository_visibility: Default::default(),
        repository_name_with_owner: None,
        creation_provenance:
            crate::domain::issue_automation::source::IssueCreationProvenance::Unattributed,
        issue: reference(fixture.cwd(), id),
        web: None,
        remote_state: IssueRemoteState::Open,
        labels: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        assignees: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        author: None,
        label_actors: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Unknown,
        content: IssueSourceContent::Shared(snapshot(body)),
    }
}

fn withheld(fixture: &Fixture, id: &str) -> IssueSourceObservation {
    let mut observation = observed(fixture, id, "unused shared fixture");
    observation.content = IssueSourceContent::Withheld {
        basis: snapshot("unused").basis,
        policy_version: "policy-v1".to_owned(),
        reason: IssueContentWithheldReason::UserConfirmationRequired,
    };
    observation
}

fn request(fixture: &Fixture, state: &StoredIssueCollection) -> IssueDiscoveryRequest {
    let repository = IssueSourceRepository::from_reference(&reference(fixture.cwd(), "I_anchor"));
    IssueDiscoveryRequest {
        target: if state.binding_id.is_some() {
            IssueDiscoveryTarget::Repository(repository)
        } else {
            IssueDiscoveryTarget::GitHubLocator {
                owner: "fixture-owner".to_owned(),
                name: "fixture-repository".to_owned(),
                project_scope: repository.project_scope,
                expected_repository_id: None,
            }
        },
        selection: IssueDiscoverySelection::AllOpen,
        cursor: state.discovery_cursor.clone(),
        page_size: 3,
        disclosure: IssueSourceDisclosure {
            policy: IssueDisclosurePolicy::builtin(),
            policy_version: "policy-v1".to_owned(),
        },
    }
}

fn page(
    fixture: &Fixture,
    request: &IssueDiscoveryRequest,
    observations: Vec<IssueSourceObservation>,
    after: Option<&str>,
) -> IssueDiscoveryPage {
    let repository = IssueSourceRepository::from_reference(&reference(fixture.cwd(), "I_anchor"));
    IssueDiscoveryPage {
        jira_connection: None,
        jira_binding: None,
        next_cursor: after.map(|after| {
            IssueDiscoveryCursor::new(
                repository.clone(),
                request.selection.clone(),
                request.page_size,
                &request.disclosure,
                after,
            )
            .unwrap()
        }),
        repository,
        scanned_count: observations.len() as u16,
        observations,
    }
}

fn commit(result: IssueCollectionApplyResult) -> IssueCollectionCommit {
    match result {
        IssueCollectionApplyResult::Applied(commit) => *commit,
        result => panic!("expected a committed collection page, got {result:?}"),
    }
}

fn mode(result: IssueCollectionModeSaveResult) -> StoredIssueCollection {
    match result {
        IssueCollectionModeSaveResult::Saved { state, .. } => *state,
        result => panic!("expected a committed collection mode, got {result:?}"),
    }
}

fn unknown(issue: &crate::domain::issue_work::IssueReference) -> IssueSourceRead {
    IssueSourceRead::Unknown {
        issue: issue.clone(),
        reason: IssueSourceFailure::ProviderUnavailable,
    }
}

fn resync_one(
    fixture: &Fixture,
    state: &StoredIssueCollection,
    value: IssueSourceRead,
) -> IssueCollectionCommit {
    let adapter = fixture.adapter();
    let batch = adapter
        .read_resync_batch(fixture.cwd(), state.fence(), 1)
        .unwrap()
        .unwrap();
    assert_eq!(batch.targets.len(), 1);
    commit(
        adapter
            .apply_resync_batch(fixture.cwd(), &batch, &[value])
            .unwrap(),
    )
}

#[test]
fn write_results_keep_their_committed_projection_after_later_mode_and_binding_changes() {
    let fixture = Fixture::new("collector-commit-projection");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let discovered = commit(
        adapter
            .apply_discovery_page(
                fixture.cwd(),
                initial.fence(),
                &request,
                &page(
                    &fixture,
                    &request,
                    vec![observed(&fixture, "I_1", "shared")],
                    None,
                ),
            )
            .unwrap(),
    );
    assert_eq!(discovered.projection.state, discovered.state);
    let discovery_projection = discovered.projection.clone();

    let (off_state, off_projection) = match adapter
        .set_collection_mode(
            fixture.cwd(),
            Some(discovered.state.epoch),
            IssueCollectionMode::Off,
        )
        .unwrap()
    {
        IssueCollectionModeSaveResult::Saved {
            state,
            projection,
            changed: true,
        } => {
            assert_eq!(*state, projection.state);
            (*state, *projection)
        }
        result => panic!("expected an Off commit, got {result:?}"),
    };
    let mut changed_binding = off_projection.binding.clone().unwrap();
    changed_binding.set_autonomy(IssueAutomationAutonomy::Propose);
    adapter
        .save_binding(fixture.cwd(), 1, &changed_binding)
        .unwrap();
    let resumed = mode(
        adapter
            .set_collection_mode(
                fixture.cwd(),
                Some(off_state.epoch),
                IssueCollectionMode::Observe,
            )
            .unwrap(),
    );
    let latest = adapter
        .get_collection_projection(fixture.cwd())
        .unwrap()
        .unwrap();

    assert_eq!(off_projection.state.config.mode, IssueCollectionMode::Off);
    assert_eq!(off_projection.state.epoch, initial.epoch + 1);
    assert_eq!(
        off_projection.binding.as_ref().unwrap().autonomy(),
        IssueAutomationAutonomy::Observe
    );
    assert_eq!(latest.state, resumed);
    assert_eq!(latest.binding, Some(changed_binding));
    assert!(latest.catalog_revision > off_projection.catalog_revision);
    assert!(off_projection.catalog_revision > discovery_projection.catalog_revision);
    assert_eq!(discovered.projection, discovery_projection);
    assert_eq!(
        discovered.projection.state.config.mode,
        IssueCollectionMode::Observe
    );
    assert_eq!(
        discovered.projection.binding.as_ref().unwrap().autonomy(),
        IssueAutomationAutonomy::Observe
    );
}

#[test]
fn an_explicit_off_is_durable_before_repository_resolution_and_survives_default_initialization() {
    let fixture = Fixture::new("collector-off-first");
    let adapter = fixture.adapter();
    assert!(adapter.get_collection(fixture.cwd()).unwrap().is_none());
    let off = mode(
        adapter
            .set_collection_mode(fixture.cwd(), None, IssueCollectionMode::Off)
            .unwrap(),
    );
    assert_eq!(off.epoch, 1);
    assert!(off.binding_id.is_none());
    assert!(adapter.list_bindings(fixture.cwd()).unwrap().is_empty());
    assert_eq!(
        fixture.adapter().ensure_collection(fixture.cwd()).unwrap(),
        off
    );
    assert_eq!(
        adapter.get_collection(fixture.cwd()).unwrap(),
        Some(off.clone())
    );

    let observe = mode(
        adapter
            .set_collection_mode(fixture.cwd(), Some(1), IssueCollectionMode::Observe)
            .unwrap(),
    );
    assert_eq!((observe.epoch, observe.revision), (2, 2));
    assert!(matches!(
        adapter
            .set_collection_mode(fixture.cwd(), Some(1), IssueCollectionMode::Observe)
            .unwrap(),
        IssueCollectionModeSaveResult::Saved { changed: false, .. }
    ));
    let off_again = mode(
        adapter
            .set_collection_mode(fixture.cwd(), Some(2), IssueCollectionMode::Off)
            .unwrap(),
    );
    assert!(matches!(
        adapter
            .set_collection_mode(fixture.cwd(), Some(1), IssueCollectionMode::Observe)
            .unwrap(),
        IssueCollectionModeSaveResult::Conflict { .. }
    ));
    assert_eq!(adapter.ensure_collection(fixture.cwd()).unwrap(), off_again);
    for invalid in [0, u64::MAX] {
        assert!(
            adapter
                .set_collection_mode(fixture.cwd(), Some(invalid), IssueCollectionMode::Observe)
                .is_err()
        );
    }
}

#[test]
fn page_failure_rolls_back_earlier_rows_binding_and_cursor_then_replay_commits_once() {
    let fixture = Fixture::new("collector-atomic-page");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    assert_eq!(initial.config.mode, IssueCollectionMode::Observe);
    let request = request(&fixture, &initial);
    let mut broken = withheld(&fixture, "I_C");
    let IssueSourceContent::Withheld { basis, .. } = &mut broken.content else {
        unreachable!()
    };
    basis.field_hash.sha256 = "invalid-fingerprint".to_owned();
    let mut page = page(
        &fixture,
        &request,
        vec![
            observed(&fixture, "I_A", "shared A"),
            observed(&fixture, "I_B", "shared B"),
            broken,
        ],
        Some("after-C"),
    );
    assert!(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .is_err()
    );
    assert_eq!(
        adapter.get_collection(fixture.cwd()).unwrap(),
        Some(initial.clone())
    );
    assert!(adapter.list_bindings(fixture.cwd()).unwrap().is_empty());
    assert!(
        adapter
            .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
            .unwrap()
            .items
            .is_empty()
    );

    page.observations[2] = observed(&fixture, "I_C", "shared C");
    let applied = commit(
        fixture
            .adapter()
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
    );
    assert_eq!(applied.newly_tracked.len(), 3);
    assert_eq!(applied.state.discovery_cursor, page.next_cursor);
    assert!(matches!(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
        IssueCollectionApplyResult::Discarded { .. }
    ));
    assert_eq!(adapter.list_issues(fixture.cwd(), None).unwrap().len(), 3);
    let request = self::request(&fixture, &applied.state);
    page.next_cursor = None;
    let replay = commit(
        adapter
            .apply_discovery_page(fixture.cwd(), applied.state.fence(), &request, &page)
            .unwrap(),
    );
    assert!(replay.newly_tracked.is_empty());
    assert!(replay.state.discovery_cursor.is_none());
    assert_eq!(
        adapter
            .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
            .unwrap()
            .items
            .len(),
        3
    );
}

#[test]
fn withheld_unknown_and_shared_use_one_catalog_identity_without_a_stale_snapshot_projection() {
    let fixture = Fixture::new("collector-withheld");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let page = page(&fixture, &request, vec![withheld(&fixture, "I_A")], None);
    let first = commit(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
    );
    assert_eq!(first.newly_tracked, vec![reference(fixture.cwd(), "I_A")]);
    assert!(adapter.list_issues(fixture.cwd(), None).unwrap().is_empty());
    let first_view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert!(first_view.items[0].tracked.is_none());
    assert!(matches!(
        first_view.items[0].current.as_ref().unwrap(),
        IssueSourceRead::Observed(value) if matches!(&value.content, IssueSourceContent::Withheld { .. })
    ));
    let alternate = binding_for("alternate-binding", &reference(fixture.cwd(), "I_A"));
    adapter.insert_binding(fixture.cwd(), &alternate).unwrap();
    assert!(
        adapter
            .insert_issue(
                fixture.cwd(),
                &discovered(&alternate, "I_A", "wrong binding")
            )
            .is_err()
    );
    assert!(adapter.list_issues(fixture.cwd(), None).unwrap().is_empty());
    let failed = resync_one(
        &fixture,
        &first.state,
        unknown(&reference(fixture.cwd(), "I_A")),
    );
    assert!(failed.newly_tracked.is_empty());
    assert!(adapter.list_issues(fixture.cwd(), None).unwrap().is_empty());
    let shared = resync_one(
        &fixture,
        &failed.state,
        IssueSourceRead::Observed(Box::new(observed(
            &fixture,
            "I_A",
            "historical-shared-copy-must-not-leak",
        ))),
    );
    assert!(shared.newly_tracked.is_empty());
    let issue = adapter
        .get_issue(fixture.cwd(), &reference(fixture.cwd(), "I_A"))
        .unwrap()
        .unwrap();
    let ready = transition_and_save(
        &adapter,
        fixture.cwd(),
        &issue,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    let mut pending = ready;
    pending.transition(IssueTransition::AwaitDecision).unwrap();
    pending.set_flag(IssueAutomationFlag::Held, true);
    pending.set_flag(IssueAutomationFlag::Stale, true);
    let pending = save_changed(&adapter, fixture.cwd(), &pending);

    let failed = resync_one(&fixture, &shared.state, unknown(pending.reference()));
    let current = adapter
        .get_issue(fixture.cwd(), pending.reference())
        .unwrap()
        .unwrap();
    assert_eq!(current.state(), IssueAutomationState::AwaitingDecision);
    assert_eq!(
        current.decision_resume(),
        Some(IssueDecisionResumePhase::Ready)
    );
    assert!(current.flags().held && current.flags().stale && current.flags().unknown);
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert!(matches!(
        view.items[0].current,
        Some(IssueSourceRead::Unknown { .. })
    ));
    assert!(!format!("{view:?}").contains("historical-shared-copy-must-not-leak"));

    let withheld = resync_one(
        &fixture,
        &failed.state,
        IssueSourceRead::Observed(Box::new(withheld(&fixture, "I_A"))),
    );
    assert!(withheld.newly_tracked.is_empty());
    let current = adapter
        .get_issue(fixture.cwd(), pending.reference())
        .unwrap()
        .unwrap();
    assert!(current.flags().held && current.flags().stale && !current.flags().unknown);
    assert_eq!(current.state(), pending.state());
    assert_eq!(current.snapshot(), pending.snapshot());
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert!(!format!("{view:?}").contains("historical-shared-copy-must-not-leak"));
    let mut closed = observed(&fixture, "I_A", "current shared copy");
    closed.remote_state = IssueRemoteState::Closed;
    let closed = resync_one(
        &fixture,
        &withheld.state,
        IssueSourceRead::Observed(Box::new(closed)),
    );
    assert!(closed.newly_tracked.is_empty());
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert!(
        matches!(view.items[0].current.as_ref(), Some(IssueSourceRead::Observed(value)) if value.remote_state == IssueRemoteState::Closed)
    );
    assert_eq!(
        view.items[0].tracked.as_ref().unwrap().state,
        pending.state()
    );
}

#[test]
fn off_commit_discards_late_discovery_resync_and_status_writes_even_after_observe_returns() {
    let fixture = Fixture::new("collector-off-inflight");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let page = page(
        &fixture,
        &request,
        vec![observed(&fixture, "I_A", "current shared")],
        None,
    );
    let first = commit(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
    );
    let batch = adapter
        .read_resync_batch(fixture.cwd(), first.state.fence(), 1)
        .unwrap()
        .unwrap();
    let off = mode(
        adapter
            .set_collection_mode(
                fixture.cwd(),
                Some(first.state.epoch),
                IssueCollectionMode::Off,
            )
            .unwrap(),
    );
    assert!(
        adapter
            .read_resync_batch(fixture.cwd(), off.fence(), 1)
            .unwrap()
            .is_none()
    );
    for result in [
        adapter
            .apply_resync_batch(fixture.cwd(), &batch, &[unknown(&batch.targets[0])])
            .unwrap(),
        adapter
            .apply_discovery_page(fixture.cwd(), first.state.fence(), &request, &page)
            .unwrap(),
        adapter
            .record_collection_attempt(fixture.cwd(), first.state.fence(), 100, None)
            .unwrap(),
    ] {
        assert!(matches!(
            result,
            IssueCollectionApplyResult::Discarded { .. }
        ));
    }
    assert_eq!(
        adapter.get_collection(fixture.cwd()).unwrap(),
        Some(off.clone())
    );
    let observe = mode(
        adapter
            .set_collection_mode(fixture.cwd(), Some(off.epoch), IssueCollectionMode::Observe)
            .unwrap(),
    );
    assert!(matches!(
        adapter
            .apply_resync_batch(fixture.cwd(), &batch, &[unknown(&batch.targets[0])])
            .unwrap(),
        IssueCollectionApplyResult::Discarded { .. }
    ));
    let failed = commit(
        adapter
            .record_collection_attempt(
                fixture.cwd(),
                observe.fence(),
                100,
                Some(IssueCollectionFailure::Source(
                    IssueSourceFailure::ProviderUnavailable,
                )),
            )
            .unwrap(),
    );
    assert_eq!(failed.state.status.consecutive_failures, 1);
    assert_eq!(failed.state.status.last_success_at_ms, None);
    let success = commit(
        adapter
            .record_collection_attempt(fixture.cwd(), failed.state.fence(), 200, None)
            .unwrap(),
    );
    assert_eq!(success.state.status.consecutive_failures, 0);
    assert_eq!(success.state.status.last_success_at_ms, Some(200));
    assert_eq!(success.state.discovery_cursor, first.state.discovery_cursor);
    assert_eq!(
        fixture.adapter().get_collection(fixture.cwd()).unwrap(),
        Some(success.state)
    );
    let current = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert!(matches!(
        current.items[0].current,
        Some(IssueSourceRead::Observed(_))
    ));
}

#[test]
fn bounded_resync_includes_legacy_and_withheld_rows_and_finishes_a_fixed_sweep() {
    let fixture = Fixture::new("collector-resync-sweep");
    let adapter = fixture.adapter();
    let (binding, initial_issue) = fixture.insert();
    transition_and_save(
        &adapter,
        fixture.cwd(),
        &initial_issue,
        IssueTransition::Cancel,
    );
    for id in ["I_A", "I_B", "I_C"] {
        adapter
            .insert_issue(fixture.cwd(), &discovered(&binding, id, "legacy shared"))
            .unwrap();
    }
    let mut done = adapter
        .get_issue(fixture.cwd(), &reference(fixture.cwd(), "I_A"))
        .unwrap()
        .unwrap();
    for change in [
        IssueTransition::Assess(IssueTriageResult::Ready),
        IssueTransition::Queue,
        IssueTransition::Start,
        IssueTransition::OpenReview,
        IssueTransition::Complete,
    ] {
        done = transition_and_save(&adapter, fixture.cwd(), &done, change);
    }
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let page = page(&fixture, &request, vec![withheld(&fixture, "I_D")], None);
    let discovered = commit(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
    );
    let batch = adapter
        .read_resync_batch(fixture.cwd(), discovered.state.fence(), 1)
        .unwrap()
        .unwrap();
    assert_eq!(batch.targets, vec![reference(fixture.cwd(), "I_B")]);
    assert_eq!(
        batch.next_cursor.as_ref().unwrap().through,
        reference(fixture.cwd(), "I_D")
    );
    let first = commit(
        adapter
            .apply_resync_batch(fixture.cwd(), &batch, &[unknown(&batch.targets[0])])
            .unwrap(),
    );
    assert!(first.newly_tracked.is_empty());
    adapter
        .insert_issue(
            fixture.cwd(),
            &self::discovered(&binding, "I_E", "later legacy shared"),
        )
        .unwrap();
    let second = adapter
        .read_resync_batch(fixture.cwd(), first.state.fence(), 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        second.targets,
        vec![
            reference(fixture.cwd(), "I_C"),
            reference(fixture.cwd(), "I_D")
        ]
    );
    assert!(second.next_cursor.is_none());
    let reads = second.targets.iter().map(unknown).collect::<Vec<_>>();
    let finished = commit(
        adapter
            .apply_resync_batch(fixture.cwd(), &second, &reads)
            .unwrap(),
    );
    let next = adapter
        .read_resync_batch(fixture.cwd(), finished.state.fence(), 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        next.targets,
        ["I_B", "I_C", "I_D", "I_E"].map(|id| reference(fixture.cwd(), id))
    );
}

#[test]
fn discovery_reset_changes_the_epoch_but_preserves_resync_and_cannot_rebind_the_repository() {
    let fixture = Fixture::new("collector-reset");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let page = page(
        &fixture,
        &request,
        vec![
            observed(&fixture, "I_A", "shared"),
            withheld(&fixture, "I_B"),
        ],
        Some("after-B"),
    );
    let first = commit(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
    );
    let batch = adapter
        .read_resync_batch(fixture.cwd(), first.state.fence(), 1)
        .unwrap()
        .unwrap();
    let first = commit(
        adapter
            .apply_resync_batch(fixture.cwd(), &batch, &[unknown(&batch.targets[0])])
            .unwrap(),
    );
    let reset = commit(
        adapter
            .reset_discovery_cursor(fixture.cwd(), first.state.fence())
            .unwrap(),
    );
    assert_eq!(reset.state.epoch, first.state.epoch + 1);
    assert_eq!(reset.state.revision, first.state.revision + 1);
    assert!(reset.state.discovery_cursor.is_none());
    assert_eq!(reset.state.resync_cursor, first.state.resync_cursor);
    let noop = commit(
        adapter
            .reset_discovery_cursor(fixture.cwd(), reset.state.fence())
            .unwrap(),
    );
    assert_eq!(noop.state, reset.state);
    assert!(matches!(
        adapter
            .apply_discovery_page(fixture.cwd(), first.state.fence(), &request, &page)
            .unwrap(),
        IssueCollectionApplyResult::Discarded { .. }
    ));
    let mut new_request = self::request(&fixture, &reset.state);
    let mut other_page = self::page(&fixture, &new_request, Vec::new(), None);
    other_page.repository.repository_id = "R_different".to_owned();
    new_request.target = IssueDiscoveryTarget::Repository(other_page.repository.clone());
    assert!(
        adapter
            .apply_discovery_page(
                fixture.cwd(),
                reset.state.fence(),
                &new_request,
                &other_page
            )
            .is_err()
    );
    assert_eq!(
        adapter.get_collection(fixture.cwd()).unwrap(),
        Some(reset.state)
    );
}

#[test]
fn catalog_pagination_is_a_binary_union_and_never_fills_unobserved_rows_from_legacy_snapshots() {
    let fixture = Fixture::new("collector-union-pages");
    let adapter = fixture.adapter();
    let (binding, _) = fixture.insert();
    adapter
        .insert_issue(
            fixture.cwd(),
            &discovered(&binding, "I_0", "unobserved historical copy"),
        )
        .unwrap();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let page = page(
        &fixture,
        &request,
        vec![
            observed(&fixture, "I_1", "new current copy"),
            withheld(&fixture, "I_A"),
            withheld(&fixture, "I_a"),
        ],
        None,
    );
    let result = commit(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
    );
    assert_eq!(result.newly_tracked.len(), 2);
    let catalog_revision = adapter
        .read_triage_batch(fixture.cwd(), None, 1)
        .unwrap()
        .revision;
    let mut query = IssueCollectionQuery {
        binding_id: Some(binding.id().clone()),
        after: None,
        limit: 1,
    };
    let mut items = Vec::new();
    loop {
        let page = adapter
            .query_collection_issues(fixture.cwd(), &query)
            .unwrap();
        assert_eq!(page.revision, catalog_revision);
        query.after = page.next_after;
        items.extend(page.items);
        if query.after.is_none() {
            break;
        }
    }
    assert_eq!(
        items
            .iter()
            .map(|item| item.issue.issue_id.as_str())
            .collect::<Vec<_>>(),
        ["I_0", "I_1", "I_A", "I_a"]
    );
    assert!(items[0].current.is_none());
    assert!(!format!("{items:?}").contains("unobserved historical copy"));
    let mut other_reference = reference(fixture.cwd(), "I_1");
    other_reference.repository_id = "R_other".to_owned();
    let other_binding = binding_for("other-binding", &other_reference);
    adapter
        .insert_binding(fixture.cwd(), &other_binding)
        .unwrap();
    adapter
        .insert_issue(
            fixture.cwd(),
            &discovered(&other_binding, "I_1", "other repository"),
        )
        .unwrap();
    let all = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(all.items.len(), 5);
    let filtered = adapter
        .query_collection_issues(
            fixture.cwd(),
            &IssueCollectionQuery {
                binding_id: Some(other_binding.id().clone()),
                ..IssueCollectionQuery::default()
            },
        )
        .unwrap();
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].issue, other_reference);
}

fn finish_tracking_intervention(
    fixture: &Fixture,
    issue: &crate::domain::issue_work::IssueReference,
    command_id: &str,
    action: IssueInterventionAction,
) -> IssueInterventionReceipt {
    let adapter = fixture.adapter();
    let receipt = IssueInterventionReceipt {
        issue: issue.clone(),
        command_id: command_id.to_owned(),
        action: action.label().to_owned(),
        work_order_id: None,
        authorization_epoch: adapter
            .issue_authorization_epoch(fixture.cwd(), issue)
            .unwrap()
            + 1,
        changed: true,
        delivery: None,
        handoff: None,
    };
    adapter
        .begin_issue_intervention(
            fixture.cwd(),
            &IssueInterventionRequest {
                command_id: command_id.to_owned(),
                issue: issue.clone(),
                action,
            },
        )
        .unwrap();
    adapter
        .finish_issue_intervention(fixture.cwd(), &receipt)
        .unwrap();
    receipt
}

#[test]
fn untracked_legacy_rows_remain_in_catalog_and_triage_pages() {
    let fixture = Fixture::new("collector-untracked-legacy-pages");
    let adapter = fixture.adapter();
    let (binding, still_tracked) = fixture.insert();
    let legacy = adapter
        .insert_issue(
            fixture.cwd(),
            &discovered(&binding, "I_0", "unobserved historical copy"),
        )
        .unwrap()
        .issue;
    adapter.ensure_collection(fixture.cwd()).unwrap();
    finish_tracking_intervention(
        &fixture,
        legacy.reference(),
        "untrack-legacy",
        IssueInterventionAction::Untrack,
    );

    // Reopen the adapter so identity and tracking come from the durable rows.
    let adapter = fixture.adapter();
    let mut query = IssueCollectionQuery {
        binding_id: Some(binding.id().clone()),
        after: None,
        limit: 1,
    };
    let first = adapter
        .query_collection_issues(fixture.cwd(), &query)
        .unwrap();
    assert_eq!(first.items.len(), 1);
    let item = &first.items[0];
    assert_eq!(&item.issue, legacy.reference());
    assert_eq!(&item.binding_id, binding.id());
    assert!(item.tracked.is_none());
    assert_eq!(item.tracked_revision, Some(legacy.revision()));
    assert!(item.current.is_none());
    assert!(item.observation_revision.is_none());
    assert!(!format!("{item:?}").contains("unobserved historical copy"));
    assert_eq!(first.next_after.as_ref(), Some(legacy.reference()));

    let triage = adapter.read_triage_batch(fixture.cwd(), None, 1).unwrap();
    assert_eq!(triage.items, first.items);
    assert_eq!(triage.next_after, first.next_after);
    query.after = first.next_after;
    let second = adapter
        .query_collection_issues(fixture.cwd(), &query)
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(&second.items[0].issue, still_tracked.reference());
    assert!(second.items[0].tracked.is_some());
    assert!(second.next_after.is_none());
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), legacy.reference())
            .unwrap(),
        Some(legacy.clone())
    );

    finish_tracking_intervention(
        &fixture,
        legacy.reference(),
        "track-legacy",
        IssueInterventionAction::Track,
    );
    query.after = None;
    let retracked = adapter
        .query_collection_issues(fixture.cwd(), &query)
        .unwrap();
    assert_eq!(retracked.items.len(), 1);
    assert_eq!(&retracked.items[0].issue, legacy.reference());
    assert!(retracked.items[0].tracked.is_some());
    assert!(retracked.items[0].current.is_none());
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), legacy.reference())
            .unwrap(),
        Some(legacy)
    );
}

#[test]
fn untracked_legacy_rows_resync_without_retracking() {
    for batched in [false, true] {
        for unavailable in [false, true] {
            let fixture = Fixture::new("collector-untracked-legacy-resync");
            let adapter = fixture.adapter();
            let (binding, legacy) = fixture.insert();
            let state = adapter.ensure_collection(fixture.cwd()).unwrap();
            let receipt = finish_tracking_intervention(
                &fixture,
                legacy.reference(),
                "untrack-legacy",
                IssueInterventionAction::Untrack,
            );
            let batch = adapter
                .read_resync_batch(fixture.cwd(), state.fence(), 1)
                .unwrap()
                .unwrap();
            assert_eq!(batch.targets, vec![legacy.reference().clone()]);
            let read = if unavailable {
                unknown(legacy.reference())
            } else {
                IssueSourceRead::Observed(Box::new(observed(&fixture, "I_1", "fresh shared copy")))
            };
            let result = if batched {
                adapter
                    .apply_resync_batch(fixture.cwd(), &batch, std::slice::from_ref(&read))
                    .unwrap()
            } else {
                adapter
                    .apply_issue_resync(fixture.cwd(), state.fence(), legacy.reference(), &read)
                    .unwrap()
            };
            let applied = commit(result);
            assert!(applied.newly_tracked.is_empty());
            let page = fixture
                .adapter()
                .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
                .unwrap();
            assert_eq!(page.items.len(), 1);
            let item = &page.items[0];
            assert_eq!(&item.issue, legacy.reference());
            assert_eq!(&item.binding_id, binding.id());
            assert!(item.tracked.is_none());
            assert_eq!(item.current.as_ref(), Some(&read));
            assert!(item.observation_revision.is_some());
            let persisted = adapter
                .get_issue(fixture.cwd(), legacy.reference())
                .unwrap()
                .unwrap();
            assert_eq!(persisted.reference(), legacy.reference());
            assert_eq!(persisted.binding_id(), binding.id());
            assert_eq!(persisted.state(), legacy.state());
            assert_eq!(item.tracked_revision, Some(persisted.revision()));
            assert_eq!(
                adapter
                    .get_issue_intervention(fixture.cwd(), "untrack-legacy")
                    .unwrap()
                    .unwrap()
                    .receipt,
                Some(receipt)
            );
        }
    }
}

#[test]
fn wrong_result_identity_or_missing_batch_results_do_not_advance_resync() {
    let fixture = Fixture::new("collector-resync-validation");
    let adapter = fixture.adapter();
    fixture.insert();
    let state = adapter.ensure_collection(fixture.cwd()).unwrap();
    let batch = adapter
        .read_resync_batch(fixture.cwd(), state.fence(), 1)
        .unwrap()
        .unwrap();
    assert!(
        adapter
            .apply_resync_batch(fixture.cwd(), &batch, &[])
            .is_err()
    );
    assert!(
        adapter
            .apply_resync_batch(
                fixture.cwd(),
                &batch,
                &[unknown(&reference(fixture.cwd(), "I_wrong"))],
            )
            .is_err()
    );
    let mut forged = batch.clone();
    forged.targets[0] = reference(fixture.cwd(), "I_not_tracked");
    assert!(matches!(
        adapter
            .apply_resync_batch(fixture.cwd(), &forged, &[unknown(&forged.targets[0])])
            .unwrap(),
        IssueCollectionApplyResult::Discarded { .. }
    ));
    assert_eq!(adapter.get_collection(fixture.cwd()).unwrap(), Some(state));
}

#[test]
fn exact_v1_migration_preserves_binding_workflow_revisions_flags_and_original_definitions() {
    let fixture = Fixture::new("collector-v1-migration");
    let mut binding = fixture.binding("legacy-binding");
    binding.set_autonomy(IssueAutomationAutonomy::Gated);
    let mut issue = discovered(&binding, "I_legacy", "legacy shared copy");
    issue
        .transition(IssueTransition::Assess(IssueTriageResult::Ready))
        .unwrap();
    issue.transition(IssueTransition::AwaitDecision).unwrap();
    issue.set_flag(IssueAutomationFlag::Held, true);
    issue.set_flag(IssueAutomationFlag::Stale, true);
    issue.set_flag(IssueAutomationFlag::Unknown, true);
    let mut record = issue.to_record();
    record.revision = 7;
    let expected_issue = TrackedIssue::hydrate(record.clone()).unwrap();
    {
        let mut opened = private_store::open(&fixture.runtime).unwrap();
        let (connection, _) = opened.parts();
        connection.execute_batch(BINDINGS_SCHEMA).unwrap();
        connection.execute_batch(ISSUES_SCHEMA).unwrap();
        let transaction = connection.transaction().unwrap();
        transaction.execute(
            "INSERT INTO issue_automation_bindings VALUES (?1, 'github', 'github.com', 'R_1', ?2, 'gated', 4)",
            params![binding.id().as_str(), fixture.cwd()],
        ).unwrap();
        insert_issue_row(&transaction, &record).unwrap();
        transaction
            .pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        transaction.pragma_update(None, "user_version", 1).unwrap();
        transaction.commit().unwrap();
    }
    let adapter = fixture.adapter();
    assert!(adapter.get_collection(fixture.cwd()).unwrap().is_none());
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), expected_issue.reference())
            .unwrap(),
        Some(expected_issue)
    );
    let stored_binding = adapter
        .get_binding(fixture.cwd(), binding.id())
        .unwrap()
        .unwrap();
    assert_eq!(stored_binding.binding, binding);
    assert_eq!(stored_binding.revision, 4);
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(view.items.len(), 1);
    assert!(view.items[0].current.is_none());
    let database = Connection::open(fixture.database()).unwrap();
    let version: i64 = database
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    for (name, expected) in [
        ("issue_automation_bindings", BINDINGS_SCHEMA),
        ("tracked_issues", ISSUES_SCHEMA),
    ] {
        let actual: String = database
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(normalized_schema(&actual), normalized_schema(expected));
    }
}

#[test]
fn v1_with_an_unexpected_schema_object_is_not_partially_migrated() {
    let fixture = Fixture::new("collector-invalid-v1");
    {
        let mut opened = private_store::open(&fixture.runtime).unwrap();
        let (connection, _) = opened.parts();
        connection.execute_batch(BINDINGS_SCHEMA).unwrap();
        connection.execute_batch(ISSUES_SCHEMA).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES ('kept');",
            )
            .unwrap();
        connection
            .pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
    }
    assert!(fixture.adapter().ensure_collection(fixture.cwd()).is_err());
    let database = Connection::open(fixture.database()).unwrap();
    let version: i64 = database
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
    let retained: String = database
        .query_row("SELECT value FROM retained", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained, "kept");
    let added: i64 = database.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name IN ('issue_collection_control', 'issue_source_observations')",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(added, 0);
}

#[test]
fn corrupt_current_observation_or_discovery_checkpoint_fails_closed() {
    let fixture = Fixture::new("collector-corrupt-observation");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let page = page(
        &fixture,
        &request,
        vec![withheld(&fixture, "I_A")],
        Some("after-A"),
    );
    let result = commit(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
    );
    let wrong = IssueSourceRead::Observed(Box::new(withheld(&fixture, "I_wrong")));
    {
        let database = Connection::open(fixture.database()).unwrap();
        database
            .execute(
                "UPDATE issue_source_observations SET current_read = ?1",
                [serde_json::to_string(&wrong).unwrap()],
            )
            .unwrap();
    }
    assert!(
        adapter
            .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
            .is_err()
    );
    let mut invalid_cursor = result.state.discovery_cursor.as_ref().unwrap().to_record();
    invalid_cursor.version += 1;
    {
        let database = Connection::open(fixture.database()).unwrap();
        database
            .execute(
                "UPDATE issue_collection_control SET discovery_cursor = ?1",
                [serde_json::to_string(&invalid_cursor).unwrap()],
            )
            .unwrap();
    }
    assert!(adapter.get_collection(fixture.cwd()).is_err());
    assert!(
        adapter
            .reset_discovery_cursor(fixture.cwd(), result.state.fence())
            .is_err()
    );
}

#[test]
fn two_collectors_with_one_fence_cannot_both_apply_a_page_or_emit_first_tracking() {
    use std::sync::{Arc, Barrier};

    let fixture = Fixture::new("collector-fence-race");
    let initial = fixture.adapter().ensure_collection(fixture.cwd()).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles = ["first shared copy", "second shared copy"]
        .into_iter()
        .map(|body| {
            let adapter = fixture.adapter();
            let workspace = fixture.cwd().to_owned();
            let request = request(&fixture, &initial);
            let page = page(
                &fixture,
                &request,
                vec![observed(&fixture, "I_A", body)],
                None,
            );
            let expected = initial.fence();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter
                    .apply_discovery_page(&workspace, expected, &request, &page)
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, IssueCollectionApplyResult::Applied(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, IssueCollectionApplyResult::Discarded { .. }))
            .count(),
        1
    );
    let newly_tracked = results
        .iter()
        .filter_map(|result| match result {
            IssueCollectionApplyResult::Applied(commit) => Some(commit.newly_tracked.len()),
            IssueCollectionApplyResult::Discarded { .. } => None,
        })
        .sum::<usize>();
    assert_eq!(newly_tracked, 1);
    let adapter = fixture.adapter();
    assert_eq!(
        adapter
            .get_collection(fixture.cwd())
            .unwrap()
            .unwrap()
            .revision,
        initial.revision + 1
    );
    assert_eq!(adapter.list_issues(fixture.cwd(), None).unwrap().len(), 1);
    assert_eq!(
        adapter
            .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn current_web_metadata_round_trips_without_reusing_an_older_shared_link_or_content() {
    let fixture = Fixture::new("collector-current-web");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let mut first = observed(&fixture, "I_A", "historical-body-must-not-leak");
    first.web = Some(IssueSourceWeb {
        number: 17,
        url: "https://github.com/sample/widgets/issues/17".to_owned(),
    });
    let page = page(&fixture, &request, vec![first.clone()], None);
    let stored = commit(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .unwrap(),
    );
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(
        view.items[0].current,
        Some(IssueSourceRead::Observed(Box::new(first)))
    );

    let mut current = withheld(&fixture, "I_A");
    current.web = Some(IssueSourceWeb {
        number: 17,
        url: "https://github.com/moved/renamed/issues/17".to_owned(),
    });
    let withheld = resync_one(
        &fixture,
        &stored.state,
        IssueSourceRead::Observed(Box::new(current.clone())),
    );
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(
        view.items[0].current,
        Some(IssueSourceRead::Observed(Box::new(current)))
    );
    let projected = format!("{view:?}");
    assert!(!projected.contains("historical-body-must-not-leak"));
    assert!(!projected.contains("https://github.com/sample/widgets/issues/17"));
    assert!(projected.contains("https://github.com/moved/renamed/issues/17"));

    resync_one(
        &fixture,
        &withheld.state,
        unknown(&reference(fixture.cwd(), "I_A")),
    );
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert!(matches!(
        &view.items[0].current,
        Some(IssueSourceRead::Unknown { .. })
    ));
    assert!(!format!("{view:?}").contains("https://"));
    assert!(!format!("{view:?}").contains("historical-body-must-not-leak"));
}

#[test]
fn legacy_current_json_without_web_is_readable_but_invalid_persisted_web_is_rejected() {
    let fixture = Fixture::new("collector-legacy-web");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let mut value = observed(&fixture, "I_A", "legacy shared body");
    value.web = Some(IssueSourceWeb {
        number: 17,
        url: "https://github.com/sample/widgets/issues/17".to_owned(),
    });
    let page = page(&fixture, &request, vec![value.clone()], None);
    adapter
        .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
        .unwrap();
    let mut encoded =
        serde_json::to_value(IssueSourceRead::Observed(Box::new(value.clone()))).unwrap();
    encoded["value"].as_object_mut().unwrap().remove("web");
    {
        let database = Connection::open(fixture.database()).unwrap();
        database
            .execute(
                "UPDATE issue_source_observations SET current_read = ?1",
                [encoded.to_string()],
            )
            .unwrap();
    }
    value.web = None;
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(
        view.items[0].current,
        Some(IssueSourceRead::Observed(Box::new(value)))
    );

    encoded["value"]["web"] = serde_json::json!({"number":17,"url":"javascript:alert(1)"});
    {
        let database = Connection::open(fixture.database()).unwrap();
        database
            .execute(
                "UPDATE issue_source_observations SET current_read = ?1",
                [encoded.to_string()],
            )
            .unwrap();
    }
    assert!(
        adapter
            .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
            .is_err()
    );
}

#[test]
fn invalid_new_web_metadata_cannot_advance_a_collection_page_or_create_rows() {
    let fixture = Fixture::new("collector-invalid-new-web");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let mut invalid = observed(&fixture, "I_B", "must-not-commit");
    invalid.web = Some(IssueSourceWeb {
        number: 17,
        url: "https://github.com/owner/repository/issues/18".to_owned(),
    });
    let page = page(
        &fixture,
        &request,
        vec![observed(&fixture, "I_A", "must-roll-back"), invalid],
        Some("after-invalid"),
    );
    assert!(
        adapter
            .apply_discovery_page(fixture.cwd(), initial.fence(), &request, &page)
            .is_err()
    );
    assert_eq!(
        adapter.get_collection(fixture.cwd()).unwrap(),
        Some(initial)
    );
    assert!(adapter.list_bindings(fixture.cwd()).unwrap().is_empty());
    assert!(
        adapter
            .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn confirmed_mode_cas_checks_mode_epoch_and_revision_and_never_reuses_an_old_scope() {
    let fixture = Fixture::new("collector-confirmed-mode-cas");
    let adapter = fixture.adapter();
    let off = mode(
        adapter
            .set_collection_mode(fixture.cwd(), None, IssueCollectionMode::Off)
            .unwrap(),
    );
    let before = adapter
        .get_collection_projection(fixture.cwd())
        .unwrap()
        .unwrap();
    for (expected, from) in [
        (
            IssueCollectionFence {
                epoch: off.epoch,
                revision: off.revision + 1,
            },
            IssueCollectionMode::Off,
        ),
        (off.fence(), IssueCollectionMode::Observe),
    ] {
        assert!(matches!(
            adapter
                .set_collection_mode_exact(
                    fixture.cwd(),
                    expected,
                    from,
                    IssueCollectionMode::Observe
                )
                .unwrap(),
            IssueCollectionModeSaveResult::Conflict { .. },
        ));
        assert_eq!(
            adapter
                .get_collection_projection(fixture.cwd())
                .unwrap()
                .unwrap(),
            before
        );
    }
    let saved = adapter
        .set_collection_mode_exact(
            fixture.cwd(),
            off.fence(),
            IssueCollectionMode::Off,
            IssueCollectionMode::Observe,
        )
        .unwrap();
    let observe = match saved {
        IssueCollectionModeSaveResult::Saved {
            state,
            projection,
            changed: true,
        } => {
            assert_eq!(*state, projection.state);
            assert_eq!(projection.catalog_revision, before.catalog_revision + 1);
            *state
        }
        other => panic!("expected an exact committed mode change, got {other:?}"),
    };
    assert_eq!(observe.epoch, off.epoch + 1);
    assert_eq!(observe.revision, off.revision + 1);
    assert!(matches!(
        adapter
            .set_collection_mode_exact(
                fixture.cwd(),
                off.fence(),
                IssueCollectionMode::Off,
                IssueCollectionMode::Observe,
            )
            .unwrap(),
        IssueCollectionModeSaveResult::Conflict { .. },
    ));
    assert!(matches!(
        adapter
            .set_collection_mode_exact(
                fixture.cwd(),
                observe.fence(),
                IssueCollectionMode::Observe,
                IssueCollectionMode::Observe,
            )
            .unwrap(),
        IssueCollectionModeSaveResult::Saved { changed: false, .. },
    ));
    assert_eq!(
        fixture.adapter().get_collection(fixture.cwd()).unwrap(),
        Some(observe)
    );
}

#[test]
fn confirmed_mode_cas_rejects_a_revision_change_without_an_epoch_change() {
    let fixture = Fixture::new("collector-confirmed-revision-cas");
    let adapter = fixture.adapter();
    let before = adapter.ensure_collection(fixture.cwd()).unwrap();
    let after = commit(
        adapter
            .record_collection_attempt(fixture.cwd(), before.fence(), 100, None)
            .unwrap(),
    );
    assert_eq!(after.state.epoch, before.epoch);
    assert_eq!(after.state.revision, before.revision + 1);
    assert!(matches!(
        adapter
            .set_collection_mode_exact(
                fixture.cwd(),
                before.fence(),
                IssueCollectionMode::Observe,
                IssueCollectionMode::Off,
            )
            .unwrap(),
        IssueCollectionModeSaveResult::Conflict { .. },
    ));
    assert_eq!(
        adapter
            .get_collection_projection(fixture.cwd())
            .unwrap()
            .unwrap(),
        after.projection
    );
}
