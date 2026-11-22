use super::super::tests::{Fixture, transition_and_save};
use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueExecutionSaveResult, IssueTriageFence, IssueTriageSaveResult,
};
use crate::application::service::issue_automation_policy::resolve_issue_automation_policy;
use crate::application::service::issue_policy::issue_policy_version;
use crate::domain::issue_automation::coordinator::{
    IssueWorkCheckpoint, IssueWorkOrder, issue_work_order_id,
};
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::execution::{
    IssueExecutionPhase, IssueExecutionQuestion, IssueWorkExecutionObservation,
};
use crate::domain::issue_automation::execution_answer::answer_command_id;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest,
};
use crate::domain::issue_automation::policy::IssueAutomationGateAction;
use crate::domain::issue_automation::source::{
    IssueCreationProvenance, IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence,
    IssueRemoteState, IssueSourceActor, IssueSourceActorKind, IssueSourceContent,
    IssueSourceObservation, IssueSourceRead,
};
use crate::domain::issue_automation::triage::{IssueTriageInput, triage_issue};
use crate::domain::issue_automation::{IssueAutomationFlag, IssueTransition, IssueTriageResult};
use crate::domain::issue_policy::IssuePolicy;
use crate::domain::issue_validation::IssueRequirements;
use std::sync::{Arc, Barrier};

const ANSWER: &str = "공유 정책을 통과한 사람의 실제 답변";
const CONFIRMATION: &str = "desktop-confirmation:0123456789abcdef0123456789abcdef";

struct Prepared {
    work: IssueWorkCheckpoint,
    intent: IssueExecutionAnswerIntent,
    request: IssueDecisionRequest,
}

fn shared(value: &str) -> IssueDecisionSharedText {
    IssueDecisionSharedText::new_shared(value).unwrap()
}

fn seed(fixture: &Fixture) -> Prepared {
    seed_issue(fixture, "I_1")
}

fn seed_issue(fixture: &Fixture, issue_id: &str) -> Prepared {
    let adapter = fixture.adapter();
    let control = adapter.ensure_collection(fixture.cwd()).unwrap();
    let binding = fixture.binding("binding-1");
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    let discovered = super::super::tests::discovered(&binding, issue_id, "masked body");
    let tracked = adapter
        .insert_issue(fixture.cwd(), &discovered)
        .unwrap()
        .issue;
    let task_id = if issue_id == "I_1" {
        "task-1"
    } else {
        "task-2"
    };
    let mut snapshot = tracked.snapshot().clone();
    snapshot.policy_version = issue_policy_version(&IssuePolicy::builtin());
    let source = IssueSourceRead::Observed(Box::new(IssueSourceObservation {
        creation_provenance: IssueCreationProvenance::Unattributed,
        issue: tracked.reference().clone(),
        web: None,
        remote_state: IssueRemoteState::Open,
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
            id: "U_human".into(),
            login: "human".into(),
        }),
        label_actors: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Absent,
        content: IssueSourceContent::Shared(snapshot),
    }));
    adapter
        .apply_issue_resync(fixture.cwd(), control.fence(), tracked.reference(), &source)
        .unwrap();
    let tracked = adapter
        .get_issue(fixture.cwd(), tracked.reference())
        .unwrap()
        .unwrap();
    let tracked = transition_and_save(
        &adapter,
        fixture.cwd(),
        &tracked,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    let tracked = transition_and_save(&adapter, fixture.cwd(), &tracked, IssueTransition::Queue);
    let batch = adapter.read_triage_batch(fixture.cwd(), None, 10).unwrap();
    let item = batch
        .items
        .into_iter()
        .find(|item| item.issue == *tracked.reference())
        .unwrap();
    let policy = resolve_issue_automation_policy(None, None).unwrap();
    let assessment = triage_issue(IssueTriageInput {
        issue: &item.issue,
        current: item.current.as_ref(),
        policy: &policy.policy,
        automation_policy_version: &policy.version,
        runtime_selection_revision: 0,
        requirements: &IssueRequirements::default(),
        validation_policy_version: &issue_policy_version(&IssuePolicy::builtin()),
        held: false,
        pinned_source_commit: None,
        previous: None,
    })
    .unwrap();
    let saved = adapter
        .save_triage(
            fixture.cwd(),
            IssueTriageFence {
                collection_epoch: batch.collection.unwrap().epoch,
                observation_revision: item.observation_revision,
                tracked_revision: item.tracked_revision,
                assessment_revision: None,
            },
            &assessment,
        )
        .unwrap();
    assert!(matches!(saved, IssueTriageSaveResult::Saved { .. }));
    let issue = tracked.reference().clone();
    let handoff = tracked.snapshot().shared_copy.clone();
    let work = IssueWorkCheckpoint {
        authorization: None,
        permit_authorization_epoch: 0,
        order: IssueWorkOrder {
            work_order_id: issue_work_order_id(&issue),
            issue: issue.clone(),
            field_hash: tracked.snapshot().basis.field_hash.clone(),
            title: handoff.title.clone(),
            handoff,
            acceptance: vec!["기존 작업을 검증한다.".into()],
            scope: fixture.cwd().into(),
            branch_hint: None,
            link_reference: None,
            profile: None,
            priority: 0,
            attempt: 1,
        },
        decision_id: IssueDecisionId::new(format!("answer-fixture-start-{issue_id}")).unwrap(),
        approved_basis: IssueDecisionBasis::from_assessment(
            &assessment,
            IssueAutomationGate::Start,
            None,
        )
        .unwrap(),
        task_ids: vec![task_id.into()],
        permit_source: None,
        permit_revision: 0,
        delivery_granted: false,
        blocked_reason: None,
        revision: 1,
    };
    work.validate().unwrap();
    adapter.with_connection(fixture.cwd(), |connection, guard| {
        transaction(connection, guard, |tx| {
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
    let question = IssueExecutionQuestion {
        question_id: "question-1".into(),
        task_id: task_id.into(),
        session_key: "isolated-session-1".into(),
        thread_id: "thread-original".into(),
        slot_id: "slot-1".into(),
        worktree_path: fixture.root.join("isolated").to_string_lossy().into_owned(),
        branch_name: "akra/question-1".into(),
        lease_generation: "generation-old".into(),
        awaiting_task_updated_at: "2026-11-21T00:00:00Z".into(),
    };
    let observed = IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Running,
        task_ids: work.task_ids.clone(),
        active_task_id: Some(question.task_id.clone()),
        execution_ids: vec![question.task_id.clone()],
        question: Some(question.clone()),
        ..Default::default()
    };
    assert!(matches!(
        adapter
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(3))
            .unwrap(),
        IssueExecutionSaveResult::Saved { .. }
    ));
    let definition = IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(issue.clone()),
        gate: IssueAutomationGate::Spec,
        kind: IssueDecisionKind::AgentQuestion,
        question: shared("원래 작업 대화를 확인하고 답변을 입력해 주세요."),
        options: vec![
            IssueDecisionOption {
                id: "reply".into(),
                label: shared("답변"),
                effect: shared("같은 작업 대화에 한 번 전달한다."),
            },
            IssueDecisionOption {
                id: "wait".into(),
                label: shared("대기"),
                effect: shared("작업을 계속 대기시킨다."),
            },
        ],
        recommended: None,
        basis: IssueDecisionBasis::from_assessment(
            &assessment,
            IssueAutomationGate::Spec,
            Some(work.order.work_order_id.clone()),
        )
        .unwrap(),
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "issue-coordinator".into(),
        evidence: vec![format!("question:{}", question.question_id)],
    };
    let open = IssueDecisionRequest::new(
        IssueDecisionId::new(format!("answer-fixture-question-{issue_id}")).unwrap(),
        definition,
    )
    .unwrap();
    let request = open
        .answer_for_gate(
            &open.definition().basis,
            IssueAutomationGateAction::Human,
            IssueDecisionAgentAnswer {
                option: "reply".into(),
                note: Some(shared(ANSWER)),
                caller: Some("Desktop".into()),
                channel: "native".into(),
                at: "2026-11-21T00:01:00Z".into(),
            },
            Some(CONFIRMATION),
        )
        .unwrap()
        .request;
    store_request(fixture, &request);
    let intent = IssueExecutionAnswerIntent {
        command_id: answer_command_id(&work.order.work_order_id, request.id()),
        issue,
        work_order_id: work.order.work_order_id.clone(),
        decision_id: request.id().clone(),
        question,
        authorization_epoch: 0,
        answer_sha256: answer_hash(ANSWER),
    };
    intent.validate().unwrap();
    Prepared {
        work,
        intent,
        request,
    }
}

fn store_request(fixture: &Fixture, request: &IssueDecisionRequest) {
    let adapter = fixture.adapter();
    adapter.with_connection(fixture.cwd(), |connection, guard| {
        transaction(connection, guard, |tx| {
            let issue = request.issue();
            let tracked = super::super::read_issue(tx, fixture.cwd(), issue)?.unwrap();
            tx.execute(
                "INSERT INTO issue_decision_requests
                 (decision_id,workspace_dir,provider,server_id,repository_id,issue_id,binding_id,uniqueness_key,state,request,revision)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,1)
                 ON CONFLICT(workspace_dir,decision_id) DO UPDATE SET
                    uniqueness_key=excluded.uniqueness_key,state=excluded.state,request=excluded.request",
                params![
                    request.id().as_str(), fixture.cwd(), super::super::enum_name(&issue.provider)?,
                    issue.server_id, issue.repository_id, issue.issue_id, tracked.binding_id().as_str(),
                    request.uniqueness_key(), super::super::enum_name(&request.state())?,
                    serde_json::to_string(&request.to_record())?,
                ],
            )?;
            Ok(())
        })
    }).unwrap();
}

fn pending_records(
    fixture: &Fixture,
    work: &IssueWorkCheckpoint,
) -> Vec<IssueExecutionAnswerRecord> {
    fixture
        .adapter()
        .pending_issue_execution_answers(fixture.cwd(), &work.order.work_order_id)
        .unwrap()
}

fn completed_intervention(
    fixture: &Fixture,
    intent: &IssueExecutionAnswerIntent,
    action: IssueInterventionAction,
) {
    let adapter = fixture.adapter();
    let request = IssueInterventionRequest {
        command_id: "answer-fixture-intervention".into(),
        issue: intent.issue.clone(),
        action,
    };
    adapter
        .begin_issue_intervention(fixture.cwd(), &request)
        .unwrap();
    adapter
        .finish_issue_intervention(
            fixture.cwd(),
            &IssueInterventionReceipt {
                command_id: request.command_id.clone(),
                issue: request.issue.clone(),
                action: request.action.label().into(),
                work_order_id: Some(intent.work_order_id.clone()),
                authorization_epoch: 1,
                changed: true,
                delivery: None,
                handoff: None,
            },
        )
        .unwrap();
}

#[test]
fn first_claim_is_durable_and_replay_never_regrants_delivery_or_stores_the_answer() {
    let fixture = Fixture::new("answer-replay");
    let prepared = seed(&fixture);
    let adapter = fixture.adapter();
    let before = adapter
        .get_issue_execution(fixture.cwd(), &prepared.intent.issue)
        .unwrap();
    let first = adapter
        .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
        .unwrap();
    assert!(first.created);
    assert_eq!(first.record.status, IssueExecutionAnswerStatus::Unknown);
    assert_eq!(first.record.revision, 1);
    assert_eq!(
        pending_records(&fixture, &prepared.work),
        [first.record.clone()]
    );
    let encoded: String = adapter
        .with_connection(fixture.cwd(), |connection, _| {
            Ok(
                connection.query_row("SELECT record FROM issue_execution_answers", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .unwrap();
    assert!(!encoded.contains(ANSWER));
    assert!(!encoded.contains("answer-fixture-start"));
    completed_intervention(&fixture, &prepared.intent, IssueInterventionAction::Hold);
    let replay = fixture
        .adapter()
        .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
        .unwrap();
    assert!(!replay.created);
    assert_eq!(replay.record, first.record);
    assert_eq!(
        adapter
            .get_issue_execution(fixture.cwd(), &prepared.intent.issue)
            .unwrap(),
        before
    );
    let mut reused = prepared.intent;
    reused.answer_sha256 = answer_hash("different shared reply");
    assert!(
        adapter
            .claim_issue_execution_answer(fixture.cwd(), &reused)
            .is_err()
    );
}

#[test]
fn two_connections_grant_only_one_initial_answer_delivery() {
    let fixture = Fixture::new("answer-claim-race");
    let prepared = seed(&fixture);
    let barrier = Arc::new(Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let adapter = fixture.adapter();
            let workspace = fixture.cwd().to_owned();
            let intent = prepared.intent.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter
                    .claim_issue_execution_answer(&workspace, &intent)
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|claim| claim.created).count(), 1);
    assert_eq!(results[0].record, results[1].record);
    assert_eq!(pending_records(&fixture, &prepared.work).len(), 1);
}

#[test]
fn receipt_cas_preserves_terminal_ack_and_rejects_invalid_thread_or_generation() {
    let fixture = Fixture::new("answer-ack");
    let prepared = seed(&fixture);
    let adapter = fixture.adapter();
    adapter
        .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
        .unwrap();
    let queued = adapter
        .save_issue_execution_answer(
            fixture.cwd(),
            &prepared.intent.command_id,
            1,
            &IssueExecutionAnswerStatus::Queued,
        )
        .unwrap()
        .unwrap();
    assert_eq!(queued.revision, 2);
    let sending = adapter
        .save_issue_execution_answer(
            fixture.cwd(),
            &prepared.intent.command_id,
            2,
            &IssueExecutionAnswerStatus::Unknown,
        )
        .unwrap()
        .unwrap();
    assert_eq!(sending.revision, 3);
    assert_eq!(pending_records(&fixture, &prepared.work), [sending]);
    let accepted = IssueExecutionAnswerStatus::Accepted {
        thread_id: prepared.intent.question.thread_id.clone(),
        turn_id: "turn-ack".into(),
        lease_generation: "generation-new".into(),
    };
    assert!(
        adapter
            .save_issue_execution_answer(fixture.cwd(), &prepared.intent.command_id, 1, &accepted)
            .unwrap()
            .is_none()
    );
    for invalid in [
        IssueExecutionAnswerStatus::Accepted {
            thread_id: "another-thread".into(),
            turn_id: "turn-ack".into(),
            lease_generation: "generation-new".into(),
        },
        IssueExecutionAnswerStatus::Accepted {
            thread_id: prepared.intent.question.thread_id.clone(),
            turn_id: "turn-ack".into(),
            lease_generation: prepared.intent.question.lease_generation.clone(),
        },
    ] {
        assert!(
            adapter
                .save_issue_execution_answer(
                    fixture.cwd(),
                    &prepared.intent.command_id,
                    2,
                    &invalid
                )
                .is_err()
        );
    }
    let final_record = adapter
        .save_issue_execution_answer(fixture.cwd(), &prepared.intent.command_id, 3, &accepted)
        .unwrap()
        .unwrap();
    assert_eq!(final_record.revision, 4);
    assert!(pending_records(&fixture, &prepared.work).is_empty());
    assert_eq!(
        adapter
            .save_issue_execution_answer(fixture.cwd(), &prepared.intent.command_id, 1, &accepted)
            .unwrap(),
        Some(final_record.clone())
    );
    assert!(
        adapter
            .save_issue_execution_answer(
                fixture.cwd(),
                &prepared.intent.command_id,
                4,
                &IssueExecutionAnswerStatus::RejectedBeforeStart {
                    reason: "cancelled".into()
                }
            )
            .is_err()
    );
    assert_eq!(
        adapter
            .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
            .unwrap()
            .record,
        final_record
    );
}

#[test]
fn concurrent_ack_and_definite_rejection_commit_only_one_terminal_receipt() {
    let fixture = Fixture::new("answer-terminal-race");
    let prepared = seed(&fixture);
    fixture
        .adapter()
        .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let statuses = [
        IssueExecutionAnswerStatus::Accepted {
            thread_id: prepared.intent.question.thread_id.clone(),
            turn_id: "turn-race".into(),
            lease_generation: "generation-new".into(),
        },
        IssueExecutionAnswerStatus::RejectedBeforeStart {
            reason: "cancelled".into(),
        },
    ];
    let handles = statuses
        .into_iter()
        .map(|status| {
            let adapter = fixture.adapter();
            let workspace = fixture.cwd().to_owned();
            let command_id = prepared.intent.command_id.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter
                    .save_issue_execution_answer(&workspace, &command_id, 1, &status)
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let committed = handles
        .into_iter()
        .filter_map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(committed.len(), 1);
    assert!(!committed[0].status.pending());
    assert!(pending_records(&fixture, &prepared.work).is_empty());
}

#[test]
fn human_reply_kind_evidence_basis_and_note_must_all_match() {
    for case in [
        "agent",
        "wait",
        "no_note",
        "wrong_note",
        "wrong_kind",
        "wrong_evidence",
        "wrong_gate",
        "wrong_epoch",
        "wrong_hash",
    ] {
        let fixture = Fixture::new(case);
        let prepared = seed(&fixture);
        let mut record = prepared.request.to_record();
        match case {
            "agent" => {
                let answer = record.answer.as_mut().unwrap();
                answer.by = IssueDecisionAnswerBy::Agent;
                answer.confirmation = None;
            }
            "wait" => record.answer.as_mut().unwrap().option = "wait".into(),
            "no_note" => record.answer.as_mut().unwrap().note = None,
            "wrong_note" => record.answer.as_mut().unwrap().note = Some(shared("다른 답변")),
            "wrong_kind" => record.definition.kind = IssueDecisionKind::SpecConfirmation,
            "wrong_evidence" => record.definition.evidence = vec!["question:another".into()],
            "wrong_gate" => record.definition.basis.gate_revision += 1,
            "wrong_epoch" => record.definition.basis.authorization_epoch += 1,
            "wrong_hash" => {
                record.definition.basis.field_hash.as_mut().unwrap().sha256 = "b".repeat(64)
            }
            _ => unreachable!(),
        }
        let request = IssueDecisionRequest::hydrate(record).unwrap();
        store_request(&fixture, &request);
        assert!(
            fixture
                .adapter()
                .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
                .is_err(),
            "{case}"
        );
        assert!(
            pending_records(&fixture, &prepared.work).is_empty(),
            "{case}"
        );
    }
}

#[test]
fn full_question_coordinates_and_work_lineage_are_fenced_before_claim() {
    for case in [
        "thread",
        "generation",
        "task",
        "session",
        "slot",
        "cwd",
        "branch",
        "timestamp",
    ] {
        let fixture = Fixture::new(case);
        let prepared = seed(&fixture);
        let mut intent = prepared.intent;
        match case {
            "thread" => intent.question.thread_id.push_str("-changed"),
            "generation" => intent.question.lease_generation.push_str("-changed"),
            "task" => intent.question.task_id = "not-in-lineage".into(),
            "session" => intent.question.session_key.push_str("-changed"),
            "slot" => intent.question.slot_id.push_str("-changed"),
            "cwd" => intent.question.worktree_path.push_str("-changed"),
            "branch" => intent.question.branch_name.push_str("-changed"),
            "timestamp" => intent
                .question
                .awaiting_task_updated_at
                .push_str("-changed"),
            _ => unreachable!(),
        }
        assert!(
            fixture
                .adapter()
                .claim_issue_execution_answer(fixture.cwd(), &intent)
                .is_err(),
            "{case}"
        );
        assert!(
            pending_records(&fixture, &prepared.work).is_empty(),
            "{case}"
        );
    }
}

#[test]
fn held_stale_unknown_untracked_cancelled_and_pending_controls_refuse_new_claims() {
    for case in [
        "held",
        "stale",
        "unknown",
        "untracked",
        "cancelled",
        "taken_over",
        "pending",
        "autonomy",
    ] {
        let fixture = Fixture::new(case);
        let prepared = seed(&fixture);
        let adapter = fixture.adapter();
        match case {
            "held" | "stale" | "unknown" => {
                let mut tracked = adapter
                    .get_issue(fixture.cwd(), &prepared.intent.issue)
                    .unwrap()
                    .unwrap();
                let revision = tracked.revision();
                let flag = match case {
                    "held" => IssueAutomationFlag::Held,
                    "stale" => IssueAutomationFlag::Stale,
                    _ => IssueAutomationFlag::Unknown,
                };
                tracked.set_flag(flag, true);
                adapter
                    .save_issue(fixture.cwd(), revision, &tracked)
                    .unwrap();
            }
            "untracked" => {
                completed_intervention(&fixture, &prepared.intent, IssueInterventionAction::Untrack)
            }
            "cancelled" => completed_intervention(
                &fixture,
                &prepared.intent,
                IssueInterventionAction::Cancel { confirm: true },
            ),
            "taken_over" => completed_intervention(
                &fixture,
                &prepared.intent,
                IssueInterventionAction::TakeOver,
            ),
            "pending" => {
                adapter
                    .begin_issue_intervention(
                        fixture.cwd(),
                        &IssueInterventionRequest {
                            command_id: "pending-hold".into(),
                            issue: prepared.intent.issue.clone(),
                            action: IssueInterventionAction::Hold,
                        },
                    )
                    .unwrap();
            }
            "autonomy" => {
                use crate::domain::issue_automation::autonomy::{
                    IssueAutonomyIntent, IssueAutonomyLevel,
                };
                adapter
                    .begin_issue_autonomy(
                        fixture.cwd(),
                        &IssueAutonomyIntent {
                            command_id: "pending-autonomy".into(),
                            level: IssueAutonomyLevel::Off,
                            expected_revision: 0,
                            policy_version: "a".repeat(64),
                        },
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            adapter
                .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
                .is_err(),
            "{case}"
        );
        assert!(
            pending_records(&fixture, &prepared.work).is_empty(),
            "{case}"
        );
    }
}

#[test]
fn malformed_persisted_receipt_is_rejected_without_regranting_delivery() {
    let fixture = Fixture::new("answer-corrupt");
    let prepared = seed(&fixture);
    let adapter = fixture.adapter();
    let first = adapter
        .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
        .unwrap();
    let mut corrupt = first.record;
    corrupt.intent.question.thread_id = "forged-thread".into();
    adapter
        .with_connection(fixture.cwd(), |connection, guard| {
            transaction(connection, guard, |tx| {
                tx.execute(
                    "UPDATE issue_execution_answers SET revision=2,record=?1",
                    [serde_json::to_string(&corrupt)?],
                )?;
                Ok(())
            })
        })
        .unwrap();
    assert!(
        adapter
            .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
            .is_err()
    );
    assert!(
        adapter
            .pending_issue_execution_answers(fixture.cwd(), &prepared.work.order.work_order_id)
            .is_err()
    );
}

#[test]
fn version_six_and_seven_migrations_preserve_rows_and_create_exact_answer_schema() {
    for previous_version in [6, 7] {
        let fixture = Fixture::new("answer-migration");
        let (_, tracked) = fixture.insert();
        let connection = Connection::open(fixture.database()).unwrap();
        connection
            .execute_batch(
                "DROP TABLE issue_execution_answers;
                 DROP TABLE issue_execution_receipts;
                 DROP TABLE issue_execution_progress;",
            )
            .unwrap();
        if previous_version < 7 {
            connection
                .execute_batch(
                    "DROP TABLE issue_autonomy_work_fences;
                     DROP TABLE issue_autonomy_commands;
                     DROP TABLE issue_autonomy_selection;",
                )
                .unwrap();
        }
        connection
            .pragma_update(None, "user_version", previous_version)
            .unwrap();
        drop(connection);
        assert_eq!(
            fixture
                .adapter()
                .get_issue(fixture.cwd(), tracked.reference())
                .unwrap(),
            Some(tracked)
        );
        let connection = Connection::open(fixture.database()).unwrap();
        assert_eq!(
            super::super::schema_identity(&connection).unwrap(),
            (super::super::APPLICATION_ID, 8)
        );
        super::super::validate_schema(&connection).unwrap();
        for (name, expected) in std::iter::once((TABLE, SCHEMA)).chain(INDEXES) {
            let sql: String = connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name=?1",
                    [name],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                super::super::normalized_schema(&sql),
                super::super::normalized_schema(expected)
            );
        }
    }
}

#[test]
fn missing_answer_table_in_current_schema_is_not_silently_recreated() {
    let fixture = Fixture::new("answer-schema-missing");
    fixture.insert();
    let connection = Connection::open(fixture.database()).unwrap();
    connection
        .execute_batch("DROP TABLE issue_execution_answers")
        .unwrap();
    drop(connection);
    assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());
    let connection = Connection::open(fixture.database()).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name=?1",
            [TABLE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn the_same_question_id_is_independent_between_work_orders() {
    for status in [
        IssueExecutionAnswerStatus::Unknown,
        IssueExecutionAnswerStatus::Queued,
        IssueExecutionAnswerStatus::Accepted {
            thread_id: "thread-original".into(),
            turn_id: "accepted-turn".into(),
            lease_generation: "accepted-generation".into(),
        },
        IssueExecutionAnswerStatus::RejectedBeforeStart {
            reason: "work_controlled".into(),
        },
    ] {
        let fixture = Fixture::new("answer-work-scope");
        let first = seed_issue(&fixture, "I_1");
        let adapter = fixture.adapter();
        let claimed = adapter
            .claim_issue_execution_answer(fixture.cwd(), &first.intent)
            .unwrap();
        let original = adapter
            .save_issue_execution_answer(
                fixture.cwd(),
                &first.intent.command_id,
                claimed.record.revision,
                &status,
            )
            .unwrap()
            .unwrap();
        let second = seed_issue(&fixture, "I_2");
        assert_eq!(
            second.intent.question.question_id,
            first.intent.question.question_id
        );
        assert_ne!(second.intent.work_order_id, first.intent.work_order_id);
        let claimed = adapter
            .claim_issue_execution_answer(fixture.cwd(), &second.intent)
            .unwrap();
        assert!(claimed.created);
        assert_eq!(claimed.record.status, IssueExecutionAnswerStatus::Unknown);
        assert_eq!(
            pending_records(&fixture, &second.work),
            vec![claimed.record]
        );
        let replay = adapter
            .claim_issue_execution_answer(fixture.cwd(), &first.intent)
            .unwrap();
        assert!(!replay.created);
        assert_eq!(replay.record, original);
    }
}

#[test]
fn an_answer_question_column_must_match_the_immutable_intent() {
    let fixture = Fixture::new("answer-question-identity");
    let prepared = seed(&fixture);
    let adapter = fixture.adapter();
    adapter
        .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
        .unwrap();
    let connection = Connection::open(fixture.database()).unwrap();
    connection
        .execute(
            "UPDATE issue_execution_answers SET question_id='another-question'
             WHERE workspace_dir=?1 AND command_id=?2",
            params![fixture.cwd(), prepared.intent.command_id],
        )
        .unwrap();
    drop(connection);
    assert!(
        adapter
            .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
            .is_err()
    );
    assert!(
        adapter
            .pending_issue_execution_answers(fixture.cwd(), &prepared.work.order.work_order_id)
            .is_err()
    );
}

#[test]
fn missing_answer_question_indexes_are_not_silently_recreated() {
    for (name, _) in INDEXES {
        let fixture = Fixture::new("answer-index-missing");
        fixture.insert();
        let connection = Connection::open(fixture.database()).unwrap();
        connection
            .execute_batch(&format!("DROP INDEX {name}"))
            .unwrap();
        drop(connection);
        assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());
        let connection = Connection::open(fixture.database()).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn version_seven_migration_preserves_completed_and_pending_owned_autonomy_fences() {
    use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};

    for complete_before_migration in [false, true] {
        let fixture = Fixture::new("answer-migration-owned-autonomy");
        let prepared = seed(&fixture);
        let adapter = fixture.adapter();
        let intent = IssueAutonomyIntent {
            command_id: "migration-autonomy-off".into(),
            level: IssueAutonomyLevel::Off,
            expected_revision: 0,
            policy_version: resolve_issue_automation_policy(None, None).unwrap().version,
        };
        adapter
            .begin_issue_autonomy(fixture.cwd(), &intent)
            .unwrap();
        let reserved = adapter
            .claim_issue_autonomy_fence(fixture.cwd(), &intent, &prepared.work)
            .unwrap();
        assert!(reserved.receipt.is_none());
        let receipt = IssueInterventionReceipt {
            command_id: reserved.request.command_id.clone(),
            issue: prepared.work.order.issue.clone(),
            action: reserved.request.action.label().into(),
            work_order_id: Some(prepared.work.order.work_order_id.clone()),
            authorization_epoch: 1,
            changed: true,
            delivery: None,
            handoff: None,
        };
        adapter
            .finish_issue_intervention(fixture.cwd(), &receipt)
            .unwrap();
        if complete_before_migration {
            assert!(
                adapter
                    .finish_issue_autonomy(fixture.cwd(), &intent)
                    .unwrap()
                    .changed
            );
        }
        let selection = adapter.issue_autonomy_selection(fixture.cwd()).unwrap();
        let stored_work = adapter
            .get_issue_work(fixture.cwd(), &prepared.work.order.issue)
            .unwrap()
            .unwrap();
        let command = adapter
            .get_issue_autonomy_command(fixture.cwd(), &intent.command_id)
            .unwrap()
            .unwrap();
        let intervention = adapter
            .get_issue_intervention(fixture.cwd(), &receipt.command_id)
            .unwrap()
            .unwrap();
        assert_eq!(intervention.receipt.as_ref(), Some(&receipt));
        assert_eq!(
            command.completed_revision.is_some(),
            complete_before_migration
        );
        assert_eq!(selection.pending.is_none(), complete_before_migration);
        let persisted_owner_and_receipts = |connection: &Connection| {
            connection
                .query_row(
                    "SELECT fence.control_command_id, autonomy.intent,
                            autonomy.completed_revision, control.request_json, control.receipt_json
                     FROM issue_autonomy_work_fences AS fence
                     JOIN issue_autonomy_commands AS autonomy
                       ON autonomy.workspace_dir=fence.workspace_dir
                      AND autonomy.command_id=fence.autonomy_command_id
                     JOIN issue_intervention_commands AS control
                       ON control.workspace_dir=fence.workspace_dir
                      AND control.command_id=fence.control_command_id
                     WHERE fence.workspace_dir=?1 AND fence.autonomy_command_id=?2
                       AND fence.work_order_id=?3",
                    params![
                        fixture.cwd(),
                        intent.command_id,
                        prepared.work.order.work_order_id
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    },
                )
                .unwrap()
        };
        let connection = Connection::open(fixture.database()).unwrap();
        let before = persisted_owner_and_receipts(&connection);
        connection
            .execute_batch(
                "DROP TABLE issue_execution_answers;
                 DROP TABLE issue_execution_receipts;
                 DROP TABLE issue_execution_progress;",
            )
            .unwrap();
        connection.pragma_update(None, "user_version", 7).unwrap();
        // This is the exact parent schema, including a real owned fence and receipt.
        super::super::validate_schema_version(&connection, 7).unwrap();
        assert_eq!(
            super::super::schema_identity(&connection).unwrap(),
            (super::super::APPLICATION_ID, 7)
        );
        assert_eq!(persisted_owner_and_receipts(&connection), before);
        drop(connection);
        drop(adapter);

        let reopened = fixture.adapter();
        assert_eq!(
            reopened.issue_autonomy_selection(fixture.cwd()).unwrap(),
            selection
        );
        assert_eq!(
            reopened
                .get_issue_autonomy_command(fixture.cwd(), &intent.command_id)
                .unwrap(),
            Some(command.clone())
        );
        assert_eq!(
            reopened
                .get_issue_intervention(fixture.cwd(), &receipt.command_id)
                .unwrap(),
            Some(intervention.clone())
        );
        assert_eq!(
            reopened
                .get_issue_work(fixture.cwd(), &prepared.work.order.issue)
                .unwrap(),
            Some(stored_work)
        );
        let connection = Connection::open(fixture.database()).unwrap();
        assert_eq!(
            super::super::schema_identity(&connection).unwrap(),
            (super::super::APPLICATION_ID, 8)
        );
        super::super::validate_schema(&connection).unwrap();
        assert_eq!(persisted_owner_and_receipts(&connection), before);
        let indexes = connection
            .prepare("PRAGMA index_list(issue_execution_answers)")
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, bool>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(indexes.contains(&("issue_execution_answers_live_question".into(), true, true)));
        assert!(indexes.contains(&(
            "issue_execution_answers_question_history".into(),
            false,
            false
        )));
        drop(connection);

        assert_eq!(
            reopened
                .begin_issue_autonomy(fixture.cwd(), &intent)
                .unwrap(),
            command
        );
        if !complete_before_migration {
            assert_eq!(
                reopened
                    .claim_issue_autonomy_fence(fixture.cwd(), &intent, &prepared.work)
                    .unwrap(),
                intervention
            );
        }
        let finalized = reopened
            .finish_issue_autonomy(fixture.cwd(), &intent)
            .unwrap();
        assert_eq!(finalized.changed, !complete_before_migration);
        assert_eq!(finalized.command.completed_revision, Some(1));
        assert!(
            !reopened
                .finish_issue_autonomy(fixture.cwd(), &intent)
                .unwrap()
                .changed
        );
        assert_eq!(
            reopened
                .get_issue_intervention(fixture.cwd(), &receipt.command_id)
                .unwrap(),
            Some(intervention)
        );
        assert!(
            reopened
                .pending_issue_interventions(fixture.cwd())
                .unwrap()
                .is_empty()
        );
    }
}

fn record_human_recovery_for_current_question(
    fixture: &Fixture,
    prepared: &Prepared,
    classification: crate::domain::issue_automation::execution::IssueExecutionTerminalClass,
) {
    use crate::domain::issue_automation::execution::IssueExecutionTerminalRecord;

    let adapter = fixture.adapter();
    let current = adapter
        .get_issue_execution(fixture.cwd(), &prepared.intent.issue)
        .unwrap()
        .unwrap();
    let terminal_id = format!("answer-human-recovery-{classification:?}");
    let observation = IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Terminal,
        task_ids: prepared.work.task_ids.clone(),
        active_task_id: Some(prepared.intent.question.task_id.clone()),
        execution_ids: vec![prepared.intent.question.task_id.clone()],
        terminals: vec![IssueExecutionTerminalRecord {
            record_id: terminal_id.clone(),
            classification,
        }],
        current_terminal_id: Some(terminal_id),
        question: Some(prepared.intent.question.clone()),
        ..Default::default()
    };
    observation.validate().unwrap();
    let saved = adapter
        .apply_issue_execution(
            fixture.cwd(),
            &prepared.work,
            current.progress.revision,
            &observation,
            Some(3),
        )
        .unwrap();
    let IssueExecutionSaveResult::Saved { record, .. } = saved else {
        panic!("expected the current question's terminal observation to be saved");
    };
    assert!(record.progress.requires_human_recovery());
    assert_eq!(record.question.as_ref(), Some(&prepared.intent.question));
}

#[test]
fn a_human_recovery_saved_after_question_approval_refuses_the_first_answer_claim() {
    use crate::domain::issue_automation::execution::IssueExecutionTerminalClass;

    for classification in [
        IssueExecutionTerminalClass::ContractRecovery,
        IssueExecutionTerminalClass::PullRequestClosed,
        IssueExecutionTerminalClass::IdentityConflict,
    ] {
        let fixture = Fixture::new("answer-current-human-recovery");
        let prepared = seed(&fixture);
        // The exact question and confirmed answer existed before the other tracker persisted
        // the blocking terminal. The claim must read that terminal in its own transaction.
        record_human_recovery_for_current_question(&fixture, &prepared, classification);
        let adapter = fixture.adapter();
        assert!(
            adapter
                .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
                .is_err(),
            "{classification:?}",
        );
        assert!(pending_records(&fixture, &prepared.work).is_empty());
        let connection = Connection::open(fixture.database()).unwrap();
        let answer_rows: i64 = connection
            .query_row("SELECT count(*) FROM issue_execution_answers", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(answer_rows, 0);
        assert_eq!(
            adapter
                .get_issue_decision(fixture.cwd(), prepared.request.id())
                .unwrap()
                .unwrap()
                .request,
            prepared.request,
        );
    }
}

#[test]
fn a_human_recovery_terminal_does_not_change_existing_answer_receipts() {
    use crate::domain::issue_automation::execution::IssueExecutionTerminalClass;

    for status in [
        IssueExecutionAnswerStatus::Unknown,
        IssueExecutionAnswerStatus::Queued,
        IssueExecutionAnswerStatus::Accepted {
            thread_id: "thread-original".into(),
            turn_id: "accepted-before-recovery".into(),
            lease_generation: "generation-before-recovery".into(),
        },
        IssueExecutionAnswerStatus::RejectedBeforeStart {
            reason: "work_controlled".into(),
        },
    ] {
        let fixture = Fixture::new("answer-replay-after-human-recovery");
        let prepared = seed(&fixture);
        let adapter = fixture.adapter();
        let initial = adapter
            .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
            .unwrap();
        let original = adapter
            .save_issue_execution_answer(
                fixture.cwd(),
                &prepared.intent.command_id,
                initial.record.revision,
                &status,
            )
            .unwrap()
            .unwrap();
        record_human_recovery_for_current_question(
            &fixture,
            &prepared,
            IssueExecutionTerminalClass::ContractRecovery,
        );

        let replay = fixture
            .adapter()
            .claim_issue_execution_answer(fixture.cwd(), &prepared.intent)
            .unwrap();
        assert!(!replay.created);
        assert_eq!(replay.record, original);
        let connection = Connection::open(fixture.database()).unwrap();
        let answer_rows: i64 = connection
            .query_row("SELECT count(*) FROM issue_execution_answers", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(answer_rows, 1);
    }
}
