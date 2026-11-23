use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueDecisionFence, IssueDecisionSaveResult, IssueTriageSaveResult,
};
use crate::application::service::issue_automation_policy::resolve_issue_automation_policy;
use crate::application::service::issue_policy::issue_policy_version;
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};
use crate::domain::issue_automation::coordinator::{
    IssueWorkOrder, issue_work_order_id, start_decision_id,
};
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::policy::{IssueAutomationGate, IssueAutomationGateAction};
use crate::domain::issue_automation::source::*;
use crate::domain::issue_automation::triage::{
    IssueTriageInput, issue_acceptance_criteria, triage_issue,
};
use crate::domain::issue_policy::IssuePolicy;
use crate::domain::issue_validation::IssueRequirements;

#[derive(Clone)]
pub(super) struct Prepared {
    pub fence: IssueTriageFence,
    pub assessment: IssueTriageAssessment,
    pub work: IssueWorkCheckpoint,
    pub observation: IssueSourceObservation,
}

pub(super) fn budget(running: u32, daily: u32) -> IssueAutomationBudget {
    IssueAutomationBudget {
        max_running: running,
        max_started_per_day: daily,
        max_attempts: 3,
    }
}

pub(super) fn prepare(
    fixture: &Fixture,
    id: &str,
    authority: IssueWorkStartAuthority,
    budget: &IssueAutomationBudget,
) -> Prepared {
    prepare_with_default_answer(fixture, id, authority, budget, true)
}

pub(super) fn prepare_unanswered_default(
    fixture: &Fixture,
    id: &str,
    budget: &IssueAutomationBudget,
) -> Prepared {
    prepare_with_default_answer(
        fixture,
        id,
        IssueWorkStartAuthority::TimeoutDefault,
        budget,
        false,
    )
}

pub(super) fn prepare_unanswered_arbiter(
    fixture: &Fixture,
    id: &str,
    budget: &IssueAutomationBudget,
) -> Prepared {
    prepare_with_default_answer(
        fixture,
        id,
        IssueWorkStartAuthority::TimeoutArbiter,
        budget,
        false,
    )
}

fn prepare_with_default_answer(
    fixture: &Fixture,
    id: &str,
    authority: IssueWorkStartAuthority,
    budget: &IssueAutomationBudget,
    answer_default: bool,
) -> Prepared {
    let adapter = fixture.adapter();
    let control = adapter.ensure_collection(fixture.cwd()).unwrap();
    let mut selection = adapter.issue_autonomy_selection(fixture.cwd()).unwrap();
    if selection.requested_level.is_none() {
        let intent = IssueAutonomyIntent {
            command_id: "fixture-native-selection".into(),
            level: if authority == IssueWorkStartAuthority::Human {
                IssueAutonomyLevel::Propose
            } else {
                IssueAutonomyLevel::Gated
            },
            expected_revision: selection.revision,
            policy_version: "a".repeat(64),
        };
        adapter
            .begin_issue_autonomy(fixture.cwd(), &intent)
            .unwrap();
        adapter
            .finish_issue_autonomy(fixture.cwd(), &intent)
            .unwrap();
        selection = adapter.issue_autonomy_selection(fixture.cwd()).unwrap();
    }
    let binding = fixture.binding("admission-binding");
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    let tracked = adapter
        .insert_issue(
            fixture.cwd(),
            &super::super::super::tests::discovered(
                &binding,
                id,
                "## 완료 기준\n- 결과를 검증한다.\n",
            ),
        )
        .unwrap()
        .issue;
    let mut snapshot = tracked.snapshot().clone();
    snapshot.policy_version = issue_policy_version(&IssuePolicy::builtin());
    let observation = IssueSourceObservation {
        content_problems: vec![],
        jira: None,
        repository_visibility: Default::default(),
        repository_name_with_owner: None,
        issue: tracked.reference().clone(),
        web: None,
        remote_state: IssueRemoteState::Open,
        creation_provenance: IssueCreationProvenance::Unattributed,
        labels: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        assignees: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        author: Some(IssueSourceActor {
            kind: IssueSourceActorKind::UserAccount,
            id: "U_author".into(),
            login: "author".into(),
        }),
        label_actors: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Absent,
        content: IssueSourceContent::Shared(snapshot),
    };
    adapter
        .apply_issue_resync(
            fixture.cwd(),
            control.fence(),
            tracked.reference(),
            &IssueSourceRead::Observed(Box::new(observation.clone())),
        )
        .unwrap();
    let tracked = adapter
        .get_issue(fixture.cwd(), tracked.reference())
        .unwrap()
        .unwrap();
    transition_and_save(
        &adapter,
        fixture.cwd(),
        &tracked,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    let (fence, item) = subject(fixture, tracked.reference());
    let mode = if authority == IssueWorkStartAuthority::Human {
        "propose"
    } else {
        "gated"
    };
    let start_gate = match authority {
        IssueWorkStartAuthority::TimeoutDefault => "timeout_default",
        IssueWorkStartAuthority::TimeoutArbiter => "timeout_arbiter",
        _ => "auto",
    };
    let policy = resolve_issue_automation_policy(Some(&format!(
        "[binding.github]\nmode = \"{mode}\"\n[authority]\nautonomy_cap = \"gated\"\n[select]\nmode = \"all_open\"\ntrusted_actors = [{{provider=\"github\",server_id=\"github.com\",actor_id=\"U_author\"}}]\n[gates.gated.normal]\nstart = \"{start_gate}\"\n[budget]\nmax_running={}\nmax_started_per_day={}\nmax_attempts={}\n",
        budget.max_running, budget.max_started_per_day, budget.max_attempts,
    )), None).unwrap();
    let assessment = triage_issue(IssueTriageInput {
        runtime_selection_revision: selection.revision,
        issue: &item.issue,
        current: item.current.as_ref(),
        policy: &policy.policy,
        automation_policy_version: &policy.version,
        requirements: &IssueRequirements::default(),
        validation_policy_version: &issue_policy_version(&IssuePolicy::builtin()),
        held: false,
        pinned_source_commit: None,
        previous: item.triage.as_ref().map(|triage| &triage.assessment),
    })
    .unwrap();
    assert!(matches!(
        adapter
            .save_triage(fixture.cwd(), fence, &assessment)
            .unwrap(),
        IssueTriageSaveResult::Saved { .. }
    ));
    let (fence, _) = subject(fixture, &item.issue);
    let approved_basis =
        IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None).unwrap();
    let decision_id = if authority != IssueWorkStartAuthority::Policy {
        let id = start_decision_id(&item.issue, &approved_basis);
        let is_default = authority == IssueWorkStartAuthority::TimeoutDefault;
        let is_arbiter = authority == IssueWorkStartAuthority::TimeoutArbiter;
        let shared = |text: &str| IssueDecisionSharedText::new_shared(text).unwrap();
        let request = IssueDecisionRequest::new(
            id.clone(),
            IssueDecisionDefinition {
                delivery_review: None,
                subject: IssueDecisionSubject::Issue(item.issue.clone()),
                gate: IssueAutomationGate::Start,
                kind: IssueDecisionKind::StartApproval,
                question: shared("작업을 시작할까요?"),
                options: vec![IssueDecisionOption {
                    id: "start".into(),
                    label: shared("시작"),
                    effect: shared("작업을 시작한다."),
                }],
                recommended: is_default.then(|| "start".into()),
                basis: approved_basis.clone(),
                on_timeout: if is_default {
                    IssueDecisionTimeout::Default
                } else if is_arbiter {
                    IssueDecisionTimeout::Arbiter
                } else {
                    IssueDecisionTimeout::Wait
                },
                due_at: (is_default || is_arbiter).then(|| "1970-01-01T00:00:01Z".into()),
                asked_by: "fixture".into(),
                evidence: vec![],
            },
        )
        .unwrap();
        assert!(matches!(
            adapter
                .ensure_issue_decision(fixture.cwd(), fence, &assessment, &request)
                .unwrap(),
            IssueDecisionSaveResult::Saved(_)
        ));
        if is_default {
            if answer_default {
                assert!(matches!(
                    adapter
                        .timeout_issue_decision(
                            fixture.cwd(),
                            IssueDecisionFence {
                                triage: fence,
                                decision_revision: 1,
                            },
                            &assessment,
                            &id,
                            1_000,
                        )
                        .unwrap(),
                    IssueDecisionSaveResult::Saved(commit)
                        if commit.changed
                            && commit.record.request.answer().is_some_and(|answer|
                                answer.by == IssueDecisionAnswerBy::Default)
                ));
            }
        } else if !is_arbiter {
            let answered = request
                .answer_for_gate(
                    &approved_basis,
                    IssueAutomationGateAction::Human,
                    IssueDecisionAgentAnswer {
                        option: "start".into(),
                        note: None,
                        caller: Some("Desktop".into()),
                        channel: "native".into(),
                        at: "2026-11-22T00:00:00Z".into(),
                    },
                    Some("desktop-confirmation:0123456789abcdef0123456789abcdef"),
                )
                .unwrap()
                .request;
            assert!(matches!(
                adapter
                    .save_issue_decision(
                        fixture.cwd(),
                        IssueDecisionFence {
                            triage: fence,
                            decision_revision: 1
                        },
                        &assessment,
                        &answered,
                    )
                    .unwrap(),
                IssueDecisionSaveResult::Saved(_)
            ));
        }
        Some(id)
    } else {
        None
    };
    let snapshot = match &observation.content {
        IssueSourceContent::Shared(snapshot) => snapshot,
        _ => unreachable!(),
    };
    let work = IssueWorkCheckpoint {
        delivery_assessment: None,
        delivery_revision: 0,
        authorization: None,
        permit_authorization_epoch: 0,
        order: IssueWorkOrder {
            work_order_id: issue_work_order_id(&item.issue),
            issue: item.issue.clone(),
            field_hash: snapshot.basis.field_hash.clone(),
            title: snapshot.shared_copy.title.clone(),
            handoff: snapshot.shared_copy.clone(),
            acceptance: issue_acceptance_criteria(&snapshot.shared_copy.body).unwrap(),
            scope: fixture.cwd().into(),
            branch_hint: None,
            link_reference: None,
            provider_link: false,
            profile: None,
            priority: 0,
            attempt: 1,
        },
        start_authority: authority,
        decision_id,
        approved_basis,
        task_ids: vec![],
        permit_source: None,
        permit_revision: 0,
        delivery_granted: false,
        blocked_reason: None,
        revision: 0,
    };
    work.validate().unwrap();
    Prepared {
        fence,
        assessment,
        work,
        observation,
    }
}

pub(super) fn subject(
    fixture: &Fixture,
    issue: &crate::domain::issue_work::IssueReference,
) -> (
    IssueTriageFence,
    crate::domain::issue_automation::collection::IssueCollectionItem,
) {
    let batch = fixture
        .adapter()
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap();
    let item = batch
        .items
        .into_iter()
        .find(|item| item.issue == *issue)
        .unwrap();
    let fence = IssueTriageFence {
        binding_run_revision: item.binding_run_revision,
        collection_epoch: batch.collection.unwrap().epoch,
        observation_revision: item.observation_revision,
        tracked_revision: item.tracked_revision,
        assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
    };
    (fence, item)
}

pub(super) fn run(
    fixture: &Fixture,
    prepared: &Prepared,
    budget: &IssueAutomationBudget,
    now: u64,
) -> IssueWorkAdmissionResult {
    fixture
        .adapter()
        .admit_issue_work(
            fixture.cwd(),
            prepared.fence,
            &prepared.assessment,
            &prepared.work,
            budget,
            now,
        )
        .unwrap()
}

pub(super) fn saved(result: IssueWorkAdmissionResult) -> IssueWorkCheckpoint {
    match result {
        IssueWorkAdmissionResult::Saved { record, .. } => *record,
        other => panic!("expected admission, got {other:?}"),
    }
}

pub(super) fn count(fixture: &Fixture, table: &str) -> i64 {
    Connection::open(fixture.database())
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

pub(super) fn transition(fixture: &Fixture, prepared: &Prepared, transition: IssueTransition) {
    let adapter = fixture.adapter();
    let tracked = adapter
        .get_issue(fixture.cwd(), &prepared.work.order.issue)
        .unwrap()
        .unwrap();
    transition_and_save(&adapter, fixture.cwd(), &tracked, transition);
}
