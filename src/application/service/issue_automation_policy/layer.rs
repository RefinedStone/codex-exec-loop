use serde::Deserialize;

use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::{
    ISSUE_AUTOMATION_POLICY_SCHEMA_VERSION, IssueAutomationAdditionalHumanRule,
    IssueAutomationCloseWhen, IssueAutomationGate, IssueAutomationGateAction,
    IssueAutomationGitHubBoard, IssueAutomationHumanRule, IssueAutomationJiraBinding,
    IssueAutomationJiraCloseTransition, IssueAutomationLinearBinding,
    IssueAutomationLinearCloseState, IssueAutomationPolicyValueError, IssueAutomationRiskRule,
    IssueAutomationSelectionMode, IssueAutomationTrustedActor,
    MAX_ISSUE_AUTOMATION_TIMEOUT_SECONDS, validate_gate_action, validate_glob, validate_repository,
    validate_text_value,
};

use super::{IssueAutomationPolicyBlocked, IssueAutomationPolicyScope};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicyLayer {
    pub schema_version: Option<u32>,
    pub binding: Option<BindingLayer>,
    pub authority: Option<AuthorityLayer>,
    pub labels: Option<LabelsLayer>,
    pub select: Option<SelectionLayer>,
    pub classify: Option<ClassificationLayer>,
    pub gates: Option<GatesLayer>,
    pub timeouts: Option<TimeoutsLayer>,
    pub budget: Option<BudgetLayer>,
    pub arbiter: Option<ArbiterLayer>,
    pub feedback: Option<FeedbackLayer>,
    pub close: Option<CloseLayer>,
    pub protection: Option<ProtectionLayer>,
    pub human_required: Option<HumanRequiredLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindingLayer {
    pub github: Option<GitHubBindingLayer>,
    pub jira: Option<JiraBindingLayer>,
    pub linear: Option<LinearBindingLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LinearBindingLayer {
    pub team_id: Option<String>,
    pub expected_viewer_id: Option<String>,
    pub mode: Option<IssueAutomationAutonomy>,
    pub close_state: Option<IssueAutomationLinearCloseState>,
}

impl LinearBindingLayer {
    pub(super) fn has_scope(&self) -> bool {
        self.team_id.is_some() || self.expected_viewer_id.is_some()
    }

    pub(super) fn binding(&self) -> IssueAutomationLinearBinding {
        IssueAutomationLinearBinding {
            team_id: self.team_id.clone().unwrap_or_default(),
            expected_viewer_id: self.expected_viewer_id.clone().unwrap_or_default(),
            mode: self.mode.unwrap_or(IssueAutomationAutonomy::Observe),
            close_state: None,
        }
    }
}

/// A project may narrow a user's state grant, but cannot move it to another team or account.
pub(super) fn validate_linear_layers(
    user: &PolicyLayer,
    project: &PolicyLayer,
) -> Result<(), IssueAutomationPolicyBlocked> {
    let user = user
        .binding
        .as_ref()
        .and_then(|binding| binding.linear.as_ref());
    let project = project
        .binding
        .as_ref()
        .and_then(|binding| binding.linear.as_ref());
    if let Some(user) = user.filter(|linear| linear.close_state.is_some()) {
        if !user.has_scope() {
            return Err(IssueAutomationPolicyBlocked::invalid(
                IssueAutomationPolicyScope::User,
                invalid(
                    "binding.linear.close_state",
                    "explicit_user_source_scope_required",
                ),
            ));
        }
        if let Some(project) = project.filter(|linear| linear.has_scope()) {
            let granted = user.binding();
            let requested = project.binding();
            if granted.team_id != requested.team_id
                || granted.expected_viewer_id != requested.expected_viewer_id
            {
                return Err(IssueAutomationPolicyBlocked::invalid(
                    IssueAutomationPolicyScope::Project,
                    invalid("binding.linear.close_state", "state_grant_scope_changed"),
                ));
            }
        }
    }
    if let Some(requested) = project.and_then(|linear| linear.close_state.as_ref()) {
        let granted = user.and_then(|linear| linear.close_state.as_ref());
        if granted.is_none_or(|granted| !requested.restricts(granted)) {
            return Err(IssueAutomationPolicyBlocked::invalid(
                IssueAutomationPolicyScope::Project,
                invalid(
                    "binding.linear.close_state",
                    "project_cannot_expand_state_grant",
                ),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JiraBindingLayer {
    pub site_url: Option<String>,
    pub cloud_id: Option<String>,
    pub project_id: Option<String>,
    pub expected_account_id: Option<String>,
    pub jql: Option<String>,
    pub mode: Option<IssueAutomationAutonomy>,
    pub close_transition: Option<IssueAutomationJiraCloseTransition>,
}

impl JiraBindingLayer {
    pub(super) fn has_scope(&self) -> bool {
        self.site_url.is_some()
            || self.cloud_id.is_some()
            || self.project_id.is_some()
            || self.expected_account_id.is_some()
            || self.jql.is_some()
    }

    pub(super) fn binding(&self) -> IssueAutomationJiraBinding {
        IssueAutomationJiraBinding {
            site_url: self.site_url.clone().unwrap_or_default(),
            cloud_id: self.cloud_id.clone().unwrap_or_default(),
            project_id: self.project_id.clone().unwrap_or_default(),
            expected_account_id: self.expected_account_id.clone().unwrap_or_default(),
            jql: self.jql.clone().unwrap_or_default(),
            mode: self.mode.unwrap_or(IssueAutomationAutonomy::Observe),
            close_transition: None,
        }
    }
}

/// A project may narrow a user's workflow grant, but cannot move it to another tenant/project.
pub(super) fn validate_jira_layers(
    user: &PolicyLayer,
    project: &PolicyLayer,
) -> Result<(), IssueAutomationPolicyBlocked> {
    let user = user
        .binding
        .as_ref()
        .and_then(|binding| binding.jira.as_ref());
    let project = project
        .binding
        .as_ref()
        .and_then(|binding| binding.jira.as_ref());
    if let Some(user) = user.filter(|jira| jira.close_transition.is_some()) {
        if !user.has_scope() {
            return Err(IssueAutomationPolicyBlocked::invalid(
                IssueAutomationPolicyScope::User,
                invalid(
                    "binding.jira.close_transition",
                    "explicit_user_source_scope_required",
                ),
            ));
        }
        if let Some(project) = project.filter(|jira| jira.has_scope()) {
            let granted = user.binding();
            let requested = project.binding();
            if granted.site_url != requested.site_url
                || granted.cloud_id != requested.cloud_id
                || granted.project_id != requested.project_id
                || granted.expected_account_id != requested.expected_account_id
            {
                return Err(IssueAutomationPolicyBlocked::invalid(
                    IssueAutomationPolicyScope::Project,
                    invalid(
                        "binding.jira.close_transition",
                        "transition_grant_scope_changed",
                    ),
                ));
            }
        }
    }
    if let Some(requested) = project.and_then(|jira| jira.close_transition.as_ref()) {
        let granted = user.and_then(|jira| jira.close_transition.as_ref());
        if granted.is_none_or(|granted| !requested.restricts(granted)) {
            return Err(IssueAutomationPolicyBlocked::invalid(
                IssueAutomationPolicyScope::Project,
                invalid(
                    "binding.jira.close_transition",
                    "project_cannot_expand_transition_grant",
                ),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GitHubBindingLayer {
    pub repository: Option<String>,
    pub mode: Option<IssueAutomationAutonomy>,
    /// 표 전체가 한 단위이다. 두 파일의 반쪽 좌표를 섞지 않는다.
    pub board: Option<IssueAutomationGitHubBoard>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuthorityLayer {
    pub autonomy_cap: Option<IssueAutomationAutonomy>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LabelsLayer {
    pub auto: Option<String>,
    pub gated: Option<String>,
    pub manual: Option<String>,
    pub hold: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SelectionLayer {
    pub mode: Option<IssueAutomationSelectionMode>,
    pub labels_any: Option<Vec<String>>,
    pub trusted_actors: Option<Vec<IssueAutomationTrustedActor>>,
    pub comment_commands: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClassificationLayer {
    pub rule: Option<Vec<IssueAutomationRiskRule>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GatesLayer {
    pub propose: Option<RiskGatesLayer>,
    pub gated: Option<RiskGatesLayer>,
    pub auto: Option<RiskGatesLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RiskGatesLayer {
    pub low: Option<GateSetLayer>,
    pub normal: Option<GateSetLayer>,
    pub high: Option<GateSetLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GateSetLayer {
    pub start: Option<IssueAutomationGateAction>,
    pub spec: Option<IssueAutomationGateAction>,
    pub plan: Option<IssueAutomationGateAction>,
    pub delivery: Option<IssueAutomationGateAction>,
    pub merge: Option<IssueAutomationGateAction>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TimeoutsLayer {
    pub start: Option<DurationSeconds>,
    pub remind_before: Option<DurationSeconds>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct DurationSeconds(pub u64);

impl TryFrom<String> for DurationSeconds {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let Some(unit) = value.as_bytes().last().copied() else {
            return Err("duration_unit_required");
        };
        let multiplier = match unit {
            b's' => 1,
            b'm' => 60,
            b'h' => 60 * 60,
            b'd' => 24 * 60 * 60,
            _ => return Err("invalid_duration_unit"),
        };
        let digits = &value[..value.len() - 1];
        if digits.is_empty()
            || !digits.bytes().all(|character| character.is_ascii_digit())
            || (digits.len() > 1 && digits.starts_with('0'))
        {
            return Err("unsigned_integer_duration_required");
        }
        let seconds = digits
            .parse::<u64>()
            .ok()
            .and_then(|number| number.checked_mul(multiplier))
            .filter(|seconds| *seconds <= MAX_ISSUE_AUTOMATION_TIMEOUT_SECONDS)
            .ok_or("duration_out_of_range")?;
        Ok(Self(seconds))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BudgetLayer {
    pub max_running: Option<u32>,
    pub max_started_per_day: Option<u32>,
    pub max_attempts: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArbiterLayer {
    pub min_confidence: Option<u16>,
    pub max_running: Option<u32>,
    pub max_started_per_day: Option<u32>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FeedbackLayer {
    pub comment: Option<bool>,
    pub labels: Option<bool>,
    pub transition: Option<bool>,
    pub telegram: Option<bool>,
    pub board_field: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CloseLayer {
    pub when: Option<IssueAutomationCloseWhen>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProtectionLayer {
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HumanRequiredLayer {
    pub rules: Option<Vec<IssueAutomationHumanRule>>,
    pub additional: Option<Vec<IssueAutomationAdditionalHumanRule>>,
}

pub(super) fn parse(
    contents: Option<&str>,
    scope: IssueAutomationPolicyScope,
) -> Result<PolicyLayer, IssueAutomationPolicyBlocked> {
    let Some(contents) = contents else {
        return Ok(PolicyLayer::default());
    };
    let layer: PolicyLayer =
        toml::from_str(contents).map_err(|_| IssueAutomationPolicyBlocked {
            scope,
            field: "policy",
            reason: "invalid_toml_or_shape",
        })?;
    validate(&layer).map_err(|error| IssueAutomationPolicyBlocked::invalid(scope, error))?;
    // 보드에 쓸 선택지는 쓰기 허가와 같이 사용자 파일에서만 온다. 저장소 파일은 열 조건만 정한다.
    if scope == IssueAutomationPolicyScope::Project
        && github_board(&layer).is_some_and(|board| board.progress.is_some())
    {
        return Err(IssueAutomationPolicyBlocked::invalid(
            scope,
            invalid("binding.github.board.progress", "user_scope_only"),
        ));
    }
    Ok(layer)
}

pub(super) fn github_board(layer: &PolicyLayer) -> Option<&IssueAutomationGitHubBoard> {
    layer
        .binding
        .as_ref()
        .and_then(|binding| binding.github.as_ref())
        .and_then(|github| github.board.as_ref())
}

/// 생략값을 내장값으로 채우지 않고, 파일에 명시한 값과 교차 조건만 먼저 검사한다.
/// 뒤의 파일이 덮어써도 앞의 잘못된 선언이 가려지지 않는다.
fn validate(layer: &PolicyLayer) -> Result<(), IssueAutomationPolicyValueError> {
    if let Some(board) = github_board(layer) {
        board.validate()?;
    }
    if let Some(jira) = layer
        .binding
        .as_ref()
        .and_then(|binding| binding.jira.as_ref())
    {
        // Source coordinates form one explicit selection; never combine half of two identities.
        if jira.has_scope() {
            jira.binding().validate()?;
        }
        if let Some(transition) = &jira.close_transition {
            transition.validate()?;
        }
    }
    if let Some(linear) = layer
        .binding
        .as_ref()
        .and_then(|binding| binding.linear.as_ref())
    {
        // The team and the account form one explicit selection; never combine two halves.
        if linear.has_scope() {
            linear.binding().validate()?;
        }
        if let Some(close) = &linear.close_state {
            close.validate()?;
        }
    }
    if let Some(arbiter) = &layer.arbiter {
        if arbiter.min_confidence.is_some_and(|value| value > 10_000) {
            return Err(invalid("arbiter.min_confidence", "confidence_out_of_range"));
        }
        if arbiter
            .timeout_seconds
            .is_some_and(|value| value == 0 || value > MAX_ISSUE_AUTOMATION_TIMEOUT_SECONDS)
        {
            return Err(invalid("arbiter.timeout_seconds", "duration_out_of_range"));
        }
    }
    if layer
        .schema_version
        .is_some_and(|version| version != ISSUE_AUTOMATION_POLICY_SCHEMA_VERSION)
    {
        return Err(invalid("schema_version", "unsupported_version"));
    }
    if let Some(repository) = layer
        .binding
        .as_ref()
        .and_then(|binding| binding.github.as_ref())
        .and_then(|github| github.repository.as_ref())
    {
        validate_repository(repository)?;
    }
    if let Some(labels) = &layer.labels {
        let mut seen = Vec::new();
        for (field, name) in [
            ("labels.auto", labels.auto.as_deref()),
            ("labels.gated", labels.gated.as_deref()),
            ("labels.manual", labels.manual.as_deref()),
            ("labels.hold", labels.hold.as_deref()),
        ] {
            if let Some(name) = name {
                validate_text_value(name, field)?;
                if seen.contains(&name) {
                    return Err(invalid(field, "duplicate_label_name"));
                }
                seen.push(name);
            }
        }
    }
    if let Some(selection) = &layer.select {
        if let Some(labels) = &selection.labels_any {
            for label in labels {
                validate_text_value(label, "select.labels_any")?;
            }
        }
        if let Some(actors) = &selection.trusted_actors {
            for actor in actors {
                validate_text_value(&actor.server_id, "select.trusted_actors.server_id")?;
                validate_text_value(&actor.actor_id, "select.trusted_actors.actor_id")?;
            }
        }
    }
    if let Some(rules) = layer
        .classify
        .as_ref()
        .and_then(|classify| classify.rule.as_ref())
    {
        for rule in rules {
            rule.validate()?;
        }
    }
    if let Some(gates) = &layer.gates {
        validate_gates(gates)?;
    }
    if let Some(timeouts) = &layer.timeouts {
        if timeouts.start.is_some_and(|duration| duration.0 == 0) {
            return Err(invalid("timeouts.start", "duration_out_of_range"));
        }
        if let (Some(start), Some(remind)) = (timeouts.start, timeouts.remind_before)
            && remind.0 >= start.0
        {
            return Err(invalid("timeouts.remind_before", "must_precede_start"));
        }
    }
    let transition = layer
        .feedback
        .as_ref()
        .and_then(|feedback| feedback.transition);
    let close = layer.close.as_ref().and_then(|close| close.when);
    if transition == Some(false) && close == Some(IssueAutomationCloseWhen::Integration) {
        return Err(invalid("close.when", "integration_requires_transition"));
    }
    if let Some(paths) = layer
        .protection
        .as_ref()
        .and_then(|protection| protection.paths.as_ref())
    {
        for path in paths {
            validate_glob(path, "protection.paths")?;
        }
    }
    if let Some(rules) = layer
        .human_required
        .as_ref()
        .and_then(|human| human.additional.as_ref())
    {
        for rule in rules {
            rule.validate()?;
        }
    }
    Ok(())
}

fn validate_gates(gates: &GatesLayer) -> Result<(), IssueAutomationPolicyValueError> {
    for (mode, risks) in [
        (IssueAutomationAutonomy::Propose, &gates.propose),
        (IssueAutomationAutonomy::Gated, &gates.gated),
        (IssueAutomationAutonomy::Auto, &gates.auto),
    ] {
        let Some(risks) = risks else {
            continue;
        };
        for risk in [&risks.low, &risks.normal, &risks.high]
            .into_iter()
            .flatten()
        {
            for (gate, action) in [
                (IssueAutomationGate::Start, risk.start),
                (IssueAutomationGate::Spec, risk.spec),
                (IssueAutomationGate::Plan, risk.plan),
                (IssueAutomationGate::Delivery, risk.delivery),
                (IssueAutomationGate::Merge, risk.merge),
            ] {
                if let Some(action) = action {
                    validate_gate_action(mode, gate, action)?;
                }
            }
        }
    }
    Ok(())
}

fn invalid(field: &'static str, reason: &'static str) -> IssueAutomationPolicyValueError {
    IssueAutomationPolicyValueError::new(field, reason)
}
