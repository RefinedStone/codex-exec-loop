use super::super::tests::{Fixture, discovered, snapshot as shared_snapshot, transition_and_save};
use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueExecutionSaveResult, IssueTriageFence, IssueTriageSaveResult,
};
use crate::application::service::issue_automation_policy::resolve_issue_automation_policy;
use crate::application::service::issue_policy::issue_policy_version;
use crate::domain::issue_automation::coordinator::{
    IssueWorkCheckpoint, IssueWorkOrder, issue_work_order_id,
};
use crate::domain::issue_automation::decision::{IssueDecisionBasis, IssueDecisionId};
use crate::domain::issue_automation::execution::{
    IssueExecutionIntegration, IssueExecutionPhase, IssueWorkExecutionObservation,
};
use crate::domain::issue_automation::feedback::*;
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_automation::triage::{IssueTriageInput, triage_issue};
use crate::domain::issue_automation::{
    IssueAutomationFlag, IssueAutomationState, IssueTransition, IssueTriageResult,
};
use crate::domain::issue_policy::IssuePolicy;
use crate::domain::issue_validation::IssueRequirements;
use rusqlite::{Connection, params};

mod admission;
mod binding_pause;
mod closure;
mod jira;
mod migration;
mod receipts;

const USER_POLICY: &str = "[feedback]\ncomment = true\nlabels = true\n";

fn seed(fixture: &Fixture, issue_id: &str) -> IssueReference {
    let adapter = fixture.adapter();
    let control = adapter.ensure_collection(fixture.cwd()).unwrap();
    let binding = fixture.binding("feedback-binding");
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    let tracked = adapter
        .insert_issue(
            fixture.cwd(),
            &discovered(&binding, issue_id, "공유된 작업 설명"),
        )
        .unwrap()
        .issue;
    let mut shared = shared_snapshot("## 완료 기준\n- 결과를 검증한다.\n");
    shared.policy_version = issue_policy_version(&IssuePolicy::builtin());
    let observed = IssueSourceObservation {
        content_problems: vec![],
        jira: None,
        linear: None,
        repository_visibility: Default::default(),
        repository_name_with_owner: None,
        issue: tracked.reference().clone(),
        web: None,
        remote_state: IssueRemoteState::Open,
        creation_provenance: IssueCreationProvenance::Unattributed,
        labels: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        assignees: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        author: Some(IssueSourceActor {
            id: "U_human".into(),
            login: "human".into(),
            kind: IssueSourceActorKind::UserAccount,
        }),
        label_actors: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Absent,
        content: IssueSourceContent::Shared(shared),
    };
    adapter
        .apply_issue_resync(
            fixture.cwd(),
            control.fence(),
            tracked.reference(),
            &IssueSourceRead::Observed(Box::new(observed)),
        )
        .unwrap();
    refresh_assessment(fixture, tracked.reference());
    tracked.reference().clone()
}

fn refresh_assessment(fixture: &Fixture, issue: &IssueReference) {
    let adapter = fixture.adapter();
    let batch = adapter.read_triage_batch(fixture.cwd(), None, 100).unwrap();
    let item = batch
        .items
        .into_iter()
        .find(|item| item.issue == *issue)
        .unwrap();
    let policy = resolve_issue_automation_policy(Some(USER_POLICY), None).unwrap();
    let autonomy = adapter.issue_autonomy_selection(fixture.cwd()).unwrap();
    let assessment = triage_issue(IssueTriageInput {
        issue,
        current: item.current.as_ref(),
        policy: &policy.policy,
        automation_policy_version: &policy.version,
        runtime_selection_revision: autonomy.revision,
        requirements: &IssueRequirements::default(),
        validation_policy_version: &issue_policy_version(&IssuePolicy::builtin()),
        held: item
            .tracked
            .as_ref()
            .is_some_and(|tracked| tracked.flags.held),
        pinned_source_commit: None,
        previous: item.triage.as_ref().map(|triage| &triage.assessment),
    })
    .unwrap();
    assert!(matches!(
        adapter
            .save_triage(
                fixture.cwd(),
                IssueTriageFence {
                    binding_run_revision: item.binding_run_revision,
                    collection_epoch: batch.collection.unwrap().epoch,
                    observation_revision: item.observation_revision,
                    tracked_revision: item.tracked_revision,
                    assessment_revision: item.triage.as_ref().map(|triage| triage.revision),
                },
                &assessment
            )
            .unwrap(),
        IssueTriageSaveResult::Saved { .. }
    ));
}

fn current(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
) -> IssueFeedbackSnapshot {
    adapter.snapshot(issue).unwrap().unwrap()
}

fn desired(body: &str) -> IssueFeedbackDesired {
    IssueFeedbackDesired {
        comment_body: body.into(),
        comment_body_sha256: feedback_body_hash(body),
    }
}

fn save_desired(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
    body: &str,
) -> IssueFeedbackState {
    match adapter
        .save_desired(issue, current(adapter, issue).fence(), &desired(body))
        .unwrap()
    {
        IssueFeedbackSaveResult::Saved(state) => *state,
        other => panic!("expected desired feedback, got {other:?}"),
    }
}

fn comment_intent(snapshot: &IssueFeedbackSnapshot, id: &str) -> IssueFeedbackIntent {
    let desired = snapshot.state.desired.as_ref().unwrap();
    let body = format!("{}\n\n{}", desired.comment_body, feedback_marker(id));
    let body_sha256 = feedback_body_hash(&body);
    let operation = match &snapshot.state.comment_owner {
        Some(owner) => IssueFeedbackOperation::UpdateComment {
            owner: Box::new(owner.clone()),
            body,
            body_sha256,
            content_sha256: desired.comment_body_sha256.clone(),
        },
        None => IssueFeedbackOperation::CreateComment {
            body,
            body_sha256,
            content_sha256: desired.comment_body_sha256.clone(),
        },
    };
    make_intent(snapshot, id, operation)
}

fn make_intent(
    snapshot: &IssueFeedbackSnapshot,
    id: &str,
    operation: IssueFeedbackOperation,
) -> IssueFeedbackIntent {
    let basis = snapshot.item.triage.as_ref().unwrap().assessment.basis();
    let intent = IssueFeedbackIntent {
        intent_id: id.into(),
        issue: snapshot.item.issue.clone(),
        actor_id: "U_feedback".into(),
        connection: None,
        automation_policy_version: basis.automation_policy_version.clone(),
        disclosure_policy_version: basis.disclosure_policy_version.clone().unwrap(),
        desired_revision: snapshot.state.desired_revision,
        operation,
        recorded_at: "2026-11-21T00:00:00Z".into(),
    };
    intent.validate().unwrap();
    intent
}

fn label() -> IssueSourceLabel {
    IssueSourceLabel {
        id: "LA_progress".into(),
        name: ISSUE_FEEDBACK_PROGRESS_LABEL.into(),
    }
}

fn recorded(
    adapter: &SqliteIssueAutomationStoreAdapter,
    intent: &IssueFeedbackIntent,
) -> IssueFeedbackIntentRecord {
    match adapter
        .record(current(adapter, &intent.issue).fence(), intent)
        .unwrap()
    {
        IssueFeedbackRecordResult::Created(record) => record,
        other => panic!("expected a recorded intent, got {other:?}"),
    }
}

fn sent(
    adapter: &SqliteIssueAutomationStoreAdapter,
    record: &IssueFeedbackIntentRecord,
) -> IssueFeedbackIntentRecord {
    match adapter
        .claim_sent(
            &record.intent.issue,
            &record.intent.intent_id,
            record.revision,
            current(adapter, &record.intent.issue).fence(),
        )
        .unwrap()
    {
        IssueFeedbackClaimResult::Claimed(record) => record,
        other => panic!("expected the sole send claim, got {other:?}"),
    }
}

fn comment_receipt(record: &IssueFeedbackIntentRecord, id: &str) -> IssueFeedbackReceipt {
    let body_sha256 = match &record.intent.operation {
        IssueFeedbackOperation::CreateComment { body_sha256, .. }
        | IssueFeedbackOperation::UpdateComment { body_sha256, .. } => body_sha256.clone(),
        _ => panic!("expected a comment intent"),
    };
    IssueFeedbackReceipt {
        issue: record.intent.issue.clone(),
        intent_id: record.intent.intent_id.clone(),
        actor_id: record.intent.actor_id.clone(),
        evidence: IssueFeedbackEvidence::Comment {
            comment_id: id.into(),
            body_sha256,
        },
    }
}

fn label_receipt(
    record: &IssueFeedbackIntentRecord,
    event_id: &str,
    applied: bool,
) -> IssueFeedbackReceipt {
    IssueFeedbackReceipt {
        issue: record.intent.issue.clone(),
        intent_id: record.intent.intent_id.clone(),
        actor_id: record.intent.actor_id.clone(),
        evidence: IssueFeedbackEvidence::Label {
            label: label(),
            event_id: event_id.into(),
            applied,
        },
    }
}

fn confirmed(
    adapter: &SqliteIssueAutomationStoreAdapter,
    record: &IssueFeedbackIntentRecord,
    receipt: IssueFeedbackReceipt,
) -> IssueFeedbackIntentRecord {
    adapter
        .complete(
            &record.intent.issue,
            &record.intent.intent_id,
            record.revision,
            &IssueFeedbackCompletion::Confirmed(Box::new(receipt)),
        )
        .unwrap()
        .unwrap()
}

fn existing_work(fixture: &Fixture, issue: &IssueReference) -> IssueWorkCheckpoint {
    let adapter = fixture.adapter();
    let tracked = adapter.get_issue(fixture.cwd(), issue).unwrap().unwrap();
    let tracked = transition_and_save(
        &adapter,
        fixture.cwd(),
        &tracked,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    transition_and_save(&adapter, fixture.cwd(), &tracked, IssueTransition::Queue);
    let snapshot = current(&adapter, issue);
    let Some(IssueSourceRead::Observed(observed)) = &snapshot.item.current else {
        panic!("missing source");
    };
    let IssueSourceContent::Shared(shared) = &observed.content else {
        panic!("missing shared copy");
    };
    let mut approved_basis = IssueDecisionBasis::from_assessment(
        &snapshot.item.triage.as_ref().unwrap().assessment,
        IssueAutomationGate::Start,
        None,
    )
    .unwrap();
    approved_basis.authorization_epoch = snapshot.authorization_epoch;
    let work = IssueWorkCheckpoint {
        delivery_assessment: None,
        delivery_revision: 0,
        authorization: None,
        permit_authorization_epoch: 0,
        order: IssueWorkOrder {
            work_order_id: issue_work_order_id(issue),
            issue: issue.clone(),
            field_hash: shared.basis.field_hash.clone(),
            title: shared.shared_copy.title.clone(),
            handoff: shared.shared_copy.clone(),
            acceptance: vec!["결과를 검증한다.".into()],
            scope: fixture.cwd().into(),
            branch_hint: None,
            link_reference: None,
            provider_link: false,
            profile: None,
            priority: 0,
            attempt: 1,
        },
        start_authority:
            crate::domain::issue_automation::coordinator::IssueWorkStartAuthority::Human,
        decision_id: Some(
            IssueDecisionId::new(format!("feedback-start-{}", issue.issue_id)).unwrap(),
        ),
        approved_basis,
        task_ids: vec![format!("task-{}", issue.issue_id)],
        permit_source: None,
        permit_revision: 0,
        delivery_granted: false,
        blocked_reason: None,
        revision: 1,
    };
    work.validate().unwrap();
    // A valid existing checkpoint models a pre-feedback execution. Its admission
    // is independent of the publication guarantees tested below.
    adapter.with_connection(fixture.cwd(), |connection, guard| {
        super::super::transaction(connection, guard, |tx| {
            tx.execute(
                "INSERT INTO issue_work_checkpoints
                 (workspace_dir,provider,server_id,repository_id,issue_id,work_order_id,checkpoint,revision)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,1)",
                params![fixture.cwd(), super::super::enum_name(&issue.provider)?, issue.server_id,
                    issue.repository_id, issue.issue_id, work.order.work_order_id, serde_json::to_string(&work)?],
            )?;
            Ok(())
        })
    }).unwrap();
    work
}

fn integrate(fixture: &Fixture, work: &IssueWorkCheckpoint) {
    assert!(matches!(
        fixture
            .adapter()
            .apply_issue_execution(
                fixture.cwd(),
                work,
                0,
                &IssueWorkExecutionObservation {
                    phase: IssueExecutionPhase::Integrated,
                    task_ids: work.task_ids.clone(),
                    execution_ids: work.task_ids.clone(),
                    integration: Some(IssueExecutionIntegration {
                        record_id: format!("integration-{}", work.order.issue.issue_id),
                        source_sha: "1".repeat(40),
                        integration_sha: "2".repeat(40),
                        pull_request_url: None,
                    }),
                    ..Default::default()
                },
                Some(3),
            )
            .unwrap(),
        IssueExecutionSaveResult::Saved { .. }
    ));
}

fn count(fixture: &Fixture, table: &str) -> i64 {
    Connection::open(fixture.database())
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
