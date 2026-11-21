//! Dedicated, ephemeral Codex delegation. The sandbox permits local reads.
//! This is not an all-tools-off backend for untrusted review inputs.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use super::approval::AppServerApprovalBroker;
use super::connection::{
    AppServerApprovalMode, AppServerConnection, AppServerConnectionConfig,
    AppServerTurnInterruptSignal,
};
use super::{AppServerEventSender, AppServerEventTrySendError, protected_thread_workspace};
use crate::application::port::conversation_stream::ConversationStreamEvent;
use crate::domain::delegation::{Delivery, EndReason};
use crate::domain::turn_terminal::{ConversationTurnErrorInfo, ConversationTurnTerminalOutcome};

mod driver;
pub(crate) use driver::CodexThreadDriver;

const MAX_TEXT_BYTES: usize = 1_048_576;
const POLL: Duration = Duration::from_millis(25);

#[derive(Clone, Default)]
pub(crate) struct CodexThreadBackend {
    #[cfg(test)]
    fixture: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CodexThreadOutcome {
    Complete(String),
    End {
        reason: EndReason,
        delivery: Delivery,
    },
}

pub(crate) struct CodexThreadRequest<'a> {
    pub model: Option<&'a str>,
    pub developer_instructions: &'a str,
    pub text: &'a str,
    pub output_schema: Option<Value>,
}

impl CodexThreadBackend {
    #[cfg(test)]
    fn probe(&self) -> Result<()> {
        let directory = IsolatedDirectory::create()?;
        let signal = AppServerTurnInterruptSignal::default();
        let mut connection = self.connect(&directory, signal)?;
        prepare_thread(&mut connection, &directory.cwd, None, "")?;
        Ok(())
    }

    pub(crate) fn ready(&self) -> bool {
        self.connect_ready().is_ok()
    }

    fn connect_ready(&self) -> Result<()> {
        let directory = IsolatedDirectory::create()?;
        let mut connection = self.connect(&directory, AppServerTurnInterruptSignal::default())?;
        require_configured_auth(&mut connection)?;
        prepare_thread(&mut connection, &directory.cwd, None, "")?;
        Ok(())
    }

    pub(crate) fn run(
        &self,
        request: CodexThreadRequest<'_>,
        cancellation: &AtomicBool,
        timeout: Duration,
        on_started: impl FnOnce(),
    ) -> CodexThreadOutcome {
        use CodexThreadOutcome::{Complete, End};
        use Delivery::{NotSent, Observed, Uncertain};
        if request.text.len() > MAX_TEXT_BYTES
            || request.developer_instructions.len() > MAX_TEXT_BYTES
        {
            return End {
                reason: EndReason::Rejected,
                delivery: NotSent,
            };
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let stopped = || cancellation.load(Ordering::Acquire) || Instant::now() >= deadline;
        if stopped() {
            return End {
                reason: EndReason::Interrupted,
                delivery: NotSent,
            };
        }
        let prepared = (|| {
            let directory = IsolatedDirectory::create()?;
            let signal = AppServerTurnInterruptSignal::default();
            let mut connection = self.connect(&directory, signal.clone())?;
            require_configured_auth(&mut connection)?;
            let thread = prepare_thread(
                &mut connection,
                &directory.cwd,
                request.model,
                request.developer_instructions,
            )?;
            Ok::<_, anyhow::Error>((connection, directory, signal, thread))
        })();
        let Ok((mut connection, directory, signal, thread)) = prepared else {
            return End {
                reason: EndReason::Unavailable,
                delivery: NotSent,
            };
        };
        if stopped() {
            return End {
                reason: EndReason::Interrupted,
                delivery: NotSent,
            };
        }
        let sink = FinalText::default();
        let outcome = std::thread::scope(|scope| {
            let (done, receiver) = std::sync::mpsc::channel::<()>();
            let signal_ref = &signal;
            scope.spawn(move || {
                loop {
                    if stopped() {
                        signal_ref.request_stop_all_sessions();
                        break;
                    }
                    if !matches!(
                        receiver.recv_timeout(POLL),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
            });
            let _done = done;
            let mut params = json!({
                "threadId": thread,
                "cwd": directory.cwd,
                "input": [{"type":"text","text":request.text,"textElements":[]}],
                "approvalPolicy":"never",
                "sandboxPolicy":{"type":"readOnly","networkAccess":false}
            });
            if let Some(schema) = request.output_schema {
                params["outputSchema"] = schema;
            }
            let Ok(started) = connection.start_isolated_turn(params, &sink, &signal) else {
                return End {
                    reason: EndReason::Interrupted,
                    delivery: Uncertain,
                };
            };
            if started.turn.id.is_empty() || started.turn.id.len() > 4096 {
                return End {
                    reason: EndReason::Unknown,
                    delivery: Uncertain,
                };
            }
            on_started();
            let Ok(receipt) =
                connection.wait_for_turn_stream(&thread, &started.turn.id, &signal, 0, &sink)
            else {
                return End {
                    reason: EndReason::Interrupted,
                    delivery: Observed,
                };
            };
            match receipt.outcome {
                ConversationTurnTerminalOutcome::Completed
                    if receipt.is_completed_and_confirmed() && !stopped() =>
                {
                    match sink.0.take() {
                        Some(text) if !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES => {
                            Complete(text)
                        }
                        _ => End {
                            reason: EndReason::ResultInvalid,
                            delivery: Observed,
                        },
                    }
                }
                ConversationTurnTerminalOutcome::Completed if stopped() => End {
                    reason: EndReason::Interrupted,
                    delivery: Observed,
                },
                ConversationTurnTerminalOutcome::Interrupted => End {
                    reason: EndReason::Interrupted,
                    delivery: Observed,
                },
                ConversationTurnTerminalOutcome::Failed { error } => End {
                    reason: classify(error.info.as_ref()),
                    delivery: Observed,
                },
                _ => End {
                    reason: EndReason::Unknown,
                    delivery: Observed,
                },
            }
        });
        // Kill and reap the subprocess before removing its workspace and private home.
        drop(connection);
        outcome
    }

    fn connect(
        &self,
        directory: &IsolatedDirectory,
        signal: AppServerTurnInterruptSignal,
    ) -> Result<AppServerConnection> {
        #[cfg(test)]
        let config = match &self.fixture {
            Some(fixture) => AppServerConnectionConfig::isolated_delegation_fixture(
                &directory.cwd,
                &directory.home,
                fixture,
            ),
            None => {
                AppServerConnectionConfig::isolated_delegation(&directory.cwd, &directory.home)?
            }
        };
        #[cfg(not(test))]
        let config =
            AppServerConnectionConfig::isolated_delegation(&directory.cwd, &directory.home)?;
        let mut connection = AppServerConnection::spawn(
            "akra-codex-delegation".into(),
            env!("CARGO_PKG_VERSION").into(),
            config,
            Arc::new(AppServerApprovalBroker::default()),
            AppServerApprovalMode::Unattended,
            signal,
        )?;
        connection.initialize()?;
        Ok(connection)
    }
}

fn prepare_thread(
    connection: &mut AppServerConnection,
    cwd: &Path,
    model: Option<&str>,
    instructions: &str,
) -> Result<String> {
    let workspace = protected_thread_workspace(cwd.to_str().context("invalid isolation path")?)?;
    let effective = connection.isolated_request("config/read", json!({"cwd":workspace.cwd}))?;
    let config = effective["config"]
        .as_object()
        .context("missing effective config")?;
    for field in ["mcp_servers", "plugins", "hooks"] {
        ensure!(
            config
                .get(field)
                .is_none_or(|value| value.is_null()
                    || value.as_object().is_some_and(serde_json::Map::is_empty)),
            "incompatible configuration in isolated app-server"
        );
    }
    let mut overrides = workspace.config;
    overrides.insert("web_search".into(), json!("disabled"));
    overrides.insert("sandbox_read_only".into(), json!({"network_access":false}));
    let mut params = json!({
        "cwd":workspace.cwd,
        "modelProvider":"openai",
        "sandbox":"read-only",
        "approvalPolicy":"never",
        "ephemeral":true,
        "developerInstructions":instructions,
        "config":overrides
    });
    if let Some(model) = model {
        params["model"] = json!(model);
    }
    let response = connection.isolated_request("thread/start", params)?;
    ensure!(
        response["cwd"]
            .as_str()
            .is_some_and(|applied| Path::new(applied) == cwd)
            && response["approvalPolicy"] == "never"
            && response["sandbox"]["type"] == "readOnly"
            && !response["sandbox"]["networkAccess"]
                .as_bool()
                .unwrap_or(false)
            && response["modelProvider"] == "openai"
            && response["thread"].get("path").is_some_and(Value::is_null)
            && response["instructionSources"]
                .as_array()
                .is_none_or(Vec::is_empty),
        "app-server did not apply the isolated thread policy"
    );
    let thread_id = response["thread"]["id"]
        .as_str()
        .context("thread id missing")?;
    ensure!(
        !thread_id.is_empty() && thread_id.len() <= 4096,
        "invalid thread id"
    );
    let mcp = connection.isolated_request(
        "mcpServerStatus/list",
        json!({"threadId":thread_id,"limit":1}),
    )?;
    ensure!(
        mcp["data"].as_array().is_some_and(Vec::is_empty) && mcp["nextCursor"].is_null(),
        "MCP server available on isolated thread"
    );
    Ok(thread_id.to_owned())
}

fn require_configured_auth(connection: &mut AppServerConnection) -> Result<()> {
    let account = connection.isolated_request("account/read", json!({"refreshToken":false}))?;
    ensure!(account["account"].is_object(), "no isolated Codex account");
    Ok(())
}

/// The draft JSON comes from the masked, validated draft service, not from a
/// provider response or the general delegation entry point.
pub(crate) fn issue_draft_review_request<'a>(
    model: Option<&'a str>,
    draft_json: &'a str,
) -> CodexThreadRequest<'a> {
    use crate::application::service::issue_review::ISSUE_DRAFT_REVIEW_SKILL_TEXT;
    CodexThreadRequest {
        model,
        developer_instructions: ISSUE_DRAFT_REVIEW_SKILL_TEXT,
        text: draft_json,
        output_schema: None,
    }
}

pub(crate) fn issue_draft_review_reply(reply: String) -> CodexThreadOutcome {
    use crate::application::service::issue_review::{IssueReviewOutcome, parse_model_reply};
    let response = parse_model_reply(&reply);
    if matches!(response, IssueReviewOutcome::Failed { .. }) {
        return CodexThreadOutcome::End {
            reason: EndReason::ResultInvalid,
            delivery: Delivery::Observed,
        };
    }
    CodexThreadOutcome::Complete(reply)
}

#[derive(Default)]
struct FinalText(RefCell<Option<String>>);

impl AppServerEventSender for FinalText {
    fn try_send_prebounded(
        &self,
        event: ConversationStreamEvent,
    ) -> std::result::Result<(), AppServerEventTrySendError> {
        if let ConversationStreamEvent::AgentMessageCompleted { phase, text, .. } = event
            && phase
                .as_deref()
                .is_none_or(|phase| phase == "final" || phase == "final_answer")
        {
            self.0.replace(Some(text));
        }
        Ok(())
    }
}

fn classify(info: Option<&ConversationTurnErrorInfo>) -> EndReason {
    use ConversationTurnErrorInfo::*;
    match info {
        Some(UsageLimitExceeded | SessionBudgetExceeded) => EndReason::Quota,
        Some(ServerOverloaded | HttpConnectionFailed { .. } | InternalServerError) => {
            EndReason::Transient
        }
        Some(ResponseStreamDisconnected { .. }) => EndReason::Interrupted,
        Some(Unauthorized) => EndReason::NeedsHuman,
        Some(BadRequest | CyberPolicy | ContextWindowExceeded) => EndReason::Rejected,
        _ => EndReason::Unknown,
    }
}

struct IsolatedDirectory {
    root: PathBuf,
    cwd: PathBuf,
    home: PathBuf,
}

impl IsolatedDirectory {
    fn create() -> Result<Self> {
        let temp = super::normalize_absolute_workspace_path(&std::env::temp_dir());
        ensure!(temp.is_absolute(), "absolute isolation directory required");
        // A caller's TMPDIR must not make the isolated process discover repository config.
        for ancestor in fs::canonicalize(&temp)?.ancestors() {
            for marker in [".git", "AGENTS.md", "AGENTS.override.md"] {
                ensure!(
                    !ancestor.join(marker).try_exists()?,
                    "isolation parent has project metadata"
                );
            }
        }
        let root = temp.join(format!("akra-delegation-{:032x}", rand::random::<u128>()));
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt;
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder
        };
        #[cfg(not(unix))]
        let builder = fs::DirBuilder::new();
        builder.create(&root)?;
        let directory = Self {
            cwd: root.join("work"),
            home: root.join("home"),
            root,
        };
        crate::private_directory::ensure_private_directory(&directory.root)?;
        builder.create(&directory.cwd)?;
        builder.create(&directory.home)?;
        Ok(directory)
    }
}

impl Drop for IsolatedDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
pub(crate) use tests::Fixture as CodexThreadFixture;
