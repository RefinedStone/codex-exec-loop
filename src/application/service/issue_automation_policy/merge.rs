use crate::domain::issue_automation::policy::{
    IssueAutomationGateSet, IssueAutomationPolicy, IssueAutomationPolicyGates,
    IssueAutomationRiskGates,
};

use super::layer::{GateSetLayer, GatesLayer, PolicyLayer, RiskGatesLayer};
use super::provenance::PolicyProvenance;
use super::{
    IssueAutomationPolicyOriginScope as Scope, IssueAutomationPolicyResolution as Resolution,
};

pub(super) fn merge(
    user: PolicyLayer,
    project: PolicyLayer,
) -> (IssueAutomationPolicy, PolicyProvenance) {
    let selection_explicit = user
        .select
        .as_ref()
        .is_some_and(|select| select.labels_any.is_some())
        || project
            .select
            .as_ref()
            .is_some_and(|select| select.labels_any.is_some());
    let mut policy = IssueAutomationPolicy::builtin();
    let mut origins = PolicyProvenance::default();
    apply_common(&mut policy, &user, Scope::User, &mut origins);
    apply_user_grants(&mut policy, &user, &mut origins);
    apply_common(&mut policy, &project, Scope::Project, &mut origins);
    restrict_with_project(&mut policy, &project, &mut origins);
    // 어느 layer도 명시하지 않았을 때만 최종 실행 라벨로 선택 기본값을 만든다.
    // 명시한 빈 목록과 이름 변경 전의 명시 목록은 그대로 유지한다.
    if !selection_explicit {
        policy.select.labels_any = policy.labels.selection_labels();
        origins.derive(
            "select.labels_any",
            &["labels.auto", "labels.gated", "labels.manual"],
        );
    }
    // normalize()가 수행하는 동일 min의 두 입력 출처를 보존한다.
    origins.cap("binding.github.mode", "authority.autonomy_cap");
    (policy, origins)
}

/// 일반 leaf는 project > user > builtin이다. 보호와 사람 필수 조건은 항상 더한다.
fn apply_common(
    policy: &mut IssueAutomationPolicy,
    layer: &PolicyLayer,
    scope: Scope,
    origins: &mut PolicyProvenance,
) {
    assign(
        &mut policy.schema_version,
        layer.schema_version,
        "schema_version",
        scope,
        origins,
    );
    if let Some(github) = layer
        .binding
        .as_ref()
        .and_then(|binding| binding.github.as_ref())
    {
        if let Some(repository) = &github.repository {
            policy.binding.github.repository = Some(repository.clone());
            origins.replace("binding.github.repository", scope);
        }
        assign(
            &mut policy.binding.github.mode,
            github.mode,
            "binding.github.mode",
            scope,
            origins,
        );
    }
    if let Some(labels) = &layer.labels {
        for (target, value, key) in [
            (&mut policy.labels.auto, &labels.auto, "labels.auto"),
            (&mut policy.labels.gated, &labels.gated, "labels.gated"),
            (&mut policy.labels.manual, &labels.manual, "labels.manual"),
            (&mut policy.labels.hold, &labels.hold, "labels.hold"),
        ] {
            if let Some(value) = value {
                target.clone_from(value);
                origins.replace(key, scope);
            }
        }
    }
    if let Some(selection) = &layer.select {
        assign(
            &mut policy.select.mode,
            selection.mode,
            "select.mode",
            scope,
            origins,
        );
        if let Some(labels) = &selection.labels_any {
            policy.select.labels_any = labels.clone();
            origins.replace("select.labels_any", scope);
        }
    }
    if let Some(rules) = layer
        .classify
        .as_ref()
        .and_then(|classify| classify.rule.as_ref())
    {
        policy.classify.rule = rules.clone();
        origins.replace("classify.rule", scope);
    }
    if let Some(gates) = &layer.gates {
        apply_gates(&mut policy.gates, gates, scope, origins);
    }
    if let Some(timeouts) = &layer.timeouts {
        assign(
            &mut policy.timeouts.start_seconds,
            timeouts.start.map(|value| value.0),
            "timeouts.start",
            scope,
            origins,
        );
        assign(
            &mut policy.timeouts.remind_before_seconds,
            timeouts.remind_before.map(|value| value.0),
            "timeouts.remind_before",
            scope,
            origins,
        );
    }
    assign(
        &mut policy.close.when,
        layer.close.as_ref().and_then(|close| close.when),
        "close.when",
        scope,
        origins,
    );
    if let Some(paths) = layer
        .protection
        .as_ref()
        .and_then(|protection| protection.paths.as_ref())
    {
        policy.protection.paths.extend(paths.iter().cloned());
        origins.combine("protection.paths", scope, Resolution::Union);
    }
    if let Some(human) = &layer.human_required {
        if let Some(rules) = &human.rules {
            policy.human_required.rules.extend(rules.iter().copied());
            origins.combine("human_required.rules", scope, Resolution::Union);
        }
        if let Some(additional) = &human.additional {
            policy
                .human_required
                .additional
                .extend(additional.iter().cloned());
            origins.combine("human_required.additional", scope, Resolution::Union);
        }
    }
}

/// 권한 확대는 사용자 파일에서만 온다. 누락은 보수적인 내장 grant를 유지한다.
fn apply_user_grants(
    policy: &mut IssueAutomationPolicy,
    user: &PolicyLayer,
    origins: &mut PolicyProvenance,
) {
    assign(
        &mut policy.authority.autonomy_cap,
        user.authority
            .as_ref()
            .and_then(|authority| authority.autonomy_cap),
        "authority.autonomy_cap",
        Scope::User,
        origins,
    );
    if let Some(budget) = &user.budget {
        for (key, target, value) in [
            (
                "budget.max_running",
                &mut policy.budget.max_running,
                budget.max_running,
            ),
            (
                "budget.max_started_per_day",
                &mut policy.budget.max_started_per_day,
                budget.max_started_per_day,
            ),
            (
                "budget.max_attempts",
                &mut policy.budget.max_attempts,
                budget.max_attempts,
            ),
        ] {
            assign(target, value, key, Scope::User, origins);
        }
    }
    if let Some(feedback) = &user.feedback {
        for (key, target, value) in [
            (
                "feedback.comment",
                &mut policy.feedback.comment,
                feedback.comment,
            ),
            (
                "feedback.labels",
                &mut policy.feedback.labels,
                feedback.labels,
            ),
            (
                "feedback.transition",
                &mut policy.feedback.transition,
                feedback.transition,
            ),
        ] {
            assign(target, value, key, Scope::User, origins);
        }
    }
    if let Some(actors) = user
        .select
        .as_ref()
        .and_then(|selection| selection.trusted_actors.as_ref())
    {
        policy.select.trusted_actors = actors.clone();
        origins.replace("select.trusted_actors", Scope::User);
    }
}

/// 프로젝트는 cap/budget의 min, feedback의 AND, trusted actor의 교집합만 만든다.
fn restrict_with_project(
    policy: &mut IssueAutomationPolicy,
    project: &PolicyLayer,
    origins: &mut PolicyProvenance,
) {
    if let Some(cap) = project
        .authority
        .as_ref()
        .and_then(|authority| authority.autonomy_cap)
    {
        policy.authority.autonomy_cap = policy.authority.autonomy_cap.min(cap);
        origins.combine(
            "authority.autonomy_cap",
            Scope::Project,
            Resolution::Minimum,
        );
    }
    if let Some(budget) = &project.budget {
        for (key, target, value) in [
            (
                "budget.max_running",
                &mut policy.budget.max_running,
                budget.max_running,
            ),
            (
                "budget.max_started_per_day",
                &mut policy.budget.max_started_per_day,
                budget.max_started_per_day,
            ),
            (
                "budget.max_attempts",
                &mut policy.budget.max_attempts,
                budget.max_attempts,
            ),
        ] {
            if let Some(value) = value {
                *target = (*target).min(value);
                origins.combine(key, Scope::Project, Resolution::Minimum);
            }
        }
    }
    if let Some(feedback) = &project.feedback {
        for (key, target, value) in [
            (
                "feedback.comment",
                &mut policy.feedback.comment,
                feedback.comment,
            ),
            (
                "feedback.labels",
                &mut policy.feedback.labels,
                feedback.labels,
            ),
            (
                "feedback.transition",
                &mut policy.feedback.transition,
                feedback.transition,
            ),
        ] {
            if let Some(value) = value {
                *target &= value;
                origins.combine(key, Scope::Project, Resolution::And);
            }
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
        origins.combine(
            "select.trusted_actors",
            Scope::Project,
            Resolution::Intersection,
        );
    }
}

fn apply_gates(
    policy: &mut IssueAutomationPolicyGates,
    layer: &GatesLayer,
    scope: Scope,
    origins: &mut PolicyProvenance,
) {
    for (mode, target, values) in [
        ("propose", &mut policy.propose, &layer.propose),
        ("gated", &mut policy.gated, &layer.gated),
        ("auto", &mut policy.auto, &layer.auto),
    ] {
        if let Some(values) = values {
            apply_risks(target, values, mode, scope, origins);
        }
    }
}

fn apply_risks(
    policy: &mut IssueAutomationRiskGates,
    layer: &RiskGatesLayer,
    mode: &str,
    scope: Scope,
    origins: &mut PolicyProvenance,
) {
    for (risk, target, values) in [
        ("low", &mut policy.low, &layer.low),
        ("normal", &mut policy.normal, &layer.normal),
        ("high", &mut policy.high, &layer.high),
    ] {
        if let Some(values) = values {
            apply_gate_set(
                target,
                values,
                &format!("gates.{mode}.{risk}"),
                scope,
                origins,
            );
        }
    }
}

fn apply_gate_set(
    policy: &mut IssueAutomationGateSet,
    layer: &GateSetLayer,
    prefix: &str,
    scope: Scope,
    origins: &mut PolicyProvenance,
) {
    for (gate, target, value) in [
        ("start", &mut policy.start, layer.start),
        ("spec", &mut policy.spec, layer.spec),
        ("plan", &mut policy.plan, layer.plan),
        ("delivery", &mut policy.delivery, layer.delivery),
        ("merge", &mut policy.merge, layer.merge),
    ] {
        assign(target, value, &format!("{prefix}.{gate}"), scope, origins);
    }
}

fn assign<T>(
    target: &mut T,
    value: Option<T>,
    key: &str,
    scope: Scope,
    origins: &mut PolicyProvenance,
) {
    if let Some(value) = value {
        *target = value;
        origins.replace(key, scope);
    }
}
