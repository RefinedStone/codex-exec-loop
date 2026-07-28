use std::sync::mpsc;

#[cfg(test)]
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
#[cfg(test)]
use crate::application::service::conversation_service::ConversationService;
#[cfg(test)]
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent;
#[cfg(test)]
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneComposition;
#[cfg(test)]
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
#[cfg(test)]
use crate::application::service::planning::PlanningServices;
#[cfg(test)]
use crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution;
#[cfg(test)]
use crate::application::service::session_service::SessionService;
#[cfg(test)]
use crate::application::service::startup_service::StartupService;
use crate::composition::native_client_runtime::{
    NativeClientDispatchOutcome, NativeClientEvent, NativeClientRuntime,
    NativeTuiApplicationComposition,
};
#[cfg(test)]
use crate::core::app::StartupReadySnapshot;
#[cfg(test)]
use crate::core::app::TurnStreamEvent;
use crate::core::app::{
    AppCommand, AppEvent, ConversationSnapshot as CoreConversationSnapshot, CoreDispatchOutcome,
    CoreInput, SessionCatalogLoadIntent, SessionCatalogLoadMode, SessionCatalogSnapshot,
    StartupSnapshot,
};
#[cfg(test)]
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::operator_alert::OperatorAlert;

use super::{
    AutoFollowControlEvent, AutoFollowOverlayUiEvent, AutoFollowOverlayUiState,
    AutoFollowSnapshotPresentation, ConversationComposerEffect, ConversationComposerEvent,
    ConversationInputEvent, ConversationIntentEffect, ConversationIntentEvent,
    ConversationIntentMode, ConversationIntentState, ConversationLifecycleEffect,
    ConversationLifecycleEvent, ConversationLifecycleState, ConversationRuntimeEffect,
    ConversationRuntimeEvent, ConversationState, ConversationViewModel,
    GithubReviewPollingBootstrap, NativeTuiApp, PendingResumedSessionPlanningRefresh,
    PlanningInitOverlayUiState, SESSION_PAGE_SIZE, SessionOverlayUiState, SessionState,
    ShellChromeEffect, ShellChromeEvent, ShellChromeReduction, ShellChromeState, ShellOverlay,
    ShellOverlayExitMode, ShellOverlayTransition, StartupState, max_auto_turns_command,
    reduce_auto_follow_overlay_ui, reduce_conversation_input, reduce_conversation_intents,
    reduce_conversation_lifecycle, reduce_conversation_runtime_with_transition,
    reduce_shell_chrome, startup_ascii_art_enabled_from_environment,
};

// Background control-plane and poll results are lower volume than token events,
// but still cross thread boundaries. A fixed queue keeps a stalled terminal from
// retaining unbounded notices while leaving room for one full render batch and
// a short producer burst. Capacity must stay above the shell's 128-message drain
// budget so the event loop can observe backlog and yield without a same-thread
// producer deadlock.
pub(super) const TUI_BACKGROUND_CHANNEL_CAPACITY: usize = 256;

/* NativeTuiApp is assembled as reducer-owned state plus composition-owned runtime
 * facade. Runtime files keep pure reducers away from threads and raw services:
 * reducers return effects, the typed client runtime executes them, and
 * ShellRuntime later drains their messages back into reducers.
 */
#[derive(Debug, Clone)]
pub(super) enum BackgroundMessage {
    #[cfg(test)]
    StartupLoaded(Result<Box<StartupReadySnapshot>, String>),
    #[cfg(test)]
    ConversationLoaded(Result<ConversationSnapshot, String>),
    #[cfg(test)]
    ConversationStream {
        correlation: crate::core::app::TurnSubmissionCorrelation,
        event: ConversationStreamEvent,
    },
    #[cfg(test)]
    ConversationRuntimeNotice(String),
    OperatorAlert(OperatorAlert),
    #[cfg(test)]
    PostTurnEvaluationCompleted {
        correlation: crate::core::app::PostTurnEvaluationCorrelation,
        execution: Box<PostTurnEvaluationExecution>,
    },
}

pub(super) struct NativeTuiAppRuntimeChannels {
    tx: mpsc::SyncSender<BackgroundMessage>,
    rx: mpsc::Receiver<BackgroundMessage>,
}

impl NativeTuiAppRuntimeChannels {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(TUI_BACKGROUND_CHANNEL_CAPACITY);
        Self { tx, rx }
    }
}

#[cfg(test)]
pub(super) fn core_turn_stream_event_from_application(
    event: ConversationStreamEvent,
) -> TurnStreamEvent {
    match event {
        ConversationStreamEvent::AttachmentObserved { profile } => {
            TurnStreamEvent::AttachmentObserved { profile }
        }
        ConversationStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
            runtime_envelope,
        } => TurnStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
            runtime_envelope,
        },
        ConversationStreamEvent::TurnStarted {
            turn_id,
            runtime_request,
        } => TurnStreamEvent::TurnStarted {
            turn_id,
            runtime_request,
        },
        ConversationStreamEvent::RuntimeEnvelopeObserved { observation } => {
            TurnStreamEvent::RuntimeEnvelopeObserved { observation }
        }
        ConversationStreamEvent::ItemLifecycleObserved { observation } => {
            TurnStreamEvent::ItemLifecycleObserved { observation }
        }
        ConversationStreamEvent::ProgressiveActivityObserved { batch } => {
            TurnStreamEvent::ProgressiveActivityObserved { batch }
        }
        ConversationStreamEvent::StatusUpdated { text } => TurnStreamEvent::StatusUpdated { text },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        } => TurnStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        },
        ConversationStreamEvent::ToolActivity { activity } => {
            TurnStreamEvent::ToolActivity { activity }
        }
        ConversationStreamEvent::ApprovalReviewUpdated { review } => {
            TurnStreamEvent::ApprovalReviewUpdated { review }
        }
        ConversationStreamEvent::ApprovalRequested { request } => {
            TurnStreamEvent::ApprovalRequested { request }
        }
        ConversationStreamEvent::ApprovalResolved {
            request_identity,
            resolution,
        } => TurnStreamEvent::ApprovalResolved {
            request_identity,
            resolution,
        },
        ConversationStreamEvent::TurnInterruptRequestFailed { message } => {
            TurnStreamEvent::TurnInterruptRequestFailed { message }
        }
        ConversationStreamEvent::TurnRetrying {
            thread_id,
            turn_id,
            error,
        } => TurnStreamEvent::TurnRetrying {
            thread_id,
            turn_id,
            error,
        },
        ConversationStreamEvent::TurnTerminal { receipt } => TurnStreamEvent::TurnTerminal {
            receipt,
            execution_snapshot_capture: None,
        },
        ConversationStreamEvent::Failed { message } => TurnStreamEvent::Failed { message },
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::parallel_peek_overlay_ui::ParallelPeekConversationPreview;
    use crate::adapter::inbound::tui::app::test_helpers;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
    use crate::application::port::outbound::review_center_repository_port::{
        ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterRepositoryPort,
        ReviewCenterThreadProjection,
    };
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::application::port::outbound::startup_probe_port::{
        AppServerStartupContext, StartupProbePort,
    };
    use crate::application::service::review_center::ReviewCenterReadService;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{
        ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessage,
        ConversationMessageKind, ConversationToolActivity, ConversationToolActivityKind,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
        ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    };
    use crate::domain::parallel_mode::{
        ParallelModeControlPlaneWorkerEvent, ParallelModeControlPlaneWorkerEventKind,
    };
    use crate::domain::planning::{PlanningWorkerPanelState, PlanningWorkerStatus};
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogRequest};
    use crate::domain::session_summary::SessionSummary;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use anyhow::Result;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn application_stream_events_map_to_core_stream_events() {
        let activity = ConversationToolActivity {
            kind: ConversationToolActivityKind::CommandExecution,
            text: "cargo test".to_string(),
            file_change_count: 0,
        };
        let review = ConversationApprovalReview {
            target_item_id: "tool-1".to_string(),
            status: ConversationApprovalReviewStatus::InProgress,
            risk_level: Some("medium".to_string()),
            rationale: Some("needs approval".to_string()),
        };
        let attachment_profile = TerminalBridgeAttachmentProfile::codex_app_server_launch();
        let terminal_receipt =
            crate::domain::turn_terminal::ConversationTurnTerminalReceipt::completed(
                "thread-1",
                "turn-1",
                vec!["docs/plan.md".to_string()],
            )
            .with_application_delivery(
                crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
            );
        let progressive_batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence: 0,
                thread_id: "thread-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("item-1".to_string()),
                kind: ConversationProgressiveActivityKind::AgentMessageDelta,
                payload: ConversationProgressiveActivityPayload::AgentMessageDelta {
                    phase: Some("analysis".to_string()),
                    text: "hello".to_string(),
                    source_bytes: 5,
                    truncated_bytes: 0,
                },
            },
        )
        .expect("progressive mapping fixture should be valid");

        let cases = vec![
            (
                ConversationStreamEvent::attachment_observed(attachment_profile),
                TurnStreamEvent::AttachmentObserved {
                    profile: attachment_profile,
                },
            ),
            (
                ConversationStreamEvent::ThreadPrepared {
                    thread_id: "thread-1".to_string(),
                    title: "Title".to_string(),
                    cwd: "/repo".to_string(),
                    runtime_envelope: Box::default(),
                },
                TurnStreamEvent::ThreadPrepared {
                    thread_id: "thread-1".to_string(),
                    title: "Title".to_string(),
                    cwd: "/repo".to_string(),
                    runtime_envelope: Box::default(),
                },
            ),
            (
                ConversationStreamEvent::TurnStarted {
                    turn_id: "turn-1".to_string(),
                    runtime_request: Box::default(),
                },
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-1".to_string(),
                    runtime_request: Box::default(),
                },
            ),
            (
                ConversationStreamEvent::StatusUpdated {
                    text: "thinking".to_string(),
                },
                TurnStreamEvent::StatusUpdated {
                    text: "thinking".to_string(),
                },
            ),
            (
                ConversationStreamEvent::ProgressiveActivityObserved {
                    batch: Box::new(progressive_batch.clone()),
                },
                TurnStreamEvent::ProgressiveActivityObserved {
                    batch: Box::new(progressive_batch),
                },
            ),
            (
                ConversationStreamEvent::AgentMessageCompleted {
                    item_id: "item-2".to_string(),
                    phase: None,
                    text: "done".to_string(),
                },
                TurnStreamEvent::AgentMessageCompleted {
                    item_id: "item-2".to_string(),
                    phase: None,
                    text: "done".to_string(),
                },
            ),
            (
                ConversationStreamEvent::ToolActivity {
                    activity: activity.clone(),
                },
                TurnStreamEvent::ToolActivity { activity },
            ),
            (
                ConversationStreamEvent::ApprovalReviewUpdated {
                    review: review.clone(),
                },
                TurnStreamEvent::ApprovalReviewUpdated { review },
            ),
            (
                ConversationStreamEvent::TurnTerminal {
                    receipt: terminal_receipt.clone(),
                },
                TurnStreamEvent::TurnTerminal {
                    receipt: terminal_receipt,
                    execution_snapshot_capture: None,
                },
            ),
            (
                ConversationStreamEvent::Failed {
                    message: "stream failed".to_string(),
                },
                TurnStreamEvent::Failed {
                    message: "stream failed".to_string(),
                },
            ),
        ];

        for (application_event, expected) in cases {
            assert_eq!(
                core_turn_stream_event_from_application(application_event),
                expected
            );
        }
    }

    #[test]
    fn tui_background_channel_is_bounded_and_disconnects_producers() {
        let channels = NativeTuiAppRuntimeChannels::new();
        for sequence in 0..TUI_BACKGROUND_CHANNEL_CAPACITY {
            channels
                .tx
                .try_send(BackgroundMessage::ConversationRuntimeNotice(
                    sequence.to_string(),
                ))
                .expect("messages within the fixed capacity should be admitted");
        }
        assert!(matches!(
            channels
                .tx
                .try_send(BackgroundMessage::ConversationRuntimeNotice(
                    "overflow".to_string(),
                )),
            Err(mpsc::TrySendError::Full(_))
        ));

        let NativeTuiAppRuntimeChannels { tx, rx } = channels;
        drop(rx);
        assert!(
            tx.send(BackgroundMessage::ConversationRuntimeNotice(
                "disconnected".to_string(),
            ))
            .is_err()
        );
    }

    #[test]
    fn shared_core_snapshot_identity_does_not_suppress_tui_events() {
        let mut app = test_helpers::test_native_tui_app();
        let first = app.reduce_core_client_event(CoreInput::ConversationRuntimeNotice(
            "first notice".to_string(),
        ));
        let second = app.reduce_core_client_event(CoreInput::ConversationRuntimeNotice(
            "second notice".to_string(),
        ));

        assert!(Arc::ptr_eq(&first.snapshot, &second.snapshot));
        app.apply_core_dispatch_outcome(first);
        app.apply_core_dispatch_outcome(second);

        let ConversationState::Ready(conversation) = &app.conversation.lifecycle.conversation_state
        else {
            panic!("test conversation should remain ready");
        };
        assert!(
            conversation
                .runtime_notices
                .ends_with(&["first notice".to_string(), "second notice".to_string()])
        );
    }

    #[derive(Default)]
    struct FakeReviewCenterRepository {
        thread_reviews: Mutex<Vec<ReviewCenterThreadProjection>>,
        pending_inbox: Mutex<Vec<ReviewCenterInboxItem>>,
        history: Mutex<Vec<ReviewCenterHistoryEntry>>,
        thread_review_load_count: AtomicUsize,
        pending_inbox_replace_count: AtomicUsize,
        thread_review_load_entered: Option<mpsc::Sender<()>>,
        thread_review_load_release: Mutex<Option<mpsc::Receiver<()>>>,
    }

    impl FakeReviewCenterRepository {
        fn with_thread_review_load_gate(
            entered: mpsc::Sender<()>,
            release: mpsc::Receiver<()>,
        ) -> Self {
            Self {
                thread_review_load_entered: Some(entered),
                thread_review_load_release: Mutex::new(Some(release)),
                ..Self::default()
            }
        }
    }

    impl ReviewCenterRepositoryPort for FakeReviewCenterRepository {
        fn load_thread_reviews(
            &self,
            _workspace_dir: &str,
            _thread_id: &str,
        ) -> Result<Vec<ReviewCenterThreadProjection>> {
            self.thread_review_load_count.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = &self.thread_review_load_entered {
                let _ = entered.send(());
            }
            if let Some(release) = self
                .thread_review_load_release
                .lock()
                .expect("thread review release mutex poisoned")
                .as_ref()
            {
                release
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|error| anyhow::anyhow!("review persistence gate failed: {error}"))?;
            }
            Ok(self
                .thread_reviews
                .lock()
                .expect("thread review mutex poisoned")
                .clone())
        }

        fn load_pending_inbox(&self, _workspace_dir: &str) -> Result<Vec<ReviewCenterInboxItem>> {
            Ok(self
                .pending_inbox
                .lock()
                .expect("pending inbox mutex poisoned")
                .clone())
        }

        fn load_recent_history(
            &self,
            _workspace_dir: &str,
        ) -> Result<Vec<ReviewCenterHistoryEntry>> {
            Ok(self.history.lock().expect("history mutex poisoned").clone())
        }

        fn upsert_thread_review(
            &self,
            _workspace_dir: &str,
            review: &ReviewCenterThreadProjection,
        ) -> Result<()> {
            let mut thread_reviews = self
                .thread_reviews
                .lock()
                .expect("thread review mutex poisoned");
            if let Some(existing) = thread_reviews
                .iter_mut()
                .find(|existing| existing.review_id == review.review_id)
            {
                *existing = review.clone();
            } else {
                thread_reviews.push(review.clone());
            }
            Ok(())
        }

        fn replace_pending_inbox(
            &self,
            _workspace_dir: &str,
            inbox: &[ReviewCenterInboxItem],
        ) -> Result<()> {
            *self
                .pending_inbox
                .lock()
                .expect("pending inbox mutex poisoned") = inbox.to_vec();
            self.pending_inbox_replace_count
                .fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn append_history_entry(
            &self,
            _workspace_dir: &str,
            entry: &ReviewCenterHistoryEntry,
        ) -> Result<()> {
            self.history
                .lock()
                .expect("history mutex poisoned")
                .push(entry.clone());
            Ok(())
        }
    }

    #[derive(Default)]
    struct ReviewPersistenceRuntimePort;

    impl StartupProbePort for ReviewPersistenceRuntimePort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }
    }

    impl SessionCatalogPort for ReviewPersistenceRuntimePort {
        fn load_session_catalog(&self, _request: SessionCatalogRequest) -> Result<SessionCatalog> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into())
        }
    }

    impl InteractiveTurnRuntimePort for ReviewPersistenceRuntimePort {
        fn runtime_control_truth(
            &self,
        ) -> crate::domain::conversation::ConversationRuntimeControlTruth {
            crate::domain::conversation::ConversationRuntimeControlTruth::codex_app_server()
        }

        fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            })
        }

        fn request_stop_all_sessions(&self) -> Result<()> {
            Ok(())
        }

        fn run_new_thread_stream(
            &self,
            cwd: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
        ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
            crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
                &event_sender,
                "test-thread",
                cwd,
            )
        }

        fn run_turn_stream(
            &self,
            thread_id: &str,
            _prompt: &str,
            _options: crate::domain::conversation::ConversationTurnOptions,
            event_sender: crate::application::service::conversation_runtime_event::ConversationStreamSender,
        ) -> Result<crate::domain::turn_terminal::ConversationTurnTerminalReceipt> {
            crate::application::service::conversation_runtime_event::emit_confirmed_test_terminal_receipt(
                &event_sender,
                thread_id,
                "/tmp/test-workspace",
            )
        }
    }

    fn review_persistence_app(
        review_repository: Arc<dyn ReviewCenterRepositoryPort>,
    ) -> NativeTuiApp {
        let runtime_port = Arc::new(ReviewPersistenceRuntimePort);
        let planning = test_helpers::test_planning_services(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        let parallel_mode_binding = NativeTuiParallelModeBinding::from_composition(
            test_helpers::test_parallel_mode_control_plane_composition(planning),
        );
        let conversation_service =
            ConversationService::new(runtime_port.clone()).with_review_center_read_service(
                ReviewCenterReadService::new("/tmp/root", review_repository),
            );
        NativeTuiApp::new(
            StartupService::new(runtime_port.clone()),
            SessionService::new(runtime_port),
            conversation_service,
            parallel_mode_binding,
        )
    }

    fn prepare_review_persistence_turn(
        app: &mut NativeTuiApp,
    ) -> crate::core::app::TurnSubmissionCorrelation {
        let correlation = app.runtime.client_runtime.begin_test_turn_submission();
        for event in [
            TurnStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Thread".to_string(),
                cwd: "/tmp/root".to_string(),
                runtime_envelope: Box::default(),
            },
            TurnStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
            },
        ] {
            dispatch_review_persistence_stream_event(app, correlation, event);
        }
        correlation
    }

    fn dispatch_review_persistence_stream_event(
        app: &mut NativeTuiApp,
        correlation: crate::core::app::TurnSubmissionCorrelation,
        event: TurnStreamEvent,
    ) {
        let outcome = app
            .reduce_core_client_event(CoreInput::ConversationStreamUpdated { correlation, event });
        app.apply_core_dispatch_outcome(outcome);
    }

    #[test]
    fn tui_parallel_binding_shares_one_automation_guard_with_post_turn_delivery() {
        let planning = test_helpers::test_planning_services(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        let composition = test_helpers::test_parallel_mode_control_plane_composition(planning);
        let parallel_turns = composition.parallel_mode_turn_service();
        let mut app = test_helpers::test_native_tui_app_with_parallel_mode_composition(composition);
        let workspace = "/tmp/shared-automation-guard".to_string();

        app.open_parallel_mode_automation_epoch(workspace.clone());
        let epoch_id = app
            .runtime
            .client_runtime
            .current_parallel_epoch_id_for_workspace(&workspace)
            .expect("open epoch should expose its id");
        assert!(parallel_turns.automation_epoch_is_active(&workspace, epoch_id));

        app.close_parallel_mode_automation_epoch();
        assert!(!parallel_turns.automation_epoch_is_active(&workspace, epoch_id));
    }

    #[test]
    fn duplicate_approval_review_updates_keep_one_history_entry() {
        let review_repository = Arc::new(FakeReviewCenterRepository::default());
        let mut app = review_persistence_app(review_repository.clone());
        let correlation = prepare_review_persistence_turn(&mut app);
        let review = ConversationApprovalReview {
            target_item_id: "tool-9".to_string(),
            status: ConversationApprovalReviewStatus::Unknown("human_review_requested".to_string()),
            risk_level: Some("medium".to_string()),
            rationale: Some("Need operator follow-up".to_string()),
        };
        dispatch_review_persistence_stream_event(
            &mut app,
            correlation,
            TurnStreamEvent::ApprovalReviewUpdated {
                review: review.clone(),
            },
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while review_repository
            .history
            .lock()
            .expect("history mutex poisoned")
            .is_empty()
            && Instant::now() < deadline
        {
            app.poll_client_runtime_events(16);
            std::thread::yield_now();
        }
        let duplicate_deadline = Instant::now() + Duration::from_secs(2);
        while review_repository
            .pending_inbox_replace_count
            .load(Ordering::SeqCst)
            < 2
            && Instant::now() < duplicate_deadline
        {
            app.poll_client_runtime_events(16);
            dispatch_review_persistence_stream_event(
                &mut app,
                correlation,
                TurnStreamEvent::ApprovalReviewUpdated {
                    review: review.clone(),
                },
            );
            std::thread::yield_now();
        }
        assert_eq!(
            review_repository
                .pending_inbox_replace_count
                .load(Ordering::SeqCst),
            2,
            "both serialized duplicate writes should finish their final repository mutation"
        );
        assert_eq!(
            review_repository
                .thread_review_load_count
                .load(Ordering::SeqCst),
            2
        );
        while app.poll_client_runtime_events(16) {}

        let thread_reviews = review_repository
            .thread_reviews
            .lock()
            .expect("thread review mutex poisoned")
            .clone();
        assert_eq!(thread_reviews.len(), 1);
        assert_eq!(thread_reviews[0].thread_id, "thread-1");
        assert_eq!(thread_reviews[0].review_id, "tool-9");
        assert_eq!(thread_reviews[0].review_label, "manual handoff");
        assert_eq!(thread_reviews[0].review_state, "waiting");

        let inbox = review_repository
            .pending_inbox
            .lock()
            .expect("pending inbox mutex poisoned")
            .clone();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].review_id, "tool-9");
        assert_eq!(inbox[0].thread_id, "thread-1");
        assert_eq!(inbox[0].inbox_state, "waiting");

        let history = review_repository
            .history
            .lock()
            .expect("history mutex poisoned")
            .clone();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].review_id, "tool-9");
        assert_eq!(history[0].thread_id, "thread-1");
        assert_eq!(
            history[0].event_kind,
            "manual_handoff_human_review_requested"
        );
    }

    #[test]
    fn approval_review_persistence_does_not_block_stream_dispatch_or_render() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let review_repository = Arc::new(FakeReviewCenterRepository::with_thread_review_load_gate(
            entered_tx, release_rx,
        ));
        let mut app = review_persistence_app(review_repository.clone());
        let correlation = prepare_review_persistence_turn(&mut app);
        let (returned_tx, returned_rx) = mpsc::sync_channel(1);

        let dispatch_thread = std::thread::spawn(move || {
            dispatch_review_persistence_stream_event(
                &mut app,
                correlation,
                TurnStreamEvent::ApprovalReviewUpdated {
                    review: ConversationApprovalReview {
                        target_item_id: "tool-gated".to_string(),
                        status: ConversationApprovalReviewStatus::InProgress,
                        risk_level: Some("medium".to_string()),
                        rationale: Some("gated repository".to_string()),
                    },
                },
            );
            let _screen_model =
                crate::adapter::inbound::tui::app::shell_presentation::ConversationScreenModel::from_app(
                    &app,
                );
            let review_is_projected = matches!(
                &app.conversation.lifecycle.conversation_state,
                ConversationState::Ready(conversation)
                    if conversation.approval_review().is_some_and(|review| {
                        review.target_item_id == "tool-gated"
                    })
            );
            returned_tx
                .send((app, review_is_projected))
                .expect("test receiver should remain connected");
        });

        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("repository worker should reach the gate");
        let returned = returned_rx.recv_timeout(Duration::from_secs(2));
        release_tx
            .send(())
            .expect("repository worker should remain gated");
        let (mut app, review_is_projected) = returned
            .expect("stream dispatch and screen projection must return before repository release");
        dispatch_thread
            .join()
            .expect("stream dispatch thread should not panic");
        assert!(review_is_projected);

        let deadline = Instant::now() + Duration::from_secs(2);
        while review_repository
            .history
            .lock()
            .expect("history mutex poisoned")
            .is_empty()
            && Instant::now() < deadline
        {
            app.poll_client_runtime_events(16);
            std::thread::yield_now();
        }
        assert_eq!(
            review_repository
                .history
                .lock()
                .expect("history mutex poisoned")
                .len(),
            1
        );
    }

    #[test]
    fn turn_budget_draft_closes_only_after_acceptance_or_context_change() {
        let mut app = test_helpers::test_native_tui_app();
        app.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditStarted {
            current_value: "off".to_string(),
        });
        app.dispatch_auto_follow_controls(AutoFollowControlEvent::DraftWorkspaceSynced {
            workspace_directory: "/tmp/root".to_string(),
        });
        assert_eq!(app.max_auto_turns_edit_buffer(), Some("off"));

        app.dispatch_auto_follow_controls(AutoFollowControlEvent::MaxAutoTurnsUpdated {
            value: "invalid".to_string(),
        });
        assert_eq!(app.max_auto_turns_edit_buffer(), Some("off"));
        assert_eq!(app.current_max_auto_turns_label(), "off");

        app.dispatch_auto_follow_controls(AutoFollowControlEvent::MaxAutoTurnsUpdated {
            value: "5".to_string(),
        });
        assert_eq!(app.max_auto_turns_edit_buffer(), None);
        assert_eq!(app.current_max_auto_turns_label(), "5");

        app.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditStarted {
            current_value: "5".to_string(),
        });
        app.dispatch_auto_follow_controls(AutoFollowControlEvent::DraftWorkspaceSynced {
            workspace_directory: "/tmp/other".to_string(),
        });
        assert_eq!(app.max_auto_turns_edit_buffer(), None);
    }

    #[test]
    fn workspace_and_conversation_supersession_reset_tui_intent_state() {
        let mut app = test_helpers::test_native_tui_app();
        let initial_history_identity_revision =
            app.conversation.conversation_history_identity_revision;

        app.dispatch_auto_follow_controls(AutoFollowControlEvent::DraftWorkspaceSynced {
            workspace_directory: "/tmp/root".to_string(),
        });

        app.dispatch_auto_follow_controls(AutoFollowControlEvent::DraftWorkspaceSynced {
            workspace_directory: "/tmp/other".to_string(),
        });

        arm_pending_manual_prompt_for_identity_test(&mut app, "new draft prompt");
        app.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditStarted {
            current_value: "off".to_string(),
        });
        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory: "/tmp/root".to_string(),
        });
        assert_ne!(
            app.conversation.conversation_history_identity_revision,
            initial_history_identity_revision
        );
        assert!(app.conversation.pending_manual_prompt_preparation.is_none());
        assert!(matches!(
            &app.conversation.lifecycle.conversation_state,
            ConversationState::Ready(conversation) if conversation.cwd == "/tmp/root"
        ));
        assert_eq!(app.max_auto_turns_edit_buffer(), None);

        let draft_history_identity_revision =
            app.conversation.conversation_history_identity_revision;
        arm_pending_manual_prompt_for_identity_test(&mut app, "session prompt");
        app.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditStarted {
            current_value: "off".to_string(),
        });
        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
            session: test_session_summary("thread-2"),
            fallback_workspace_directory: "/tmp/root".to_string(),
        });
        assert_eq!(
            app.conversation.conversation_history_identity_revision,
            draft_history_identity_revision
        );
        assert!(app.conversation.pending_manual_prompt_preparation.is_none());
        assert!(matches!(
            app.conversation.lifecycle.conversation_state,
            ConversationState::Loading
        ));
        assert_eq!(app.max_auto_turns_edit_buffer(), None);
        app.apply_core_conversation_snapshot(test_core_conversation_snapshot("thread-2"));
        assert_ne!(
            app.conversation.conversation_history_identity_revision,
            draft_history_identity_revision,
            "only accepted Ready publishes the loaded history identity"
        );
    }

    #[test]
    fn draft_promotion_and_same_thread_reattach_preserve_history_identity() {
        let mut app = test_helpers::test_native_tui_app();
        let history_identity_revision = app.conversation.conversation_history_identity_revision;
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should start ready");
        };
        conversation.record_thread_prepared(
            "thread-same".to_string(),
            "Same thread".to_string(),
            "/tmp/root".to_string(),
        );
        assert_eq!(
            app.conversation.conversation_history_identity_revision,
            history_identity_revision
        );

        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
            session: test_session_summary("thread-same"),
            fallback_workspace_directory: "/tmp/root".to_string(),
        });
        assert_eq!(
            app.conversation.conversation_history_identity_revision,
            history_identity_revision
        );
        app.apply_core_conversation_snapshot(CoreConversationSnapshot::Failed {
            message: "temporary failure".to_string(),
        });
        assert_eq!(
            app.conversation.conversation_history_thread_id.as_deref(),
            Some("thread-same")
        );
        assert_eq!(
            app.conversation.conversation_history_identity_revision,
            history_identity_revision
        );

        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
            session: test_session_summary("thread-same"),
            fallback_workspace_directory: "/tmp/root".to_string(),
        });
        app.apply_core_conversation_snapshot(test_core_conversation_snapshot("thread-same"));
        assert_eq!(
            app.conversation.conversation_history_identity_revision, history_identity_revision,
            "a failed same-thread reattach must not make its retry a new history"
        );
    }

    #[test]
    fn deferred_session_load_preserves_ready_history_identity_until_core_accepts_it() {
        let mut app = test_helpers::test_native_tui_app();
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should start ready");
        };
        conversation.record_thread_prepared(
            "thread-a".to_string(),
            "Thread A".to_string(),
            "/tmp/root".to_string(),
        );
        app.dispatch_client_event(CoreInput::Command(AppCommand::RenameSession(
            crate::domain::recent_sessions::SessionRenameRequest::new("thread-b", "Thread B"),
        )));
        let history_identity_revision = app.conversation.conversation_history_identity_revision;

        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
            session: test_session_summary("thread-b"),
            fallback_workspace_directory: "/tmp/root".to_string(),
        });

        assert!(matches!(
            &app.conversation.lifecycle.conversation_state,
            ConversationState::Ready(conversation) if conversation.thread_id == "thread-a"
        ));
        assert_eq!(
            app.conversation.conversation_history_identity_revision, history_identity_revision,
            "a deferred load intent must not publish a terminal history reset"
        );
    }

    #[test]
    fn conversation_lifecycle_closes_only_epochs_owned_by_the_workspace_being_left() {
        let mut app = test_helpers::test_native_tui_app();
        app.runtime
            .client_runtime
            .force_parallel_epoch_for_test("/tmp/worker-b", 1);
        assert!(
            app.runtime
                .client_runtime
                .parallel_automation_epoch_is_active_for_test("/tmp/worker-b", 1)
        );

        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory: "/tmp/root".to_string(),
        });

        assert_eq!(
            app.runtime.client_runtime.parallel_epoch_snapshot(),
            crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneEpochSnapshot {
                workspace_directory: None,
                current_epoch_id: None,
            }
        );
        assert!(
            !app.runtime
                .client_runtime
                .parallel_automation_epoch_is_active_for_test("/tmp/worker-b", 1)
        );

        let draft_projection = app.planning_runtime_projection_snapshot();
        app.apply_parallel_mode_control_plane_background_event(
            ParallelModeControlPlaneBackgroundEvent::WorkerEvent {
                event: ParallelModeControlPlaneWorkerEvent::new(
                    "/tmp/worker-b",
                    1,
                    "task-b",
                    "Worker B",
                    ParallelModeControlPlaneWorkerEventKind::Completed,
                    vec!["worker B completed".to_string()],
                ),
                has_actionable_queue_head: false,
            },
        );
        assert_eq!(app.planning_runtime_projection_snapshot(), draft_projection);

        app.runtime
            .client_runtime
            .force_parallel_epoch_for_test("/tmp/root", 2);
        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory: "/tmp/root".to_string(),
        });
        assert_eq!(
            app.runtime
                .client_runtime
                .current_parallel_epoch_id_for_workspace("/tmp/root"),
            Some(2)
        );

        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
            session: SessionSummary {
                id: "thread-b".to_string(),
                name: Some("Thread B".to_string()),
                preview: "preview".to_string(),
                cwd: "/tmp/worker-b".to_string(),
                source: "test".to_string(),
                model_provider: "test".to_string(),
                updated_at_epoch: 1,
                status_type: "idle".to_string(),
                path: "/tmp/worker-b/thread-b".to_string(),
                git_branch: None,
            },
            fallback_workspace_directory: "/tmp/root".to_string(),
        });
        assert!(
            app.runtime
                .client_runtime
                .parallel_epoch_snapshot()
                .current_epoch_id
                .is_none()
        );
    }

    #[test]
    fn tui_startup_projection_applies_only_core_accepted_snapshots() {
        let mut app = test_helpers::test_native_tui_app();
        app.apply_core_startup_snapshot(StartupSnapshot::Loading);
        assert!(matches!(
            app.shell.chrome.startup_state,
            StartupState::Loading
        ));
        app.apply_core_startup_snapshot(StartupSnapshot::Ready(test_startup_ready_snapshot(
            "/tmp/latest",
        )));
        assert!(matches!(
            &app.shell.chrome.startup_state,
            StartupState::Ready(ready) if ready.workspace_path == "/tmp/latest"
        ));
    }

    #[test]
    fn tui_conversation_projection_applies_only_core_accepted_snapshots() {
        let mut app = test_helpers::test_native_tui_app();
        let history_identity_revision = app.conversation.conversation_history_identity_revision;
        app.apply_core_conversation_snapshot(CoreConversationSnapshot::Loading);
        assert!(matches!(
            app.conversation.lifecycle.conversation_state,
            ConversationState::Loading
        ));
        assert_eq!(
            app.conversation.conversation_history_identity_revision,
            history_identity_revision
        );

        app.apply_core_conversation_snapshot(test_core_conversation_snapshot("thread-b"));
        assert!(matches!(
            &app.conversation.lifecycle.conversation_state,
            ConversationState::Ready(conversation) if conversation.thread_id == "thread-b"
        ));
        let loaded_history_identity_revision =
            app.conversation.conversation_history_identity_revision;
        assert_ne!(loaded_history_identity_revision, history_identity_revision);
    }

    #[test]
    fn core_conversation_lifecycle_alone_resets_planning_worker_projection() {
        let seeded = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RepairFailed,
            last_summary: Some("previous conversation".to_string()),
            ..PlanningWorkerPanelState::default()
        };
        let cases = [
            ("idle", CoreConversationSnapshot::Idle, true),
            ("loading", CoreConversationSnapshot::Loading, true),
            (
                "ready",
                test_core_conversation_snapshot("thread-ready"),
                false,
            ),
            (
                "failed",
                CoreConversationSnapshot::Failed {
                    message: "load failed".to_string(),
                },
                false,
            ),
        ];

        for (name, snapshot, resets_projection) in cases {
            let mut app = test_helpers::test_native_tui_app();
            app.planning
                .planning_worker_panel_state
                .replace_for_test(seeded.clone());

            app.apply_core_conversation_snapshot(snapshot);

            let expected = if resets_projection {
                PlanningWorkerPanelState::default()
            } else {
                seeded.clone()
            };
            assert_eq!(
                app.planning.planning_worker_panel_state.current(),
                &expected,
                "{name} must obey the Core-owned lifecycle reset contract"
            );
        }
    }

    #[test]
    fn cancelled_post_turn_authority_resets_worker_panel_and_late_drop_cannot_revive_it() {
        let mut app = test_helpers::test_native_tui_app();
        let mut previous_runtime = app.conversation_runtime_projection();
        previous_runtime.post_turn = crate::core::app::PostTurnAuthoritySnapshot::Evaluating {
            correlation: crate::core::app::PostTurnEvaluationCorrelation::new(
                1,
                "thread-1",
                "turn-1",
                "/tmp/root",
                "/tmp/root",
            ),
            started_at: Instant::now(),
        };
        app.apply_conversation_runtime_projection(previous_runtime.clone());
        app.planning
            .planning_worker_panel_state
            .replace_for_test(PlanningWorkerPanelState {
                status: PlanningWorkerStatus::RefreshRunning,
                last_summary: Some("post-turn evaluation".to_string()),
                ..PlanningWorkerPanelState::default()
            });

        let mut cancelled_runtime = previous_runtime.clone();
        cancelled_runtime.post_turn = crate::core::app::PostTurnAuthoritySnapshot::Idle;
        app.apply_core_event_with_previous_runtime(
            AppEvent::ConversationRuntimeAuthorityChanged(Box::new(cancelled_runtime.clone())),
            &previous_runtime,
        );
        assert_eq!(
            app.planning.planning_worker_panel_state.current(),
            &PlanningWorkerPanelState::default()
        );

        // Core drops a completion whose correlation was cancelled. The only
        // projection TUI can observe is the already-settled authority snapshot.
        app.apply_core_event_with_previous_runtime(
            AppEvent::ConversationRuntimeAuthorityChanged(Box::new(cancelled_runtime.clone())),
            &cancelled_runtime,
        );
        assert_eq!(
            app.planning.planning_worker_panel_state.current(),
            &PlanningWorkerPanelState::default()
        );
    }

    #[test]
    fn parallel_peek_projection_requires_the_visible_matching_preview() {
        let mut app = test_helpers::test_native_tui_app();
        app.shell
            .parallel_peek_overlay_ui_state
            .open_preview(test_parallel_peek_preview("thread-current"));

        app.apply_parallel_peek_conversation_load(
            crate::core::app::ParallelPeekLoadCorrelation::new(1, "thread-current"),
            Err("hidden failure".to_string()),
        );
        assert_eq!(
            app.shell
                .parallel_peek_overlay_ui_state
                .preview()
                .map(|preview| preview.status_text.as_str()),
            Some("conversation snapshot loading")
        );

        app.shell.chrome.shell_overlay = ShellOverlay::ParallelPeek;
        app.apply_parallel_peek_conversation_load(
            crate::core::app::ParallelPeekLoadCorrelation::new(2, "thread-stale"),
            Err("stale failure".to_string()),
        );
        assert_eq!(
            app.shell
                .parallel_peek_overlay_ui_state
                .preview()
                .map(|preview| preview.status_text.as_str()),
            Some("conversation snapshot loading")
        );

        app.apply_parallel_peek_conversation_load(
            crate::core::app::ParallelPeekLoadCorrelation::new(3, "thread-current"),
            Err("current failure".to_string()),
        );
        assert_eq!(
            app.shell
                .parallel_peek_overlay_ui_state
                .preview()
                .map(|preview| preview.status_text.as_str()),
            Some("conversation snapshot failed: current failure")
        );
    }

    #[test]
    fn shell_chrome_supersession_resets_parallel_peek_projection() {
        let mut app = test_helpers::test_native_tui_app();
        app.shell.chrome.shell_overlay = ShellOverlay::ParallelPeek;
        app.shell
            .parallel_peek_overlay_ui_state
            .open_preview(test_parallel_peek_preview("thread-current"));
        app.shell
            .parallel_peek_overlay_ui_state
            .scroll_conversation_older(10);

        app.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);

        assert_eq!(app.shell.chrome.shell_overlay, ShellOverlay::Queue);
        assert!(app.shell.parallel_peek_overlay_ui_state.preview().is_none());
        assert_eq!(
            app.shell
                .parallel_peek_overlay_ui_state
                .conversation_scroll_from_bottom(),
            0
        );
    }

    fn test_parallel_peek_preview(thread_id: &str) -> ParallelPeekConversationPreview {
        ParallelPeekConversationPreview {
            agent_id: "agent-peek".to_string(),
            slot_id: "slot-peek".to_string(),
            task_title: "Inspect conversation".to_string(),
            thread_id: Some(thread_id.to_string()),
            snapshot: None,
            status_text: "conversation snapshot loading".to_string(),
        }
    }

    fn test_startup_ready_snapshot(workspace_path: &str) -> Box<StartupReadySnapshot> {
        Box::new(StartupReadySnapshot {
            cwd: workspace_path.to_string(),
            workspace_path: workspace_path.to_string(),
            can_continue: true,
            codex_binary: crate::core::app::StartupDiagnosticSnapshot {
                ok: true,
                detail: "/usr/bin/codex".to_string(),
            },
            workspace: crate::core::app::StartupDiagnosticSnapshot {
                ok: true,
                detail: workspace_path.to_string(),
            },
            app_server_initialize: crate::core::app::StartupDiagnosticSnapshot {
                ok: true,
                detail: "initialized".to_string(),
            },
            account: crate::core::app::StartupDiagnosticSnapshot {
                ok: true,
                detail: "authenticated".to_string(),
            },
            attachment: crate::core::app::StartupAttachmentSnapshot {
                mode_label: "provider-launched".to_string(),
                recovery_anchor_label: "provider-thread-id".to_string(),
            },
            warnings: Vec::new(),
            schema_snapshot: "embedded schema".to_string(),
        })
    }

    fn test_core_conversation_snapshot(thread_id: &str) -> CoreConversationSnapshot {
        CoreConversationSnapshot::Ready(Box::new(
            crate::core::app::ConversationReadySnapshot::from(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: thread_id.to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            }),
        ))
    }

    fn test_session_summary(thread_id: &str) -> SessionSummary {
        SessionSummary {
            id: thread_id.to_string(),
            name: Some(thread_id.to_string()),
            preview: format!("{thread_id} preview"),
            cwd: "/tmp/root".to_string(),
            source: "test".to_string(),
            model_provider: "test".to_string(),
            updated_at_epoch: 1,
            status_type: "idle".to_string(),
            path: format!("/tmp/root/{thread_id}"),
            git_branch: None,
        }
    }

    fn arm_pending_manual_prompt_for_identity_test(app: &mut NativeTuiApp, transcript_text: &str) {
        let workspace_directory = app.planning_workspace_directory();
        app.conversation.pending_manual_prompt_preparation = Some(
            crate::adapter::inbound::tui::app::PendingManualPromptPreparation {
                correlation: crate::domain::planning::ManualPromptCorrelation {
                    request_id: 1,
                    generation: 1,
                    workspace_directory,
                },
                source_input_buffer: transcript_text.to_string(),
                transcript_text: transcript_text.to_string(),
                parallel_mode_enabled_at_submission: false,
                delivery: crate::adapter::inbound::tui::app::ManualPromptDelivery::StartTurn,
                parent_turn_id: None,
            },
        );
    }

    #[test]
    fn composer_dispatch_preserves_semantic_conversation_state() {
        let mut app = test_helpers::test_native_tui_app();
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should start with a ready conversation");
        };
        conversation.messages.push(ConversationMessage::new(
            ConversationMessageKind::User,
            "existing transcript",
            None,
            None,
        ));
        let approval_review = ConversationApprovalReview {
            target_item_id: "tool-1".to_string(),
            status: ConversationApprovalReviewStatus::InProgress,
            risk_level: Some("medium".to_string()),
            rationale: Some("confirm this command".to_string()),
        };
        let mut runtime_snapshot = conversation.runtime_snapshot().clone();
        runtime_snapshot.active_turn = Some(crate::core::app::ActiveTurnSnapshot {
            correlation: crate::core::app::TurnSubmissionCorrelation::new(1),
            phase: crate::core::app::ActiveTurnPhase::Running,
            workspace_directory: "/tmp/root".to_string(),
            turn_id: Some("turn-1".to_string()),
            prompt_origin: crate::core::app::CorePromptOrigin::Manual,
            started_at: Instant::now(),
        });
        runtime_snapshot.approval_review = Some(approval_review);
        conversation.apply_runtime_snapshot(runtime_snapshot);
        conversation.record_turn_started("turn-1".to_string());
        conversation.status_text = "semantic status".to_string();

        let messages_before = conversation.messages.clone();
        let active_turn_before = conversation.active_turn_id().map(str::to_string);
        let approval_before = conversation.approval_review().cloned();
        let planning_repair_before = conversation.planning_repair_state.clone();
        let status_before = conversation.status_text.clone();

        app.dispatch_conversation_input(ConversationComposerEvent::CharacterTyped {
            character: 'x',
        });

        let ConversationState::Ready(conversation) = &app.conversation.lifecycle.conversation_state
        else {
            panic!("composer dispatch should preserve ready conversation state");
        };
        assert_eq!(conversation.composer.input_buffer, "x");
        assert_eq!(conversation.messages, messages_before);
        assert_eq!(
            conversation.active_turn_id().map(str::to_string),
            active_turn_before
        );
        assert_eq!(conversation.approval_review().cloned(), approval_before);
        assert_eq!(conversation.planning_repair_state, planning_repair_before);
        assert_eq!(conversation.status_text, status_before);
    }

    #[test]
    fn semantic_status_dispatch_preserves_composer_state() {
        let mut app = test_helpers::test_native_tui_app();
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should start with a ready conversation");
        };
        conversation.composer.input_buffer = ":p".to_string();
        conversation.composer.sync_inline_shell_command_palette();
        conversation.composer.arm_startup_submit();
        let composer_before = conversation.composer.clone();

        app.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: "runtime status".to_string(),
        });

        let ConversationState::Ready(conversation) = &app.conversation.lifecycle.conversation_state
        else {
            panic!("status dispatch should preserve ready conversation state");
        };
        assert_eq!(conversation.composer, composer_before);
        assert_eq!(conversation.status_text, "runtime status");
    }
}

#[cfg(test)]
pub(crate) struct NativeTuiParallelModeBinding {
    parallel_turns: ParallelModeTurnService,
    planning_feature: PlanningServices,
    parallel_mode_control_plane: ParallelModeControlPlaneComposition,
    runtime_channels: NativeTuiAppRuntimeChannels,
}

#[cfg(test)]
impl NativeTuiParallelModeBinding {
    pub(crate) fn from_composition(
        composition: ParallelModeControlPlaneComposition,
    ) -> NativeTuiParallelModeBinding {
        let runtime_channels = NativeTuiAppRuntimeChannels::new();
        NativeTuiParallelModeBinding {
            parallel_turns: composition.parallel_mode_turn_service(),
            planning_feature: composition.planning().clone(),
            parallel_mode_control_plane: composition,
            runtime_channels,
        }
    }
}

impl NativeTuiApp {
    #[cfg(test)]
    pub(super) fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_mode_binding: NativeTuiParallelModeBinding,
    ) -> Self {
        let turn_control_truth = conversation_service.runtime_control_truth();
        let NativeTuiParallelModeBinding {
            parallel_turns,
            planning_feature,
            parallel_mode_control_plane,
            runtime_channels,
        } = parallel_mode_binding;
        let client_runtime = NativeClientRuntime::new_for_test(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_turns,
            parallel_mode_control_plane,
        );
        Self::new_with_bound_application(
            client_runtime,
            runtime_channels,
            turn_control_truth,
            GithubReviewPollingBootstrap::disabled(),
        )
    }

    pub(super) fn new_with_github_review_polling(
        application: NativeTuiApplicationComposition,
        github_review_polling: GithubReviewPollingBootstrap,
    ) -> Self {
        let runtime_channels = NativeTuiAppRuntimeChannels::new();
        let application = application.bind_client_runtime();
        let (client_runtime, turn_control_truth) = application.into_parts();
        Self::new_with_bound_application(
            client_runtime,
            runtime_channels,
            turn_control_truth,
            github_review_polling,
        )
    }

    #[cfg(test)]
    pub(super) fn new_with_github_review_polling_setup_loader(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_mode_binding: NativeTuiParallelModeBinding,
        github_review_polling: GithubReviewPollingBootstrap,
        loader: impl Fn(
                &crate::core::app::GithubReviewPollingSetupRequest,
            ) -> anyhow::Result<
                Option<(
                    crate::domain::github_review::GithubPullRequestTarget,
                    crate::application::service::github_review_poller_service::GithubReviewPollerService,
                )>,
            > + Send
            + Sync
            + 'static,
    ) -> Self {
        let turn_control_truth = conversation_service.runtime_control_truth();
        let NativeTuiParallelModeBinding {
            parallel_turns,
            planning_feature,
            parallel_mode_control_plane,
            runtime_channels,
        } = parallel_mode_binding;
        let client_runtime = NativeClientRuntime::new_with_github_review_polling_setup_loader(
            startup_service,
            session_service,
            conversation_service,
            planning_feature,
            parallel_turns,
            parallel_mode_control_plane,
            loader,
        );
        Self::new_with_bound_application(
            client_runtime,
            runtime_channels,
            turn_control_truth,
            github_review_polling,
        )
    }

    fn new_with_bound_application(
        client_runtime: NativeClientRuntime,
        runtime_channels: NativeTuiAppRuntimeChannels,
        turn_control_truth: crate::domain::conversation::ConversationRuntimeControlTruth,
        github_review_polling: GithubReviewPollingBootstrap,
    ) -> Self {
        let GithubReviewPollingBootstrap {
            state: github_review_polling_state,
        } = github_review_polling;

        // The first draft is tied to the process working directory so startup can
        // render planning/runtime context before any session is selected.
        let workspace_directory = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let initial_conversation = ConversationViewModel::new_draft_with_truth(
            workspace_directory.clone(),
            turn_control_truth,
        );
        let mut app = Self {
            shell: super::NativeTuiShellState {
                chrome: ShellChromeState::default(),
                supersession_mud_ui_state: super::SupersessionMudUiState::default(),
                parallel_peek_overlay_ui_state: super::ParallelPeekOverlayUiState::default(),
                progressive_activity_overlay_ui_state:
                    super::ProgressiveActivityOverlayUiState::default(),
                help_scroll_offset: 0,
                reviews_overlay_ui_state: super::reviews_overlay_ui::ReviewsOverlayUiState::default(
                ),
                parallel_supervisor_event_log: super::ParallelSupervisorEventLog::default(),
                session_overlay_ui_state: SessionOverlayUiState::new(SESSION_PAGE_SIZE),
                tui_language: super::TuiLanguage::default(),
                language_selection_overlay_ui_state:
                    super::LanguageSelectionOverlayUiState::default(),
                model_selection_overlay_ui_state: super::ModelSelectionOverlayUiState::default(),
                view_selection_overlay_ui_state: super::ViewSelectionOverlayUiState::default(),
                inline_history_render_mode: super::InlineHistoryRenderMode::from_environment(),
                history_insert_mode: super::HistoryInsertionMode::from_environment(),
                show_startup_ascii_art: startup_ascii_art_enabled_from_environment(),
            },
            conversation: super::NativeTuiConversationState {
                lifecycle: ConversationLifecycleState {
                    conversation_state: ConversationState::ready(initial_conversation),
                    turn_control_truth,
                },
                pending_manual_prompt_preparation: None,
                prompt_input_revision: 0,
                turn_steer_confirmation: None,
                pending_turn_steer: None,
                conversation_history_identity_revision: 0,
                conversation_history_thread_id: None,
                turn_options: Default::default(),
                conversation_view_mode: super::ConversationViewMode::default(),
                auto_follow_overlay_ui_state: AutoFollowOverlayUiState::default(),
            },
            planning: super::NativeTuiPlanningState {
                planning_ui_intent_revision: 0,
                pending_resumed_session_planning_refresh: None,
                queue_overlay_ui_state: super::queue_overlay_ui::QueueOverlayUiState::default(),
                queue_mutation_ui_state: super::queue_overlay_ui::QueueMutationUiState::default(),
                directions_maintenance_overlay_ui_state:
                    super::DirectionsMaintenanceOverlayUiState::default(),
                planning_init_overlay_ui_state: PlanningInitOverlayUiState::default(),
                planning_runtime_refresh_ui_state: super::PlanningRuntimeRefreshUiState::default(),
                planning_workspace_operation_ui_state:
                    super::PlanningWorkspaceOperationUiState::default(),
                planning_draft_editor_ui_state: super::PlanningDraftEditorUiState::default(),
                planning_worker_panel_state: super::CorePlanningWorkerPanelProjection::default(),
                planning_worker_visibility: super::PlanningWorkerVisibility::from_environment(),
            },
            runtime: super::NativeTuiRuntimeState {
                client_runtime,
                github_review_polling_state,
                tx: runtime_channels.tx,
                rx: runtime_channels.rx,
            },
        };
        app.refresh_ready_conversation_planning_runtime_projection_for_workspace(
            &workspace_directory,
        );
        app
    }

    fn take_shell_chrome_state(&mut self) -> ShellChromeState {
        std::mem::take(&mut self.shell.chrome)
    }

    fn apply_shell_chrome_state(&mut self, state: ShellChromeState) {
        self.shell.chrome = state;
    }

    pub(super) fn advance_planning_ui_intent_revision(&mut self) {
        self.planning.planning_ui_intent_revision = self
            .planning
            .planning_ui_intent_revision
            .wrapping_add(1)
            .max(1);
    }

    pub(super) fn dispatch_shell_chrome(&mut self, event: ShellChromeEvent) {
        let ShellChromeReduction {
            state,
            effects,
            overlay_transition,
        } = reduce_shell_chrome(self.take_shell_chrome_state(), event);
        self.apply_shell_chrome_state(state);
        if let Some(transition) = overlay_transition {
            self.apply_shell_overlay_transition(transition);
        }
        for effect in effects {
            self.execute_shell_chrome_effect(effect);
        }
    }

    fn apply_shell_overlay_transition(&mut self, transition: ShellOverlayTransition) {
        assert_eq!(
            self.shell.chrome.shell_overlay, transition.to,
            "shell overlay transition must be applied after reducer state"
        );
        self.advance_planning_ui_intent_revision();

        match transition.exit_mode {
            ShellOverlayExitMode::Suspend => {
                assert_eq!(
                    transition.to,
                    ShellOverlay::Approval,
                    "only entry into Approval may suspend a departed overlay"
                );
            }
            ShellOverlayExitMode::Exit => match transition.from {
                ShellOverlay::ModelSelection => {
                    self.shell.model_selection_overlay_ui_state =
                        super::ModelSelectionOverlayUiState::default();
                }
                ShellOverlay::ViewSelection => {
                    self.shell.view_selection_overlay_ui_state =
                        super::ViewSelectionOverlayUiState::default();
                }
                ShellOverlay::LanguageSelection => {
                    self.shell.language_selection_overlay_ui_state =
                        super::LanguageSelectionOverlayUiState::default();
                }
                ShellOverlay::ParallelPeek => self.shell.parallel_peek_overlay_ui_state.reset(),
                ShellOverlay::Activity => self.shell.progressive_activity_overlay_ui_state.reset(),
                ShellOverlay::Reviews => self.shell.reviews_overlay_ui_state.reset(),
                ShellOverlay::Queue => self.planning.queue_overlay_ui_state.reset(),
                ShellOverlay::DirectionsMaintenance => {
                    self.planning
                        .directions_maintenance_overlay_ui_state
                        .reset();
                    self.planning.planning_draft_editor_ui_state.reset();
                }
                ShellOverlay::PlanningInit => {
                    self.planning
                        .planning_runtime_refresh_ui_state
                        .clear_loading();
                    self.planning.planning_init_overlay_ui_state.reset();
                    self.planning.planning_draft_editor_ui_state.reset();
                    self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
                }
                ShellOverlay::Hidden
                | ShellOverlay::Startup
                | ShellOverlay::Sessions
                | ShellOverlay::Supersession
                | ShellOverlay::Help
                | ShellOverlay::Approval => {}
            },
        }
    }

    pub(super) fn poll_client_runtime_events(&mut self, max_inputs: usize) -> bool {
        let mut changed = false;
        for _ in 0..max_inputs {
            let Some(outcome) = self.runtime.client_runtime.poll_pending_client_event() else {
                break;
            };
            changed = true;
            self.apply_native_client_dispatch_outcome(outcome);
        }
        changed
    }

    pub(super) fn apply_native_client_dispatch_outcome(
        &mut self,
        outcome: NativeClientDispatchOutcome,
    ) -> Option<AppliedNativeParallelDispatch> {
        match outcome {
            NativeClientDispatchOutcome::Core(outcome) => {
                self.apply_core_dispatch_outcome(outcome);
                None
            }
            NativeClientDispatchOutcome::Parallel(outcome) => {
                let presentation_changed = self
                    .apply_parallel_mode_control_plane_presentation_events(
                        outcome.presentation_events,
                    );
                Some(AppliedNativeParallelDispatch {
                    presentation_changed,
                })
            }
            NativeClientDispatchOutcome::Combined { core, parallel } => {
                self.apply_core_dispatch_outcome(core);
                let presentation_changed = self
                    .apply_parallel_mode_control_plane_presentation_events(
                        parallel.presentation_events,
                    );
                Some(AppliedNativeParallelDispatch {
                    presentation_changed,
                })
            }
        }
    }

    pub(super) fn apply_core_dispatch_outcome(&mut self, outcome: CoreDispatchOutcome) {
        let previous_runtime = self.conversation_runtime_projection();
        self.apply_conversation_runtime_projection(outcome.snapshot.conversation_runtime.clone());
        for event in outcome.events {
            self.apply_core_event_with_previous_runtime(event, &previous_runtime);
        }
        // A Core event may synchronously dispatch a newer nested outcome. Read
        // the runtime facade again so the outer transition cannot restore its
        // older projection over that nested result.
        self.apply_conversation_runtime_projection(
            self.runtime.client_runtime.snapshot().conversation_runtime,
        );
    }

    #[cfg(test)]
    pub(super) fn apply_core_event(&mut self, event: AppEvent) {
        let previous_runtime = self.conversation_runtime_projection();
        self.apply_core_event_with_previous_runtime(event, &previous_runtime);
    }

    fn apply_core_event_with_previous_runtime(
        &mut self,
        event: AppEvent,
        previous_runtime: &crate::core::app::ConversationRuntimeSnapshot,
    ) {
        match event {
            AppEvent::StartupChanged { snapshot, .. } => self.apply_core_startup_snapshot(snapshot),
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Idle) => {
                self.shell.chrome.session_state = SessionState::Idle;
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Loading) => {
                self.shell.chrome.session_state = SessionState::Loading;
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Ready(ready)) => {
                self.dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(Ok(*ready.catalog)));
                self.shell.session_overlay_ui_state.reset();
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Failed { message }) => {
                self.dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(Err(message)));
                self.shell.session_overlay_ui_state.reset();
            }
            AppEvent::SessionRenameAdmissionResolved(_) => {}
            AppEvent::SessionRenameCompleted {
                correlation,
                result,
            } => self.apply_session_rename_completion(correlation, result),
            AppEvent::ConversationChanged { snapshot, .. } => {
                self.apply_core_conversation_snapshot(snapshot)
            }
            AppEvent::ParallelPeekConversationLoaded {
                correlation,
                result,
            } => {
                self.apply_parallel_peek_conversation_load(correlation, result);
            }
            AppEvent::GithubReviewPollingSetupStarted { correlation } => {
                self.record_github_review_polling_setup_started(correlation);
            }
            AppEvent::GithubReviewPollingSetupCompleted {
                correlation,
                result,
            } => {
                self.record_github_review_polling_setup_completion(
                    std::time::Instant::now(),
                    correlation,
                    result,
                );
            }
            AppEvent::GithubReviewPollStarted { correlation } => {
                self.record_github_review_poll_started(correlation);
            }
            AppEvent::GithubReviewPollCompleted {
                correlation,
                result,
            } => {
                self.record_github_review_poll_completion(
                    std::time::Instant::now(),
                    correlation,
                    result.map(|result| *result),
                );
            }
            AppEvent::ReviewCenterLoadStarted { .. } => {}
            AppEvent::ReviewCenterLoaded {
                correlation,
                snapshot,
            } => {
                if self.apply_reviews_overlay_loaded(correlation, snapshot)
                    == super::reviews_overlay_ui::ReviewsOverlayLoadCompletion::ReloadRequired
                {
                    self.start_reviews_overlay_authority_load();
                }
            }
            AppEvent::QueueAuthorityLoadStarted { .. } => {}
            AppEvent::QueueAuthorityLoaded {
                correlation,
                result,
            } => {
                if self.apply_queue_overlay_authority_loaded(correlation, result)
                    == super::queue_overlay_ui::QueueOverlayAuthorityLoadCompletion::ReloadRequired
                {
                    self.start_queue_overlay_authority_load();
                }
            }
            AppEvent::DirectionsMaintenanceLoadStarted { .. } => {}
            AppEvent::DirectionsMaintenanceLoaded {
                correlation,
                result,
            } => {
                if self.apply_directions_maintenance_loaded(correlation, result)
                    == super::directions_maintenance_ui::DirectionsMaintenanceLoadCompletion::ReloadRequired
                {
                    self.start_directions_maintenance_overview_load(None);
                }
            }
            AppEvent::PlanningRuntimeRefreshStarted { correlation } => {
                if let Some(pending) = self.planning.pending_resumed_session_planning_refresh.as_mut()
                    && pending.correlation.workspace_directory == correlation.workspace_directory
                {
                    pending.correlation = correlation.clone();
                }
                self.planning.planning_runtime_refresh_ui_state
                    .rebind(correlation);
            }
            AppEvent::PlanningRuntimeRefreshed {
                correlation,
                result,
            } => {
                let presentation_revision = self.planning.planning_ui_intent_revision;
                let pending_resume = if self
                    .planning.pending_resumed_session_planning_refresh
                    .as_ref()
                    .is_some_and(|pending| pending.correlation == correlation)
                {
                    self.planning.pending_resumed_session_planning_refresh.take()
                } else {
                    None
                };
                if let Some(pending) = pending_resume
                    && correlation.workspace_directory == self.planning_workspace_directory()
                {
                    match &result {
                        Ok(_) => {
                            self.surface_resumed_session_planning_context_if_unchanged(&pending)
                        }
                        Err(error) => self
                            .surface_resumed_session_planning_error_if_unchanged(&pending, error),
                    }
                }
                if correlation.workspace_directory == self.planning_workspace_directory() {
                    match self
                        .planning.planning_runtime_refresh_ui_state
                        .apply_completion(correlation.clone(), presentation_revision, result)
                    {
                        super::PlanningRuntimeRefreshUiCompletion::Applied {
                            operation,
                            result,
                        } => self.apply_planning_runtime_refresh_completion(operation, result),
                        super::PlanningRuntimeRefreshUiCompletion::Superseded { operation } => {
                            self.discard_superseded_planning_runtime_refresh(operation)
                        }
                        super::PlanningRuntimeRefreshUiCompletion::Rejected => {}
                    }
                } else if self
                    .planning.planning_runtime_refresh_ui_state
                    .cancel(&correlation)
                    .is_some()
                    && self.shell.chrome.shell_overlay == ShellOverlay::PlanningInit
                {
                    self.close_shell_overlay();
                    self.dispatch_conversation_input(
                        super::ConversationInputEvent::StatusMessageShown {
                            status_text:
                                "planning setup closed because its workspace context changed"
                                    .to_string(),
                        },
                    );
                }
            }
            AppEvent::PlanningRuntimeRefreshCancelled { correlation } => {
                if self
                    .planning.pending_resumed_session_planning_refresh
                    .as_ref()
                    .is_some_and(|pending| pending.correlation == correlation)
                {
                    self.planning.pending_resumed_session_planning_refresh = None;
                }
                if self
                    .planning.planning_runtime_refresh_ui_state
                    .cancel(&correlation)
                    .is_some()
                    && self.shell.chrome.shell_overlay == ShellOverlay::PlanningInit
                {
                    self.close_shell_overlay();
                    self.dispatch_conversation_input(
                        super::ConversationInputEvent::StatusMessageShown {
                            status_text:
                                "planning setup closed because its workspace context changed"
                                    .to_string(),
                        },
                    );
                }
            }
            AppEvent::PlanningWorkspaceOperationAdmissionResolved(admission) => {
                self.apply_planning_workspace_operation_admission(admission);
            }
            AppEvent::PlanningWorkspaceResetCompleted {
                correlation,
                result,
            } => {
                self.apply_planning_workspace_reset_completion(correlation, result);
            }
            AppEvent::PlanningSimpleDraftStaged {
                correlation,
                result,
            } => {
                self.apply_simple_planning_draft_stage_completion(correlation, result);
            }
            AppEvent::PlanningEditorStaged {
                correlation,
                result,
            } => {
                self.apply_planning_editor_stage_completion(correlation, result);
            }
            AppEvent::PlanningEditorMutationCompleted {
                correlation,
                result,
            } => {
                self.apply_planning_editor_mutation_completion(correlation, result);
            }
            AppEvent::PlanningSimpleEditorLoaded {
                correlation,
                result,
            } => {
                self.apply_simple_planning_editor_load_completion(correlation, result);
            }
            AppEvent::PlanningSimpleDraftPromoted {
                correlation,
                result,
            } => {
                self.apply_simple_planning_draft_promotion_completion(correlation, result);
            }
            AppEvent::QueueMutationStarted { correlation } => {
                self.apply_queue_mutation_started(correlation);
            }
            AppEvent::QueueMutationCompleted {
                correlation,
                result,
            } => {
                self.apply_queue_mutation_completion(correlation, *result);
            }
            AppEvent::StopRequestAdmissionResolved(admission) => {
                self.apply_stop_request_admission(admission);
            }
            AppEvent::StopRequestAttemptCompleted {
                correlation,
                attempt,
                result,
            } => {
                self.apply_stop_request_attempt_completion(correlation, attempt, result);
            }
            AppEvent::TurnSubmissionAdmissionResolved(
                crate::core::app::TurnSubmissionAdmission::RejectedStopPending { .. },
            ) => {
                self.dispatch_conversation_input(
                    super::ConversationInputEvent::StatusMessageShown {
                        status_text:
                            "turn start is waiting for the pending stop request to settle; retry shortly"
                                .to_string(),
                    },
                );
            }
            AppEvent::TurnSubmissionAdmissionResolved(_) => {}
            AppEvent::TurnSteerAdmissionResolved(_) => {}
            AppEvent::ApprovalDecisionAdmissionResolved(_) => {}
            AppEvent::ApprovalDecisionSubmissionCompleted {
                correlation: _,
                result,
            } => {
                if let Err(error) = result {
                    self.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::ApprovalDecisionSubmissionFailed {
                            error,
                        },
                    );
                }
            }
            AppEvent::ConversationRuntimeAuthorityChanged(snapshot) => {
                if previous_runtime.post_turn.is_in_flight()
                    && !snapshot.post_turn.is_in_flight()
                {
                    self.planning
                        .planning_worker_panel_state
                        .reset_for_conversation_lifecycle();
                }
                self.apply_conversation_runtime_projection(*snapshot);
            }
            AppEvent::ManualPromptPreparationAdmissionResolved(_) => {}
            AppEvent::TurnSteerCompleted {
                correlation,
                result,
            } => self.apply_turn_steer_completion(correlation, result),
            AppEvent::TurnStreamSnapshotChanged(stream_snapshot) => {
                self.dispatch_conversation_runtime_transition(
                    ConversationRuntimeEvent::StreamSnapshotApplied(stream_snapshot),
                    previous_runtime,
                );
            }
            AppEvent::ManualPromptPrepared(result) => {
                self.apply_manual_prompt_preparation(*result);
            }
            AppEvent::PostTurnEvaluationStarted(state) => {
                self.planning
                    .planning_worker_panel_state
                    .apply_started(state);
            }
            AppEvent::PostTurnContinuationRoutingRequested { .. } => {
                // NativeClientRuntime consumes this Core event and re-enters
                // Core with the exact route resolution before TUI projection.
            }
            AppEvent::PostTurnEvaluationCompleted {
                correlation,
                execution,
                route_resolution,
            } => {
                self.apply_post_turn_evaluation_execution(
                    correlation,
                    *execution,
                    route_resolution,
                );
            }
            AppEvent::ConversationTurnWorkspaceChanged {
                workspace_directory: _,
            } => {}
            AppEvent::ParallelModeSupervisorSnapshotInvalidated => {
                self.invalidate_parallel_mode_supervisor_snapshot();
            }
            AppEvent::SnapshotChanged(_) => {}
        }
    }

    fn apply_core_startup_snapshot(&mut self, snapshot: StartupSnapshot) {
        match snapshot {
            StartupSnapshot::Loading => {
                self.shell.chrome.startup_state = StartupState::Loading;
            }
            StartupSnapshot::Idle => {
                self.shell.chrome.startup_state = StartupState::Idle;
            }
            StartupSnapshot::Ready(ready) => {
                let workspace_directory = ready.workspace_path.clone();
                self.dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                    result: Ok(ready),
                    session_page_size: SESSION_PAGE_SIZE,
                });
                self.sync_draft_shell_workspace(&workspace_directory);
                self.resolve_startup_submit_queue();
            }
            StartupSnapshot::Failed { message } => {
                self.dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                    result: Err(message),
                    session_page_size: SESSION_PAGE_SIZE,
                });
                self.resolve_startup_submit_queue();
            }
        }
    }

    pub(super) fn dispatch_client_event(&mut self, input: CoreInput) {
        let outcome = self.reduce_core_client_event(input);
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn reduce_core_client_event(&mut self, input: CoreInput) -> CoreDispatchOutcome {
        match self
            .runtime
            .client_runtime
            .dispatch_client_event(NativeClientEvent::core(input))
        {
            NativeClientDispatchOutcome::Core(outcome) => outcome,
            NativeClientDispatchOutcome::Parallel(_) => {
                unreachable!("Core client event must return a Core outcome")
            }
            NativeClientDispatchOutcome::Combined { core, parallel } => {
                // Combined is normally produced by polled post-turn routing and
                // applied through `apply_native_client_dispatch_outcome`. Keep
                // this admission-oriented seam exhaustive for test/immediate
                // executors; the Core outcome remains available to the caller.
                self.apply_parallel_mode_control_plane_presentation_events(
                    parallel.presentation_events,
                );
                core
            }
        }
    }

    pub(super) fn dispatch_parallel_client_event(
        &mut self,
        event: NativeClientEvent,
    ) -> AppliedNativeParallelDispatch {
        assert!(
            !matches!(&event, NativeClientEvent::Core(_)),
            "parallel dispatch must not receive a Core event"
        );
        let outcome = self.runtime.client_runtime.dispatch_client_event(event);
        self.apply_native_client_dispatch_outcome(outcome)
            .expect("parallel client event must return a parallel outcome")
    }

    pub(super) fn apply_core_conversation_snapshot(&mut self, snapshot: CoreConversationSnapshot) {
        let loaded_successfully = matches!(&snapshot, CoreConversationSnapshot::Ready(_));
        let load_finished = matches!(
            &snapshot,
            CoreConversationSnapshot::Ready(_) | CoreConversationSnapshot::Failed { .. }
        );
        if matches!(
            &snapshot,
            CoreConversationSnapshot::Idle | CoreConversationSnapshot::Loading
        ) {
            self.planning
                .planning_worker_panel_state
                .reset_for_conversation_lifecycle();
        }
        if matches!(&snapshot, CoreConversationSnapshot::Loading) {
            self.planning.pending_resumed_session_planning_refresh = None;
        }
        let draft_workspace_directory = self.current_workspace_directory();
        self.dispatch_conversation_lifecycle(
            ConversationLifecycleEvent::CoreConversationSnapshotApplied {
                snapshot,
                draft_workspace_directory,
            },
        );
        if !load_finished {
            return;
        }
        if loaded_successfully {
            let workspace_directory = self.planning_workspace_directory();
            let resume_status_context = match &self.conversation.lifecycle.conversation_state {
                ConversationState::Ready(conversation) => Some((
                    conversation.thread_id.clone(),
                    conversation.status_text.clone(),
                )),
                ConversationState::Loading | ConversationState::Failed(_) => None,
            };
            if let Some((thread_id, status_text)) = resume_status_context
                && let Some((correlation, outcome)) =
                    self.begin_planning_runtime_projection_refresh(&workspace_directory)
            {
                // Bind resume copy before an immediate test executor can project completion.
                self.planning.pending_resumed_session_planning_refresh =
                    Some(PendingResumedSessionPlanningRefresh {
                        correlation,
                        thread_id,
                        status_text,
                    });
                self.apply_core_dispatch_outcome(outcome);
            } else {
                self.planning.pending_resumed_session_planning_refresh = None;
                self.surface_resumed_session_planning_context();
            }
        } else {
            self.planning.pending_resumed_session_planning_refresh = None;
        }
        // A loaded conversation resets follow-up copy because auto-turn affordances
        // belong to the active thread, not the previous shell contents.
        self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
    }

    fn execute_shell_chrome_effect(&mut self, effect: ShellChromeEffect) {
        match effect {
            ShellChromeEffect::RunStartupChecks => {
                self.dispatch_client_event(CoreInput::Command(AppCommand::RunStartupChecks {
                    workspace_directory: self.planning_workspace_directory(),
                }));
            }
            ShellChromeEffect::LoadSessionCatalog {
                mode,
                limit,
                current_workspace_directory,
            } => {
                // Session overlay requests are scoped to the visible conversation
                // workspace unless the reducer explicitly supplied another root.
                let workspace_directory = current_workspace_directory
                    .unwrap_or_else(|| self.current_workspace_directory());
                let intent = match mode {
                    SessionCatalogLoadMode::EnsureLoaded => {
                        SessionCatalogLoadIntent::ensure_loaded(limit, workspace_directory)
                    }
                    SessionCatalogLoadMode::Refresh => {
                        SessionCatalogLoadIntent::refresh(limit, workspace_directory)
                    }
                };
                self.dispatch_client_event(CoreInput::Command(AppCommand::LoadSessionCatalog(
                    intent,
                )));
            }
        }
    }

    // Moving the conversation out prevents accidental partial mutation when lifecycle
    // reducers decide between loading, failed, and ready session states.
    fn take_conversation_lifecycle_state(&mut self) -> ConversationLifecycleState {
        let replacement = ConversationLifecycleState {
            turn_control_truth: self.conversation.lifecycle.turn_control_truth,
            conversation_state: ConversationState::Loading,
        };
        std::mem::replace(&mut self.conversation.lifecycle, replacement)
    }

    fn apply_conversation_lifecycle_state(&mut self, state: ConversationLifecycleState) {
        self.conversation.lifecycle = state;
    }

    pub(super) fn dispatch_conversation_lifecycle(&mut self, event: ConversationLifecycleEvent) {
        self.capture_ready_conversation_history_thread();
        let opens_new_history = matches!(&event, ConversationLifecycleEvent::NewDraftOpened { .. });
        let loaded_history_thread_id = match &event {
            ConversationLifecycleEvent::CoreConversationSnapshotApplied {
                snapshot: CoreConversationSnapshot::Ready(ready),
                ..
            } => Some(ready.conversation.thread_id.clone()),
            ConversationLifecycleEvent::NewDraftOpened { .. }
            | ConversationLifecycleEvent::SessionChosen { .. }
            | ConversationLifecycleEvent::CoreConversationSnapshotApplied { .. } => None,
        };
        let target_workspace_directory = match &event {
            ConversationLifecycleEvent::NewDraftOpened {
                workspace_directory,
            } => Some(workspace_directory.as_str()),
            ConversationLifecycleEvent::SessionChosen {
                session,
                fallback_workspace_directory,
            } => Some(if session.cwd.trim().is_empty() {
                fallback_workspace_directory.as_str()
            } else {
                session.cwd.as_str()
            }),
            ConversationLifecycleEvent::CoreConversationSnapshotApplied {
                snapshot: CoreConversationSnapshot::Ready(ready),
                draft_workspace_directory,
            } => Some(if ready.workspace_directory.trim().is_empty() {
                draft_workspace_directory.as_str()
            } else {
                ready.workspace_directory.as_str()
            }),
            ConversationLifecycleEvent::CoreConversationSnapshotApplied { .. } => None,
        }
        .map(str::to_string);
        if let Some(target_workspace_directory) = target_workspace_directory.as_deref() {
            self.close_parallel_mode_epoch_before_workspace_transition(target_workspace_directory);
        }
        let changes_conversation_identity = matches!(
            &event,
            ConversationLifecycleEvent::NewDraftOpened { .. }
                | ConversationLifecycleEvent::SessionChosen { .. }
        );
        if changes_conversation_identity {
            self.cancel_manual_prompt_preparation_for_identity_transition();
            self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
        }
        if matches!(&event, ConversationLifecycleEvent::NewDraftOpened { .. }) {
            self.dispatch_client_event(CoreInput::Command(AppCommand::InvalidateConversationLoad));
        }
        let reduction =
            reduce_conversation_lifecycle(self.take_conversation_lifecycle_state(), event);
        self.apply_conversation_lifecycle_state(reduction.state);
        if opens_new_history {
            self.advance_conversation_history_identity_revision();
            self.conversation.conversation_history_thread_id = None;
        } else if let Some(thread_id) = loaded_history_thread_id {
            if self.conversation.conversation_history_thread_id.as_deref()
                != Some(thread_id.as_str())
            {
                self.advance_conversation_history_identity_revision();
            }
            self.conversation.conversation_history_thread_id = Some(thread_id);
        }
        self.advance_planning_ui_intent_revision();
        for effect in reduction.effects {
            self.execute_conversation_lifecycle_effect(effect);
        }
    }

    fn capture_ready_conversation_history_thread(&mut self) {
        let ConversationState::Ready(conversation) =
            &self.conversation.lifecycle.conversation_state
        else {
            return;
        };
        if self.conversation.conversation_history_thread_id.is_none()
            && conversation.has_active_thread()
        {
            self.conversation.conversation_history_thread_id = Some(conversation.thread_id.clone());
        }
    }

    fn reconcile_runtime_conversation_history_thread(&mut self) {
        let ConversationState::Ready(conversation) =
            &self.conversation.lifecycle.conversation_state
        else {
            return;
        };
        if !conversation.has_active_thread() {
            return;
        }
        let thread_id = conversation.thread_id.clone();
        if self
            .conversation
            .conversation_history_thread_id
            .as_deref()
            .is_some_and(|current| current != thread_id.as_str())
        {
            self.advance_conversation_history_identity_revision();
        }
        self.conversation.conversation_history_thread_id = Some(thread_id);
    }

    fn advance_conversation_history_identity_revision(&mut self) {
        self.conversation.conversation_history_identity_revision = self
            .conversation
            .conversation_history_identity_revision
            .wrapping_add(1)
            .max(1);
    }

    fn execute_conversation_lifecycle_effect(&mut self, effect: ConversationLifecycleEffect) {
        match effect {
            ConversationLifecycleEffect::LoadConversation {
                thread_id,
                fallback_workspace_directory,
            } => {
                self.dispatch_client_event(CoreInput::Command(AppCommand::LoadConversation {
                    thread_id,
                    fallback_workspace_directory,
                }));
            }
        }
    }

    pub(super) fn take_ready_conversation_state(&mut self) -> Option<ConversationViewModel> {
        let state = std::mem::replace(
            &mut self.conversation.lifecycle.conversation_state,
            ConversationState::Loading,
        );
        match state {
            ConversationState::Ready(conversation) => Some(*conversation),
            other => {
                self.conversation.lifecycle.conversation_state = other;
                None
            }
        }
    }

    pub(super) fn conversation_runtime_projection(
        &self,
    ) -> crate::core::app::ConversationRuntimeSnapshot {
        match &self.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => conversation.runtime_snapshot().clone(),
            ConversationState::Loading | ConversationState::Failed(_) => {
                self.runtime.client_runtime.snapshot().conversation_runtime
            }
        }
    }

    fn apply_conversation_runtime_projection(
        &mut self,
        snapshot: crate::core::app::ConversationRuntimeSnapshot,
    ) {
        if let ConversationState::Ready(conversation) =
            &mut self.conversation.lifecycle.conversation_state
        {
            conversation.apply_runtime_snapshot(snapshot);
        }
    }

    pub(super) fn dispatch_conversation_runtime(
        &mut self,
        event: ConversationRuntimeEvent,
    ) -> bool {
        let previous_runtime = self.conversation_runtime_projection();
        self.dispatch_conversation_runtime_transition(event, &previous_runtime)
    }

    fn dispatch_conversation_runtime_transition(
        &mut self,
        event: ConversationRuntimeEvent,
        previous_runtime: &crate::core::app::ConversationRuntimeSnapshot,
    ) -> bool {
        self.capture_ready_conversation_history_thread();
        let supersedes_planning_ui_intent = event.supersedes_planning_ui_intent();
        let Some(conversation) = self.take_ready_conversation_state() else {
            return false;
        };

        let reduction =
            reduce_conversation_runtime_with_transition(conversation, event, previous_runtime);
        let effects = reduction.effects;
        let requests_turn_submission = effects.iter().any(|effect| {
            matches!(
                effect,
                ConversationRuntimeEffect::RequestTurnSubmission { .. }
            )
        });
        self.conversation.lifecycle.conversation_state = ConversationState::ready(reduction.state);
        self.reconcile_runtime_conversation_history_thread();
        if supersedes_planning_ui_intent {
            self.advance_planning_ui_intent_revision();
        }
        if !requests_turn_submission && !self.conversation_has_running_turn() {
            self.conversation.turn_steer_confirmation = None;
        }
        let mut turn_submission_admitted = false;
        for effect in effects {
            turn_submission_admitted |= self.execute_conversation_runtime_effect(effect);
        }
        turn_submission_admitted
    }

    pub(super) fn dispatch_conversation_input(&mut self, event: impl Into<ConversationInputEvent>) {
        let event = event.into();
        let event = if self
            .conversation
            .pending_manual_prompt_preparation
            .is_some()
            && event.mutates_input_buffer()
        {
            ConversationInputEvent::StatusMessageShown {
                status_text:
                    "turn preparation in progress; prompt editing is locked until it finishes"
                        .to_string(),
            }
        } else {
            event
        };
        let mutates_input_buffer = event.mutates_input_buffer();
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };
        let mut conversation = conversation;
        match event {
            ConversationInputEvent::Composer(event) => {
                let composer = std::mem::take(&mut conversation.composer);
                let reduction = reduce_conversation_input(composer, event);
                conversation.composer = reduction.state;
                for effect in reduction.effects {
                    match effect {
                        ConversationComposerEffect::ReplaceStatus { status_text } => {
                            conversation.status_text = status_text;
                        }
                    }
                }
            }
            ConversationInputEvent::StatusMessageShown { status_text } => {
                conversation.record_status_message(status_text);
            }
            ConversationInputEvent::ManualPromptPreparationFailed {
                transcript_text,
                status_text,
            } => {
                conversation.record_manual_preparation_failure(transcript_text, status_text);
            }
        }
        self.conversation.lifecycle.conversation_state = ConversationState::ready(conversation);
        self.advance_planning_ui_intent_revision();
        if mutates_input_buffer {
            self.conversation.prompt_input_revision = self
                .conversation
                .prompt_input_revision
                .wrapping_add(1)
                .max(1);
        }
    }

    pub(super) fn clear_input_buffer(&mut self) {
        self.dispatch_conversation_input(ConversationComposerEvent::InputCleared);
    }

    fn conversation_intent_state(&self) -> ConversationIntentState {
        let mode = match &self.conversation.lifecycle.conversation_state {
            ConversationState::Loading => ConversationIntentMode::Loading,
            ConversationState::Failed(_) => ConversationIntentMode::Failed,
            ConversationState::Ready(conversation) if conversation.is_blank_draft() => {
                ConversationIntentMode::BlankDraft
            }
            ConversationState::Ready(_) => ConversationIntentMode::Ready,
        };

        ConversationIntentState {
            has_running_turn: self.conversation_has_running_turn(),
            blocks_navigation: matches!(
                &self.conversation.lifecycle.conversation_state,
                ConversationState::Ready(conversation) if !conversation.can_accept_manual_prompt()
            ),
            mode,
            interrupt_support: match &self.conversation.lifecycle.conversation_state {
                ConversationState::Ready(conversation) => {
                    conversation.turn_control_truth().interrupt
                }
                ConversationState::Loading | ConversationState::Failed(_) => {
                    self.conversation.lifecycle.turn_control_truth.interrupt
                }
            },
        }
    }

    pub(super) fn dispatch_conversation_intent(&mut self, event: ConversationIntentEvent) {
        let reduction = reduce_conversation_intents(self.conversation_intent_state(), event);
        for effect in reduction.effects {
            self.execute_conversation_intent_effect(effect);
        }
    }

    fn execute_conversation_intent_effect(&mut self, effect: ConversationIntentEffect) {
        match effect {
            ConversationIntentEffect::ShowStatus { status_text } => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            ConversationIntentEffect::OpenNewDraft => {
                // Core invalidation resets semantic worker projection state; this
                // intent only dismisses local chrome before opening the draft.
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                let workspace_directory = self.current_workspace_directory();
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
                    workspace_directory: workspace_directory.clone(),
                });
                self.refresh_ready_conversation_planning_runtime_projection();
            }
            ConversationIntentEffect::OpenSession { session } => {
                // Session selection requests Core lifecycle work without
                // optimistically rewriting the current semantic projection.
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
                    session,
                    fallback_workspace_directory: self.current_workspace_directory(),
                });
            }
            ConversationIntentEffect::ShowExitConfirmation => {
                self.dispatch_shell_chrome(ShellChromeEvent::ExitConfirmationShown);
            }
        }
    }

    pub(super) fn dispatch_auto_follow_controls(&mut self, event: AutoFollowControlEvent) {
        match event {
            AutoFollowControlEvent::DraftWorkspaceSynced {
                workspace_directory,
            } => {
                let changed = match &mut self.conversation.lifecycle.conversation_state {
                    ConversationState::Ready(conversation) => {
                        conversation.sync_draft_workspace(workspace_directory)
                    }
                    ConversationState::Loading | ConversationState::Failed(_) => false,
                };
                if changed {
                    self.cancel_manual_prompt_preparation_for_identity_transition();
                    self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
                    self.advance_planning_ui_intent_revision();
                }
            }
            AutoFollowControlEvent::AutoFollowPaused
            | AutoFollowControlEvent::PlanningAuthorityMutationSettled => {
                let show_operator_status =
                    matches!(event, AutoFollowControlEvent::AutoFollowPaused);
                self.dispatch_client_event(CoreInput::Command(
                    AppCommand::PausePostTurnContinuation,
                ));
                if let ConversationState::Ready(conversation) =
                    &mut self.conversation.lifecycle.conversation_state
                {
                    conversation.record_internal_continuation_paused();
                    if show_operator_status {
                        conversation.status_text =
                            "auto-follow stopped and disarmed / use :turns <positive|infinite> to re-enable"
                                .to_string();
                    }
                }
                self.advance_planning_ui_intent_revision();
            }
            AutoFollowControlEvent::MaxAutoTurnsUpdated { value } => {
                let Some(command) = max_auto_turns_command(&value) else {
                    if let ConversationState::Ready(conversation) =
                        &mut self.conversation.lifecycle.conversation_state
                    {
                        conversation.status_text =
                            "auto-follow unchanged / use a positive whole number, infinite, off, or 0"
                                .to_string();
                    }
                    return;
                };
                self.dispatch_client_event(CoreInput::Command(command));
                if let ConversationState::Ready(conversation) =
                    &mut self.conversation.lifecycle.conversation_state
                {
                    conversation.clear_auto_follow_skip();
                    let auto_follow = conversation.auto_follow_state();
                    conversation.status_text = if auto_follow.is_enabled() {
                        format!(
                            "auto-follow enabled / turn budget {}",
                            auto_follow.max_auto_turns_label()
                        )
                    } else {
                        "auto-follow disabled / use :turns <positive|infinite> to enable"
                            .to_string()
                    };
                }
                self.advance_planning_ui_intent_revision();
                self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
            }
        }
    }

    pub(super) fn dispatch_auto_follow_overlay_ui(&mut self, event: AutoFollowOverlayUiEvent) {
        let state = std::mem::take(&mut self.conversation.auto_follow_overlay_ui_state);
        self.conversation.auto_follow_overlay_ui_state =
            reduce_auto_follow_overlay_ui(state, event);
    }
}

pub(super) struct AppliedNativeParallelDispatch {
    pub(super) presentation_changed: bool,
}
