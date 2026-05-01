use super::protocol::{ApprovalPolicyValue, ApprovalsReviewerValue, SandboxModeValue};

const APPROVAL_POLICY_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY";
const APPROVALS_REVIEWER_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER";
const SANDBOX_MODE_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AppServerExecutionPolicy {
    pub(super) approval_policy: ApprovalPolicyValue,
    pub(super) approvals_reviewer: Option<ApprovalsReviewerValue>,
    pub(super) sandbox_mode: SandboxModeValue,
}

impl Default for AppServerExecutionPolicy {
    fn default() -> Self {
        Self {
            // Default to full access so turns do not stall waiting for approvals the TUI
            // cannot yet resolve interactively.
            approval_policy: ApprovalPolicyValue::Never,
            approvals_reviewer: Some(ApprovalsReviewerValue::User),
            sandbox_mode: SandboxModeValue::DangerFullAccess,
        }
    }
}

impl AppServerExecutionPolicy {
    pub(super) fn from_environment() -> Self {
        Self::from_env_values(
            std::env::var(APPROVAL_POLICY_ENV_VAR).ok().as_deref(),
            std::env::var(APPROVALS_REVIEWER_ENV_VAR).ok().as_deref(),
            std::env::var(SANDBOX_MODE_ENV_VAR).ok().as_deref(),
        )
    }

    fn from_env_values(
        approval_policy_value: Option<&str>,
        approvals_reviewer_value: Option<&str>,
        sandbox_mode_value: Option<&str>,
    ) -> Self {
        let mut policy = Self::default();

        if let Some(approval_policy) = parse_approval_policy_value(approval_policy_value) {
            policy.approval_policy = approval_policy;
        }
        if let Some(approvals_reviewer) = parse_approvals_reviewer_value(approvals_reviewer_value) {
            policy.approvals_reviewer = Some(approvals_reviewer);
        }
        if let Some(sandbox_mode) = parse_sandbox_mode_value(sandbox_mode_value) {
            policy.sandbox_mode = sandbox_mode;
        }

        policy
    }
}

fn normalize_execution_policy_value(value: Option<&str>) -> Option<String> {
    let raw_value = value?.trim();
    if raw_value.is_empty() {
        return None;
    }

    Some(raw_value.to_ascii_lowercase().replace(['_', ' '], "-"))
}

fn parse_approval_policy_value(value: Option<&str>) -> Option<ApprovalPolicyValue> {
    match normalize_execution_policy_value(value).as_deref() {
        Some("untrusted") => Some(ApprovalPolicyValue::Untrusted),
        Some("on-failure") => Some(ApprovalPolicyValue::OnFailure),
        Some("on-request") => Some(ApprovalPolicyValue::OnRequest),
        Some("never") => Some(ApprovalPolicyValue::Never),
        _ => None,
    }
}

fn parse_approvals_reviewer_value(value: Option<&str>) -> Option<ApprovalsReviewerValue> {
    match normalize_execution_policy_value(value).as_deref() {
        Some("user") => Some(ApprovalsReviewerValue::User),
        Some("guardian-subagent") => Some(ApprovalsReviewerValue::GuardianSubagent),
        _ => None,
    }
}

fn parse_sandbox_mode_value(value: Option<&str>) -> Option<SandboxModeValue> {
    match normalize_execution_policy_value(value).as_deref() {
        Some("read-only") => Some(SandboxModeValue::ReadOnly),
        Some("workspace-write") => Some(SandboxModeValue::WorkspaceWrite),
        Some("danger-full-access") => Some(SandboxModeValue::DangerFullAccess),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        APPROVAL_POLICY_ENV_VAR, APPROVALS_REVIEWER_ENV_VAR, AppServerExecutionPolicy,
        SANDBOX_MODE_ENV_VAR,
    };
    use crate::adapter::outbound::app_server::protocol::{
        ApprovalPolicyValue, ApprovalsReviewerValue, SandboxModeValue,
    };

    #[test]
    fn execution_policy_defaults_to_full_access_without_approvals() {
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(None, None, None),
            AppServerExecutionPolicy {
                approval_policy: ApprovalPolicyValue::Never,
                approvals_reviewer: Some(ApprovalsReviewerValue::User),
                sandbox_mode: SandboxModeValue::DangerFullAccess,
            }
        );
    }

    #[test]
    fn execution_policy_parses_environment_overrides() {
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(
                Some("on_request"),
                Some("guardian-subagent"),
                Some("workspace write")
            ),
            AppServerExecutionPolicy {
                approval_policy: ApprovalPolicyValue::OnRequest,
                approvals_reviewer: Some(ApprovalsReviewerValue::GuardianSubagent),
                sandbox_mode: SandboxModeValue::WorkspaceWrite,
            }
        );
    }

    #[test]
    fn execution_policy_ignores_invalid_environment_values() {
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(Some("bogus"), Some("nope"), Some("unknown")),
            AppServerExecutionPolicy::default()
        );
    }

    #[test]
    fn execution_policy_environment_variable_names_are_stable() {
        assert_eq!(
            APPROVAL_POLICY_ENV_VAR,
            "CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY"
        );
        assert_eq!(
            APPROVALS_REVIEWER_ENV_VAR,
            "CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER"
        );
        assert_eq!(
            SANDBOX_MODE_ENV_VAR,
            "CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE"
        );
    }
}
