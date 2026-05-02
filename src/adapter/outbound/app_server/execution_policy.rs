/*
 * 학습 주석: execution_policy 모듈은 Akra 설정을 Codex app-server protocol 값으로 투영하는
 * outbound adapter 경계다. `CodexAppServerAdapter::from_environment`가 이 값을 한 번 읽고,
 * 새 thread 생성, 기존 thread reattach, turn start, planning/parallel worker thread 시작 payload에
 * 같은 approval/sandbox 정책을 반복 주입한다.
 */
use super::protocol::{ApprovalPolicyValue, ApprovalsReviewerValue, SandboxModeValue};

/*
 * 학습 주석: 이 env var 이름들은 Akra 쪽 운영 override 계약이다. app-server protocol field 이름을
 * 직접 env var로 노출하지 않고 adapter-owned prefix를 붙여, upstream schema 변화와 local deployment
 * 설정을 분리한다.
 */
const APPROVAL_POLICY_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY";
const APPROVALS_REVIEWER_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER";
const SANDBOX_MODE_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE";

#[derive(Clone, Debug, PartialEq, Eq)]
/*
 * 학습 주석: AppServerExecutionPolicy는 app-server 세션을 시작할 때마다 함께 보내는 실행 안전성
 * envelope다. application 계층은 "작업을 실행해 달라"는 port만 호출하고, approval/sandbox 세부값은
 * 이 outbound adapter가 app-server protocol vocabulary로 정리해 붙인다.
 */
pub(super) struct AppServerExecutionPolicy {
    // 학습 주석: tool 실행 전에 app-server가 approval을 요구할지 결정하는 protocol 값이다.
    pub(super) approval_policy: ApprovalPolicyValue,
    // 학습 주석: approval이 필요한 설정일 때 누가 검토할지 나타낸다. 현재 기본은 사용자 검토 경로다.
    pub(super) approvals_reviewer: Option<ApprovalsReviewerValue>,
    // 학습 주석: app-server process/turn이 사용할 filesystem/network sandbox 강도다.
    pub(super) sandbox_mode: SandboxModeValue,
}

impl Default for AppServerExecutionPolicy {
    /*
     * 학습 주석: 기본값은 intentionally permissive하다. 현재 TUI는 app-server approval prompt를
     * 완전한 interactive loop로 처리하지 못하므로, default가 approval을 요구하면 turn stream이
     * 멈추고 사용자는 아무 피드백 없이 대기할 수 있다. 운영자가 더 엄격한 모드를 원하면 env override로
     * 좁히는 구조다.
     */
    fn default() -> Self {
        Self {
            approval_policy: ApprovalPolicyValue::Never,
            approvals_reviewer: Some(ApprovalsReviewerValue::User),
            sandbox_mode: SandboxModeValue::DangerFullAccess,
        }
    }
}

impl AppServerExecutionPolicy {
    /*
     * 학습 주석: adapter 생성 시점에 process environment를 읽어 실행 정책 snapshot을 만든다.
     * 이후 같은 adapter instance가 main session, hidden planning worker, parallel worker에 같은 정책을
     * 적용하므로 한 실행 중에 env var가 바뀌어도 active adapter policy가 흔들리지 않는다.
     */
    pub(super) fn from_environment() -> Self {
        Self::from_env_values(
            std::env::var(APPROVAL_POLICY_ENV_VAR).ok().as_deref(),
            std::env::var(APPROVALS_REVIEWER_ENV_VAR).ok().as_deref(),
            std::env::var(SANDBOX_MODE_ENV_VAR).ok().as_deref(),
        )
    }

    /*
     * 학습 주석: from_env_values는 테스트 가능한 parser entrypoint다. 각 override는 독립적으로 적용되어,
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

/*
 * 학습 주석: operator-facing env value는 dash, underscore, space가 섞일 수 있다. normalization 단계에서
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

/*
 * 학습 주석: approval policy parser는 app-server protocol enum의 허용 값만 통과시킨다.
 * 알 수 없는 문자열은 None이 되어 default policy를 유지하므로, 잘못된 env var 하나가 TUI startup을
 * 실패시키지 않는다.
 */
fn parse_approval_policy_value(value: Option<&str>) -> Option<ApprovalPolicyValue> {
    match normalize_execution_policy_value(value).as_deref() {
        Some("untrusted") => Some(ApprovalPolicyValue::Untrusted),
        Some("on-failure") => Some(ApprovalPolicyValue::OnFailure),
        Some("on-request") => Some(ApprovalPolicyValue::OnRequest),
        Some("never") => Some(ApprovalPolicyValue::Never),
        _ => None,
    }
}

/*
 * 학습 주석: approvals reviewer는 approval이 켜졌을 때만 실질적인 의미가 있지만, thread/turn payload는
 * 항상 같은 field set을 받을 수 있다. parser를 따로 두어 reviewer vocabulary가 approval policy와
 * 독립적으로 확장될 수 있게 한다.
 */
fn parse_approvals_reviewer_value(value: Option<&str>) -> Option<ApprovalsReviewerValue> {
    match normalize_execution_policy_value(value).as_deref() {
        Some("user") => Some(ApprovalsReviewerValue::User),
        Some("guardian-subagent") => Some(ApprovalsReviewerValue::GuardianSubagent),
        _ => None,
    }
}

/*
 * 학습 주석: sandbox mode는 thread start/resume payload에서는 `SandboxModeValue`로 들어가고,
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
        APPROVAL_POLICY_ENV_VAR, APPROVALS_REVIEWER_ENV_VAR, AppServerExecutionPolicy,
        SANDBOX_MODE_ENV_VAR,
    };
    use crate::adapter::outbound::app_server::protocol::{
        ApprovalPolicyValue, ApprovalsReviewerValue, SandboxModeValue,
    };

    #[test]
    fn execution_policy_defaults_to_full_access_without_approvals() {
        /*
         * 학습 주석: default는 main TUI path의 startup 계약이다. approval loop가 준비되지 않은 상태에서
         * app-server가 permission prompt를 기다리면 사용자 turn이 멈추므로 이 값을 회귀 테스트로 고정한다.
         */
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
        /*
         * 학습 주석: underscore와 space도 받아들이는지 확인한다. 이 테스트가 깨지면 운영 문서나
         * systemd/env 파일에서 쓰던 느슨한 입력 형태가 갑자기 무효가 될 수 있다.
         */
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
        /*
         * 학습 주석: invalid override는 hard error가 아니라 default fallback이어야 한다. app-server
         * adapter가 실행 정책 parse 문제로 전체 startup을 막으면 TUI 복구 경로가 사라지기 때문이다.
         */
        assert_eq!(
            AppServerExecutionPolicy::from_env_values(Some("bogus"), Some("nope"), Some("unknown")),
            AppServerExecutionPolicy::default()
        );
    }

    #[test]
    fn execution_policy_environment_variable_names_are_stable() {
        /*
         * 학습 주석: env var 이름은 배포 스크립트와 운영 문서의 외부 계약이다. 내부 protocol field가
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
