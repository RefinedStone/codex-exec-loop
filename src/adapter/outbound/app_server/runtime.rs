/*
 * runtime.rs는 Codex app-server child process를 매 요청마다 새로 띄울지, 짧은 요청끼리 공유할지를
 * 결정하는 운영 layer다. 긴 turn stream은 connection을 독점할 수 있지만 startup checks, recent
 * sessions, snapshot 같은 짧은 요청은 재사용 connection이 훨씬 빠르다. 그래서 이 모듈은 connection
 * cache, terminal bridge attachment, 사용자에게 보여줄 runtime notice, retry/fallback 정책을 한곳에 둔다.
 */
use anyhow::{Context, Result};

use super::CodexAppServerAdapter;
use super::connection::AppServerConnection;
use super::protocol::{initialize_detail, sort_and_dedup_warnings};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

#[derive(Default)]
pub(super) struct SharedAppServerRuntime {
    /*
     * 이 struct는 단순 connection cache가 아니다. app-server process가 새로 붙으면 UI는 initialize detail과
     * terminal bridge attachment를 다시 보여줘야 하고, caller는 이전 stream warning과 retry notice도 함께
     * 받아야 한다. 그래서 process handle과 사용자-facing metadata를 같은 lifetime으로 관리한다.
     */
    connection: Option<AppServerConnection>,
    initialize_detail: Option<String>,
    attachment_profile: Option<TerminalBridgeAttachmentProfile>,
    pending_notices: Vec<String>,
}

impl SharedAppServerRuntime {
    pub(super) fn ensure_connected(&mut self, adapter: &CodexAppServerAdapter) -> Result<()> {
        /*
         * ensure_connected는 lazy initialization과 health check를 겸한다. 살아 있는 shared process는 그대로 쓰고,
         * 죽은 process는 reset한 뒤 adapter.open_connection으로 새 child를 띄워 initialize handshake까지 끝낸다.
         * reconnect notice는 reset 이후에 다시 넣어, process metadata는 지우되 사용자에게 보여줄 이력은 잃지 않는다.
         */
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
        self.attachment_profile = Some(TerminalBridgeAttachmentProfile::codex_app_server_launch());
        self.connection = Some(connection);
        if let Some(notice) = reconnect_notice {
            self.push_notice(notice);
        }
        Ok(())
    }

    pub(super) fn initialize_detail(&self) -> Result<&str> {
        // caller가 ensure_connected 순서를 어기면 app-server initialize response 없이 startup UI를 만들 수 없다.
        self.initialize_detail
            .as_deref()
            .context("shared runtime initialize detail was not available")
    }

    pub(super) fn attachment_profile(&self) -> Result<TerminalBridgeAttachmentProfile> {
        // attachment profile은 TerminalBridgeAttachmentProfile이 Copy라 caller가 stream event에 바로 실어 보낼 수 있다.
        self.attachment_profile
            .context("shared runtime attachment profile was not available")
    }

    pub(super) fn connection_mut(&mut self) -> Result<&mut AppServerConnection> {
        // with_shared_runtime은 이 mutable borrow 동안 shared connection을 독점해 JSON-RPC request/response 순서를 지킨다.
        self.connection
            .as_mut()
            .context("shared app-server runtime was not connected")
    }

    pub(super) fn reset(&mut self) {
        // notice는 reset 뒤 retry 결과와 함께 보여줘야 하므로 process-bound fields만 지운다.
        self.initialize_detail = None;
        self.attachment_profile = None;
        self.connection.take();
    }

    pub(super) fn push_notice(&mut self, notice: String) {
        self.pending_notices.push(notice);
    }

    pub(super) fn push_notices(&mut self, notices: Vec<String>) {
        self.pending_notices.extend(notices);
    }

    pub(super) fn take_notices(&mut self) -> Vec<String> {
        // stream warning과 retry notice가 같은 UI batch로 나가도록 drain 시점에 정렬과 중복 제거를 수행한다.
        let mut notices = std::mem::take(&mut self.pending_notices);
        sort_and_dedup_warnings(&mut notices);
        notices
    }
}

pub(super) struct SharedRuntimeOutput<T> {
    // value는 startup checks, recent sessions, snapshot처럼 caller별로 다른 domain projection이다.
    pub(super) value: T,
    // warnings는 app-server stream warning과 runtime fallback/retry notice를 합친 사용자-facing 진단이다.
    pub(super) warnings: Vec<String>,
    // attachment_profile은 같은 response에서 terminal bridge가 어떤 process에 붙었는지 알리는 event 재료다.
    pub(super) attachment_profile: TerminalBridgeAttachmentProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestRuntimeMode {
    // Shared는 idle shared connection을 사용해 짧은 request latency를 줄이는 기본 경로다.
    Shared,
    // IsolatedFallback은 active turn stream이 shared connection을 쓰는 동안 짧은 request를 별도 process로 처리하는 경로다.
    IsolatedFallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestFailureOutcome {
    // shared process의 첫 실패는 stale child나 깨진 protocol state일 수 있어 reset 후 새 process로 재시도한다.
    RetryAfterSharedReset,
    // isolated fallback은 reset할 shared state가 없으므로 같은 mode로 한 번만 더 시도한다.
    RetryWithoutReset,
    // shared retry까지 실패하면 caller가 request kind별 context를 붙여 사용자에게 돌려준다.
    ReturnSharedFailure,
    // isolated retry까지 실패하면 active turn 완료 후 재시도하라는 context를 붙여 실패를 돌려준다.
    ReturnIsolatedFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SharedRuntimeRequestKind {
    // StartupChecks는 TUI boot path에서 app-server health와 launch metadata를 확인한다.
    StartupChecks,
    // RecentSessions는 session list mapping이 필요하지만 긴 turn stream과 독립적으로 요청될 수 있다.
    RecentSessions,
    // ConversationSnapshot은 기존 thread 재개 전 현재 conversation projection을 짧게 조회하는 경로다.
    ConversationSnapshot,
}

impl SharedRuntimeRequestKind {
    fn label(self) -> &'static str {
        // label은 notice와 error context 모두에 쓰이므로 request kind별 문구를 한곳에서 맞춘다.
        match self {
            Self::StartupChecks => "startup checks request",
            Self::RecentSessions => "recent sessions request",
            Self::ConversationSnapshot => "conversation snapshot request",
        }
    }

    pub(super) fn isolated_fallback_notice(self) -> String {
        // fallback notice는 기능이 성공했더라도 shared stream 경합 때문에 별도 process를 썼다는 운영 정보를 남긴다.
        format!(
            "{} used an isolated app-server connection while a turn stream was active",
            self.label()
        )
    }

    pub(super) fn retry_reset_notice(self, error: &anyhow::Error) -> String {
        // reset retry notice는 첫 실패 원인을 보존해 transient child failure와 실제 request failure를 구분하게 한다.
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
    /*
     * retry 정책을 순수 함수로 분리해 with_shared_runtime의 control flow를 검증 가능하게 만든다.
     * shared connection의 첫 실패는 오래된 process나 깨진 protocol state일 수 있어 reset 후 재시도하고,
     * isolated fallback의 첫 실패는 reset 대상이 없으므로 같은 mode에서 한 번만 더 시도한다.
     */
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
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[test]
    fn reset_preserves_pending_runtime_notices() {
        // reset은 process lifetime만 끊고, 다음 UI response에 실어야 할 runtime notice는 보존해야 한다.
        let mut runtime = SharedAppServerRuntime::default();
        runtime.push_notice("runtime retried".to_string());

        runtime.reset();

        assert_eq!(runtime.take_notices(), vec!["runtime retried".to_string()]);
        assert!(runtime.take_notices().is_empty());
    }

    #[test]
    fn take_notices_normalizes_stream_and_retry_messages() {
        // app-server warning과 retry notice는 같은 diagnostics surface로 나가므로 drain 시점에 중복 제거된다.
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
    fn reset_clears_attachment_profile_until_runtime_reconnects() {
        // reset 후 attachment_profile이 남아 있으면 UI가 죽은 app-server process에 붙은 것처럼 표시할 수 있다.
        let mut runtime = SharedAppServerRuntime {
            attachment_profile: Some(TerminalBridgeAttachmentProfile::codex_app_server_launch()),
            ..SharedAppServerRuntime::default()
        };

        runtime.reset();

        let error = runtime
            .attachment_profile()
            .expect_err("attachment profile should clear on reset");
        assert_eq!(
            error.to_string(),
            "shared runtime attachment profile was not available"
        );
    }

    #[test]
    fn shared_request_failures_retry_once_after_reset() {
        // shared mode만 첫 실패에서 runtime reset을 요구한다.
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
        // isolated fallback failure는 active shared stream을 건드리지 않고 fallback process만 한 번 더 시도한다.
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
