use anyhow::{Context, Result};

use super::CodexAppServerAdapter;
use super::connection::AppServerConnection;
use super::protocol::{initialize_detail, sort_and_dedup_warnings};

#[derive(Default)]
pub(super) struct SharedAppServerRuntime {
    connection: Option<AppServerConnection>,
    initialize_detail: Option<String>,
    pending_notices: Vec<String>,
}

impl SharedAppServerRuntime {
    pub(super) fn ensure_connected(&mut self, adapter: &CodexAppServerAdapter) -> Result<()> {
        let reconnect_notice = match self.connection.as_mut() {
            Some(connection) => {
                if connection.is_alive()? {
                    return Ok(());
                }

                Some(
                    "shared runtime reconnected after the previous app-server process exited"
                        .to_string(),
                )
            }
            None => None,
        };

        self.reset();

        let mut connection = adapter.open_connection()?;
        let initialize_response = connection.initialize()?;
        self.initialize_detail = Some(initialize_detail(&initialize_response));
        self.connection = Some(connection);
        if let Some(notice) = reconnect_notice {
            self.push_notice(notice);
        }
        Ok(())
    }

    pub(super) fn initialize_detail(&self) -> Result<&str> {
        self.initialize_detail
            .as_deref()
            .context("shared runtime initialize detail was not available")
    }

    pub(super) fn connection_mut(&mut self) -> Result<&mut AppServerConnection> {
        self.connection
            .as_mut()
            .context("shared app-server runtime was not connected")
    }

    pub(super) fn reset(&mut self) {
        self.initialize_detail = None;
        self.connection.take();
    }

    pub(super) fn push_notice(&mut self, notice: String) {
        self.pending_notices.push(notice);
    }

    pub(super) fn push_notices(&mut self, notices: Vec<String>) {
        self.pending_notices.extend(notices);
    }

    pub(super) fn take_notices(&mut self) -> Vec<String> {
        let mut notices = std::mem::take(&mut self.pending_notices);
        sort_and_dedup_warnings(&mut notices);
        notices
    }
}

pub(super) struct SharedRuntimeOutput<T> {
    pub(super) value: T,
    pub(super) warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestRuntimeMode {
    Shared,
    IsolatedFallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestFailureOutcome {
    RetryAfterSharedReset,
    RetryWithoutReset,
    ReturnSharedFailure,
    ReturnIsolatedFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SharedRuntimeRequestKind {
    StartupChecks,
    RecentSessions,
    ConversationSnapshot,
}

impl SharedRuntimeRequestKind {
    fn label(self) -> &'static str {
        match self {
            Self::StartupChecks => "startup checks request",
            Self::RecentSessions => "recent sessions request",
            Self::ConversationSnapshot => "conversation snapshot request",
        }
    }

    pub(super) fn isolated_fallback_notice(self) -> String {
        format!(
            "{} used an isolated app-server connection while a turn stream was active",
            self.label()
        )
    }

    pub(super) fn retry_reset_notice(self, error: &anyhow::Error) -> String {
        format!(
            "shared runtime reset after {} failure; retrying with a fresh app-server connection ({error})",
            self.label()
        )
    }

    pub(super) fn shared_retry_failure_context(self) -> String {
        format!(
            "{} still failed after resetting the shared runtime; open diagnostics and rerun the request",
            self.label()
        )
    }

    pub(super) fn isolated_retry_failure_context(self) -> String {
        format!(
            "{} still failed on isolated retry while the shared turn stream was busy; rerun the request after the active turn completes",
            self.label()
        )
    }
}

pub(super) fn request_failure_outcome(
    mode: RequestRuntimeMode,
    attempt: usize,
) -> RequestFailureOutcome {
    match (mode, attempt) {
        (RequestRuntimeMode::Shared, 0) => RequestFailureOutcome::RetryAfterSharedReset,
        (RequestRuntimeMode::IsolatedFallback, 0) => RequestFailureOutcome::RetryWithoutReset,
        (RequestRuntimeMode::Shared, _) => RequestFailureOutcome::ReturnSharedFailure,
        (RequestRuntimeMode::IsolatedFallback, _) => RequestFailureOutcome::ReturnIsolatedFailure,
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::{
        RequestFailureOutcome, RequestRuntimeMode, SharedAppServerRuntime,
        SharedRuntimeRequestKind, request_failure_outcome,
    };

    #[test]
    fn reset_preserves_pending_runtime_notices() {
        let mut runtime = SharedAppServerRuntime::default();
        runtime.push_notice("runtime retried".to_string());

        runtime.reset();

        assert_eq!(runtime.take_notices(), vec!["runtime retried".to_string()]);
        assert!(runtime.take_notices().is_empty());
    }

    #[test]
    fn take_notices_normalizes_stream_and_retry_messages() {
        let mut runtime = SharedAppServerRuntime::default();
        runtime.push_notice("shared runtime reset".to_string());
        runtime.push_notices(vec![
            "stream warning".to_string(),
            "shared runtime reset".to_string(),
        ]);

        assert_eq!(
            runtime.take_notices(),
            vec![
                "shared runtime reset".to_string(),
                "stream warning".to_string(),
            ]
        );
    }

    #[test]
    fn shared_request_failures_retry_once_after_reset() {
        assert_eq!(
            request_failure_outcome(RequestRuntimeMode::Shared, 0),
            RequestFailureOutcome::RetryAfterSharedReset
        );
        assert_eq!(
            request_failure_outcome(RequestRuntimeMode::Shared, 1),
            RequestFailureOutcome::ReturnSharedFailure
        );
    }

    #[test]
    fn isolated_fallback_failures_retry_without_resetting_shared_runtime() {
        assert_eq!(
            request_failure_outcome(RequestRuntimeMode::IsolatedFallback, 0),
            RequestFailureOutcome::RetryWithoutReset
        );
        assert_eq!(
            request_failure_outcome(RequestRuntimeMode::IsolatedFallback, 1),
            RequestFailureOutcome::ReturnIsolatedFailure
        );
    }

    #[test]
    fn request_kind_messages_include_the_request_label() {
        assert_eq!(
            SharedRuntimeRequestKind::RecentSessions.isolated_fallback_notice(),
            "recent sessions request used an isolated app-server connection while a turn stream was active"
        );
        assert_eq!(
            SharedRuntimeRequestKind::StartupChecks.shared_retry_failure_context(),
            "startup checks request still failed after resetting the shared runtime; open diagnostics and rerun the request"
        );
        assert_eq!(
            SharedRuntimeRequestKind::ConversationSnapshot.isolated_retry_failure_context(),
            "conversation snapshot request still failed on isolated retry while the shared turn stream was busy; rerun the request after the active turn completes"
        );
    }

    #[test]
    fn request_kind_reset_notice_preserves_the_request_label_and_error() {
        let notice = SharedRuntimeRequestKind::RecentSessions.retry_reset_notice(&anyhow!("boom"));

        assert_eq!(
            notice,
            "shared runtime reset after recent sessions request failure; retrying with a fresh app-server connection (boom)"
        );
    }
}
