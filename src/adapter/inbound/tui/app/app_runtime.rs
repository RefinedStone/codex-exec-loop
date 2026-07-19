use std::sync::mpsc;

#[cfg(test)]
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneComposition,
    ParallelModeControlPlaneEventSink, ParallelModeControlPlaneHandle,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
#[cfg(test)]
use crate::application::service::planning::PlanningTaskToolUseCases;
use crate::application::service::planning::{
    PlanningRuntimeUseCases, PlanningServices, PlanningWorkspaceUseCases,
};
#[cfg(test)]
use crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution;
use crate::application::service::post_turn_evaluation::PostTurnEvaluationService;
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::core_effect_runner::CoreEffectRunner;
#[cfg(test)]
use crate::core::app::StartupReadySnapshot;
#[cfg(test)]
use crate::core::app::TurnStreamEvent;
use crate::core::app::{
    AppCommand, AppEvent, ConversationLoadCorrelation,
    ConversationSnapshot as CoreConversationSnapshot, CoreDispatchOutcome, CoreInput,
    SessionCatalogSnapshot, StartupCheckCorrelation, StartupSnapshot,
};
use crate::core::runtime::{CoreRuntime, core_input_channel};
#[cfg(test)]
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::operator_alert::OperatorAlert;

use super::{
    AutoFollowControlEvent, AutoFollowOverlayUiEvent, AutoFollowOverlayUiState,
    ConversationInputEvent, ConversationIntentEffect, ConversationIntentEvent,
    ConversationIntentMode, ConversationIntentState, ConversationLifecycleEffect,
    ConversationLifecycleEvent, ConversationLifecycleState, ConversationRuntimeEffect,
    ConversationRuntimeEvent, ConversationState, ConversationViewModel, ExitConfirmationState,
    GithubReviewPollingBootstrap, NativeTuiApp, PendingResumedSessionPlanningRefresh,
    PlanningInitOverlayUiState, SESSION_PAGE_SIZE, SessionOverlayUiState, SessionState,
    ShellChromeEffect, ShellChromeEvent, ShellChromeState, ShellOverlay, StartupState,
    reduce_auto_follow_controls, reduce_auto_follow_overlay_ui, reduce_conversation_input,
    reduce_conversation_intents, reduce_conversation_lifecycle, reduce_conversation_runtime,
    reduce_shell_chrome, startup_ascii_art_enabled_from_environment,
};

// Background control-plane and poll results are lower volume than token events,
// but still cross thread boundaries. A fixed queue keeps a stalled terminal from
// retaining unbounded notices while leaving room for one full render batch and
// a short producer burst. Capacity must stay above the shell's 128-message drain
// budget so the event loop can observe backlog and yield without a same-thread
// producer deadlock.
pub(super) const TUI_BACKGROUND_CHANNEL_CAPACITY: usize = 256;

/* NativeTuiApp is assembled as reducer-owned state plus outbound service handles.
 * Runtime files keep pure reducers away from threads and ports: reducers return
 * effects, this module turns those effects into background messages, and
 * ShellRuntime later drains those messages back into reducers.
 */
#[derive(Debug, Clone)]
#[allow(dead_code)]
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
    ConversationRuntimeNotice(String),
    OperatorAlert(OperatorAlert),
    InvalidateParallelModeSupervisorSnapshot,
    ParallelModeControlPlaneEvent(Box<ParallelModeControlPlaneBackgroundEvent>),
    #[cfg(test)]
    PostTurnEvaluationCompleted(Box<PostTurnEvaluationExecution>),
}

#[derive(Clone)]
pub(super) struct TuiParallelModeControlPlaneEventSink {
    tx: mpsc::SyncSender<BackgroundMessage>,
}

impl ParallelModeControlPlaneEventSink for TuiParallelModeControlPlaneEventSink {
    fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent) {
        let _ = self
            .tx
            .send(BackgroundMessage::ParallelModeControlPlaneEvent(Box::new(
                event,
            )));
    }
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

    pub(super) fn parallel_mode_event_sink(&self) -> TuiParallelModeControlPlaneEventSink {
        TuiParallelModeControlPlaneEventSink {
            tx: self.tx.clone(),
        }
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
            approval_id,
            resolution,
        } => TurnStreamEvent::ApprovalResolved {
            approval_id,
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
        ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationToolActivity,
        ConversationToolActivityKind,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
        ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
    };
    use crate::domain::parallel_mode::{
        ParallelModeControlPlaneWorkerEvent, ParallelModeControlPlaneWorkerEventKind,
    };
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
    fn parallel_mode_event_sink_routes_background_events_to_runtime_channel() {
        let channels = NativeTuiAppRuntimeChannels::new();
        let sink = channels.parallel_mode_event_sink();
        let effect_id =
            crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneEffectId {
                sequence: 7,
                kind: crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneEffectKind::RunOrchestrator,
            };

        sink.send_control_plane_event(
            ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice {
                workspace_directory: "/repo".to_string(),
                epoch_id: 3,
                effect_id,
                notice: "parallel notice".to_string(),
            },
        );

        match channels.rx.try_recv().expect("event should be queued") {
            BackgroundMessage::ParallelModeControlPlaneEvent(event) => {
                let ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice {
                    workspace_directory,
                    epoch_id,
                    effect_id: received_effect_id,
                    notice,
                } = *event
                else {
                    panic!("unexpected control-plane event");
                };
                assert_eq!(workspace_directory, "/repo");
                assert_eq!(epoch_id, 3);
                assert_eq!(received_effect_id, effect_id);
                assert_eq!(notice, "parallel notice");
            }
            other => panic!("unexpected background message: {other:?}"),
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
        let correlation = app.core_runtime.begin_test_turn_submission();
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
            .core_runtime
            .dispatch_input(CoreInput::ConversationStreamUpdated { correlation, event });
        app.apply_core_dispatch_outcome(outcome);
    }

    #[test]
    fn tui_parallel_binding_shares_one_automation_guard_with_post_turn_delivery() {
        let planning = test_helpers::test_planning_services(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
        let binding = NativeTuiParallelModeBinding::from_composition(
            test_helpers::test_parallel_mode_control_plane_composition(planning),
        );
        let workspace = "/tmp/shared-automation-guard".to_string();

        let _ = binding.parallel_mode_control_plane.handle_command(
            crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneCommand::OpenEpoch {
                workspace_directory: workspace.clone(),
            },
        );
        let epoch_id = binding
            .parallel_mode_control_plane
            .current_epoch_id_for_workspace(&workspace)
            .expect("open epoch should expose its id");
        assert!(
            binding
                .parallel_turns
                .automation_epoch_is_active(&workspace, epoch_id)
        );

        let _ = binding.parallel_mode_control_plane.handle_command(
            crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneCommand::Disable {
                workspace_directory: workspace.clone(),
            },
        );
        assert!(
            !binding
                .parallel_turns
                .automation_epoch_is_active(&workspace, epoch_id)
        );
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
            app.poll_core_runtime_inputs(16);
            std::thread::yield_now();
        }
        let duplicate_deadline = Instant::now() + Duration::from_secs(2);
        while review_repository
            .pending_inbox_replace_count
            .load(Ordering::SeqCst)
            < 2
            && Instant::now() < duplicate_deadline
        {
            app.poll_core_runtime_inputs(16);
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
        while app.poll_core_runtime_inputs(16) {}

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
                &app.conversation_state,
                ConversationState::Ready(conversation)
                    if conversation.approval_review.as_ref().is_some_and(|review| {
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
            app.poll_core_runtime_inputs(16);
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
    fn stop_supersedes_settlement_but_turn_budget_edits_do_not() {
        let mut app = test_helpers::test_native_tui_app();

        let paused_permit = app.post_turn_continuation_gate.capture();
        app.dispatch_auto_follow_controls(AutoFollowControlEvent::AutoFollowPaused);
        assert!(!paused_permit.is_current());

        let rearmed_permit = app.post_turn_continuation_gate.capture();
        app.dispatch_auto_follow_controls(AutoFollowControlEvent::MaxAutoTurnsUpdated {
            value: "3".to_string(),
        });
        assert!(rearmed_permit.is_current());

        let invalid_edit_permit = app.post_turn_continuation_gate.capture();
        app.dispatch_auto_follow_controls(AutoFollowControlEvent::MaxAutoTurnsUpdated {
            value: "invalid".to_string(),
        });
        assert!(invalid_edit_permit.is_current());

        let disabled_permit = app.post_turn_continuation_gate.capture();
        app.dispatch_auto_follow_controls(AutoFollowControlEvent::MaxAutoTurnsUpdated {
            value: "off".to_string(),
        });
        assert!(disabled_permit.is_current());
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
    fn workspace_and_conversation_supersession_invalidate_continuation_permits() {
        let mut app = test_helpers::test_native_tui_app();

        let unchanged_workspace_permit = app.post_turn_continuation_gate.capture();
        app.dispatch_auto_follow_controls(AutoFollowControlEvent::DraftWorkspaceSynced {
            workspace_directory: "/tmp/root".to_string(),
        });
        assert!(unchanged_workspace_permit.is_current());

        app.dispatch_auto_follow_controls(AutoFollowControlEvent::DraftWorkspaceSynced {
            workspace_directory: "/tmp/other".to_string(),
        });
        assert!(!unchanged_workspace_permit.is_current());

        let new_draft_permit = app.post_turn_continuation_gate.capture();
        arm_pending_manual_prompt_for_identity_test(&mut app, "new draft prompt");
        app.pending_conversation_load = Some(ConversationLoadCorrelation::new(9, "thread-stale"));
        app.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditStarted {
            current_value: "off".to_string(),
        });
        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory: "/tmp/root".to_string(),
        });
        assert!(!new_draft_permit.is_current());
        assert!(app.pending_manual_prompt_preparation.is_none());
        assert!(app.pending_conversation_load.is_none());
        assert!(matches!(
            &app.conversation_state,
            ConversationState::Ready(conversation) if conversation.cwd == "/tmp/root"
        ));
        assert_eq!(app.max_auto_turns_edit_buffer(), None);

        let session_permit = app.post_turn_continuation_gate.capture();
        arm_pending_manual_prompt_for_identity_test(&mut app, "session prompt");
        app.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditStarted {
            current_value: "off".to_string(),
        });
        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::SessionChosen {
            session: SessionSummary {
                id: "thread-2".to_string(),
                name: Some("Thread 2".to_string()),
                preview: "preview".to_string(),
                cwd: "/tmp/root".to_string(),
                source: "test".to_string(),
                model_provider: "test".to_string(),
                updated_at_epoch: 1,
                status_type: "idle".to_string(),
                path: "/tmp/root/thread-2".to_string(),
                git_branch: None,
            },
            fallback_workspace_directory: "/tmp/root".to_string(),
        });
        assert!(!session_permit.is_current());
        assert!(app.pending_manual_prompt_preparation.is_none());
        assert!(matches!(app.conversation_state, ConversationState::Loading));
        assert_eq!(app.max_auto_turns_edit_buffer(), None);
        assert_eq!(
            app.pending_conversation_load
                .as_ref()
                .map(|pending| pending.requested_thread_id.as_str()),
            Some("thread-2")
        );
    }

    #[test]
    fn conversation_lifecycle_closes_only_epochs_owned_by_the_workspace_being_left() {
        let mut app = test_helpers::test_native_tui_app();
        app.parallel_mode_control_plane
            .force_epoch_for_test("/tmp/worker-b", 1);
        assert!(
            app.parallel_mode_control_plane
                .automation_epoch_is_active("/tmp/worker-b", 1)
        );

        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory: "/tmp/root".to_string(),
        });

        assert_eq!(
            app.parallel_mode_control_plane.epoch_snapshot(),
            crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneEpochSnapshot {
                workspace_directory: None,
                current_epoch_id: None,
            }
        );
        assert!(
            !app.parallel_mode_control_plane
                .automation_epoch_is_active("/tmp/worker-b", 1)
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

        app.parallel_mode_control_plane
            .force_epoch_for_test("/tmp/root", 2);
        app.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory: "/tmp/root".to_string(),
        });
        assert_eq!(
            app.parallel_mode_control_plane
                .current_epoch_id_for_workspace("/tmp/root"),
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
            app.parallel_mode_control_plane
                .epoch_snapshot()
                .current_epoch_id
                .is_none()
        );
    }

    #[test]
    fn tui_startup_projection_rejects_stale_success_and_failure() {
        let mut app = test_helpers::test_native_tui_app();
        let latest = StartupCheckCorrelation::new(2, "/tmp/latest");
        app.pending_startup_check = Some(latest.clone());
        app.startup_state = StartupState::Loading;
        app.session_state = SessionState::Idle;

        app.apply_correlated_startup_snapshot(
            StartupCheckCorrelation::new(1, "/tmp/stale"),
            StartupSnapshot::Ready(test_startup_ready_snapshot("/tmp/stale")),
        );
        assert!(matches!(app.startup_state, StartupState::Loading));
        assert!(matches!(app.session_state, SessionState::Idle));

        app.apply_correlated_startup_snapshot(
            latest,
            StartupSnapshot::Ready(test_startup_ready_snapshot("/tmp/latest")),
        );
        assert!(matches!(
            &app.startup_state,
            StartupState::Ready(ready) if ready.workspace_path == "/tmp/latest"
        ));
        let latest_state = app.startup_state.clone();

        app.apply_correlated_startup_snapshot(
            StartupCheckCorrelation::new(1, "/tmp/stale"),
            StartupSnapshot::Failed {
                message: "stale failure".to_string(),
            },
        );
        assert!(matches!(
            (&app.startup_state, &latest_state),
            (StartupState::Ready(current), StartupState::Ready(expected))
                if current.workspace_path == expected.workspace_path
        ));
    }

    #[test]
    fn tui_conversation_projection_rejects_stale_result() {
        let mut app = test_helpers::test_native_tui_app();
        let latest = ConversationLoadCorrelation::new(2, "thread-b");
        app.pending_conversation_load = Some(latest.clone());
        app.conversation_state = ConversationState::Loading;

        app.apply_correlated_conversation_snapshot(
            Some(ConversationLoadCorrelation::new(1, "thread-a")),
            test_core_conversation_snapshot("thread-a"),
        );
        assert!(matches!(app.conversation_state, ConversationState::Loading));

        app.apply_correlated_conversation_snapshot(
            Some(latest),
            test_core_conversation_snapshot("thread-b"),
        );
        assert!(matches!(
            &app.conversation_state,
            ConversationState::Ready(conversation) if conversation.thread_id == "thread-b"
        ));

        app.apply_correlated_conversation_snapshot(
            Some(ConversationLoadCorrelation::new(1, "thread-a")),
            CoreConversationSnapshot::Failed {
                message: "stale A failure".to_string(),
            },
        );
        assert!(matches!(
            &app.conversation_state,
            ConversationState::Ready(conversation) if conversation.thread_id == "thread-b"
        ));
    }

    #[test]
    fn parallel_peek_projection_requires_the_visible_matching_preview() {
        let mut app = test_helpers::test_native_tui_app();
        app.parallel_peek_overlay_ui_state
            .open_preview(test_parallel_peek_preview("thread-current"));

        app.apply_parallel_peek_conversation_load(
            crate::core::app::ParallelPeekLoadCorrelation::new(1, "thread-current"),
            Err("hidden failure".to_string()),
        );
        assert_eq!(
            app.parallel_peek_overlay_ui_state
                .preview()
                .map(|preview| preview.status_text.as_str()),
            Some("conversation snapshot loading")
        );

        app.shell_overlay = ShellOverlay::ParallelPeek;
        app.apply_parallel_peek_conversation_load(
            crate::core::app::ParallelPeekLoadCorrelation::new(2, "thread-stale"),
            Err("stale failure".to_string()),
        );
        assert_eq!(
            app.parallel_peek_overlay_ui_state
                .preview()
                .map(|preview| preview.status_text.as_str()),
            Some("conversation snapshot loading")
        );

        app.apply_parallel_peek_conversation_load(
            crate::core::app::ParallelPeekLoadCorrelation::new(3, "thread-current"),
            Err("current failure".to_string()),
        );
        assert_eq!(
            app.parallel_peek_overlay_ui_state
                .preview()
                .map(|preview| preview.status_text.as_str()),
            Some("conversation snapshot failed: current failure")
        );
    }

    #[test]
    fn shell_chrome_supersession_resets_parallel_peek_projection() {
        let mut app = test_helpers::test_native_tui_app();
        app.shell_overlay = ShellOverlay::ParallelPeek;
        app.parallel_peek_overlay_ui_state
            .open_preview(test_parallel_peek_preview("thread-current"));
        app.parallel_peek_overlay_ui_state
            .scroll_conversation_older(10);

        app.dispatch_shell_chrome(ShellChromeEvent::QueueOverlayShown);

        assert_eq!(app.shell_overlay, ShellOverlay::Queue);
        assert!(app.parallel_peek_overlay_ui_state.preview().is_none());
        assert_eq!(
            app.parallel_peek_overlay_ui_state
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

    fn arm_pending_manual_prompt_for_identity_test(app: &mut NativeTuiApp, transcript_text: &str) {
        let workspace_directory = app.planning_workspace_directory();
        app.pending_manual_prompt_preparation = Some(
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
}

#[derive(Clone)]
pub(super) struct NativeTuiApplicationHandle {
    planning_feature: NativeTuiPlanningHandle,
}

impl NativeTuiApplicationHandle {
    fn new(planning_feature: PlanningServices) -> Self {
        Self {
            planning_feature: NativeTuiPlanningHandle::new(planning_feature),
        }
    }

    pub(super) fn planning(&self) -> &NativeTuiPlanningHandle {
        &self.planning_feature
    }
}
#[derive(Clone)]
pub(super) struct NativeTuiPlanningHandle {
    services: PlanningServices,
}

impl NativeTuiPlanningHandle {
    fn new(services: PlanningServices) -> Self {
        Self { services }
    }

    pub(super) fn workspace(&self) -> &PlanningWorkspaceUseCases {
        &self.services.workspace
    }

    pub(super) fn runtime(&self) -> &PlanningRuntimeUseCases {
        &self.services.runtime
    }

    #[cfg(test)]
    pub(super) fn queue(&self) -> &crate::application::service::planning::PlanningQueueUseCases {
        &self.services.queue
    }

    #[cfg(test)]
    pub(super) fn task_tool(&self) -> &PlanningTaskToolUseCases {
        &self.services.task_tool
    }
}

pub(crate) struct NativeTuiParallelModeBinding {
    parallel_turns: ParallelModeTurnService,
    planning_feature: PlanningServices,
    parallel_mode_control_plane:
        ParallelModeControlPlaneHandle<TuiParallelModeControlPlaneEventSink>,
    runtime_channels: NativeTuiAppRuntimeChannels,
}

impl NativeTuiParallelModeBinding {
    pub(crate) fn from_composition(
        composition: ParallelModeControlPlaneComposition,
    ) -> NativeTuiParallelModeBinding {
        let runtime_channels = NativeTuiAppRuntimeChannels::new();
        let parallel_mode_control_plane =
            composition.bind_event_sink(runtime_channels.parallel_mode_event_sink());
        NativeTuiParallelModeBinding {
            parallel_turns: composition.parallel_mode_turn_service(),
            planning_feature: composition.planning().clone(),
            parallel_mode_control_plane,
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
        Self::new_with_github_review_polling(
            startup_service,
            session_service,
            conversation_service,
            parallel_mode_binding,
            GithubReviewPollingBootstrap::disabled(),
        )
    }

    pub(super) fn new_with_github_review_polling(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_mode_binding: NativeTuiParallelModeBinding,
        github_review_polling: GithubReviewPollingBootstrap,
    ) -> Self {
        Self::new_with_github_review_polling_and_effect_runner(
            startup_service,
            session_service,
            conversation_service,
            parallel_mode_binding,
            github_review_polling,
            |runner| runner,
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
        Self::new_with_github_review_polling_and_effect_runner(
            startup_service,
            session_service,
            conversation_service,
            parallel_mode_binding,
            github_review_polling,
            |runner| runner.with_github_review_polling_setup_loader(loader),
        )
    }

    fn new_with_github_review_polling_and_effect_runner(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        parallel_mode_binding: NativeTuiParallelModeBinding,
        github_review_polling: GithubReviewPollingBootstrap,
        configure_effect_runner: impl FnOnce(CoreEffectRunner) -> CoreEffectRunner,
    ) -> Self {
        let GithubReviewPollingBootstrap {
            state: github_review_polling_state,
        } = github_review_polling;
        let NativeTuiParallelModeBinding {
            parallel_turns,
            planning_feature,
            parallel_mode_control_plane,
            runtime_channels,
        } = parallel_mode_binding;
        let (core_input_sender, core_input_receiver) = core_input_channel();
        let core_effect_runner = configure_effect_runner(CoreEffectRunner::new(
            startup_service.clone(),
            session_service.clone(),
            conversation_service.clone(),
            planning_feature.clone(),
            parallel_turns.clone(),
            PostTurnEvaluationService::new(planning_feature.clone(), parallel_turns.clone()),
            core_input_sender,
        ));
        let core_runtime = CoreRuntime::new(core_effect_runner, core_input_receiver);
        let turn_control_truth = conversation_service.runtime_control_truth();
        let application = NativeTuiApplicationHandle::new(planning_feature);

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
            shell_overlay: ShellOverlay::Hidden,
            approval_return_overlay: None,
            exit_confirmation_state: ExitConfirmationState::Hidden,
            startup_state: StartupState::Idle,
            pending_startup_check: None,
            session_state: SessionState::Idle,
            supersession_mud_ui_state: super::SupersessionMudUiState::default(),
            parallel_peek_overlay_ui_state: super::ParallelPeekOverlayUiState::default(),
            progressive_activity_overlay_ui_state:
                super::ProgressiveActivityOverlayUiState::default(),
            help_scroll_offset: 0,
            queue_overlay_ui_state: super::queue_overlay_ui::QueueOverlayUiState::default(),
            queue_mutation_ui_state: super::queue_overlay_ui::QueueMutationUiState::default(),
            reviews_overlay_ui_state: super::reviews_overlay_ui::ReviewsOverlayUiState::default(),
            parallel_supervisor_event_log: super::ParallelSupervisorEventLog::default(),
            pending_manual_prompt_preparation: None,
            prompt_input_revision: 0,
            planning_ui_intent_revision: 0,
            turn_steer_confirmation: None,
            pending_turn_steer: None,
            parallel_mode_control_plane,
            global_runtime_notice_state: super::GlobalRuntimeNoticeState::default(),
            conversation_state: ConversationState::ready(initial_conversation),
            pending_conversation_load: None,
            pending_resumed_session_planning_refresh: None,
            selected_session_index: 0,
            session_overlay_ui_state: SessionOverlayUiState::new(SESSION_PAGE_SIZE),
            tui_language: super::TuiLanguage::default(),
            language_selection_overlay_ui_state: super::LanguageSelectionOverlayUiState::default(),
            model_selection_overlay_ui_state: super::ModelSelectionOverlayUiState::default(),
            view_selection_overlay_ui_state: super::ViewSelectionOverlayUiState::default(),
            auto_follow_overlay_ui_state: AutoFollowOverlayUiState::default(),
            directions_maintenance_overlay_ui_state:
                super::DirectionsMaintenanceOverlayUiState::default(),
            planning_init_overlay_ui_state: PlanningInitOverlayUiState::default(),
            planning_runtime_refresh_ui_state: super::PlanningRuntimeRefreshUiState::default(),
            planning_workspace_operation_ui_state:
                super::PlanningWorkspaceOperationUiState::default(),
            planning_draft_editor_ui_state: super::PlanningDraftEditorUiState::default(),
            application,
            core_runtime,
            turn_control_truth,
            turn_options: Default::default(),
            conversation_view_mode: super::ConversationViewMode::default(),
            planning_worker_panel_state: super::PlanningWorkerPanelState::default(),
            post_turn_continuation_gate: crate::domain::planning::PostTurnContinuationGate::default(
            ),
            planning_worker_visibility: super::PlanningWorkerVisibility::from_environment(),
            github_review_polling_state,
            inline_history_render_mode: super::InlineHistoryRenderMode::from_environment(),
            history_insert_mode: super::HistoryInsertionMode::from_environment(),
            show_startup_ascii_art: startup_ascii_art_enabled_from_environment(),
            tx: runtime_channels.tx,
            rx: runtime_channels.rx,
        };
        app.refresh_ready_conversation_planning_runtime_projection_for_workspace(
            &workspace_directory,
        );
        app
    }

    // Shell chrome state is split across NativeTuiApp fields for ergonomic access by
    // renderers, then reassembled here so the reducer still owns one coherent value.
    fn take_shell_chrome_state(&mut self) -> ShellChromeState {
        ShellChromeState {
            shell_overlay: self.shell_overlay,
            approval_return_overlay: self.approval_return_overlay,
            exit_confirmation_state: self.exit_confirmation_state,
            startup_state: std::mem::replace(&mut self.startup_state, StartupState::Idle),
            session_state: std::mem::replace(&mut self.session_state, SessionState::Idle),
            selected_session_index: self.selected_session_index,
        }
    }

    fn apply_shell_chrome_state(&mut self, state: ShellChromeState) {
        self.shell_overlay = state.shell_overlay;
        self.approval_return_overlay = state.approval_return_overlay;
        self.exit_confirmation_state = state.exit_confirmation_state;
        self.startup_state = state.startup_state;
        self.session_state = state.session_state;
        self.selected_session_index = state.selected_session_index;
    }

    pub(super) fn advance_planning_ui_intent_revision(&mut self) {
        self.planning_ui_intent_revision = self.planning_ui_intent_revision.wrapping_add(1).max(1);
    }

    pub(super) fn dispatch_shell_chrome(&mut self, event: ShellChromeEvent) {
        let previous_overlay = self.shell_overlay;
        let directions_suspended_for_approval = previous_overlay
            == ShellOverlay::DirectionsMaintenance
            && matches!(&event, ShellChromeEvent::ApprovalOverlayShown);
        let reduction = reduce_shell_chrome(self.take_shell_chrome_state(), event);
        self.apply_shell_chrome_state(reduction.state);
        if previous_overlay != self.shell_overlay {
            self.advance_planning_ui_intent_revision();
        }
        if previous_overlay == ShellOverlay::Reviews && self.shell_overlay != ShellOverlay::Reviews
        {
            self.reviews_overlay_ui_state.reset();
        }
        if previous_overlay == ShellOverlay::Queue && self.shell_overlay != ShellOverlay::Queue {
            self.queue_overlay_ui_state.reset();
        }
        if !directions_suspended_for_approval
            && previous_overlay == ShellOverlay::DirectionsMaintenance
            && self.shell_overlay != ShellOverlay::DirectionsMaintenance
        {
            self.directions_maintenance_overlay_ui_state.reset();
            self.planning_draft_editor_ui_state.reset();
        }
        if previous_overlay == ShellOverlay::ParallelPeek
            && self.shell_overlay != ShellOverlay::ParallelPeek
        {
            self.parallel_peek_overlay_ui_state.reset();
        }
        if previous_overlay == ShellOverlay::PlanningInit
            && self.shell_overlay != ShellOverlay::PlanningInit
        {
            self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
        }
        for effect in reduction.effects {
            self.execute_shell_chrome_effect(effect);
        }
    }

    pub(super) fn poll_core_runtime_inputs(&mut self, max_inputs: usize) -> bool {
        let mut changed = false;
        for _ in 0..max_inputs {
            let Some(outcome) = self.core_runtime.poll_pending_input() else {
                break;
            };
            changed = true;
            self.apply_core_dispatch_outcome(outcome);
        }
        changed
    }

    pub(super) fn apply_core_dispatch_outcome(&mut self, outcome: CoreDispatchOutcome) {
        for event in outcome.events {
            self.apply_core_event(event);
        }
    }

    pub(super) fn apply_core_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::StartupChanged {
                correlation,
                snapshot,
            } => self.apply_correlated_startup_snapshot(correlation, snapshot),
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Idle) => {
                self.session_state = SessionState::Idle;
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Loading) => {
                self.session_state = SessionState::Loading;
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Ready(ready)) => {
                self.dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(Ok(*ready.catalog)));
                self.session_overlay_ui_state.reset();
            }
            AppEvent::SessionCatalogChanged(SessionCatalogSnapshot::Failed { message }) => {
                self.dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(Err(message)));
                self.session_overlay_ui_state.reset();
            }
            AppEvent::SessionRenameCompleted {
                correlation,
                result,
            } => self.apply_session_rename_completion(correlation, result),
            AppEvent::ConversationChanged {
                correlation,
                snapshot,
            } => self.apply_correlated_conversation_snapshot(correlation, snapshot),
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
                if let Some(pending) = self.pending_resumed_session_planning_refresh.as_mut()
                    && pending.correlation.workspace_directory == correlation.workspace_directory
                {
                    pending.correlation = correlation.clone();
                }
                self.planning_runtime_refresh_ui_state
                    .rebind(correlation);
            }
            AppEvent::PlanningRuntimeRefreshed {
                correlation,
                result,
            } => {
                let presentation_revision = self.planning_ui_intent_revision;
                let pending_resume = if self
                    .pending_resumed_session_planning_refresh
                    .as_ref()
                    .is_some_and(|pending| pending.correlation == correlation)
                {
                    self.pending_resumed_session_planning_refresh.take()
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
                        .planning_runtime_refresh_ui_state
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
                    .planning_runtime_refresh_ui_state
                    .cancel(&correlation)
                    .is_some()
                    && self.shell_overlay == ShellOverlay::PlanningInit
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
                    .pending_resumed_session_planning_refresh
                    .as_ref()
                    .is_some_and(|pending| pending.correlation == correlation)
                {
                    self.pending_resumed_session_planning_refresh = None;
                }
                if self
                    .planning_runtime_refresh_ui_state
                    .cancel(&correlation)
                    .is_some()
                    && self.shell_overlay == ShellOverlay::PlanningInit
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
                correlation,
                result,
            } => {
                if let Err(error) = result {
                    self.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::ApprovalDecisionSubmissionFailed {
                            approval_id: correlation.approval_id,
                            error,
                        },
                    );
                }
            }
            AppEvent::ManualPromptPreparationAdmissionResolved(_) => {}
            AppEvent::TurnSteerCompleted {
                correlation,
                result,
            } => self.apply_turn_steer_completion(correlation, result),
            AppEvent::TurnStreamSnapshotChanged(stream_snapshot) => {
                self.dispatch_conversation_runtime(
                    ConversationRuntimeEvent::StreamSnapshotApplied(stream_snapshot),
                );
            }
            AppEvent::ManualPromptPrepared(result) => {
                self.apply_manual_prompt_preparation(*result);
            }
            AppEvent::PostTurnEvaluationCompleted(execution) => {
                self.apply_post_turn_evaluation_execution(*execution);
            }
            AppEvent::ConversationTurnWorkspaceChanged {
                workspace_directory,
            } => {
                self.sync_active_turn_workspace_directory(&workspace_directory);
            }
            AppEvent::ParallelModeSupervisorSnapshotInvalidated => {
                self.invalidate_parallel_mode_supervisor_snapshot();
            }
            AppEvent::SnapshotChanged(_) => {}
        }
    }

    fn apply_correlated_startup_snapshot(
        &mut self,
        correlation: StartupCheckCorrelation,
        snapshot: StartupSnapshot,
    ) {
        match snapshot {
            StartupSnapshot::Loading => {
                if self
                    .pending_startup_check
                    .as_ref()
                    .is_some_and(|pending| pending.generation > correlation.generation)
                {
                    return;
                }
                self.pending_startup_check = Some(correlation);
                self.startup_state = StartupState::Loading;
            }
            StartupSnapshot::Idle => {
                if self.pending_startup_check.as_ref() == Some(&correlation) {
                    self.pending_startup_check = None;
                    self.startup_state = StartupState::Idle;
                }
            }
            StartupSnapshot::Ready(ready) => {
                if self.pending_startup_check.as_ref() != Some(&correlation) {
                    return;
                }
                self.pending_startup_check = None;
                let workspace_directory = ready.workspace_path.clone();
                self.dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                    result: Ok(ready),
                    session_page_size: SESSION_PAGE_SIZE,
                });
                self.sync_draft_shell_workspace(&workspace_directory);
                self.resolve_startup_submit_queue();
            }
            StartupSnapshot::Failed { message } => {
                if self.pending_startup_check.as_ref() != Some(&correlation) {
                    return;
                }
                self.pending_startup_check = None;
                self.dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                    result: Err(message),
                    session_page_size: SESSION_PAGE_SIZE,
                });
                self.resolve_startup_submit_queue();
            }
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn apply_correlated_conversation_snapshot(
        &mut self,
        correlation: Option<ConversationLoadCorrelation>,
        snapshot: CoreConversationSnapshot,
    ) {
        match (&correlation, &snapshot) {
            (Some(correlation), CoreConversationSnapshot::Loading) => {
                if self
                    .pending_conversation_load
                    .as_ref()
                    .is_some_and(|pending| pending.generation > correlation.generation)
                {
                    return;
                }
                self.pending_conversation_load = Some(correlation.clone());
            }
            (Some(correlation), CoreConversationSnapshot::Ready(ready)) => {
                if self.pending_conversation_load.as_ref() != Some(correlation)
                    || ready.conversation.thread_id != correlation.requested_thread_id
                {
                    return;
                }
                self.pending_conversation_load = None;
            }
            (Some(correlation), CoreConversationSnapshot::Failed { .. }) => {
                if self.pending_conversation_load.as_ref() != Some(correlation) {
                    return;
                }
                self.pending_conversation_load = None;
            }
            (None, CoreConversationSnapshot::Idle) => {
                self.pending_conversation_load = None;
            }
            _ => return,
        }
        self.apply_core_conversation_snapshot(snapshot);
    }

    pub(super) fn dispatch_core_command(&mut self, command: AppCommand) {
        let outcome = self.core_runtime.dispatch_command(command);
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn dispatch_core_input(&mut self, input: CoreInput) {
        let outcome = self.core_runtime.dispatch_input(input);
        self.apply_core_dispatch_outcome(outcome);
    }

    pub(super) fn apply_core_conversation_snapshot(&mut self, snapshot: CoreConversationSnapshot) {
        let loaded_successfully = matches!(&snapshot, CoreConversationSnapshot::Ready(_));
        let load_finished = matches!(
            &snapshot,
            CoreConversationSnapshot::Ready(_) | CoreConversationSnapshot::Failed { .. }
        );
        if matches!(&snapshot, CoreConversationSnapshot::Loading) {
            self.reset_planning_worker_panel_state();
            self.pending_resumed_session_planning_refresh = None;
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
            let resume_status_context = match &self.conversation_state {
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
                self.pending_resumed_session_planning_refresh =
                    Some(PendingResumedSessionPlanningRefresh {
                        correlation,
                        thread_id,
                        status_text,
                    });
                self.apply_core_dispatch_outcome(outcome);
            } else {
                self.pending_resumed_session_planning_refresh = None;
                self.surface_resumed_session_planning_context();
            }
        } else {
            self.pending_resumed_session_planning_refresh = None;
        }
        // A loaded conversation resets follow-up copy because auto-turn affordances
        // belong to the active thread, not the previous shell contents.
        self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
    }

    fn execute_shell_chrome_effect(&mut self, effect: ShellChromeEffect) {
        match effect {
            ShellChromeEffect::RunStartupChecks => {
                self.dispatch_core_command(AppCommand::RunStartupChecks {
                    workspace_directory: self.planning_workspace_directory(),
                });
            }
            ShellChromeEffect::LoadSessionCatalog {
                limit,
                current_workspace_directory,
            } => {
                // Session overlay requests are scoped to the visible conversation
                // workspace unless the reducer explicitly supplied another root.
                let workspace_directory = current_workspace_directory
                    .unwrap_or_else(|| self.current_workspace_directory());
                self.dispatch_core_command(AppCommand::LoadSessionCatalog {
                    limit,
                    workspace_directory,
                });
            }
        }
    }

    // Moving the conversation out prevents accidental partial mutation when lifecycle
    // reducers decide between loading, failed, and ready session states.
    fn take_conversation_lifecycle_state(&mut self) -> ConversationLifecycleState {
        ConversationLifecycleState {
            conversation_state: std::mem::replace(
                &mut self.conversation_state,
                ConversationState::Loading,
            ),
            turn_control_truth: self.turn_control_truth,
        }
    }

    fn apply_conversation_lifecycle_state(&mut self, state: ConversationLifecycleState) {
        self.conversation_state = state.conversation_state;
    }

    pub(super) fn reset_planning_worker_panel_state(&mut self) {
        self.planning_worker_panel_state = super::PlanningWorkerPanelState::default();
    }

    pub(super) fn dispatch_conversation_lifecycle(&mut self, event: ConversationLifecycleEvent) {
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
            self.post_turn_continuation_gate.advance();
            self.cancel_manual_prompt_preparation_for_identity_transition();
            self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
        }
        if matches!(&event, ConversationLifecycleEvent::NewDraftOpened { .. }) {
            self.dispatch_core_command(AppCommand::InvalidateConversationLoad);
        }
        let reduction =
            reduce_conversation_lifecycle(self.take_conversation_lifecycle_state(), event);
        self.apply_conversation_lifecycle_state(reduction.state);
        self.advance_planning_ui_intent_revision();
        self.surface_global_runtime_notices_if_ready();
        for effect in reduction.effects {
            self.execute_conversation_lifecycle_effect(effect);
        }
    }

    fn execute_conversation_lifecycle_effect(&mut self, effect: ConversationLifecycleEffect) {
        match effect {
            ConversationLifecycleEffect::LoadConversation {
                thread_id,
                fallback_workspace_directory,
            } => {
                self.dispatch_core_command(AppCommand::LoadConversation {
                    thread_id,
                    fallback_workspace_directory,
                });
            }
        }
    }

    pub(super) fn take_ready_conversation_state(&mut self) -> Option<ConversationViewModel> {
        let state = std::mem::replace(&mut self.conversation_state, ConversationState::Loading);
        match state {
            ConversationState::Ready(conversation) => Some(*conversation),
            other => {
                self.conversation_state = other;
                None
            }
        }
    }

    pub(super) fn dispatch_conversation_runtime(
        &mut self,
        event: ConversationRuntimeEvent,
    ) -> bool {
        let post_turn_context = self.post_turn_continuation_context(&event);
        let Some(conversation) = self.take_ready_conversation_state() else {
            return false;
        };

        let reduction = reduce_conversation_runtime(conversation, event);
        let mut effects = reduction.effects;
        let requests_turn_submission = effects.iter().any(|effect| {
            matches!(
                effect,
                ConversationRuntimeEffect::RequestTurnSubmission { .. }
            )
        });
        self.conversation_state = ConversationState::ready(reduction.state);
        self.advance_planning_ui_intent_revision();
        if !requests_turn_submission && !self.conversation_has_running_turn() {
            self.turn_steer_confirmation = None;
        }
        self.route_post_turn_continuation_effects(post_turn_context, &mut effects);
        let mut turn_submission_admitted = false;
        for effect in effects {
            turn_submission_admitted |= self.execute_conversation_runtime_effect(effect);
        }
        turn_submission_admitted
    }

    pub(super) fn dispatch_conversation_input(&mut self, event: ConversationInputEvent) {
        let event =
            if self.pending_manual_prompt_preparation.is_some() && event.mutates_input_buffer() {
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
        let reduction = reduce_conversation_input(conversation, event);
        self.conversation_state = ConversationState::ready(reduction.state);
        self.advance_planning_ui_intent_revision();
        if mutates_input_buffer {
            self.prompt_input_revision = self.prompt_input_revision.wrapping_add(1).max(1);
        }
    }

    pub(super) fn clear_input_buffer(&mut self) {
        self.dispatch_conversation_input(ConversationInputEvent::InputCleared);
    }

    fn conversation_intent_state(&self) -> ConversationIntentState {
        let mode = match &self.conversation_state {
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
                &self.conversation_state,
                ConversationState::Ready(conversation) if !conversation.can_accept_manual_prompt()
            ),
            mode,
            interrupt_support: match &self.conversation_state {
                ConversationState::Ready(conversation) => {
                    conversation.turn_control_truth().interrupt
                }
                ConversationState::Loading | ConversationState::Failed(_) => {
                    self.turn_control_truth.interrupt
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
                // New drafts must leave transient chrome and planning worker context behind;
                // otherwise the blank prompt can inherit stale session-side affordances.
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.reset_planning_worker_panel_state();
                let workspace_directory = self.current_workspace_directory();
                self.dispatch_conversation_lifecycle(ConversationLifecycleEvent::NewDraftOpened {
                    workspace_directory: workspace_directory.clone(),
                });
                self.refresh_ready_conversation_planning_runtime_projection();
            }
            ConversationIntentEffect::OpenSession { session } => {
                // Session selection is a lifecycle transition, not just a transcript swap.
                // Reset planning side panels before the async load result returns.
                self.dispatch_shell_chrome(ShellChromeEvent::TransientChromeDismissed);
                self.reset_planning_worker_panel_state();
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
        let invalidates_prior_requests = match &event {
            AutoFollowControlEvent::AutoFollowPaused
            | AutoFollowControlEvent::PlanningAuthorityMutationSettled => true,
            // Budget edits affect the eventual auto-prompt decision, but the
            // in-flight planning settlement still owns queue/receipt completion.
            AutoFollowControlEvent::MaxAutoTurnsUpdated { .. } => false,
            AutoFollowControlEvent::DraftWorkspaceSynced {
                workspace_directory,
            } => matches!(
                &self.conversation_state,
                ConversationState::Ready(conversation)
                    if conversation.draft_workspace_directory() != workspace_directory
            ),
        };
        if invalidates_prior_requests {
            self.post_turn_continuation_gate.advance();
        }
        if matches!(
            &event,
            AutoFollowControlEvent::DraftWorkspaceSynced {
                workspace_directory,
            } if matches!(
                &self.conversation_state,
                ConversationState::Ready(conversation)
                    if conversation.draft_workspace_directory() != workspace_directory
            )
        ) {
            self.cancel_manual_prompt_preparation_for_identity_transition();
        }
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };
        let reduction = reduce_auto_follow_controls(conversation, event);
        self.conversation_state = ConversationState::ready(reduction.state);
        self.advance_planning_ui_intent_revision();
        if reduction.close_max_auto_turns_editor {
            self.dispatch_auto_follow_overlay_ui(AutoFollowOverlayUiEvent::EditFinished);
        }
    }

    pub(super) fn dispatch_auto_follow_overlay_ui(&mut self, event: AutoFollowOverlayUiEvent) {
        let state = std::mem::take(&mut self.auto_follow_overlay_ui_state);
        self.auto_follow_overlay_ui_state = reduce_auto_follow_overlay_ui(state, event);
    }
}
