use crate::domain::issue_automation::policy::{
    IssueAutomationGateSet, IssueAutomationPolicy, IssueAutomationPolicyGates,
    IssueAutomationRiskGates,
};

use super::layer::{GateSetLayer, GatesLayer, PolicyLayer, RiskGatesLayer};

pub(super) fn merge(user: PolicyLayer, project: PolicyLayer) -> IssueAutomationPolicy {
    let mut policy = IssueAutomationPolicy::builtin();
    apply_common(&mut policy, &user);
    apply_user_grants(&mut policy, &user);
    apply_common(&mut policy, &project);
    restrict_with_project(&mut policy, &project);
    policy
}

/// 일반 leaf는 project > user > builtin이다. 보호와 사람 필수 조건은 항상 더한다.
fn apply_common(policy: &mut IssueAutomationPolicy, layer: &PolicyLayer) {
    if let Some(github) = layer
        .binding
        .as_ref()
        .and_then(|binding| binding.github.as_ref())
    {
        if let Some(repository) = &github.repository {
            policy.binding.github.repository = Some(repository.clone());
        }
        if let Some(mode) = github.mode {
            policy.binding.github.mode = mode;
        }
    }
    if let Some(selection) = &layer.select {
        if let Some(mode) = selection.mode {
            policy.select.mode = mode;
        }
        if let Some(labels) = &selection.labels_any {
            policy.select.labels_any = labels.clone();
        }
    }
    if let Some(rules) = layer
        .classify
        .as_ref()
        .and_then(|classify| classify.rule.as_ref())
    {
        policy.classify.rule = rules.clone();
    }
    if let Some(gates) = &layer.gates {
        apply_gates(&mut policy.gates, gates);
    }
    if let Some(timeouts) = &layer.timeouts {
        if let Some(start) = timeouts.start {
            policy.timeouts.start_seconds = start.0;
        }
        if let Some(remind) = timeouts.remind_before {
            policy.timeouts.remind_before_seconds = remind.0;
        }
    }
    if let Some(close) = layer.close.as_ref().and_then(|close| close.when) {
        policy.close.when = close;
    }
    if let Some(paths) = layer
        .protection
        .as_ref()
        .and_then(|protection| protection.paths.as_ref())
    {
        policy.protection.paths.extend(paths.iter().cloned());
    }
    if let Some(human) = &layer.human_required {
        if let Some(rules) = &human.rules {
            policy.human_required.rules.extend(rules.iter().copied());
        }
        if let Some(additional) = &human.additional {
            policy
                .human_required
                .additional
                .extend(additional.iter().cloned());
        }
    }
}

/// 권한 확대는 사용자 파일에서만 온다. 누락은 보수적인 내장 grant를 유지한다.
fn apply_user_grants(policy: &mut IssueAutomationPolicy, user: &PolicyLayer) {
    if let Some(cap) = user
        .authority
        .as_ref()
        .and_then(|authority| authority.autonomy_cap)
    {
        policy.authority.autonomy_cap = cap;
    }
    if let Some(budget) = &user.budget {
        if let Some(value) = budget.max_running {
            policy.budget.max_running = value;
        }
        if let Some(value) = budget.max_started_per_day {
            policy.budget.max_started_per_day = value;
        }
        if let Some(value) = budget.max_attempts {
            policy.budget.max_attempts = value;
        }
    }
    if let Some(feedback) = &user.feedback {
        if let Some(value) = feedback.comment {
            policy.feedback.comment = value;
        }
        if let Some(value) = feedback.labels {
            policy.feedback.labels = value;
        }
        if let Some(value) = feedback.transition {
            policy.feedback.transition = value;
        }
    }
    if let Some(actors) = user
        .select
        .as_ref()
        .and_then(|selection| selection.trusted_actors.as_ref())
    {
        policy.select.trusted_actors = actors.clone();
    }
}

/// 프로젝트는 cap/budget의 min, feedback의 AND, trusted actor의 교집합만 만든다.
fn restrict_with_project(policy: &mut IssueAutomationPolicy, project: &PolicyLayer) {
    if let Some(cap) = project
        .authority
        .as_ref()
        .and_then(|authority| authority.autonomy_cap)
    {
        policy.authority.autonomy_cap = policy.authority.autonomy_cap.min(cap);
    }
    if let Some(budget) = &project.budget {
        if let Some(value) = budget.max_running {
            policy.budget.max_running = policy.budget.max_running.min(value);
        }
        if let Some(value) = budget.max_started_per_day {
            policy.budget.max_started_per_day = policy.budget.max_started_per_day.min(value);
        }
        if let Some(value) = budget.max_attempts {
            policy.budget.max_attempts = policy.budget.max_attempts.min(value);
        }
    }
    if let Some(feedback) = &project.feedback {
        if let Some(value) = feedback.comment {
            policy.feedback.comment &= value;
        }
        if let Some(value) = feedback.labels {
            policy.feedback.labels &= value;
        }
        if let Some(value) = feedback.transition {
            policy.feedback.transition &= value;
        }
    }
    if let Some(actors) = project
        .select
        .as_ref()
        .and_then(|selection| selection.trusted_actors.as_ref())
    {
        policy
            .select
            .trusted_actors
            .retain(|actor| actors.contains(actor));
    }
}

fn apply_gates(policy: &mut IssueAutomationPolicyGates, layer: &GatesLayer) {
    if let Some(propose) = &layer.propose {
        apply_risks(&mut policy.propose, propose);
    }
    if let Some(gated) = &layer.gated {
        apply_risks(&mut policy.gated, gated);
    }
    if let Some(auto) = &layer.auto {
        apply_risks(&mut policy.auto, auto);
    }
}

fn apply_risks(policy: &mut IssueAutomationRiskGates, layer: &RiskGatesLayer) {
    if let Some(low) = &layer.low {
        apply_gate_set(&mut policy.low, low);
    }
    if let Some(normal) = &layer.normal {
        apply_gate_set(&mut policy.normal, normal);
    }
    if let Some(high) = &layer.high {
        apply_gate_set(&mut policy.high, high);
    }
}

fn apply_gate_set(policy: &mut IssueAutomationGateSet, layer: &GateSetLayer) {
    if let Some(value) = layer.start {
        policy.start = value;
    }
    if let Some(value) = layer.spec {
        policy.spec = value;
    }
    if let Some(value) = layer.plan {
        policy.plan = value;
    }
    if let Some(value) = layer.delivery {
        policy.delivery = value;
    }
    if let Some(value) = layer.merge {
        policy.merge = value;
    }
}
