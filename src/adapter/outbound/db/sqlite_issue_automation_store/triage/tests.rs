use std::sync::{Arc, Barrier};

use super::super::tests::{Fixture, reference, snapshot};
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget,
};
use crate::application::service::issue_automation_policy::resolve_issue_automation_policy;
use crate::application::service::issue_policy::issue_policy_version;
use crate::domain::issue_automation::collection::StoredIssueCollection;
use crate::domain::issue_automation::source::{
    IssueDiscoverySelection, IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence,
    IssueRemoteState, IssueSourceActor, IssueSourceContent, IssueSourceDisclosure,
    IssueSourceObservation, IssueSourceRepository,
};
use crate::domain::issue_automation::triage::{IssueTriageInput, triage_issue};
use crate::domain::issue_automation::{IssueAutomationAutonomy, IssueAutomationFlag};
use crate::domain::issue_policy::{IssueDisclosurePolicy, IssuePolicy};
use crate::domain::issue_validation::IssueRequirements;

fn observation(fixture: &Fixture) -> IssueSourceObservation {
    let mut snapshot = snapshot("## 완료 기준\n- 변경 사항을 검증한다.\n");
    snapshot.policy_version = issue_policy_version(&IssuePolicy::builtin());
    IssueSourceObservation {
        creation_provenance:
            crate::domain::issue_automation::source::IssueCreationProvenance::Unattributed,
        issue: reference(fixture.cwd(), "I_triage"),
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
        author: Some(IssueSourceActor {
            kind: crate::domain::issue_automation::source::IssueSourceActorKind::UserAccount,
            id: "U_author".into(),
            login: "author".into(),
        }),
        label_actors: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Absent,
        content: IssueSourceContent::Shared(snapshot),
    }
}

fn observe(fixture: &Fixture, observation: IssueSourceObservation) -> StoredIssueCollection {
    let adapter = fixture.adapter();
    let current = adapter.ensure_collection(fixture.cwd()).unwrap();
    let repository = IssueSourceRepository::from_reference(&observation.issue);
    let request = IssueDiscoveryRequest {
        target: IssueDiscoveryTarget::Repository(repository.clone()),
        selection: IssueDiscoverySelection::AllOpen,
        cursor: current.discovery_cursor.clone(),
        page_size: 1,
        disclosure: IssueSourceDisclosure {
            policy: IssueDisclosurePolicy::builtin(),
            policy_version: issue_policy_version(&IssuePolicy::builtin()),
        },
    };
    let page = IssueDiscoveryPage {
        repository,
        observations: vec![observation],
        next_cursor: None,
        scanned_count: 1,
    };
    match adapter.apply_discovery_page(fixture.cwd(), current.fence(), &request, &page).unwrap() {
        crate::application::port::outbound::issue_automation_store_port::IssueCollectionApplyResult::Applied(commit) => commit.state,
        other => panic!("expected observation commit, got {other:?}"),
    }
}

fn subject(fixture: &Fixture) -> (IssueTriageFence, IssueCollectionItem) {
    let mut batch = fixture
        .adapter()
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap();
    let item = batch.items.pop().unwrap();
    let expected = IssueTriageFence {
        collection_epoch: batch.collection.unwrap().epoch,
        observation_revision: item.observation_revision,
        tracked_revision: item.tracked_revision,
        assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
    };
    (expected, item)
}

fn assess(item: &IssueCollectionItem, policy: Option<&str>) -> IssueTriageAssessment {
    let policy = resolve_issue_automation_policy(policy, None).unwrap();
    triage_issue(IssueTriageInput {
        issue: &item.issue,
        current: item.current.as_ref(),
        policy: &policy.policy,
        automation_policy_version: &policy.version,
        requirements: &IssueRequirements::default(),
        validation_policy_version: &issue_policy_version(&IssuePolicy::builtin()),
        held: item
            .tracked
            .as_ref()
            .is_some_and(|tracked| tracked.flags.held),
        pinned_source_commit: None,
        previous: item.triage.as_ref().map(|triage| &triage.assessment),
    })
    .unwrap()
}

fn stored(result: IssueTriageSaveResult) -> (IssueTriageProjection, u64, bool) {
    match result {
        IssueTriageSaveResult::Saved {
            record,
            catalog_revision,
            changed,
            ..
        } => (*record, catalog_revision, changed),
        other => panic!("expected saved triage, got {other:?}"),
    }
}

fn revision(fixture: &Fixture) -> u64 {
    fixture
        .adapter()
        .read_triage_batch(fixture.cwd(), None, 1)
        .unwrap()
        .revision
}

#[test]
fn identical_retriage_and_lost_response_retry_preserve_gate_and_catalog_revisions() {
    let fixture = Fixture::new("triage-noop");
    observe(&fixture, observation(&fixture));
    let (expected, item) = subject(&fixture);
    let workflow = fixture
        .adapter()
        .get_issue(fixture.cwd(), &item.issue)
        .unwrap();
    let candidate = assess(&item, None);
    let (first, first_catalog, changed) = stored(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), expected, &candidate)
            .unwrap(),
    );
    assert!(changed);
    assert!(
        first
            .assessment
            .gates()
            .iter()
            .all(|gate| gate.gate_revision == 1)
    );
    let (retried, retry_catalog, changed) = stored(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), expected, &candidate)
            .unwrap(),
    );
    assert!(!changed);
    assert_eq!((retried, retry_catalog), (first.clone(), first_catalog));

    let (fresh_fence, fresh) = subject(&fixture);
    let (noop, noop_catalog, changed) = stored(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), fresh_fence, &assess(&fresh, None))
            .unwrap(),
    );
    assert!(!changed);
    assert_eq!((noop, noop_catalog), (first.clone(), first_catalog));
    assert_eq!(subject(&fixture).1.triage, Some(first));
    assert_eq!(
        fixture
            .adapter()
            .get_issue(fixture.cwd(), &item.issue)
            .unwrap(),
        workflow
    );
}

#[test]
fn metadata_only_change_is_immediately_stale_and_rejects_the_old_observation_cas() {
    let fixture = Fixture::new("triage-metadata");
    let original = observation(&fixture);
    observe(&fixture, original.clone());
    let (expected, item) = subject(&fixture);
    stored(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), expected, &assess(&item, None))
            .unwrap(),
    );
    let (old_fence, old) = subject(&fixture);
    let workflow = fixture
        .adapter()
        .get_issue(fixture.cwd(), &old.issue)
        .unwrap();

    let mut changed = original;
    changed.assignees.items.push(IssueSourceActor {
        kind: crate::domain::issue_automation::source::IssueSourceActorKind::UserAccount,
        id: "U_claimant".into(),
        login: "claimant".into(),
    });
    observe(&fixture, changed);
    let (next_fence, next) = subject(&fixture);
    assert_ne!(old.observation_revision, next.observation_revision);
    assert_eq!(old.tracked_revision, next.tracked_revision);
    assert!(next.triage.as_ref().unwrap().stale);
    assert_eq!(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), old_fence, &assess(&old, None))
            .unwrap(),
        IssueTriageSaveResult::Conflict,
    );
    let next_assessment = assess(&next, None);
    assert_eq!(
        old.triage.as_ref().unwrap().assessment.basis().field_hash,
        next_assessment.basis().field_hash
    );
    let (saved, _, changed) = stored(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), next_fence, &next_assessment)
            .unwrap(),
    );
    assert!(changed);
    assert!(!saved.stale);
    assert!(
        saved
            .assessment
            .gates()
            .iter()
            .all(|gate| gate.gate_revision == 2)
    );
    assert_eq!(
        fixture
            .adapter()
            .get_issue(fixture.cwd(), &old.issue)
            .unwrap(),
        workflow
    );
}

#[test]
fn held_flag_and_collection_off_aba_are_independent_cas_fences() {
    let fixture = Fixture::new("triage-fences");
    observe(&fixture, observation(&fixture));
    let (expected, item) = subject(&fixture);
    let candidate = assess(&item, None);
    let adapter = fixture.adapter();
    let mut tracked = adapter
        .get_issue(fixture.cwd(), &item.issue)
        .unwrap()
        .unwrap();
    tracked.set_flag(IssueAutomationFlag::Held, true);
    adapter
        .save_issue(fixture.cwd(), tracked.revision(), &tracked)
        .unwrap();
    assert_eq!(
        adapter
            .save_triage(fixture.cwd(), expected, &candidate)
            .unwrap(),
        IssueTriageSaveResult::Conflict
    );

    let (held_fence, held) = subject(&fixture);
    let held_candidate = assess(&held, None);
    adapter
        .set_collection_mode(
            fixture.cwd(),
            Some(held_fence.collection_epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
    assert_eq!(
        adapter
            .save_triage(fixture.cwd(), held_fence, &held_candidate)
            .unwrap(),
        IssueTriageSaveResult::Conflict
    );
    adapter
        .set_collection_mode(
            fixture.cwd(),
            Some(held_fence.collection_epoch + 1),
            IssueCollectionMode::Observe,
        )
        .unwrap();
    assert_eq!(
        adapter
            .save_triage(fixture.cwd(), held_fence, &held_candidate)
            .unwrap(),
        IssueTriageSaveResult::Conflict
    );

    let (current, item) = subject(&fixture);
    stored(
        adapter
            .save_triage(fixture.cwd(), current, &assess(&item, None))
            .unwrap(),
    );
    let retained = adapter
        .get_issue(fixture.cwd(), &item.issue)
        .unwrap()
        .unwrap();
    assert!(retained.flags().held);
    assert_eq!(retained.state(), tracked.state());
}

#[test]
fn supplied_assessment_must_match_current_evidence_and_checked_gate_successor() {
    let fixture = Fixture::new("triage-candidate-proof");
    observe(&fixture, observation(&fixture));
    let (expected, item) = subject(&fixture);
    let before = revision(&fixture);
    let mut altered = item.clone();
    let Some(crate::domain::issue_automation::source::IssueSourceRead::Observed(current)) =
        &mut altered.current
    else {
        panic!("expected observed fixture")
    };
    current.author = None;
    assert!(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), expected, &assess(&altered, None))
            .is_err()
    );

    let mut skipped = assess(&item, None).to_record();
    for gate in &mut skipped.gates {
        gate.gate_revision = 2;
    }
    let skipped = IssueTriageAssessment::hydrate(skipped).unwrap();
    assert!(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), expected, &skipped)
            .is_err()
    );
    assert_eq!(revision(&fixture), before);
    assert!(subject(&fixture).1.triage.is_none());
}

#[test]
fn two_adapters_cannot_both_install_different_first_assessments() {
    let fixture = Fixture::new("triage-competing-writers");
    observe(&fixture, observation(&fixture));
    let (expected, item) = subject(&fixture);
    let candidates = [
        assess(&item, None),
        assess(&item, Some("[select]\nmode = \"all_open\"\n")),
    ];
    let barrier = Arc::new(Barrier::new(2));
    let handles = candidates
        .into_iter()
        .map(|candidate| {
            let adapter = fixture.adapter();
            let workspace = fixture.cwd().to_string();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter
                    .save_triage(&workspace, expected, &candidate)
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, IssueTriageSaveResult::Saved { changed: true, .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, IssueTriageSaveResult::Conflict))
            .count(),
        1
    );
    assert_eq!(subject(&fixture).1.triage.unwrap().revision, 1);
}

#[test]
fn older_semantically_equal_retry_after_intervening_assessments_is_not_accepted() {
    let fixture = Fixture::new("triage-aba");
    observe(&fixture, observation(&fixture));
    let (old_fence, first) = subject(&fixture);
    let old_candidate = assess(&first, None);
    stored(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), old_fence, &old_candidate)
            .unwrap(),
    );
    for policy in [Some("[select]\nmode = \"all_open\"\n"), None] {
        let (fence, item) = subject(&fixture);
        stored(
            fixture
                .adapter()
                .save_triage(fixture.cwd(), fence, &assess(&item, policy))
                .unwrap(),
        );
    }
    assert_eq!(subject(&fixture).1.triage.unwrap().revision, 3);
    assert_eq!(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), old_fence, &old_candidate)
            .unwrap(),
        IssueTriageSaveResult::Conflict,
    );
}

#[test]
fn v2_migration_preserves_workflow_observation_binding_and_collection_rows() {
    let fixture = Fixture::new("triage-v2-migration");
    let state = observe(&fixture, observation(&fixture));
    let before = subject(&fixture).1;
    let adapter = fixture.adapter();
    let workflow = adapter.get_issue(fixture.cwd(), &before.issue).unwrap();
    let binding = adapter
        .get_binding(fixture.cwd(), &before.binding_id)
        .unwrap();
    adapter.with_connection(fixture.cwd(), |connection, _| {
        connection.execute_batch("DROP TABLE issue_decision_requests; DROP TABLE issue_triage_assessments; DROP TABLE issue_automation_catalog; PRAGMA user_version = 2;")?;
        Ok(())
    }).unwrap();

    let reopened = fixture.adapter();
    assert_eq!(reopened.get_collection(fixture.cwd()).unwrap(), Some(state));
    assert_eq!(
        reopened.get_issue(fixture.cwd(), &before.issue).unwrap(),
        workflow
    );
    assert_eq!(
        reopened
            .get_binding(fixture.cwd(), &before.binding_id)
            .unwrap(),
        binding
    );
    let after = subject(&fixture).1;
    assert_eq!(after, before);
    reopened
        .with_connection(fixture.cwd(), |connection, _| {
            let version: i64 =
                connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            assert_eq!(version, super::super::SCHEMA_VERSION);
            super::super::validate_schema(connection)
        })
        .unwrap();
}

#[test]
fn legacy_mutations_advance_catalog_only_when_they_change_stored_values() {
    let fixture = Fixture::new("triage-catalog-mutations");
    let adapter = fixture.adapter();
    let binding = fixture.binding("catalog-binding");
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    let initial = revision(&fixture);
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    adapter.save_binding(fixture.cwd(), 1, &binding).unwrap();
    assert_eq!(revision(&fixture), initial);

    let mut changed_binding = binding.clone();
    changed_binding.set_autonomy(IssueAutomationAutonomy::Propose);
    adapter
        .save_binding(fixture.cwd(), 1, &changed_binding)
        .unwrap();
    assert_eq!(revision(&fixture), initial + 1);
    adapter
        .save_binding(fixture.cwd(), 1, &changed_binding)
        .unwrap();
    assert_eq!(revision(&fixture), initial + 1);

    let discovered = super::super::tests::discovered(&binding, "I_catalog", "legacy shared copy");
    let inserted = adapter
        .insert_issue(fixture.cwd(), &discovered)
        .unwrap()
        .issue;
    let after_insert = revision(&fixture);
    adapter.insert_issue(fixture.cwd(), &discovered).unwrap();
    adapter
        .save_issue(fixture.cwd(), inserted.revision(), &inserted)
        .unwrap();
    assert_eq!(revision(&fixture), after_insert);
    let mut held = inserted;
    held.set_flag(IssueAutomationFlag::Held, true);
    adapter
        .save_issue(fixture.cwd(), held.revision(), &held)
        .unwrap();
    let after_update = revision(&fixture);
    assert_eq!(after_update, after_insert + 1);
    adapter
        .save_issue(fixture.cwd(), held.revision(), &held)
        .unwrap();
    assert_eq!(revision(&fixture), after_update);

    let state = adapter.ensure_collection(fixture.cwd()).unwrap();
    let after_control = revision(&fixture);
    assert_eq!(after_control, after_update + 1);
    adapter.ensure_collection(fixture.cwd()).unwrap();
    adapter
        .set_collection_mode(fixture.cwd(), Some(state.epoch), state.config.mode)
        .unwrap();
    assert_eq!(revision(&fixture), after_control);
}

#[test]
fn display_only_observation_refresh_rebinds_cas_without_a_new_semantic_assessment() {
    let fixture = Fixture::new("triage-display-refresh");
    let original = observation(&fixture);
    observe(&fixture, original.clone());
    let (fence, item) = subject(&fixture);
    let (first, _, _) = stored(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), fence, &assess(&item, None))
            .unwrap(),
    );
    let mut renamed = original;
    renamed.author.as_mut().unwrap().login = "renamed-display-login".into();
    observe(&fixture, renamed);
    let (new_fence, new_item) = subject(&fixture);
    assert!(new_item.triage.as_ref().unwrap().stale);
    let candidate = assess(&new_item, None);
    assert_eq!(candidate, first.assessment);
    let (refreshed, _, changed) = stored(
        fixture
            .adapter()
            .save_triage(fixture.cwd(), new_fence, &candidate)
            .unwrap(),
    );
    assert!(
        !changed,
        "a new observation CAS is not a new issue_triaged fact"
    );
    assert_eq!(refreshed.assessment, first.assessment);
    assert!(
        refreshed
            .assessment
            .gates()
            .iter()
            .all(|gate| gate.gate_revision == 1)
    );
    assert_eq!(
        refreshed.observation_revision,
        new_item.observation_revision
    );
    assert_eq!(refreshed.revision, first.revision + 1);
    assert!(!refreshed.stale);
}
