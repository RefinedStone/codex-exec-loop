//! 실제 command-only 조립과 같은 SQLite 장부에서 답·영수증·잠금 경계를 검증한다.
use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::domain::issue_automation::decision::{
    IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionId, IssueDecisionKind,
    IssueDecisionOption, IssueDecisionSharedText, IssueDecisionSubject, IssueDecisionTimeout,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::{
    IssueAutomationBinding, IssueAutomationBindingId, IssueAutomationSnapshot, TrackedIssue,
};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    IssueSharedCopy,
};

fn store() -> SqliteIssueAutomationStoreAdapter {
    SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    })
}

fn seed_decision(kernel: &CommandOnlyKernel) -> IssueReference {
    let ledger = store();
    let reference = IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_offline_decision".into(),
        issue_id: "I_offline_decision".into(),
        project_scope: IssueProjectScope {
            workspace_dir: kernel.workspace.clone(),
        },
    };
    let binding = IssueAutomationBinding::new(
        IssueAutomationBindingId::new("offline-decision").unwrap(),
        reference.provider,
        &reference.server_id,
        &reference.repository_id,
        reference.project_scope.clone(),
    )
    .unwrap();
    ledger.insert_binding(&kernel.workspace, &binding).unwrap();
    let tracked = TrackedIssue::discovered(
        reference.clone(),
        &binding,
        IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields("legacy title", "private original body"),
                provider_updated_at: "2026-11-20T00:00:00Z".into(),
            },
            shared_copy: IssueSharedCopy {
                title: "legacy title".into(),
                body: "private original body".into(),
            },
            policy_version: "legacy-policy".into(),
        },
    )
    .unwrap();
    ledger.insert_issue(&kernel.workspace, &tracked).unwrap();
    ledger.ensure_collection(&kernel.workspace).unwrap();
    // 정책·장부만 읽는다. source discover/resync와 실행 관문은 부르지 않는다.
    let collection = crate::composition::issue_collection::build(&kernel.workspace);
    let triage = collection.triage_service().unwrap();
    let outcome = triage
        .triage_all(&IssueCollectionCancellation::new(), &|_| {})
        .unwrap();
    assert_eq!(outcome.changed, 1);
    let detail = collection.issue(&reference).unwrap();
    let assessment = detail.item.unwrap().triage.unwrap().assessment;
    let definition = IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(reference.clone()),
        gate: IssueAutomationGate::Start,
        kind: IssueDecisionKind::AgentQuestion,
        question: IssueDecisionSharedText::new_shared("공유된 질문").unwrap(),
        options: vec![IssueDecisionOption {
            id: "acknowledge".into(),
            label: IssueDecisionSharedText::new_shared("확인").unwrap(),
            effect: IssueDecisionSharedText::new_shared("답변을 기록합니다.").unwrap(),
        }],
        recommended: None,
        basis: IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None)
            .unwrap(),
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "offline-kernel-test".into(),
        evidence: vec![],
    };
    let commit = crate::composition::issue_collection::build_decisions(&kernel.workspace)
        .ensure_request(
            IssueDecisionId::new("offline-decision").unwrap(),
            definition,
        )
        .unwrap();
    assert!(commit.changed);
    reference
}

fn answer(note: &str) -> Value {
    json!({"type":"answer_issue_decision","decision_id":"offline-decision",
        "option":"acknowledge","note":note})
}

#[test]
fn offline_decision_human_gate_rejection_preserves_open_request_and_replays_receipts() {
    let (_directory, kernel) = fixture();
    let before_planning = planning_revision(&kernel);
    let reference = seed_decision(&kernel);
    let query = json!({"type":"issue", "issue": crate::domain::kernel_event::IssueEventReference::from(&reference)});
    let open = kernel.query(query.clone(), false).unwrap();
    assert_eq!(open["open_decisions"].as_array().unwrap().len(), 1);
    let before = store()
        .get_issue_decision(
            &kernel.workspace,
            &IssueDecisionId::new("offline-decision").unwrap(),
        )
        .unwrap()
        .unwrap();

    let first = kernel
        .command(
            KernelCommandKind::AnswerIssueDecision,
            answer("local note"),
            Some("offline-answer-key".into()),
        )
        .unwrap();
    assert_eq!(
        first["rejection"]["reason"], "human_confirmation_required",
        "{first}"
    );
    assert!(first["result"].is_null());
    let reopened = CommandOnlyKernel {
        workspace: kernel.workspace.clone(),
        caller: CALLER_CLI,
    };
    let replay = reopened
        .command(
            KernelCommandKind::AnswerIssueDecision,
            answer("local note"),
            Some("offline-answer-key".into()),
        )
        .unwrap();
    assert_eq!(replay["operation"]["replayed"], true);
    assert_eq!(replay["operation"]["state"], "rejected");
    assert_eq!(replay["operation"]["op_id"], first["operation"]["op_id"]);
    assert_eq!(replay["rejection"]["reason"], "human_confirmation_required");
    assert!(
        reopened
            .command(
                KernelCommandKind::AnswerIssueDecision,
                answer("changed note"),
                Some("offline-answer-key".into()),
            )
            .is_err()
    );
    let second = reopened
        .command(
            KernelCommandKind::AnswerIssueDecision,
            answer("second note"),
            Some("independent-second-key".into()),
        )
        .unwrap();
    assert_eq!(
        second["rejection"]["reason"], "human_confirmation_required",
        "{second}"
    );

    assert_eq!(
        store()
            .get_issue_decision(
                &kernel.workspace,
                &IssueDecisionId::new("offline-decision").unwrap()
            )
            .unwrap()
            .unwrap(),
        before
    );
    let still_open = reopened.query(query, false).unwrap();
    assert_eq!(still_open["revision"], open["revision"]);
    assert_eq!(still_open["open_decisions"].as_array().unwrap().len(), 1);
    let records = reopened
        .records(crate::composition::production::kernel_event_log_port().as_ref())
        .unwrap();
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record.event, KernelEvent::IssueDecisionResolved { .. }))
            .count(),
        0
    );
    let encoded = serde_json::to_string(&records).unwrap();
    assert!(!encoded.contains("local note") && !encoded.contains("second note"));
    assert!(!encoded.contains("private original body"));
    assert!(records.iter().all(|record| matches!(
        record.event,
        KernelEvent::CommandReceived { .. } | KernelEvent::CommandRejected { .. }
    )));
    assert_eq!(planning_revision(&kernel), before_planning);
}

#[test]
fn offline_decision_answer_never_uses_the_unrecorded_tui_exception() {
    let (_directory, kernel) = fixture();
    seed_decision(&kernel);
    let location =
        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&kernel.workspace)
            .unwrap();
    let _lock = RepositoryKernelLockAdapter::acquire_for_owner(
        location,
        &kernel.workspace,
        RepositoryKernelOwner::Tui,
    )
    .unwrap();
    let refused = kernel
        .command(
            KernelCommandKind::AnswerIssueDecision,
            answer("private note"),
            None,
        )
        .unwrap();
    assert_eq!(
        refused["rejection"]["reason"], "kernel_unreachable",
        "{refused}"
    );
    let stored = store()
        .get_issue_decision(
            &kernel.workspace,
            &IssueDecisionId::new("offline-decision").unwrap(),
        )
        .unwrap()
        .unwrap();
    assert!(stored.request.answer().is_none());
    assert!(
        kernel
            .records(crate::composition::production::kernel_event_log_port().as_ref())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn offline_decision_decode_errors_do_not_echo_private_fields() {
    const CANARY: &str = "private-offline-decision-field";
    let (_directory, kernel) = fixture();
    let mut input = answer(CANARY);
    input[CANARY] = json!("unexpected");
    let error = kernel
        .command(KernelCommandKind::AnswerIssueDecision, input, None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid issue decision answer parameters"
    );
    assert!(!error.to_string().contains(CANARY));
}
