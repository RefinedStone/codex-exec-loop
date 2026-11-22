use std::sync::{Arc, Barrier};

use super::super::tests::{Fixture, reference, snapshot};
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget,
};
use crate::application::service::issue_automation_policy::resolve_issue_automation_policy;
use crate::application::service::issue_policy::issue_policy_version;
use crate::domain::issue_automation::collection::IssueCollectionQuery;
use crate::domain::issue_automation::collection::StoredIssueCollection;
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::{
    IssueDiscoverySelection, IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence,
    IssueRemoteState, IssueSourceActor, IssueSourceContent, IssueSourceDisclosure,
    IssueSourceObservation, IssueSourceRepository,
};
use crate::domain::issue_automation::triage::{IssueTriageInput, triage_issue};
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

const AGENT_POLICY: &str = r#"
[binding.github]
mode = "auto"
[authority]
autonomy_cap = "auto"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_author"}]
[gates.auto.normal]
start = "auto"
"#;

fn assess(item: &IssueCollectionItem, policy: Option<&str>) -> IssueTriageAssessment {
    let policy =
        resolve_issue_automation_policy(Some(policy.unwrap_or(AGENT_POLICY)), None).unwrap();
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

fn prepared(
    fixture: &Fixture,
) -> (
    IssueTriageFence,
    IssueTriageAssessment,
    IssueDecisionRequest,
) {
    observe(fixture, observation(fixture));
    let (fence, item) = subject(fixture);
    let assessment = assess(&item, None);
    fixture
        .adapter()
        .save_triage(fixture.cwd(), fence, &assessment)
        .unwrap();
    let (fence, item) = subject(fixture);
    let assessment = item.triage.unwrap().assessment;
    let request = request(&assessment, "decision-one");
    (fence, assessment, request)
}

fn shared(value: &str) -> IssueDecisionSharedText {
    IssueDecisionSharedText::new_shared(value).unwrap()
}

fn request(assessment: &IssueTriageAssessment, id: &str) -> IssueDecisionRequest {
    IssueDecisionRequest::new(
        IssueDecisionId::new(id).unwrap(),
        IssueDecisionDefinition {
            subject: IssueDecisionSubject::Issue(assessment.basis().issue.clone()),
            gate: IssueAutomationGate::Start,
            kind: IssueDecisionKind::StartApproval,
            question: shared("이 작업의 시작 결정을 기록할까요?"),
            options: vec![
                IssueDecisionOption {
                    id: "accept".into(),
                    label: shared("수락"),
                    effect: shared("시작 결정을 기록한다."),
                },
                IssueDecisionOption {
                    id: "reject".into(),
                    label: shared("거절"),
                    effect: shared("거절 결정을 기록한다."),
                },
            ],
            recommended: Some("accept".into()),
            basis: IssueDecisionBasis::from_assessment(
                assessment,
                IssueAutomationGate::Start,
                None,
            )
            .unwrap(),
            on_timeout: IssueDecisionTimeout::Wait,
            due_at: None,
            asked_by: "triage".into(),
            evidence: vec!["record:triage".into()],
        },
    )
    .unwrap()
}

fn answered(
    request: &IssueDecisionRequest,
    current: &IssueTriageAssessment,
    option: &str,
) -> IssueDecisionRequest {
    let basis =
        IssueDecisionBasis::from_assessment(current, IssueAutomationGate::Start, None).unwrap();
    request
        .answer_agent(
            &basis,
            IssueDecisionAgentAnswer {
                option: option.into(),
                note: Some(shared("공유된 판단")),
                caller: Some("Desktop".into()),
                channel: "native".into(),
                at: "2026-11-20T20:00:00Z".into(),
            },
        )
        .unwrap()
        .request
}

fn commit(result: IssueDecisionSaveResult) -> IssueDecisionCommit {
    match result {
        IssueDecisionSaveResult::Saved(commit) => *commit,
        IssueDecisionSaveResult::Conflict => panic!("expected committed decision"),
    }
}

#[test]
fn two_connections_keep_only_the_first_answer_and_one_catalog_change() {
    let fixture = Fixture::new("decision-race");
    let (fence, assessment, request) = prepared(&fixture);
    let initial = commit(
        fixture
            .adapter()
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
            .unwrap(),
    );
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for option in ["accept", "reject"] {
        let barrier = barrier.clone();
        let adapter = fixture.adapter();
        let workspace = fixture.cwd().to_string();
        let assessment = assessment.clone();
        let candidate = answered(&request, &assessment, option);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            commit(
                adapter
                    .save_issue_decision(
                        &workspace,
                        IssueDecisionFence {
                            triage: fence,
                            decision_revision: 1,
                        },
                        &assessment,
                        &candidate,
                    )
                    .unwrap(),
            )
        }));
    }
    barrier.wait();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.changed).count(), 1);
    assert_eq!(results[0].record, results[1].record);
    assert_eq!(results[0].record.revision, 2);
    assert_eq!(results[0].catalog_revision, initial.catalog_revision + 1);
    assert_eq!(results[0].catalog_revision, results[1].catalog_revision);
    let reopened = fixture
        .adapter()
        .get_issue_decision(fixture.cwd(), request.id())
        .unwrap()
        .unwrap();
    assert_eq!(reopened, results[0].record);
    assert_eq!(
        reopened.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Agent
    );
    assert!(reopened.request.answer().unwrap().confirmation.is_none());
    assert!(subject(&fixture).1.open_decisions.is_empty());
}

#[test]
fn repeated_definition_with_another_id_reuses_the_first_request_and_never_reopens() {
    let fixture = Fixture::new("decision-dedup");
    let (fence, assessment, first) = prepared(&fixture);
    let adapter = fixture.adapter();
    let initial = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &first)
            .unwrap(),
    );
    let another = IssueDecisionRequest::new(
        IssueDecisionId::new("decision-another").unwrap(),
        first.definition().clone(),
    )
    .unwrap();
    let repeated = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &another)
            .unwrap(),
    );
    assert!(!repeated.changed);
    assert_eq!(repeated.record, initial.record);
    assert_eq!(repeated.catalog_revision, initial.catalog_revision);
    assert_eq!(
        subject(&fixture).1.open_decisions,
        vec![initial.record.clone()]
    );
    let closed = commit(
        adapter
            .save_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: fence,
                    decision_revision: 1,
                },
                &assessment,
                &answered(&first, &assessment, "accept"),
            )
            .unwrap(),
    );
    let repeated = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &another)
            .unwrap(),
    );
    assert!(!repeated.changed);
    assert_eq!(repeated.record, closed.record);
    assert_eq!(repeated.catalog_revision, closed.catalog_revision);
    assert!(
        adapter
            .get_issue_decision(fixture.cwd(), another.id())
            .unwrap()
            .is_none()
    );
}

#[test]
fn source_changes_reject_the_old_fence_and_close_the_fresh_resolution_as_superseded() {
    let fixture = Fixture::new("decision-stale");
    let (fence, assessment, request) = prepared(&fixture);
    let adapter = fixture.adapter();
    commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
            .unwrap(),
    );
    let mut changed = observation(&fixture);
    if let IssueSourceContent::Shared(snapshot) = &mut changed.content {
        snapshot.basis.field_hash =
            crate::domain::issue_work::IssueFieldHash::of_fields("changed", "changed");
        snapshot.shared_copy.body.push_str("변경된 완료 기준");
    }
    observe(&fixture, changed);
    let stale = answered(&request, &assessment, "accept");
    assert_eq!(
        adapter
            .save_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: fence,
                    decision_revision: 1,
                },
                &assessment,
                &stale
            )
            .unwrap(),
        IssueDecisionSaveResult::Conflict
    );
    let (fresh_fence, item) = subject(&fixture);
    let current = assess(&item, None);
    let candidate = answered(&request, &current, "accept");
    assert_eq!(candidate.state(), IssueDecisionState::Superseded);
    let closed = commit(
        adapter
            .save_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: fresh_fence,
                    decision_revision: 1,
                },
                &current,
                &candidate,
            )
            .unwrap(),
    );
    assert!(closed.changed);
    assert!(closed.record.request.answer().is_none());
    assert_eq!(subject(&fixture).1.triage.unwrap().assessment, assessment);
}

#[test]
fn open_requests_survive_reopen_and_use_the_same_catalog_snapshot() {
    let fixture = Fixture::new("decision-restart");
    let (fence, assessment, request) = prepared(&fixture);
    let initial = commit(
        fixture
            .adapter()
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
            .unwrap(),
    );
    let reopened = fixture.adapter();
    let page = reopened
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(page.revision, initial.catalog_revision);
    assert_eq!(page.items[0].open_decisions, vec![initial.record.clone()]);
    assert_eq!(
        reopened
            .get_issue_decision(fixture.cwd(), request.id())
            .unwrap(),
        Some(initial.record)
    );
}

#[test]
fn a_new_request_requires_a_current_persisted_assessment() {
    let fixture = Fixture::new("decision-no-triage");
    observe(&fixture, observation(&fixture));
    let (fence, item) = subject(&fixture);
    let assessment = assess(&item, None);
    let request = request(&assessment, "not-persisted");
    assert_eq!(
        fixture
            .adapter()
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
            .unwrap(),
        IssueDecisionSaveResult::Conflict
    );
}

#[test]
fn schema_v3_migration_preserves_existing_catalog_assessment_and_source() {
    let fixture = Fixture::new("decision-migration-v3");
    let (_, _, _) = prepared(&fixture);
    let adapter = fixture.adapter();
    let before = adapter.read_triage_batch(fixture.cwd(), None, 100).unwrap();
    adapter
        .with_connection(fixture.cwd(), |connection, _guard| {
            connection
                .execute_batch("DROP TABLE issue_work_checkpoints; DROP TABLE issue_decision_requests; PRAGMA user_version = 3;")?;
            super::super::validate_schema_version(connection, 3)
        })
        .unwrap();
    let after = fixture
        .adapter()
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap();
    assert_eq!(after, before);
    fixture
        .adapter()
        .with_connection(fixture.cwd(), |connection, _guard| {
            assert_eq!(
                super::super::schema_identity(connection)?,
                (super::super::APPLICATION_ID, super::super::SCHEMA_VERSION)
            );
            super::super::validate_schema(connection)
        })
        .unwrap();
}

#[test]
fn forged_answer_for_a_changed_basis_is_rejected_without_closing_the_request() {
    let fixture = Fixture::new("decision-forged-answer");
    let (fence, assessment, request) = prepared(&fixture);
    let adapter = fixture.adapter();
    commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
            .unwrap(),
    );
    let (_, item) = subject(&fixture);
    let current = assess(&item, Some("[authority]\nautonomy_cap = \"auto\"\n"));
    let forged = answered(&request, &assessment, "accept");
    assert!(
        adapter
            .save_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: fence,
                    decision_revision: 1,
                },
                &current,
                &forged
            )
            .is_err()
    );
    assert_eq!(
        adapter
            .get_issue_decision(fixture.cwd(), request.id())
            .unwrap()
            .unwrap()
            .request
            .state(),
        IssueDecisionState::Open
    );
}

#[test]
fn distinct_questions_on_the_same_basis_remain_independent_after_the_first_answer() {
    let fixture = Fixture::new("decision-distinct-questions");
    let (fence, assessment, first) = prepared(&fixture);
    let adapter = fixture.adapter();
    let initial = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &first)
            .unwrap(),
    );
    let closed = commit(
        adapter
            .save_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: fence,
                    decision_revision: initial.record.revision,
                },
                &assessment,
                &answered(&first, &assessment, "accept"),
            )
            .unwrap(),
    );
    let mut other_definition = first.definition().clone();
    other_definition.question = shared("별개의 질문에 답을 기록할까요?");
    other_definition.options[0].effect = shared("두 번째 질문의 답만 기록한다.");
    let second = IssueDecisionRequest::new(
        IssueDecisionId::new("decision-second-question").unwrap(),
        other_definition,
    )
    .unwrap();
    assert_eq!(first.definition().basis, second.definition().basis);
    let opened = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &second)
            .unwrap(),
    );
    assert!(opened.changed);
    assert_eq!(opened.record.request, second);
    assert_eq!(opened.record.request.state(), IssueDecisionState::Open);
    assert_eq!(opened.catalog_revision, closed.catalog_revision + 1);
    assert_eq!(
        adapter
            .get_issue_decision(fixture.cwd(), first.id())
            .unwrap(),
        Some(closed.record)
    );
    assert_eq!(subject(&fixture).1.open_decisions, vec![opened.record]);
}

#[test]
fn the_store_rechecks_the_current_human_gate_before_persisting_an_agent_answer() {
    use crate::domain::issue_automation::policy::IssueAutomationGateAction;
    let fixture = Fixture::new("decision-human-store-boundary");
    observe(&fixture, observation(&fixture));
    let adapter = fixture.adapter();
    let (fence, item) = subject(&fixture);
    let human_policy = AGENT_POLICY.replace("mode = \"auto\"", "mode = \"propose\"");
    let assessment = assess(&item, Some(&human_policy));
    assert_eq!(
        assessment
            .gates()
            .iter()
            .find(|gate| gate.gate == IssueAutomationGate::Start)
            .unwrap()
            .action,
        IssueAutomationGateAction::Human,
    );
    adapter
        .save_triage(fixture.cwd(), fence, &assessment)
        .unwrap();
    let (fence, item) = subject(&fixture);
    let assessment = item.triage.unwrap().assessment;
    let request = request(&assessment, "human-store-decision");
    let initial = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
            .unwrap(),
    );
    let expected = IssueDecisionFence {
        triage: fence,
        decision_revision: initial.record.revision,
    };
    let attempted = answered(&request, &assessment, "accept");
    assert!(
        adapter
            .save_issue_decision(fixture.cwd(), expected, &assessment, &attempted)
            .is_err()
    );
    let after_rejected = adapter
        .get_issue_decision_commit(fixture.cwd(), request.id())
        .unwrap()
        .unwrap();
    assert_eq!(after_rejected.record, initial.record);
    assert_eq!(after_rejected.catalog_revision, initial.catalog_revision);
    let reference = format!("desktop-confirmation:{}", "c".repeat(32));
    let confirmed = request
        .answer_for_gate(
            &request.definition().basis,
            IssueAutomationGateAction::Human,
            IssueDecisionAgentAnswer {
                option: "accept".into(),
                note: None,
                caller: Some("Desktop".into()),
                channel: "gateway".into(),
                at: "2026-11-20T20:00:00Z".into(),
            },
            Some(&reference),
        )
        .unwrap();
    let saved = commit(
        adapter
            .save_issue_decision(fixture.cwd(), expected, &assessment, &confirmed.request)
            .unwrap(),
    );
    assert!(saved.changed);
    assert_eq!(saved.catalog_revision, initial.catalog_revision + 1);
    assert_eq!(
        saved.record.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
    assert_eq!(
        saved
            .record
            .request
            .answer()
            .unwrap()
            .confirmation
            .as_deref(),
        Some(reference.as_str())
    );
    assert_eq!(
        fixture
            .adapter()
            .get_issue_decision(fixture.cwd(), request.id())
            .unwrap()
            .unwrap(),
        saved.record
    );
}
