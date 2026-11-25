use super::*;
use crate::domain::issue_automation::decision::{
    IssueDecisionAnswerBy, IssueDecisionBasis, IssueDecisionOption, IssueDecisionSharedText,
    IssueDecisionSubject, IssueDecisionTimeout,
};
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

fn shared(value: &str) -> IssueDecisionSharedText {
    IssueDecisionSharedText::new_shared(value).unwrap()
}

fn ask() -> IssueAgentAsk {
    IssueAgentAsk {
        task_id: "task-1".into(),
        thread_id: "thread-1".into(),
        lease_generation: "generation-1".into(),
    }
}

fn definition(evidence: Vec<String>) -> IssueDecisionDefinition {
    IssueDecisionDefinition {
        delivery_review: None,
        subject: IssueDecisionSubject::Issue(IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_ask".into(),
            issue_id: "I_ask".into(),
            project_scope: IssueProjectScope {
                workspace_dir: "/workspace/ask".into(),
            },
        }),
        gate: IssueAutomationGate::Spec,
        kind: IssueDecisionKind::AgentQuestion,
        question: shared("A 와 B 중 무엇입니까?"),
        options: vec![IssueDecisionOption {
            id: "a".into(),
            label: shared("A"),
            effect: shared(AGENT_QUESTION_OPTION_EFFECT),
        }],
        recommended: None,
        basis: IssueDecisionBasis {
            delivery: None,
            authorization_epoch: 0,
            policy_version: "a".repeat(64),
            field_hash: None,
            work_order_id: Some("work-1".into()),
            gate_revision: 1,
        },
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "akra-cli".into(),
        evidence,
    }
}

fn answer(option: &str, note: Option<&str>) -> IssueDecisionAnswer {
    IssueDecisionAnswer {
        option: option.into(),
        note: note.map(shared),
        by: IssueDecisionAnswerBy::Human,
        confirmation: None,
        caller: None,
        channel: "gateway".into(),
        at: "2026-11-25T00:00:00Z".into(),
    }
}

fn waiting(lease_generation: &str) -> IssueExecutionQuestion {
    IssueExecutionQuestion {
        question_id: "question-1".into(),
        task_id: "task-1".into(),
        session_key: "session".into(),
        thread_id: "thread-1".into(),
        slot_id: "slot-1".into(),
        worktree_path: "/pool/slot-1".into(),
        branch_name: "akra-agent/slot-1/task".into(),
        lease_generation: lease_generation.into(),
        awaiting_task_updated_at: "2026-11-25T00:00:00Z".into(),
    }
}

#[test]
fn ask_coordinates_round_trip_and_bind_the_exact_waiting_lease() {
    let asked = definition(ask().evidence());
    assert_eq!(IssueAgentAsk::from_definition(&asked), Some(ask()));
    assert!(agent_question_targets(&asked, &waiting("generation-1")));
    assert!(!agent_question_targets(&asked, &waiting("generation-2")));

    let generic = definition(vec!["question:question-1".into()]);
    assert_eq!(IssueAgentAsk::from_definition(&generic), None);
    assert!(agent_question_targets(&generic, &waiting("generation-2")));
}

#[test]
fn reply_text_is_built_only_from_shared_values() {
    let asked = definition(ask().evidence());
    assert_eq!(
        agent_question_reply(&asked, &answer("a", Some("B는 범위 밖입니다"))).as_deref(),
        Some("질문: A 와 B 중 무엇입니까?\n사람의 선택: A (`a`)\n설명: B는 범위 밖입니다")
    );
    assert_eq!(agent_question_reply(&asked, &answer("missing", None)), None);

    let generic = definition(vec!["question:question-1".into()]);
    assert_eq!(
        agent_question_reply(&generic, &answer(WORKER_REPLY_OPTION, Some("답"))).as_deref(),
        Some("답")
    );
    assert_eq!(
        agent_question_reply(&generic, &answer("a", Some("답"))),
        None
    );
}
