use crate::domain::issue_automation::policy::{
    IssueAutomationGateSet, IssueAutomationGitHubBoard, IssueAutomationPolicy,
    IssueAutomationPolicyGates, IssueAutomationRiskGates,
};

use super::layer::{GateSetLayer, GatesLayer, PolicyLayer, RiskGatesLayer, github_board};
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
    // 사용자 파일의 Jira 좌표는 허가 범위일 뿐이다. 저장소 파일이 binding.jira를 선택해야 연결한다.
    let jira_selected = project
        .binding
        .as_ref()
        .is_some_and(|binding| binding.jira.is_some());
    // Linear도 같다. 사용자 파일의 팀 좌표는 저장소 파일이 binding.linear를 선택해야 연결한다.
    let linear_selected = project
        .binding
        .as_ref()
        .is_some_and(|binding| binding.linear.is_some());
    let mut policy = IssueAutomationPolicy::builtin();
    let mut origins = PolicyProvenance::default();
    let user_selection = SourceSelection {
        jira: jira_selected,
        linear: linear_selected,
    };
    let project_selection = SourceSelection {
        jira: true,
        linear: true,
    };
    apply_common(
        &mut policy,
        &user,
        Scope::User,
        user_selection,
        &mut origins,
    );
    apply_user_grants(&mut policy, &user, &mut origins);
    apply_common(
        &mut policy,
        &project,
        Scope::Project,
        project_selection,
        &mut origins,
    );
    restrict_with_project(&mut policy, &project, &mut origins);
    grant_board_progress(&mut policy, &user, &mut origins);
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
    if policy.binding.jira.is_some() {
        origins.cap("binding.jira.mode", "authority.autonomy_cap");
    }
    if policy.binding.linear.is_some() {
        origins.cap("binding.linear.mode", "authority.autonomy_cap");
    }
    (policy, origins)
}

/// Which connected-source coordinates of a layer may be applied.
#[derive(Clone, Copy)]
struct SourceSelection {
    jira: bool,
    linear: bool,
}

/// 일반 leaf는 project > user > builtin이다. 보호와 사람 필수 조건은 항상 더한다.
fn apply_common(
    policy: &mut IssueAutomationPolicy,
    layer: &PolicyLayer,
    scope: Scope,
    selection: SourceSelection,
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
        if let Some(board) = &github.board {
            // 진행 선택지는 grant_board_progress 만 붙인다.
            policy.binding.github.board = Some(IssueAutomationGitHubBoard {
                progress: None,
                ..board.clone()
            });
            origins.replace("binding.github.board", scope);
        }
    }
    if let Some(jira) = layer
        .binding
        .as_ref()
        .and_then(|binding| binding.jira.as_ref())
        .filter(|_| selection.jira)
    {
        let selected = policy.binding.jira.get_or_insert_with(|| jira.binding());
        if jira.has_scope() {
            let scope_values = jira.binding();
            selected.site_url = scope_values.site_url;
            selected.cloud_id = scope_values.cloud_id;
            selected.project_id = scope_values.project_id;
            selected.expected_account_id = scope_values.expected_account_id;
            selected.jql = scope_values.jql;
            for key in [
                "site_url",
                "cloud_id",
                "project_id",
                "expected_account_id",
                "jql",
            ] {
                origins.replace(&format!("binding.jira.{key}"), scope);
            }
        }
        assign(
            &mut selected.mode,
            jira.mode,
            "binding.jira.mode",
            scope,
            origins,
        );
    }
    if let Some(linear) = layer
        .binding
        .as_ref()
        .and_then(|binding| binding.linear.as_ref())
        .filter(|_| selection.linear)
    {
        let selected = policy
            .binding
            .linear
            .get_or_insert_with(|| linear.binding());
        if linear.has_scope() {
            let scope_values = linear.binding();
            selected.team_id = scope_values.team_id;
            selected.expected_viewer_id = scope_values.expected_viewer_id;
            for key in ["team_id", "expected_viewer_id"] {
                origins.replace(&format!("binding.linear.{key}"), scope);
            }
        }
        assign(
            &mut selected.mode,
            linear.mode,
            "binding.linear.mode",
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
    if let Some(transition) = user
        .binding
        .as_ref()
        .and_then(|binding| binding.jira.as_ref())
        .and_then(|jira| jira.close_transition.clone())
        && let Some(jira) = &mut policy.binding.jira
    {
        jira.close_transition = Some(transition);
        origins.replace("binding.jira.close_transition", Scope::User);
    }
    if let Some(close) = user
        .binding
        .as_ref()
        .and_then(|binding| binding.linear.as_ref())
        .and_then(|linear| linear.close_state.clone())
        && let Some(linear) = &mut policy.binding.linear
    {
        linear.close_state = Some(close);
        origins.replace("binding.linear.close_state", Scope::User);
    }
    if let Some(arbiter) = &user.arbiter {
        assign(
            &mut policy.arbiter.min_confidence,
            arbiter.min_confidence,
            "arbiter.min_confidence",
            Scope::User,
            origins,
        );
        assign(
            &mut policy.arbiter.max_running,
            arbiter.max_running,
            "arbiter.max_running",
            Scope::User,
            origins,
        );
        assign(
            &mut policy.arbiter.max_started_per_day,
            arbiter.max_started_per_day,
            "arbiter.max_started_per_day",
            Scope::User,
            origins,
        );
        assign(
            &mut policy.arbiter.timeout_seconds,
            arbiter.timeout_seconds,
            "arbiter.timeout_seconds",
            Scope::User,
            origins,
        );
    }
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
            (
                "feedback.telegram",
                &mut policy.feedback.telegram,
                feedback.telegram,
            ),
            (
                "feedback.board_field",
                &mut policy.feedback.board_field,
                feedback.board_field,
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
    assign(
        &mut policy.select.comment_commands,
        user.select
            .as_ref()
            .and_then(|selection| selection.comment_commands),
        "select.comment_commands",
        Scope::User,
        origins,
    );
}

/// 프로젝트는 cap/budget의 min, feedback의 AND, trusted actor의 교집합만 만든다.
fn restrict_with_project(
    policy: &mut IssueAutomationPolicy,
    project: &PolicyLayer,
    origins: &mut PolicyProvenance,
) {
    if let Some(transition) = project
        .binding
        .as_ref()
        .and_then(|binding| binding.jira.as_ref())
        .and_then(|jira| jira.close_transition.clone())
        && let Some(jira) = &mut policy.binding.jira
    {
        // Cross-layer validation has established the exact target and subset relationship.
        jira.close_transition = Some(transition);
        origins.combine(
            "binding.jira.close_transition",
            Scope::Project,
            Resolution::Intersection,
        );
    }
    if let Some(close) = project
        .binding
        .as_ref()
        .and_then(|binding| binding.linear.as_ref())
        .and_then(|linear| linear.close_state.clone())
        && let Some(linear) = &mut policy.binding.linear
    {
        // Cross-layer validation has established the exact target and subset relationship.
        linear.close_state = Some(close);
        origins.combine(
            "binding.linear.close_state",
            Scope::Project,
            Resolution::Intersection,
        );
    }
    if let Some(arbiter) = &project.arbiter {
        if let Some(value) = arbiter.min_confidence {
            policy.arbiter.min_confidence = policy.arbiter.min_confidence.max(value);
            origins.combine(
                "arbiter.min_confidence",
                Scope::Project,
                Resolution::Maximum,
            );
        }
        for (key, target, value) in [
            (
                "arbiter.max_running",
                &mut policy.arbiter.max_running,
                arbiter.max_running,
            ),
            (
                "arbiter.max_started_per_day",
                &mut policy.arbiter.max_started_per_day,
                arbiter.max_started_per_day,
            ),
        ] {
            if let Some(value) = value {
                *target = (*target).min(value);
                origins.combine(key, Scope::Project, Resolution::Minimum);
            }
        }
        if let Some(value) = arbiter.timeout_seconds {
            policy.arbiter.timeout_seconds = policy.arbiter.timeout_seconds.min(value);
            origins.combine(
                "arbiter.timeout_seconds",
                Scope::Project,
                Resolution::Minimum,
            );
        }
    }
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
            (
                "feedback.telegram",
                &mut policy.feedback.telegram,
                feedback.telegram,
            ),
            (
                "feedback.board_field",
                &mut policy.feedback.board_field,
                feedback.board_field,
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
    if let Some(enabled) = project
        .select
        .as_ref()
        .and_then(|selection| selection.comment_commands)
    {
        policy.select.comment_commands &= enabled;
        origins.combine("select.comment_commands", Scope::Project, Resolution::And);
    }
}

/// 사용자 파일이 같은 보드의 같은 필드를 가리킬 때만 진행 선택지를 붙인다.
/// 저장소 파일이 보드를 바꾸면 사용자가 허가한 선택지는 따라가지 않는다.
fn grant_board_progress(
    policy: &mut IssueAutomationPolicy,
    user: &PolicyLayer,
    origins: &mut PolicyProvenance,
) {
    let Some(user_board) = github_board(user) else {
        return;
    };
    if let Some(board) = &mut policy.binding.github.board
        && board.same_field(user_board)
        && user_board.progress.is_some()
    {
        board.progress = user_board.progress.clone();
        origins.combine(
            "binding.github.board",
            Scope::User,
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
