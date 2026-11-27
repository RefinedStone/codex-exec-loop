//! Fixed synthetic task baseline: outcome, host evidence and cost remain separate.
use super::*;
use crate::domain::conversation_progressive_activity::ConversationProgressiveTokenUsageBreakdown as Tokens;
use crate::domain::task_usage::{TaskUsageState, UsageGap, UsageTurn};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    version: u32,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    expected_output: String,
    evidence: Evidence,
    events: Vec<Event>,
    expected: Metrics,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Evidence {
    Host,
    HumanWaiver,
    StaleHost,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Advance {
        ms: u64,
    },
    Tool {
        call_id: String,
        output: String,
    },
    Usage {
        sequence: i64,
        last: u64,
        total: u64,
    },
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metrics {
    task_success: bool,
    valid_evidence: usize,
    waivers: usize,
    delivery_allowed: bool,
    elapsed_ms: u64,
    tool_calls: usize,
    observed_tokens: Option<u64>,
    usage_gaps: Vec<UsageGap>,
}

fn tokens(input: u64) -> Tokens {
    Tokens {
        input_tokens: input,
        total_tokens: input,
        ..Default::default()
    }
}

fn replay(case: &Case) -> Metrics {
    let fixture = Fixture::new(TaskEvidenceDeliveryMode::Enforce);
    match case.evidence {
        Evidence::Host => {}
        Evidence::HumanWaiver => {
            fixture.change(|snapshot| {
                let criterion = &mut snapshot.task.as_mut().unwrap().task.acceptance[0];
                criterion.channel = TaskAcceptanceChannel::Screen;
                criterion.verification_recipe = None;
            });
            fixture.synchronize_scope();
            fixture.waive();
        }
        Evidence::StaleHost => {
            *fixture.runner.current.lock().unwrap() = Ok("8".repeat(64));
        }
    }
    let mut elapsed_ms = 0_u64;
    let mut calls = BTreeSet::new();
    let mut output = None;
    let mut turn = UsageTurn::new("fixture".into(), case.id.clone(), Some(1), 0);
    for event in &case.events {
        match event {
            Event::Advance { ms } => elapsed_ms = elapsed_ms.checked_add(*ms).unwrap(),
            Event::Tool {
                call_id,
                output: observed,
            } => {
                assert!(!call_id.is_empty());
                assert!(calls.insert(call_id), "duplicate tool identity in fixture");
                output = Some(observed);
            }
            Event::Usage {
                sequence,
                last,
                total,
            } => {
                turn.observe(*sequence, tokens(*last), tokens(*total));
            }
        }
    }
    // The objective oracle compares the synthetic output, not a model's claim
    // or a delivery decision. Even valid host evidence can be insufficient.
    let task_success = output.is_some_and(|value| value == &case.expected_output);
    let decision = fixture.effect().unwrap();
    let observation = fixture.last();
    let execution = turn.execution();
    let usage_gaps = execution.gaps.clone();
    let mut usage = TaskUsageState::unavailable(case.id.clone());
    usage.executions.push(execution);
    usage.sum_observations();
    Metrics {
        task_success,
        valid_evidence: observation
            .criteria
            .iter()
            .filter(|c| c.state == TaskDeliveryCriterionState::HostPassed)
            .count(),
        waivers: observation
            .criteria
            .iter()
            .filter(|c| c.state == TaskDeliveryCriterionState::Waived)
            .count(),
        delivery_allowed: decision.allowed,
        elapsed_ms,
        tool_calls: calls.len(),
        observed_tokens: usage.observed_tokens.map(|t| t.total_tokens),
        usage_gaps,
    }
}

#[test]
fn fixed_task_evaluation_replays_independent_metrics() {
    let baseline: Baseline = serde_json::from_str(include_str!("evaluation-v1.json")).unwrap();
    assert_eq!(baseline.version, 1);
    let mut ids = BTreeSet::new();
    assert!(baseline.cases.len() >= 3);
    for case in &baseline.cases {
        assert!(ids.insert(&case.id));
        let first = replay(case);
        assert_eq!(first, case.expected, "baseline drift: {}", case.id);
        assert_eq!(replay(case), first, "non-deterministic: {}", case.id);
        println!("{} {}", case.id, serde_json::to_string(&first).unwrap());
    }
}
