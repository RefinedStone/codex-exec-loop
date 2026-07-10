/*
 * execution_policy 모듈은 Akra 설정을 Codex app-server protocol 값으로 투영하는
 * outbound adapter 경계다. `CodexAppServerAdapter::from_environment`가 이 값을 한 번 읽고,
 * 새 thread 생성, 기존 thread reattach, turn start, planning/parallel worker thread 시작 payload에
 * 같은 approval/sandbox 정책을 반복 주입한다.
 */
use super::protocol::{ApprovalPolicyValue, ApprovalsReviewerValue, SandboxModeValue};

/*
 * 이 env var 이름들은 Akra 쪽 운영 override 계약이다. app-server protocol field 이름을
 * 직접 env var로 노출하지 않고 adapter-owned prefix를 붙여, upstream schema 변화와 local deployment
 * 설정을 분리한다.
 */
const APPROVAL_POLICY_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY";
const APPROVALS_REVIEWER_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER";
const SANDBOX_MODE_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE";
const AUTO_REVIEW_WARNING: &str = "automatic approval review is explicitly enabled; it may approve app-server tool requests without operator confirmation";
const APPROVAL_POLICY_ALLOWED_VALUES: &str = "untrusted, on-request, or never";
const APPROVALS_REVIEWER_ALLOWED_VALUES: &str = "user, auto-review, or guardian-subagent (legacy)";
const SANDBOX_MODE_ALLOWED_VALUES: &str = "read-only, workspace-write, or danger-full-access";

#[derive(Clone, Debug, PartialEq, Eq)]
/*
 * AppServerExecutionPolicy는 app-server 세션을 시작할 때마다 함께 보내는 실행 안전성
 * envelope다. application 계층은 "작업을 실행해 달라"는 port만 호출하고, approval/sandbox 세부값은
 * 이 outbound adapter가 app-server protocol vocabulary로 정리해 붙인다.
 */
pub(super) struct AppServerExecutionPolicy {
    // tool 실행 전에 app-server가 approval을 요구할지 결정하는 protocol 값이다.
    pub(super) approval_policy: ApprovalPolicyValue,
    // approval이 필요한 설정일 때 누가 검토할지 나타낸다. 현재 기본은 사용자 검토 경로다.
    pub(super) approvals_reviewer: Option<ApprovalsReviewerValue>,
    // app-server process/turn이 사용할 filesystem/network sandbox 강도다.
    pub(super) sandbox_mode: SandboxModeValue,
}

impl Default for AppServerExecutionPolicy {
    /*
     * 기본값은 secure-by-default다. 명시적 override가 없으면 app-server 세션도
     * approval review와 workspace sandbox 안에서 시작한다. 더 관대한 실행이 꼭 필요하면
     * 운영자가 env override로 명시적으로 풀어야 한다.
     */
    fn default() -> Self {
        Self {
            approval_policy: ApprovalPolicyValue::OnRequest,
            approvals_reviewer: Some(ApprovalsReviewerValue::User),
            sandbox_mode: SandboxModeValue::WorkspaceWrite,
        }
    }
}

impl AppServerExecutionPolicy {
    pub(super) fn summary(&self) -> String {
        format!(
            "approval={}, reviewer={}, sandbox={}",
            approval_policy_label(self.approval_policy),
            self.approvals_reviewer
                .map(approvals_reviewer_label)
                .unwrap_or("default"),
            sandbox_mode_label(self.sandbox_mode)
        )
    }

    pub(super) fn elevated_risk_labels(&self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        if self.approval_policy == ApprovalPolicyValue::Never {
            labels.push("approval=never");
        }
        if self.sandbox_mode == SandboxModeValue::DangerFullAccess {
            labels.push("sandbox=danger-full-access");
        }
        if matches!(
            self.approvals_reviewer,
            Some(ApprovalsReviewerValue::AutoReview | ApprovalsReviewerValue::GuardianSubagent)
        ) {
            labels.push("automatic reviewer may auto-approve");
        }
        labels
    }

    /*
     * adapter 생성 시점에 process environment를 읽어 실행 정책 snapshot을 만든다.
     * 이후 같은 adapter instance가 main-session, hidden planning worker, parallel worker에 같은 정책을
     * 적용하므로 한 실행 중에 env var가 바뀌어도 active adapter policy가 흔들리지 않는다.
     */
    pub(super) fn from_environment() -> Self {
        let approval_policy_value = std::env::var(APPROVAL_POLICY_ENV_VAR).ok();
        let approvals_reviewer_value = std::env::var(APPROVALS_REVIEWER_ENV_VAR).ok();
        let sandbox_mode_value = std::env::var(SANDBOX_MODE_ENV_VAR).ok();
        let policy = Self::from_env_values(
            approval_policy_value.as_deref(),
            approvals_reviewer_value.as_deref(),
            sandbox_mode_value.as_deref(),
        );
        for warning in invalid_execution_policy_warnings(
            approval_policy_value.as_deref(),
            approvals_reviewer_value.as_deref(),
            sandbox_mode_value.as_deref(),
        ) {
            eprintln!("warning: {warning}");
            tracing::warn!(warning = %warning, "invalid app-server execution policy override");
        }
        if matches!(
            policy.approvals_reviewer,
            Some(ApprovalsReviewerValue::AutoReview | ApprovalsReviewerValue::GuardianSubagent)
        ) {
            tracing::warn!(
                environment_variable = APPROVALS_REVIEWER_ENV_VAR,
                "{AUTO_REVIEW_WARNING}"
            );
        }
        policy
    }

    /*
     * from_env_values는 테스트 가능한 parser entrypoint다. 각 override는 독립적으로 적용되어,
     * 예를 들어 sandbox 값만 올바르면 approval 값이 잘못되어도 sandbox override는 살아남는다.
     * invalid value를 오류로 중단하지 않는 이유는 app-server 연결 자체가 운영 편의를 위해 계속 떠야 하기 때문이다.
     */
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

fn approval_policy_label(value: ApprovalPolicyValue) -> &'static str {
    match value {
        ApprovalPolicyValue::Untrusted => "untrusted",
        ApprovalPolicyValue::OnRequest => "on-request",
        ApprovalPolicyValue::Never => "never",
    }
}

fn approvals_reviewer_label(value: ApprovalsReviewerValue) -> &'static str {
    match value {
        ApprovalsReviewerValue::User => "user",
        ApprovalsReviewerValue::AutoReview => "auto-review",
        ApprovalsReviewerValue::GuardianSubagent => "guardian-subagent",
    }
}

fn sandbox_mode_label(value: SandboxModeValue) -> &'static str {
    match value {
        SandboxModeValue::ReadOnly => "read-only",
        SandboxModeValue::WorkspaceWrite => "workspace-write",
        SandboxModeValue::DangerFullAccess => "danger-full-access",
    }
}

/*
 * operator-facing env value는 dash, underscore, space가 섞일 수 있다. normalization 단계에서
 * 모두 app-server enum의 kebab-case vocabulary로 맞추면 deployment script가 `on_request`,
 * `on request`, `on-request` 중 어떤 스타일을 써도 같은 정책으로 해석된다.
 */
fn normalize_execution_policy_value(value: Option<&str>) -> Option<String> {
    let raw_value = value?.trim();
    if raw_value.is_empty() {
        return None;
    }

    Some(raw_value.to_ascii_lowercase().replace(['_', ' '], "-"))
}

fn invalid_execution_policy_warnings(
    approval_policy_value: Option<&str>,
    approvals_reviewer_value: Option<&str>,
    sandbox_mode_value: Option<&str>,
) -> Vec<String> {
    let candidates = [
        (
            APPROVAL_POLICY_ENV_VAR,
            approval_policy_value,
            APPROVAL_POLICY_ALLOWED_VALUES,
            parse_approval_policy_value(approval_policy_value).is_some(),
        ),
        (
            APPROVALS_REVIEWER_ENV_VAR,
            approvals_reviewer_value,
            APPROVALS_REVIEWER_ALLOWED_VALUES,
            parse_approvals_reviewer_value(approvals_reviewer_value).is_some(),
        ),
        (
            SANDBOX_MODE_ENV_VAR,
            sandbox_mode_value,
            SANDBOX_MODE_ALLOWED_VALUES,
            parse_sandbox_mode_value(sandbox_mode_value).is_some(),
        ),
    ];

    candidates
        .into_iter()
        .filter_map(|(environment_variable, value, allowed_values, is_valid)| {
            let is_nonempty = value.is_some_and(|value| !value.trim().is_empty());
            (is_nonempty && !is_valid).then(|| {
                format!(
                    "invalid {environment_variable}; expected {allowed_values}; using secure default"
                )
            })
        })
        .collect()
}

/*
 * approval policy parser는 app-server protocol enum의 허용 값만 통과시킨다.
 * 알 수 없는 문자열은 None이 되어 default policy를 유지하므로, 잘못된 env var 하나가 TUI startup을
 * 실패시키지 않는다.
 */
fn parse_approval_policy_value(value: Option<&str>) -> Option<ApprovalPolicyValue> {
    match normalize_execution_policy_value(value).as_deref() {
        Some("untrusted") => Some(ApprovalPolicyValue::Untrusted),
        Some("on-request") => Some(ApprovalPolicyValue::OnRequest),
        Some("never") => Some(ApprovalPolicyValue::Never),
        _ => None,
    }
}

/*
 * 기본 reviewer는 user이고, automatic reviewer는 운영자가 env로 정확히 opt-in할 때만 허용한다.
 * auto-review는 canonical wire value이고 guardian-subagent는 upstream compatibility alias다.
 * automatic reviewer는 operator confirmation 없이 승인할 수 있다. reviewer가 처리하지 않고 client로 보낸
 * server approval request는 connection layer가 계속 명시적으로 거절한다.
 */
fn parse_approvals_reviewer_value(value: Option<&str>) -> Option<ApprovalsReviewerValue> {
    match normalize_execution_policy_value(value).as_deref() {
        Some("user") => Some(ApprovalsReviewerValue::User),
        Some("auto-review") => Some(ApprovalsReviewerValue::AutoReview),
        Some("guardian-subagent") => Some(ApprovalsReviewerValue::GuardianSubagent),
        _ => None,
    }
}

/*
 * sandbox mode는 thread start/resume payload에서는 `SandboxModeValue`로 들어가고,
 * turn start payload에서는 `as_turn_sandbox_policy`를 거쳐 turn-specific enum으로 바뀐다. 그래서
 * 이 parser는 adapter 안의 원본 sandbox 선택만 책임지고 protocol별 field 변환은 호출 지점에 남긴다.
 */
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
        APPROVAL_POLICY_ENV_VAR, APPROVALS_REVIEWER_ENV_VAR, AUTO_REVIEW_WARNING,
        AppServerExecutionPolicy, SANDBOX_MODE_ENV_VAR, invalid_execution_policy_warnings,
    };
    use crate::adapter::outbound::app_server::protocol::{
        ApprovalPolicyValue, ApprovalsReviewerValue, SandboxModeValue,
    };

    #[test]
    fn execution_policy_defaults_to_workspace_write_with_on_request_approval() {
        /*
         * env override가 없을 때도 app-server 기본 동작은 reviewable/sandboxed여야 한다.
         * 이 회귀 테스트가 깨지면 Akra 전체가 다시 무승인·무샌드박스로 후퇴할 수 있다.
         */
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(None, None, None),
            AppServerExecutionPolicy {
                approval_policy: ApprovalPolicyValue::OnRequest,
                approvals_reviewer: Some(ApprovalsReviewerValue::User),
                sandbox_mode: SandboxModeValue::WorkspaceWrite,
            }
        );
    }

    #[test]
    fn execution_policy_parses_environment_overrides() {
        /*
         * underscore와 space도 받아들이는지 확인한다. 이 테스트가 깨지면 운영 문서나
         * systemd/env 파일에서 쓰던 느슨한 입력 형태가 갑자기 무효가 될 수 있다.
         */
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(
                Some("never"),
                Some("user"),
                Some("danger full access")
            ),
            AppServerExecutionPolicy {
                approval_policy: ApprovalPolicyValue::Never,
                approvals_reviewer: Some(ApprovalsReviewerValue::User),
                sandbox_mode: SandboxModeValue::DangerFullAccess,
            }
        );
    }

    #[test]
    fn automatic_reviewers_require_explicit_opt_in() {
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(
                Some("on-request"),
                Some("auto_review"),
                Some("workspace-write"),
            )
            .approvals_reviewer,
            Some(ApprovalsReviewerValue::AutoReview)
        );
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(
                Some("on-request"),
                Some("guardian-subagent"),
                Some("workspace-write"),
            )
            .approvals_reviewer,
            Some(ApprovalsReviewerValue::GuardianSubagent)
        );

        for reviewer in [None, Some("automatic"), Some("guardian")] {
            let policy = AppServerExecutionPolicy::from_env_values(
                Some("on-request"),
                reviewer,
                Some("workspace-write"),
            );

            assert_eq!(
                policy.approvals_reviewer,
                Some(ApprovalsReviewerValue::User)
            );
        }

        assert!(AUTO_REVIEW_WARNING.contains("without operator confirmation"));
    }

    #[test]
    fn execution_policy_ignores_invalid_environment_values() {
        /*
         * invalid override는 hard error가 아니라 secure default fallback이어야 한다.
         * 오타 난 env 값이 다시 무승인·무샌드박스 실행으로 새어 나가면 안 된다.
         */
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(Some("bogus"), Some("nope"), Some("unknown")),
            AppServerExecutionPolicy::default()
        );
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(
                Some("on-failure"),
                Some("user"),
                Some("workspace-write"),
            ),
            AppServerExecutionPolicy::default()
        );
    }

    #[test]
    fn invalid_execution_policy_overrides_warn_without_echoing_raw_values() {
        let raw_value = "super-secret-raw-value";
        let warnings =
            invalid_execution_policy_warnings(Some(raw_value), Some(raw_value), Some(raw_value));

        assert_eq!(warnings.len(), 3);
        assert!(warnings.iter().any(|warning| {
            warning.contains(APPROVAL_POLICY_ENV_VAR) && warning.contains("on-request")
        }));
        assert!(warnings.iter().any(|warning| {
            warning.contains(APPROVALS_REVIEWER_ENV_VAR) && warning.contains("guardian-subagent")
        }));
        assert!(warnings.iter().any(|warning| {
            warning.contains(SANDBOX_MODE_ENV_VAR) && warning.contains("workspace-write")
        }));
        assert!(warnings.iter().all(|warning| !warning.contains(raw_value)));
        assert!(invalid_execution_policy_warnings(Some(" "), None, Some("\t")).is_empty());
    }

    #[test]
    fn execution_policy_environment_variable_names_are_stable() {
        /*
         * env var 이름은 배포 스크립트와 운영 문서의 외부 계약이다. 내부 protocol field가
         * 바뀌어도 이 adapter-owned 이름은 명시적으로 변경하지 않는 한 유지되어야 한다.
         */
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
