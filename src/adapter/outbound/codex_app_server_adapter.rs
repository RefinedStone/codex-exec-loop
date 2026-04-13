mod connection;
mod protocol;
mod runtime;

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, TryLockError};

use anyhow::{Result, anyhow};

use self::connection::{AppServerConnection, AppServerConnectionConfig};
use self::protocol::{
    ApprovalPolicyValue, ApprovalsReviewerValue, ReasoningEffortValue, SandboxModeValue,
    ThreadListParams, ThreadResumeParams, ThreadStartParams, TurnInputText, TurnStartParams,
    initialize_detail, sort_and_dedup_warnings, thread_title, to_conversation_snapshot,
    to_session_summary,
};
use self::runtime::{
    RequestFailureOutcome, RequestRuntimeMode, SharedAppServerRuntime, SharedRuntimeOutput,
    SharedRuntimeRequestKind, request_failure_outcome,
};
use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort, NewThreadReasoningEffort, NewThreadStreamRequest,
};
use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
use crate::domain::recent_sessions::RecentSessions;

const APPROVAL_POLICY_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY";
const APPROVALS_REVIEWER_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER";
const SANDBOX_MODE_ENV_VAR: &str = "CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE";

#[derive(Clone)]
pub struct CodexAppServerAdapter {
    client_name: String,
    client_version: String,
    connection_config: AppServerConnectionConfig,
    execution_policy: AppServerExecutionPolicy,
    shared_runtime: Arc<Mutex<SharedAppServerRuntime>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppServerExecutionPolicy {
    approval_policy: ApprovalPolicyValue,
    approvals_reviewer: Option<ApprovalsReviewerValue>,
    sandbox_mode: SandboxModeValue,
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
    fn from_environment() -> Self {
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

    Some(
        raw_value
            .to_ascii_lowercase()
            .replace('_', "-")
            .replace(' ', "-"),
    )
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

impl CodexAppServerAdapter {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self::from_environment(client_name, client_version)
    }

    pub fn from_environment(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self::with_configs(
            client_name,
            client_version,
            AppServerConnectionConfig::from_environment(),
            AppServerExecutionPolicy::from_environment(),
        )
    }

    fn with_configs(
        client_name: impl Into<String>,
        client_version: impl Into<String>,
        connection_config: AppServerConnectionConfig,
        execution_policy: AppServerExecutionPolicy,
    ) -> Self {
        Self {
            client_name: client_name.into(),
            client_version: client_version.into(),
            connection_config,
            execution_policy,
            shared_runtime: Arc::new(Mutex::new(SharedAppServerRuntime::default())),
        }
    }

    fn open_connection(&self) -> Result<AppServerConnection> {
        AppServerConnection::spawn(
            self.client_name.clone(),
            self.client_version.clone(),
            self.connection_config.clone(),
        )
    }

    fn run_new_thread_stream_request(
        &self,
        request: NewThreadStreamRequest,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let NewThreadStreamRequest {
            cwd,
            prompt,
            model,
            reasoning_effort,
        } = request;
        let effort = reasoning_effort.map(ReasoningEffortValue::from);
        let result = self.with_streaming_runtime(|connection| {
            let thread_response = connection.start_thread(ThreadStartParams {
                cwd: Some(cwd.clone()),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
                model: model.clone(),
            })?;
            let thread_id = thread_response.thread.id.clone();
            let _ = event_sender.send(ConversationStreamEvent::ThreadPrepared {
                thread_id: thread_id.clone(),
                title: thread_title(&thread_response.thread),
                cwd: thread_response.thread.cwd.clone(),
            });

            let turn_response = connection.start_turn(TurnStartParams {
                thread_id: thread_id.clone(),
                input: vec![TurnInputText::text(prompt.clone())],
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox_policy: Some(self.execution_policy.sandbox_mode.as_turn_sandbox_policy()),
                model: model.clone(),
                effort,
            })?;

            let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
                turn_id: turn_response.turn.id.clone(),
            });

            connection.wait_for_turn_stream(&thread_id, &turn_response.turn.id, &event_sender)
        });

        if let Err(error) = &result {
            let _ = event_sender.send(ConversationStreamEvent::Failed {
                message: error.to_string(),
            });
        }

        result
    }

    fn with_shared_runtime<T, F>(
        &self,
        request_kind: SharedRuntimeRequestKind,
        mut operation: F,
    ) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
        for attempt in 0..2 {
            let (mode, result) = match self.shared_runtime.try_lock() {
                Ok(mut runtime) => (
                    RequestRuntimeMode::Shared,
                    self.run_request_on_locked_runtime(&mut runtime, &mut operation),
                ),
                Err(TryLockError::WouldBlock) => (
                    RequestRuntimeMode::IsolatedFallback,
                    self.with_isolated_runtime(
                        Some(request_kind.isolated_fallback_notice()),
                        &mut operation,
                    ),
                ),
                Err(TryLockError::Poisoned(_)) => (
                    RequestRuntimeMode::Shared,
                    Err(anyhow!("shared app-server runtime mutex was poisoned")),
                ),
            };

            match result {
                Ok(output) => return Ok(output),
                Err(error) => match request_failure_outcome(mode, attempt) {
                    RequestFailureOutcome::RetryAfterSharedReset => {
                        self.reset_shared_runtime(Some(request_kind.retry_reset_notice(&error)));
                        continue;
                    }
                    RequestFailureOutcome::RetryWithoutReset => continue,
                    RequestFailureOutcome::ReturnSharedFailure => {
                        self.reset_shared_runtime(None);
                        return Err(error.context(request_kind.shared_retry_failure_context()));
                    }
                    RequestFailureOutcome::ReturnIsolatedFailure => {
                        return Err(error.context(request_kind.isolated_retry_failure_context()));
                    }
                },
            }
        }

        unreachable!("shared runtime retry loop always returns on success or final failure")
    }

    fn run_request_on_locked_runtime<T, F>(
        &self,
        runtime: &mut SharedAppServerRuntime,
        operation: &mut F,
    ) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
        runtime.ensure_connected(self)?;
        let initialize_detail = runtime.initialize_detail()?.to_string();
        let (value, connection_warnings) = {
            let connection = runtime.connection_mut()?;
            let value = operation(connection, &initialize_detail)?;
            let warnings = connection.take_warnings();
            (value, warnings)
        };
        let mut warnings = runtime.take_notices();
        warnings.extend(connection_warnings);
        sort_and_dedup_warnings(&mut warnings);
        Ok(SharedRuntimeOutput { value, warnings })
    }

    fn with_isolated_runtime<T, F>(
        &self,
        notice: Option<String>,
        operation: &mut F,
    ) -> Result<SharedRuntimeOutput<T>>
    where
        F: FnMut(&mut AppServerConnection, &str) -> Result<T>,
    {
        let mut connection = self.open_connection()?;
        let initialize_response = connection.initialize()?;
        let initialize_detail = initialize_detail(&initialize_response);
        let value = operation(&mut connection, &initialize_detail)?;
        let mut warnings = connection.take_warnings();
        if let Some(notice) = notice {
            warnings.push(notice);
        }
        sort_and_dedup_warnings(&mut warnings);
        Ok(SharedRuntimeOutput { value, warnings })
    }

    fn with_streaming_runtime<F>(&self, mut operation: F) -> Result<()>
    where
        F: FnMut(&mut AppServerConnection) -> Result<()>,
    {
        let mut runtime = self
            .shared_runtime
            .lock()
            .map_err(|_| anyhow!("shared app-server runtime mutex was poisoned"))?;
        runtime.ensure_connected(self)?;

        let (result, warnings) = {
            let connection = runtime.connection_mut()?;
            let result = operation(connection);
            let warnings = connection.take_warnings();
            (result, warnings)
        };
        runtime.push_notices(warnings);

        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                runtime.reset();
                runtime.push_notice(format!(
                    "shared runtime reset after turn stream failure; the next request will reconnect ({error})"
                ));
                Err(error)
            }
        }
    }

    fn reset_shared_runtime(&self, notice: Option<String>) {
        if let Ok(mut runtime) = self.shared_runtime.lock() {
            runtime.reset();
            if let Some(notice) = notice {
                runtime.push_notice(notice);
            }
        }
    }
}

impl CodexAppServerPort for CodexAppServerAdapter {
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        let output = self.with_shared_runtime(
            SharedRuntimeRequestKind::StartupChecks,
            |connection, initialize_detail| {
                Ok((initialize_detail.to_string(), connection.read_account()?))
            },
        )?;
        let (initialize_detail, account_response) = output.value;

        Ok(AppServerStartupContext {
            initialize_detail,
            account_detail: account_response.to_summary_text(),
            account_ok: account_response.is_authenticated(),
            warnings: output.warnings,
        })
    }

    fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions> {
        let output =
            self.with_shared_runtime(SharedRuntimeRequestKind::RecentSessions, |connection, _| {
                connection.list_threads(ThreadListParams {
                    limit: Some(limit),
                    ..ThreadListParams::default()
                })
            })?;
        let items = output
            .value
            .data
            .into_iter()
            .map(to_session_summary)
            .collect::<Vec<_>>();

        Ok(RecentSessions {
            items,
            warnings: output.warnings,
            next_cursor: output.value.next_cursor,
        })
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        let output = self.with_shared_runtime(
            SharedRuntimeRequestKind::ConversationSnapshot,
            |connection, _| connection.read_thread(thread_id, true),
        )?;
        Ok(to_conversation_snapshot(
            output.value.thread,
            output.warnings,
        ))
    }

    fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.run_new_thread_stream_request(NewThreadStreamRequest::new(cwd, prompt), event_sender)
    }

    fn run_new_thread_stream_with_overrides(
        &self,
        request: NewThreadStreamRequest,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.run_new_thread_stream_request(request, event_sender)
    }

    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        let result = self.with_streaming_runtime(|connection| {
            connection.resume_thread(ThreadResumeParams {
                thread_id: thread_id.to_string(),
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox: Some(self.execution_policy.sandbox_mode),
            })?;
            let turn_response = connection.start_turn(TurnStartParams {
                thread_id: thread_id.to_string(),
                input: vec![TurnInputText::text(prompt)],
                approval_policy: Some(self.execution_policy.approval_policy),
                approvals_reviewer: self.execution_policy.approvals_reviewer,
                sandbox_policy: Some(self.execution_policy.sandbox_mode.as_turn_sandbox_policy()),
                model: None,
                effort: None,
            })?;

            let _ = event_sender.send(ConversationStreamEvent::TurnStarted {
                turn_id: turn_response.turn.id.clone(),
            });

            connection.wait_for_turn_stream(thread_id, &turn_response.turn.id, &event_sender)
        });

        if let Err(error) = &result {
            let _ = event_sender.send(ConversationStreamEvent::Failed {
                message: error.to_string(),
            });
        }

        result
    }
}

impl From<NewThreadReasoningEffort> for ReasoningEffortValue {
    fn from(value: NewThreadReasoningEffort) -> Self {
        match value {
            NewThreadReasoningEffort::None => Self::None,
            NewThreadReasoningEffort::Minimal => Self::Minimal,
            NewThreadReasoningEffort::Low => Self::Low,
            NewThreadReasoningEffort::Medium => Self::Medium,
            NewThreadReasoningEffort::High => Self::High,
            NewThreadReasoningEffort::XHigh => Self::XHigh,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        APPROVAL_POLICY_ENV_VAR, APPROVALS_REVIEWER_ENV_VAR, AppServerExecutionPolicy,
        SANDBOX_MODE_ENV_VAR,
    };
    use crate::adapter::outbound::codex_app_server_adapter::protocol::{
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
