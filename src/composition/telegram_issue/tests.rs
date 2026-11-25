use std::sync::Mutex;

use serde_json::{Value, json};

use super::*;
use crate::domain::kernel_spec::{KernelCommandKind, ReasonCode};

struct FakeKernel {
    answer: Value,
    sent: Mutex<Vec<(KernelCommandKind, Value, Option<String>)>>,
}

impl KernelCommandPort for FakeKernel {
    fn command(
        &self,
        kind: KernelCommandKind,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<Value> {
        self.sent
            .lock()
            .unwrap()
            .push((kind, input, idempotency_key));
        Ok(self.answer.clone())
    }

    fn query(&self, _input: Value, _wait: bool) -> Result<Value> {
        unreachable!("an answer does not wait for an operation")
    }
}

fn resolved(outcome: &str) -> Value {
    json!({
        "processed": true,
        "result": {
            "type": "issue_decision_resolved",
            "decision": {
                "decision_id": "decision-1",
                "subject": {"kind": "issue", "issue": {
                    "provider": "github", "server_id": "github.com", "repository_id": "R_1",
                    "issue_id": "I_1", "project_scope": {"workspace_dir": "/repo"}
                }},
                "gate": "start",
                "kind": "start_approval",
                "basis": {"policy_version": "a".repeat(64), "field_hash": null,
                          "work_order_id": null, "gate_revision": 1},
                "state": if outcome == "answered" { "answered" } else { "superseded" },
                "answer_by": null,
                "option": null
            },
            "outcome": outcome,
            "changed": true,
            "catalog_revision": 2,
            "reason": null
        }
    })
}

fn answer(kernel: &FakeKernel) -> TelegramIssueAnswerOutcome {
    answer_through_kernel(kernel, "decision-1", "start").unwrap()
}

#[test]
fn a_telegram_answer_is_the_plain_kernel_command_without_human_confirmation() {
    let kernel = FakeKernel {
        answer: resolved("answered"),
        sent: Mutex::new(Vec::new()),
    };
    assert_eq!(answer(&kernel), TelegramIssueAnswerOutcome::Answered);
    let sent = kernel.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, KernelCommandKind::AnswerIssueDecision);
    assert_eq!(
        sent[0].1,
        json!({"type": "answer_issue_decision", "decision_id": "decision-1", "option": "start"})
    );
    assert_eq!(sent[0].2, None);
}

#[test]
fn a_superseded_basis_and_a_human_gate_are_reported_without_applying_the_answer() {
    let superseded = FakeKernel {
        answer: resolved("superseded"),
        sent: Mutex::new(Vec::new()),
    };
    assert_eq!(answer(&superseded), TelegramIssueAnswerOutcome::Superseded);

    let human = FakeKernel {
        answer: json!({
            "processed": true,
            "rejection": {
                "reason": "human_confirmation_required",
                "message": "needs a person",
                "next": [{"step": "ask_user"}]
            }
        }),
        sent: Mutex::new(Vec::new()),
    };
    assert_eq!(
        answer(&human),
        TelegramIssueAnswerOutcome::Rejected(ReasonCode::HumanConfirmationRequired)
    );

    let failed = FakeKernel {
        answer: json!({"processed": true, "failure": "the decision request was not found"}),
        sent: Mutex::new(Vec::new()),
    };
    assert_eq!(answer(&failed), TelegramIssueAnswerOutcome::Failed);
}
