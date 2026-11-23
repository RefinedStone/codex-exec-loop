use super::*;
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::policy::{IssueAutomationGate, IssueAutomationGateAction};

fn question(fixture: &Fixture) -> IssueExecutionQuestion {
    IssueExecutionQuestion {
        question_id: "question-1".into(),
        task_id: "task-1".into(),
        session_key: "isolated-session-1".into(),
        thread_id: "thread-1".into(),
        slot_id: "slot-1".into(),
        worktree_path: fixture
            .workspace
            .join("slot-1")
            .to_string_lossy()
            .into_owned(),
        branch_name: "akra/task-1".into(),
        lease_generation: "generation-old".into(),
        awaiting_task_updated_at: "2026-11-21T00:00:00Z".into(),
    }
}

fn insert_decision(
    fixture: &Fixture,
    work: &IssueWorkCheckpoint,
    id: &str,
    work_id: Option<&str>,
    kind: IssueDecisionKind,
    answered: bool,
) -> StoredIssueDecision {
    let mut basis = work.approved_basis.clone();
    basis.work_order_id = work_id.map(str::to_owned);
    let shared = |text: &str| IssueDecisionSharedText::new_shared(text).unwrap();
    let request = IssueDecisionRequest::new(
        IssueDecisionId::new(id).unwrap(),
        IssueDecisionDefinition {
            delivery_review: None,
            subject: IssueDecisionSubject::Issue(work.order.issue.clone()),
            gate: if kind == IssueDecisionKind::StartApproval {
                IssueAutomationGate::Start
            } else {
                IssueAutomationGate::Spec
            },
            kind,
            question: shared("기존 작업 대화의 질문을 확인해 주세요."),
            options: vec![IssueDecisionOption {
                id: "reply".into(),
                label: shared("답변"),
                effect: shared("같은 대화에 답변을 전달한다."),
            }],
            recommended: None,
            basis,
            on_timeout: IssueDecisionTimeout::Wait,
            due_at: None,
            asked_by: id.into(),
            evidence: vec!["question:question-1".into()],
        },
    )
    .unwrap();
    let request = if answered {
        request
            .answer_for_gate(
                &request.definition().basis,
                IssueAutomationGateAction::Human,
                IssueDecisionAgentAnswer {
                    option: "reply".into(),
                    note: Some(shared("확인한 답변입니다.")),
                    caller: Some("Desktop".into()),
                    channel: "gateway".into(),
                    at: "2026-11-21T00:01:00Z".into(),
                },
                Some("desktop-confirmation:0123456789abcdef0123456789abcdef"),
            )
            .unwrap()
            .request
    } else {
        request
    };
    fixture
        .adapter()
        .with_connection(fixture.cwd(), |connection, _| {
            connection.execute(
                "INSERT INTO issue_decision_requests
             (decision_id,workspace_dir,provider,server_id,repository_id,issue_id,binding_id,
              uniqueness_key,state,request,revision)
             VALUES (?1,?2,?3,?4,?5,?6,'binding-1',?7,?8,?9,1)",
                params![
                    request.id().as_str(),
                    fixture.cwd(),
                    super::super::super::enum_name(&work.order.issue.provider)?,
                    work.order.issue.server_id,
                    work.order.issue.repository_id,
                    work.order.issue.issue_id,
                    request.uniqueness_key(),
                    super::super::super::enum_name(&request.state())?,
                    serde_json::to_string(&request.to_record())?
                ],
            )?;
            Ok(())
        })
        .unwrap();
    StoredIssueDecision {
        request,
        revision: 1,
    }
}

fn persisted(fixture: &Fixture, decision: &StoredIssueDecision) -> StoredIssueDecision {
    fixture
        .adapter()
        .get_issue_decision_commit(fixture.cwd(), decision.request.id())
        .unwrap()
        .unwrap()
        .record
}

fn catalog(fixture: &Fixture) -> u64 {
    fixture
        .adapter()
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap()
        .revision
}

#[test]
fn changed_or_disappeared_questions_supersede_only_open_decisions_for_that_work() {
    for scenario in ["gone", "new_id", "same_id_new_coordinates"] {
        let fixture = Fixture::new("execution-question-superseded");
        let work = seed(&fixture);
        let mut observation = running(&work);
        observation.question = Some(question(&fixture));
        let first = saved(
            fixture
                .adapter()
                .apply_issue_execution(fixture.cwd(), &work, 0, &observation, Some(3))
                .unwrap(),
        );
        let open = insert_decision(
            &fixture,
            &work,
            "current-question",
            Some(&work.order.work_order_id),
            IssueDecisionKind::AgentQuestion,
            false,
        );
        let answered = insert_decision(
            &fixture,
            &work,
            "answered-question",
            Some(&work.order.work_order_id),
            IssueDecisionKind::AgentQuestion,
            true,
        );
        let independent = insert_decision(
            &fixture,
            &work,
            "independent-decision",
            None,
            IssueDecisionKind::StartApproval,
            false,
        );
        let foreign = insert_decision(
            &fixture,
            &work,
            "other-work-question",
            Some("another-work"),
            IssueDecisionKind::AgentQuestion,
            false,
        );
        let before_catalog = catalog(&fixture);
        let unchanged = fixture
            .adapter()
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                first.progress.revision,
                &observation,
                Some(3),
            )
            .unwrap();
        assert!(matches!(
            unchanged,
            IssueExecutionSaveResult::Saved { changed: false, .. }
        ));
        assert_eq!(persisted(&fixture, &open), open);
        assert_eq!(catalog(&fixture), before_catalog);
        match scenario {
            "gone" => observation.question = None,
            "new_id" => observation.question.as_mut().unwrap().question_id = "question-2".into(),
            "same_id_new_coordinates" => {
                observation.question.as_mut().unwrap().lease_generation = "new-generation".into()
            }
            _ => unreachable!(),
        }
        // A stale progress CAS must leave the old question and all events untouched.
        assert_eq!(
            fixture
                .adapter()
                .apply_issue_execution(fixture.cwd(), &work, 0, &observation, Some(3))
                .unwrap(),
            IssueExecutionSaveResult::Conflict,
        );
        assert_eq!(persisted(&fixture, &open), open);
        let second = saved(
            fixture
                .adapter()
                .apply_issue_execution(
                    fixture.cwd(),
                    &work,
                    first.progress.revision,
                    &observation,
                    Some(3),
                )
                .unwrap(),
        );
        let closed = persisted(&fixture, &open);
        assert_eq!(closed.request.state(), IssueDecisionState::Superseded);
        assert_eq!(closed.revision, open.revision + 1);
        assert_eq!(closed.request.definition(), open.request.definition());
        assert!(closed.request.answer().is_none());
        assert_eq!(persisted(&fixture, &answered), answered);
        assert_eq!(persisted(&fixture, &independent), independent);
        assert_eq!(persisted(&fixture, &foreign), foreign);
        assert_eq!(catalog(&fixture), before_catalog + 1);
        let events = fixture
            .adapter()
            .pending_issue_work_events(fixture.cwd())
            .unwrap();
        assert!(events.iter().any(|event| matches!(&event.kind,
            IssueWorkEventKind::ExecutionProgressed { progress } if progress == &second.progress)));
        let replay = fixture
            .adapter()
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                second.progress.revision,
                &observation,
                Some(3),
            )
            .unwrap();
        assert!(matches!(
            replay,
            IssueExecutionSaveResult::Saved { changed: false, .. }
        ));
        assert_eq!(persisted(&fixture, &open), closed);
        assert_eq!(catalog(&fixture), before_catalog + 1);
    }
}

#[test]
fn failure_to_commit_execution_rolls_back_question_superseding() {
    let fixture = Fixture::new("execution-question-transaction-rollback");
    let work = seed(&fixture);
    let mut observation = running(&work);
    observation.question = Some(question(&fixture));
    let adapter = fixture.adapter();
    let first = saved(
        adapter
            .apply_issue_execution(fixture.cwd(), &work, 0, &observation, Some(3))
            .unwrap(),
    );
    let open = insert_decision(
        &fixture,
        &work,
        "current-question",
        Some(&work.order.work_order_id),
        IssueDecisionKind::AgentQuestion,
        false,
    );
    let before_catalog = catalog(&fixture);
    let before_tracked = adapter
        .get_issue(fixture.cwd(), &work.order.issue)
        .unwrap()
        .unwrap();
    let mut next_progress = first.progress.clone();
    next_progress.revision += 1;
    let collision = IssueWorkEvent::new(
        work.order.issue.clone(),
        work.order.work_order_id.clone(),
        IssueWorkEventKind::ExecutionProgressed {
            progress: next_progress.clone(),
        },
        before_catalog + 1,
    );
    // Keep the exact schema intact. This key collides only at the outbox INSERT, after
    // the transaction has superseded the question and written tracked state/progress/catalog.
    adapter
        .with_connection(fixture.cwd(), |connection, _| {
            connection.execute(
                "INSERT INTO issue_work_events
                 (workspace_dir,event_id,event,revision,acknowledged) VALUES (?1,?2,?3,?4,0)",
                params![
                    fixture.cwd(),
                    collision.event_id,
                    serde_json::to_string(&collision)?,
                    sql_revision(collision.revision)?
                ],
            )?;
            Ok(())
        })
        .unwrap();
    let before_events = adapter.pending_issue_work_events(fixture.cwd()).unwrap();
    observation.question = None;
    let error = adapter
        .apply_issue_execution(
            fixture.cwd(),
            &work,
            first.progress.revision,
            &observation,
            Some(3),
        )
        .expect_err("the execution outbox INSERT must hit the fixture primary key");
    assert!(
        matches!(
            error.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(code, Some(message)))
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                    && message.contains("issue_work_events")
        ),
        "{error:#}"
    );
    assert_eq!(persisted(&fixture, &open), open);
    assert_eq!(
        adapter
            .get_issue_execution(fixture.cwd(), &work.order.issue)
            .unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap()
            .to_record(),
        before_tracked.to_record()
    );
    assert_eq!(catalog(&fixture), before_catalog);
    assert_eq!(
        adapter.pending_issue_work_events(fixture.cwd()).unwrap(),
        before_events
    );

    adapter
        .with_connection(fixture.cwd(), |connection, _| {
            let removed = connection.execute(
                "DELETE FROM issue_work_events WHERE workspace_dir=?1 AND event_id=?2",
                params![fixture.cwd(), collision.event_id],
            )?;
            assert_eq!(removed, 1);
            Ok(())
        })
        .unwrap();
    let retried = saved(
        adapter
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                first.progress.revision,
                &observation,
                Some(3),
            )
            .unwrap(),
    );
    assert_eq!(retried.progress, next_progress);
    assert!(retried.question.is_none());
    assert_eq!(
        persisted(&fixture, &open).request.state(),
        IssueDecisionState::Superseded
    );
    assert_eq!(catalog(&fixture), before_catalog + 1);
}
