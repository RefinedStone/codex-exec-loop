use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueAutomationPolicyValueError {
    pub field: &'static str,
    pub reason: &'static str,
}

impl IssueAutomationPolicyValueError {
    pub const fn new(field: &'static str, reason: &'static str) -> Self {
        Self { field, reason }
    }
}

impl std::fmt::Display for IssueAutomationPolicyValueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for IssueAutomationPolicyValueError {}

pub(super) fn validate_policy(
    policy: &IssueAutomationPolicy,
) -> Result<(), IssueAutomationPolicyValueError> {
    if policy.schema_version != ISSUE_AUTOMATION_POLICY_SCHEMA_VERSION {
        return Err(invalid("schema_version", "unsupported_version"));
    }
    policy
        .arbiter
        .validate()
        .map_err(|_| invalid("arbiter", "invalid_arbiter_settings"))?;
    if let Some(repository) = &policy.binding.github.repository {
        validate_repository(repository)?;
    }
    if let Some(board) = &policy.binding.github.board {
        board.validate()?;
    }
    if policy.binding.github.mode > policy.authority.autonomy_cap {
        return Err(invalid("binding.github.mode", "exceeds_autonomy_cap"));
    }
    if let Some(jira) = &policy.binding.jira {
        jira.validate()?;
        if jira.mode > policy.authority.autonomy_cap {
            return Err(invalid("binding.jira.mode", "exceeds_autonomy_cap"));
        }
        if policy.feedback.transition && jira.close_transition.is_none() {
            return Err(invalid(
                "binding.jira.close_transition",
                "explicit_user_transition_required",
            ));
        }
        if policy.feedback.labels {
            return Err(invalid("feedback.labels", "jira_label_writes_unsupported"));
        }
        if policy.close.when == IssueAutomationCloseWhen::ProviderLink {
            return Err(invalid(
                "close.when",
                "jira_provider_link_closure_unsupported",
            ));
        }
    }
    if let Some(linear) = &policy.binding.linear {
        if policy.binding.jira.is_some() {
            return Err(invalid(
                "binding.linear",
                "one_issue_source_binding_required",
            ));
        }
        linear.validate()?;
        if linear.mode > policy.authority.autonomy_cap {
            return Err(invalid("binding.linear.mode", "exceeds_autonomy_cap"));
        }
        if policy.feedback.transition && linear.close_state.is_none() {
            return Err(invalid(
                "binding.linear.close_state",
                "explicit_user_state_change_required",
            ));
        }
        if policy.feedback.labels {
            return Err(invalid(
                "feedback.labels",
                "linear_label_writes_unsupported",
            ));
        }
        if policy.close.when == IssueAutomationCloseWhen::ProviderLink {
            return Err(invalid(
                "close.when",
                "linear_provider_link_closure_unsupported",
            ));
        }
    }
    validate_labels(&policy.labels)?;
    for label in &policy.select.labels_any {
        validate_text_value(label, "select.labels_any")?;
    }
    for actor in &policy.select.trusted_actors {
        validate_text_value(&actor.server_id, "select.trusted_actors.server_id")?;
        validate_text_value(&actor.actor_id, "select.trusted_actors.actor_id")?;
    }
    for rule in &policy.classify.rule {
        rule.validate()?;
    }
    for (mode, risks) in [
        (IssueAutomationAutonomy::Propose, &policy.gates.propose),
        (IssueAutomationAutonomy::Gated, &policy.gates.gated),
        (IssueAutomationAutonomy::Auto, &policy.gates.auto),
    ] {
        for gates in [&risks.low, &risks.normal, &risks.high] {
            for (gate, action) in [
                (IssueAutomationGate::Start, gates.start),
                (IssueAutomationGate::Spec, gates.spec),
                (IssueAutomationGate::Plan, gates.plan),
                (IssueAutomationGate::Delivery, gates.delivery),
                (IssueAutomationGate::Merge, gates.merge),
            ] {
                validate_gate_action(mode, gate, action)?;
            }
        }
    }
    if policy.timeouts.start_seconds == 0
        || policy.timeouts.start_seconds > MAX_ISSUE_AUTOMATION_TIMEOUT_SECONDS
    {
        return Err(invalid("timeouts.start", "duration_out_of_range"));
    }
    if policy.timeouts.remind_before_seconds >= policy.timeouts.start_seconds {
        return Err(invalid("timeouts.remind_before", "must_precede_start"));
    }
    if policy.close.when == IssueAutomationCloseWhen::Integration && !policy.feedback.transition {
        return Err(invalid("close.when", "integration_requires_transition"));
    }
    for path in &policy.protection.paths {
        validate_glob(path, "protection.paths")?;
    }
    for builtin in BUILTIN_ISSUE_PROTECTION_PATHS {
        if !policy.protection.paths.iter().any(|path| path == builtin) {
            return Err(invalid("protection.paths", "builtin_protection_required"));
        }
    }
    if !IssueAutomationHumanRule::ALL
        .iter()
        .all(|rule| policy.human_required.rules.contains(rule))
    {
        return Err(invalid(
            "human_required.rules",
            "builtin_human_rules_required",
        ));
    }
    for rule in &policy.human_required.additional {
        rule.validate()?;
    }
    Ok(())
}

pub(super) fn validate_labels(
    labels: &IssueAutomationLabels,
) -> Result<(), IssueAutomationPolicyValueError> {
    let entries = [
        ("labels.auto", labels.auto.as_str()),
        ("labels.gated", labels.gated.as_str()),
        ("labels.manual", labels.manual.as_str()),
        ("labels.hold", labels.hold.as_str()),
    ];
    let mut seen = Vec::new();
    for (field, name) in entries {
        validate_text_value(name, field)?;
        if seen.contains(&name) {
            return Err(invalid(field, "duplicate_label_name"));
        }
        seen.push(name);
    }
    Ok(())
}

impl IssueAutomationRiskRule {
    pub fn validate(&self) -> Result<(), IssueAutomationPolicyValueError> {
        validate_conditions(&self.labels_any, &self.paths_any, "classify.rule")
    }
}

impl IssueAutomationAdditionalHumanRule {
    pub fn validate(&self) -> Result<(), IssueAutomationPolicyValueError> {
        validate_conditions(
            &self.labels_any,
            &self.paths_any,
            "human_required.additional",
        )?;
        if self.gates.is_empty() {
            return Err(invalid("human_required.additional.gates", "gate_required"));
        }
        Ok(())
    }
}

fn validate_conditions(
    labels: &[String],
    paths: &[String],
    field: &'static str,
) -> Result<(), IssueAutomationPolicyValueError> {
    if labels.is_empty() && paths.is_empty() {
        return Err(invalid(field, "condition_required"));
    }
    for label in labels {
        validate_text_value(label, field)?;
    }
    for path in paths {
        validate_glob(path, field)?;
    }
    Ok(())
}

pub(crate) fn validate_gate_action(
    mode: IssueAutomationAutonomy,
    gate: IssueAutomationGate,
    action: IssueAutomationGateAction,
) -> Result<(), IssueAutomationPolicyValueError> {
    use IssueAutomationAutonomy::{Gated, Propose};
    use IssueAutomationGate::{Delivery, Merge, Plan, Spec, Start};
    use IssueAutomationGateAction::{Auto, Human, TimeoutArbiter};

    let allowed = match gate {
        Spec => action == Human,
        Plan => matches!(action, Human | TimeoutArbiter),
        Merge => matches!(action, Human | Auto),
        Start | Delivery => true,
        IssueAutomationGate::Close => false,
    };
    let mode_requires_human = (mode == Propose && gate == Start)
        || (matches!(mode, Propose | Gated) && matches!(gate, Delivery | Merge));
    if !allowed || (mode_requires_human && action != Human) {
        return Err(invalid("gates", "action_not_allowed_for_gate_or_mode"));
    }
    Ok(())
}

pub(crate) fn validate_text_value(
    value: &str,
    field: &'static str,
) -> Result<(), IssueAutomationPolicyValueError> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(invalid(field, "nonempty_unpadded_text_required"));
    }
    Ok(())
}

/// 문법만 검증한다. **는 독립 segment이며 0개 이상의 경로 segment를 뜻한다.
/// *와 ?는 하나의 segment 안에서만 쓰고, [], {}, !, escape 문법은 지원하지 않는다.
pub(crate) fn validate_glob(
    value: &str,
    field: &'static str,
) -> Result<(), IssueAutomationPolicyValueError> {
    validate_text_value(value, field)?;
    if value
        .chars()
        .any(|character| matches!(character, '\\' | ':' | '[' | ']' | '{' | '}' | '!'))
    {
        return Err(invalid(field, "invalid_repository_glob"));
    }
    for segment in value.split('/') {
        if segment.is_empty()
            || matches!(segment, "." | "..")
            || (segment.contains("**") && segment != "**")
        {
            return Err(invalid(field, "invalid_repository_glob"));
        }
    }
    Ok(())
}

pub(crate) fn validate_repository(value: &str) -> Result<(), IssueAutomationPolicyValueError> {
    let invalid_repository = || invalid("binding.github.repository", "owner_and_name_required");
    let Some((owner, name)) = value.split_once('/') else {
        return Err(invalid_repository());
    };
    if owner.is_empty()
        || owner.len() > 39
        || !owner.as_bytes()[0].is_ascii_alphanumeric()
        || !owner.as_bytes()[owner.len() - 1].is_ascii_alphanumeric()
        || !owner
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || character == b'-')
        || name.is_empty()
        || name.len() > 100
        || matches!(name, "." | "..")
        || !name.bytes().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'_' | b'-' | b'.')
        })
    {
        return Err(invalid_repository());
    }
    Ok(())
}

fn invalid(field: &'static str, reason: &'static str) -> IssueAutomationPolicyValueError {
    IssueAutomationPolicyValueError::new(field, reason)
}
