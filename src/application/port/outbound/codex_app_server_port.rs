use std::sync::mpsc::Sender;

use anyhow::Result;

pub use super::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
pub use super::session_catalog_port::SessionCatalogPort;
pub use super::startup_probe_port::{AppServerStartupContext, StartupProbePort};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::domain::conversation::{ConversationRuntimeControlTruth, ConversationSnapshot};
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};

// This remains as a Codex-shaped compatibility port while application services migrate to
// capability-owned seams.
pub trait CodexAppServerPort: Send + Sync {
    fn load_startup_context(&self) -> Result<AppServerStartupContext>;
    fn load_recent_sessions(&self, limit: usize) -> Result<SessionCatalog>;
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        ConversationRuntimeControlTruth::codex_app_server()
    }
    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot>;
    fn request_stop_all_sessions(&self) -> Result<()>;
    fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
}

impl<T> StartupProbePort for T
where
    T: CodexAppServerPort + ?Sized,
{
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        CodexAppServerPort::load_startup_context(self)
    }
}

impl<T> SessionCatalogPort for T
where
    T: CodexAppServerPort + ?Sized,
{
    fn load_session_catalog(&self, request: SessionCatalogRequest) -> Result<SessionCatalog> {
        CodexAppServerPort::load_recent_sessions(self, request.limit)
    }
}

impl<T> InteractiveTurnRuntimePort for T
where
    T: CodexAppServerPort + ?Sized,
{
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        CodexAppServerPort::runtime_control_truth(self)
    }

    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        CodexAppServerPort::load_conversation_snapshot(self, thread_id)
    }

    fn request_stop_all_sessions(&self) -> Result<()> {
        CodexAppServerPort::request_stop_all_sessions(self)
    }

    fn run_new_thread_stream(
        &self,
        cwd: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        CodexAppServerPort::run_new_thread_stream(self, cwd, prompt, event_sender)
    }

    fn run_turn_stream(
        &self,
        thread_id: &str,
        prompt: &str,
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        CodexAppServerPort::run_turn_stream(self, thread_id, prompt, event_sender)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc::{Sender, channel};

    use super::{
        AppServerStartupContext, CodexAppServerPort, InteractiveTurnRuntimePort,
        SessionCatalogPort, StartupProbePort,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::domain::conversation::{
        ConversationControlSupport, ConversationRuntimeControlTruth, ConversationSnapshot,
    };
    use crate::domain::recent_sessions::{
        RecentSessions, SessionCatalog, SessionCatalogRequest, SessionCatalogTier,
    };
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[derive(Default)]
    struct FakeCodexAppServerPort {
        recent_session_limits: Mutex<Vec<usize>>,
        new_thread_streams: Mutex<Vec<(String, String)>>,
        turn_streams: Mutex<Vec<(String, String)>>,
    }

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> anyhow::Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server_launch(),
                initialize_detail: "init".to_string(),
                account_detail: "account".to_string(),
                account_ok: true,
                warnings: vec!["warning".to_string()],
            })
        }

        fn load_recent_sessions(&self, limit: usize) -> anyhow::Result<SessionCatalog> {
            self.recent_session_limits
                .lock()
                .expect("recent-session limit mutex poisoned")
                .push(limit);
            Ok(SessionCatalog::ready(
                SessionCatalogTier::ProviderBackedCatalog,
                RecentSessions {
                    items: Vec::new(),
                    warnings: Vec::new(),
                    next_cursor: None,
                },
            ))
        }

        fn load_conversation_snapshot(
            &self,
            thread_id: &str,
        ) -> anyhow::Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "title".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            })
        }

        fn request_stop_all_sessions(&self) -> anyhow::Result<()> {
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            cwd: &str,
            prompt: &str,
            event_sender: Sender<ConversationStreamEvent>,
        ) -> anyhow::Result<()> {
            self.new_thread_streams
                .lock()
                .expect("new-thread stream mutex poisoned")
                .push((cwd.to_string(), prompt.to_string()));
            let _ =
                event_sender.send(ConversationStreamEvent::codex_app_server_launch_attachment());
            let _ = event_sender.send(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: Vec::new(),
            });
            Ok(())
        }

        fn run_turn_stream(
            &self,
            thread_id: &str,
            prompt: &str,
            event_sender: Sender<ConversationStreamEvent>,
        ) -> anyhow::Result<()> {
            self.turn_streams
                .lock()
                .expect("turn stream mutex poisoned")
                .push((thread_id.to_string(), prompt.to_string()));
            let _ =
                event_sender.send(ConversationStreamEvent::codex_app_server_reattach_attachment());
            let _ = event_sender.send(ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-2".to_string(),
                changed_planning_file_paths: Vec::new(),
            });
            Ok(())
        }
    }

    #[test]
    fn startup_probe_port_delegates_to_the_legacy_codex_port() {
        let port = FakeCodexAppServerPort::default();
        let startup_probe_port: &dyn StartupProbePort = &port;

        let context = startup_probe_port
            .load_startup_context()
            .expect("startup probe should load");

        assert_eq!(context.initialize_detail, "init");
        assert_eq!(
            context.attachment_profile,
            TerminalBridgeAttachmentProfile::codex_app_server_launch()
        );
        assert_eq!(context.account_detail, "account");
        assert!(context.account_ok);
        assert_eq!(context.warnings, vec!["warning".to_string()]);
    }

    #[test]
    fn session_catalog_port_delegates_to_the_legacy_codex_port() {
        let port = FakeCodexAppServerPort::default();
        let session_catalog_port: &dyn SessionCatalogPort = &port;

        session_catalog_port
            .load_session_catalog(SessionCatalogRequest::for_workspace(25, "/tmp/root"))
            .expect("recent sessions should load");

        assert_eq!(
            *port
                .recent_session_limits
                .lock()
                .expect("recent-session limit mutex poisoned"),
            vec![25]
        );
    }

    #[test]
    fn interactive_turn_runtime_port_delegates_snapshot_and_stream_calls() {
        let port = FakeCodexAppServerPort::default();
        let runtime_port: &dyn InteractiveTurnRuntimePort = &port;
        let (new_thread_sender, new_thread_receiver) = channel();
        let (turn_sender, turn_receiver) = channel();

        let truth = runtime_port.runtime_control_truth();
        let snapshot = runtime_port
            .load_conversation_snapshot("thread-123")
            .expect("conversation snapshot should load");
        runtime_port
            .run_new_thread_stream("/tmp/root", "prompt", new_thread_sender)
            .expect("new-thread stream should run");
        runtime_port
            .run_turn_stream("thread-123", "follow-up", turn_sender)
            .expect("turn stream should run");

        assert_eq!(
            truth,
            ConversationRuntimeControlTruth::new(
                ConversationControlSupport::ManualHandoff,
                ConversationControlSupport::Unsupported,
            )
        );
        assert_eq!(snapshot.thread_id, "thread-123");
        assert_eq!(
            *port
                .new_thread_streams
                .lock()
                .expect("new-thread stream mutex poisoned"),
            vec![("/tmp/root".to_string(), "prompt".to_string())]
        );
        assert_eq!(
            *port
                .turn_streams
                .lock()
                .expect("turn stream mutex poisoned"),
            vec![("thread-123".to_string(), "follow-up".to_string())]
        );
        assert_eq!(
            new_thread_receiver
                .recv()
                .expect("new-thread stream should emit launch attachment first"),
            ConversationStreamEvent::codex_app_server_launch_attachment()
        );
        assert!(matches!(
            new_thread_receiver
                .recv()
                .expect("new-thread stream should emit a terminal event"),
            ConversationStreamEvent::TurnCompleted { .. }
        ));
        assert_eq!(
            turn_receiver
                .recv()
                .expect("turn stream should emit reattach attachment first"),
            ConversationStreamEvent::codex_app_server_reattach_attachment()
        );
        assert!(matches!(
            turn_receiver
                .recv()
                .expect("turn stream should emit a terminal event"),
            ConversationStreamEvent::TurnCompleted { .. }
        ));
    }
}
