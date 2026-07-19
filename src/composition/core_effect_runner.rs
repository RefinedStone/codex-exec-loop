use std::thread;

use anyhow::Result;

use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};
use crate::application::service::conversation_service::{
    ConversationService, LoadedConversationThreadSnapshot,
};
use crate::application::service::manual_prompt_preparation::ManualPromptPreparationService;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::{PlanningRuntimeUseCases, PlanningServices};
use crate::application::service::post_turn_evaluation::{
    POST_TURN_EVALUATION_TIMEOUT, PostTurnEvaluationService,
};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::composition::core_turn_submission;
use crate::core::app::{
    ConversationLoadCorrelation, ConversationReadySnapshot, ConversationThreadReviewSnapshot,
    ParallelPeekLoadCorrelation, ReviewCenterHistoryEntrySnapshot, ReviewCenterInboxItemSnapshot,
    ReviewCenterLoadCorrelation, ReviewCenterSnapshot, SessionCatalogLoadCorrelation,
    SessionCatalogReadySnapshot, SessionRenameCorrelation, StartupCheckCorrelation,
};
use crate::core::app::{CoreEffect, CoreEffectCompletion, CoreInput, StartupReadySnapshot};
use crate::core::runtime::CoreEffectExecutor;
use crate::core::runtime::CoreInputSender;
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogRequest};
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Clone)]
pub struct CoreEffectRunner {
    startup_service: StartupService,
    session_service: SessionService,
    conversation_service: ConversationService,
    planning_runtime: PlanningRuntimeUseCases,
    parallel_mode_turn_service: ParallelModeTurnService,
    manual_prompt_preparation_service: ManualPromptPreparationService,
    post_turn_evaluation_service: PostTurnEvaluationService,
    input_sender: CoreInputSender,
}

impl CoreEffectRunner {
    pub fn new(
        startup_service: StartupService,
        session_service: SessionService,
        conversation_service: ConversationService,
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        post_turn_evaluation_service: PostTurnEvaluationService,
        input_sender: CoreInputSender,
    ) -> Self {
        Self {
            startup_service,
            session_service,
            conversation_service,
            planning_runtime: planning_feature.runtime.clone(),
            parallel_mode_turn_service,
            manual_prompt_preparation_service: ManualPromptPreparationService::new(
                planning_feature,
            ),
            post_turn_evaluation_service,
            input_sender,
        }
    }

    pub fn spawn_startup_checks(&self, correlation: StartupCheckCorrelation) {
        let startup_service = self.startup_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let completion = startup_checks_completion(correlation, startup_service.run_checks());
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
        match effect {
            CoreEffect::RunStartupChecks { correlation } => {
                self.spawn_startup_checks(correlation);
                None
            }
            CoreEffect::LoadSessionCatalog {
                correlation,
                limit,
                workspace_directory,
            } => {
                self.spawn_session_catalog_load(correlation, limit, workspace_directory);
                None
            }
            CoreEffect::RenameSession { correlation } => {
                self.spawn_session_rename(correlation);
                None
            }
            CoreEffect::LoadConversation {
                correlation,
                fallback_workspace_directory,
            } => {
                self.spawn_conversation_load(correlation, fallback_workspace_directory);
                None
            }
            CoreEffect::LoadParallelPeekConversation { correlation } => {
                self.spawn_parallel_peek_conversation_load(correlation);
                None
            }
            CoreEffect::LoadReviewCenter { correlation } => {
                self.spawn_review_center_load(correlation);
                None
            }
            CoreEffect::PrepareManualPrompt(request) => Some(CoreInput::EffectCompleted(
                CoreEffectCompletion::ManualPromptPrepared(Box::new(
                    self.manual_prompt_preparation_service.prepare(*request),
                )),
            )),
            CoreEffect::SubmitTurn {
                correlation,
                request,
            } => {
                self.spawn_turn_submission(correlation, request);
                None
            }
            CoreEffect::SteerTurn {
                correlation,
                request,
            } => {
                self.spawn_turn_steer(correlation, request);
                None
            }
            CoreEffect::EvaluatePostTurn(request) => {
                self.spawn_post_turn_evaluation(*request);
                None
            }
        }
    }

    pub fn spawn_session_catalog_load(
        &self,
        correlation: SessionCatalogLoadCorrelation,
        limit: usize,
        workspace_directory: String,
    ) {
        let session_service = self.session_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let request = SessionCatalogRequest::for_workspace(limit, workspace_directory);
            let completion = session_catalog_completion(
                correlation,
                session_service.load_session_catalog(request),
            );
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_session_rename(&self, correlation: SessionRenameCorrelation) {
        let session_service = self.session_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = session_service.rename_session(correlation.request.clone());
            let completion = session_rename_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_conversation_load(
        &self,
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
    ) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result = conversation_service.load_thread_snapshot(
                correlation.requested_thread_id.as_str(),
                fallback_workspace_directory.as_str(),
            );
            let completion = conversation_snapshot_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_parallel_peek_conversation_load(&self, correlation: ParallelPeekLoadCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let result =
                conversation_service.load_snapshot(correlation.requested_thread_id.as_str());
            let completion = parallel_peek_conversation_completion(correlation, result);
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_review_center_load(&self, correlation: ReviewCenterLoadCorrelation) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let snapshot = load_review_center_snapshot(&conversation_service, &correlation);
            let _ = input_sender.send(CoreInput::EffectCompleted(
                CoreEffectCompletion::ReviewCenterLoaded {
                    correlation,
                    snapshot,
                },
            ));
        });
    }

    pub fn spawn_turn_submission(
        &self,
        correlation: crate::core::app::TurnSubmissionCorrelation,
        request: crate::core::app::TurnSubmissionRequest,
    ) {
        core_turn_submission::spawn_turn_submission_worker(
            correlation,
            request,
            self.conversation_service.clone(),
            self.planning_runtime.clone(),
            self.parallel_mode_turn_service.clone(),
            self.input_sender.clone(),
        );
    }

    pub fn spawn_turn_steer(
        &self,
        correlation: crate::core::app::TurnSteerCorrelation,
        request: crate::domain::conversation::ConversationTurnSteerRequest,
    ) {
        let conversation_service = self.conversation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let completion =
                turn_steer_completion(correlation, conversation_service.steer_turn(request));
            let _ = input_sender.send(CoreInput::EffectCompleted(completion));
        });
    }

    pub fn spawn_post_turn_evaluation(&self, request: crate::domain::planning::PostTurnRequest) {
        let service = self.post_turn_evaluation_service.clone();
        let input_sender = self.input_sender.clone();
        thread::spawn(move || {
            let execution = service.evaluate_with_timeout(request, POST_TURN_EVALUATION_TIMEOUT);
            let _ = input_sender.send(CoreInput::EffectCompleted(
                CoreEffectCompletion::PostTurnEvaluationCompleted(Box::new(execution)),
            ));
        });
    }
}

impl CoreEffectExecutor for CoreEffectRunner {
    fn run_effect(&self, effect: CoreEffect) -> Option<CoreInput> {
        CoreEffectRunner::run_effect(self, effect)
    }
}

fn startup_checks_completion(
    correlation: StartupCheckCorrelation,
    result: Result<StartupDiagnostics>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::StartupChecksLoaded {
        correlation,
        result: result
            .map(StartupReadySnapshot::from_diagnostics)
            .map(Box::new)
            .map_err(|error| format!("{error:#}")),
    }
}

fn session_catalog_completion(
    correlation: SessionCatalogLoadCorrelation,
    result: Result<SessionCatalog>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::SessionCatalogLoaded {
        correlation,
        result: result
            .map(SessionCatalogReadySnapshot::from_catalog)
            .map_err(|error| error.to_string()),
    }
}

fn session_rename_completion(
    correlation: SessionRenameCorrelation,
    result: Result<()>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::SessionRenamed {
        correlation,
        result: result.map_err(|error| error.to_string()),
    }
}

fn conversation_snapshot_completion(
    correlation: ConversationLoadCorrelation,
    result: Result<LoadedConversationThreadSnapshot>,
) -> CoreEffectCompletion {
    let requested_thread_id = correlation.requested_thread_id.clone();
    CoreEffectCompletion::ConversationLoaded {
        correlation,
        result: result
            .and_then(|snapshot| {
                if snapshot.conversation.thread_id == requested_thread_id {
                    Ok(snapshot)
                } else {
                    Err(anyhow::anyhow!(
                        "conversation provider returned a different thread"
                    ))
                }
            })
            .map(conversation_ready_snapshot)
            .map(Box::new)
            .map_err(|error| error.to_string()),
    }
}

fn parallel_peek_conversation_completion(
    correlation: ParallelPeekLoadCorrelation,
    result: Result<crate::domain::conversation::ConversationSnapshot>,
) -> CoreEffectCompletion {
    let requested_thread_id = correlation.requested_thread_id.clone();
    let result = result
        .and_then(|snapshot| {
            if snapshot.thread_id == requested_thread_id {
                Ok(snapshot)
            } else {
                Err(anyhow::anyhow!(
                    "conversation provider returned a different thread"
                ))
            }
        })
        .map(ConversationReadySnapshot::from)
        .map(Box::new)
        .map_err(|error| error.to_string());
    CoreEffectCompletion::ParallelPeekConversationLoaded {
        correlation,
        result,
    }
}

fn turn_steer_completion(
    correlation: crate::core::app::TurnSteerCorrelation,
    result: Result<crate::domain::conversation::ConversationTurnSteerReceipt>,
) -> CoreEffectCompletion {
    CoreEffectCompletion::TurnSteered {
        correlation,
        result: result.map_err(|error| error.to_string()),
    }
}

fn load_review_center_snapshot(
    conversation_service: &ConversationService,
    correlation: &ReviewCenterLoadCorrelation,
) -> ReviewCenterSnapshot {
    let current_thread_reviews = match correlation.active_thread_id.as_deref() {
        Some(thread_id) => conversation_service.load_review_center_thread_reviews_for_workspace(
            &correlation.workspace_directory,
            thread_id,
        ),
        None => Ok(Vec::new()),
    };
    let pending_inbox = conversation_service
        .load_review_center_pending_inbox_for_workspace(&correlation.workspace_directory);
    let recent_history = conversation_service
        .load_review_center_recent_history_for_workspace(&correlation.workspace_directory);
    review_center_snapshot(current_thread_reviews, pending_inbox, recent_history)
}

fn review_center_snapshot(
    current_thread_reviews: Result<Vec<ReviewCenterThreadProjection>>,
    pending_inbox: Result<Vec<ReviewCenterInboxItem>>,
    recent_history: Result<Vec<ReviewCenterHistoryEntry>>,
) -> ReviewCenterSnapshot {
    ReviewCenterSnapshot {
        current_thread_reviews: current_thread_reviews
            .map(|reviews| {
                reviews
                    .into_iter()
                    .map(review_center_thread_snapshot)
                    .collect()
            })
            .map_err(|error| error.to_string()),
        pending_inbox: pending_inbox
            .map(|items| {
                items
                    .into_iter()
                    .map(review_center_inbox_item_snapshot)
                    .collect()
            })
            .map_err(|error| error.to_string()),
        recent_history: recent_history
            .map(|entries| {
                entries
                    .into_iter()
                    .map(review_center_history_entry_snapshot)
                    .collect()
            })
            .map_err(|error| error.to_string()),
    }
}

fn review_center_thread_snapshot(
    review: ReviewCenterThreadProjection,
) -> ConversationThreadReviewSnapshot {
    ConversationThreadReviewSnapshot {
        thread_id: review.thread_id,
        review_id: review.review_id,
        review_label: review.review_label,
        review_state: review.review_state,
        review_summary: review.review_summary,
        requested_at: review.requested_at,
        updated_at: review.updated_at,
        handoff_target: review.handoff_target,
        handoff_note: review.handoff_note,
    }
}

fn review_center_inbox_item_snapshot(item: ReviewCenterInboxItem) -> ReviewCenterInboxItemSnapshot {
    ReviewCenterInboxItemSnapshot {
        review_id: item.review_id,
        thread_id: item.thread_id,
        inbox_state: item.inbox_state,
        summary: item.summary,
        requested_at: item.requested_at,
        last_activity_at: item.last_activity_at,
        handoff_target: item.handoff_target,
    }
}

fn review_center_history_entry_snapshot(
    entry: ReviewCenterHistoryEntry,
) -> ReviewCenterHistoryEntrySnapshot {
    ReviewCenterHistoryEntrySnapshot {
        review_id: entry.review_id,
        thread_id: entry.thread_id,
        event_kind: entry.event_kind,
        summary: entry.summary,
        recorded_at: entry.recorded_at,
    }
}

fn conversation_ready_snapshot(
    snapshot: LoadedConversationThreadSnapshot,
) -> ConversationReadySnapshot {
    ConversationReadySnapshot::from_parts(
        snapshot.conversation,
        snapshot
            .thread_review
            .into_iter()
            .map(review_center_thread_snapshot)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};
    use crate::domain::recent_sessions::{
        RecentSessions, SessionCatalogTier, SessionRenameRequest,
    };
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    fn startup_correlation() -> StartupCheckCorrelation {
        StartupCheckCorrelation::new(7)
    }

    fn session_catalog_correlation() -> SessionCatalogLoadCorrelation {
        SessionCatalogLoadCorrelation::new(8)
    }

    fn session_rename_correlation() -> SessionRenameCorrelation {
        SessionRenameCorrelation::new(9, SessionRenameRequest::new("thread-1", "Renamed"))
    }

    fn conversation_correlation(thread_id: &str) -> ConversationLoadCorrelation {
        ConversationLoadCorrelation::new(9, thread_id)
    }

    fn turn_steer_correlation() -> crate::core::app::TurnSteerCorrelation {
        crate::core::app::TurnSteerCorrelation::new(
            3,
            crate::core::app::TurnSubmissionCorrelation::new(2),
        )
    }

    #[test]
    fn startup_success_maps_to_core_completion() {
        let diagnostics = StartupDiagnostics {
            cwd: "/tmp/workspace".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "/usr/bin/codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/workspace".to_string(),
            workspace_detail: "git repo: /tmp/workspace".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::default(),
            initialize_ok: true,
            initialize_detail: "initialized".to_string(),
            account_ok: true,
            account_detail: "authenticated".to_string(),
            warnings: Vec::new(),
            schema_snapshot: "embedded schema".to_string(),
        };

        assert_eq!(
            startup_checks_completion(startup_correlation(), Ok(diagnostics)),
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_correlation(),
                result: Ok(Box::new(StartupReadySnapshot {
                    cwd: "/tmp/workspace".to_string(),
                    workspace_path: "/tmp/workspace".to_string(),
                    can_continue: true,
                    codex_binary: crate::core::app::StartupDiagnosticSnapshot {
                        ok: true,
                        detail: "/usr/bin/codex".to_string(),
                    },
                    workspace: crate::core::app::StartupDiagnosticSnapshot {
                        ok: true,
                        detail: "git repo: /tmp/workspace".to_string(),
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
                })),
            }
        );
    }

    #[test]
    fn startup_error_maps_to_core_completion() {
        let error = anyhow::anyhow!("unsafe executable ancestor")
            .context("failed to pin trusted Codex executable");
        assert_eq!(
            startup_checks_completion(startup_correlation(), Err(error)),
            CoreEffectCompletion::StartupChecksLoaded {
                correlation: startup_correlation(),
                result: Err(
                    "failed to pin trusted Codex executable: unsafe executable ancestor"
                        .to_string()
                ),
            }
        );
    }

    #[test]
    fn session_catalog_success_maps_to_core_completion() {
        let catalog = RecentSessions {
            items: Vec::new(),
            warnings: vec!["partial catalog".to_string()],
            next_cursor: None,
        }
        .into();

        assert_eq!(
            session_catalog_completion(session_catalog_correlation(), Ok(catalog)),
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(),
                result: Ok(SessionCatalogReadySnapshot {
                    catalog: Box::new(
                        RecentSessions {
                            items: Vec::new(),
                            warnings: vec!["partial catalog".to_string()],
                            next_cursor: None,
                        }
                        .into(),
                    ),
                    tier_label: SessionCatalogTier::ProviderBackedCatalog
                        .label()
                        .to_string(),
                    item_count: 0,
                    warnings: vec!["partial catalog".to_string()],
                })
            }
        );
    }

    #[test]
    fn session_catalog_error_maps_to_core_completion() {
        assert_eq!(
            session_catalog_completion(
                session_catalog_correlation(),
                Err(anyhow::anyhow!("catalog unavailable"))
            ),
            CoreEffectCompletion::SessionCatalogLoaded {
                correlation: session_catalog_correlation(),
                result: Err("catalog unavailable".to_string())
            }
        );
    }

    #[test]
    fn session_rename_result_maps_to_exact_core_completion() {
        assert_eq!(
            session_rename_completion(session_rename_correlation(), Ok(())),
            CoreEffectCompletion::SessionRenamed {
                correlation: session_rename_correlation(),
                result: Ok(()),
            }
        );
        assert_eq!(
            session_rename_completion(
                session_rename_correlation(),
                Err(anyhow::anyhow!("rename unavailable")),
            ),
            CoreEffectCompletion::SessionRenamed {
                correlation: session_rename_correlation(),
                result: Err("rename unavailable".to_string()),
            }
        );
    }

    #[test]
    fn turn_steer_result_maps_to_exact_core_completion() {
        let receipt = crate::domain::conversation::ConversationTurnSteerReceipt {
            turn_id: "turn-1".to_string(),
        };
        assert_eq!(
            turn_steer_completion(turn_steer_correlation(), Ok(receipt.clone())),
            CoreEffectCompletion::TurnSteered {
                correlation: turn_steer_correlation(),
                result: Ok(receipt),
            }
        );
        assert_eq!(
            turn_steer_completion(
                turn_steer_correlation(),
                Err(anyhow::anyhow!("steer unavailable")),
            ),
            CoreEffectCompletion::TurnSteered {
                correlation: turn_steer_correlation(),
                result: Err("steer unavailable".to_string()),
            }
        );
    }

    #[test]
    fn review_center_snapshot_maps_every_projection_field() {
        let mut thread_review = ReviewCenterThreadProjection::new(
            "thread-1",
            "review-1",
            "Manual review",
            "pending",
            "Need operator follow-up",
            "2026-07-06T10:00:00Z",
            "2026-07-06T11:00:00Z",
        );
        thread_review.handoff_target = Some("operator".to_string());
        thread_review.handoff_note = Some("resume in inbox".to_string());
        let mut inbox_item = ReviewCenterInboxItem::new(
            "review-2",
            "thread-2",
            "pending",
            "Approve filesystem access",
            "2026-07-07T10:00:00Z",
            "2026-07-07T11:00:00Z",
        );
        inbox_item.handoff_target = Some("security".to_string());
        let history_entry = ReviewCenterHistoryEntry::new(
            "review-3",
            "thread-3",
            "approved",
            "Operator approved",
            "2026-07-08T12:00:00Z",
        );

        assert_eq!(
            review_center_snapshot(
                Ok(vec![thread_review]),
                Ok(vec![inbox_item]),
                Ok(vec![history_entry]),
            ),
            ReviewCenterSnapshot {
                current_thread_reviews: Ok(vec![ConversationThreadReviewSnapshot {
                    thread_id: "thread-1".to_string(),
                    review_id: "review-1".to_string(),
                    review_label: "Manual review".to_string(),
                    review_state: "pending".to_string(),
                    review_summary: "Need operator follow-up".to_string(),
                    requested_at: "2026-07-06T10:00:00Z".to_string(),
                    updated_at: "2026-07-06T11:00:00Z".to_string(),
                    handoff_target: Some("operator".to_string()),
                    handoff_note: Some("resume in inbox".to_string()),
                }]),
                pending_inbox: Ok(vec![ReviewCenterInboxItemSnapshot {
                    review_id: "review-2".to_string(),
                    thread_id: "thread-2".to_string(),
                    inbox_state: "pending".to_string(),
                    summary: "Approve filesystem access".to_string(),
                    requested_at: "2026-07-07T10:00:00Z".to_string(),
                    last_activity_at: "2026-07-07T11:00:00Z".to_string(),
                    handoff_target: Some("security".to_string()),
                }]),
                recent_history: Ok(vec![ReviewCenterHistoryEntrySnapshot {
                    review_id: "review-3".to_string(),
                    thread_id: "thread-3".to_string(),
                    event_kind: "approved".to_string(),
                    summary: "Operator approved".to_string(),
                    recorded_at: "2026-07-08T12:00:00Z".to_string(),
                }]),
            }
        );
    }

    #[test]
    fn review_center_snapshot_preserves_partial_failures() {
        let thread_review = ReviewCenterThreadProjection::new(
            "thread-1",
            "review-1",
            "Manual review",
            "pending",
            "Needs review",
            "2026-07-06T10:00:00Z",
            "2026-07-06T11:00:00Z",
        );
        let history_entry = ReviewCenterHistoryEntry::new(
            "review-1",
            "thread-1",
            "requested",
            "Review requested",
            "2026-07-06T10:00:00Z",
        );

        let snapshot = review_center_snapshot(
            Ok(vec![thread_review]),
            Err(anyhow::anyhow!("inbox unavailable")),
            Ok(vec![history_entry]),
        );

        assert_eq!(snapshot.pending_inbox, Err("inbox unavailable".to_string()));
        assert_eq!(snapshot.current_thread_reviews.unwrap().len(), 1);
        assert_eq!(snapshot.recent_history.unwrap().len(), 1);
    }

    #[test]
    fn conversation_snapshot_success_maps_to_core_completion() {
        let conversation = crate::domain::conversation::ConversationSnapshot {
            thread_id: "thread-1".to_string(),
            title: "Core runtime".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: vec![ConversationMessage::new(
                ConversationMessageKind::User,
                "hello",
                None,
                None,
            )],
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        };
        let mut thread_review = ReviewCenterThreadProjection::new(
            "thread-1",
            "review-1",
            "Manual review",
            "pending",
            "Need operator follow-up",
            "2026-07-06T10:00:00Z",
            "2026-07-06T11:00:00Z",
        );
        thread_review.handoff_target = Some("operator".to_string());
        thread_review.handoff_note = Some("resume in inbox".to_string());

        assert_eq!(
            conversation_snapshot_completion(
                conversation_correlation("thread-1"),
                Ok(LoadedConversationThreadSnapshot {
                    conversation: conversation.clone(),
                    thread_review: vec![thread_review],
                }),
            ),
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_correlation("thread-1"),
                result: Ok(Box::new(ConversationReadySnapshot::from_parts(
                    conversation,
                    vec![ConversationThreadReviewSnapshot {
                        thread_id: "thread-1".to_string(),
                        review_id: "review-1".to_string(),
                        review_label: "Manual review".to_string(),
                        review_state: "pending".to_string(),
                        review_summary: "Need operator follow-up".to_string(),
                        requested_at: "2026-07-06T10:00:00Z".to_string(),
                        updated_at: "2026-07-06T11:00:00Z".to_string(),
                        handoff_target: Some("operator".to_string()),
                        handoff_note: Some("resume in inbox".to_string()),
                    }],
                ))),
            }
        );
    }

    #[test]
    fn conversation_snapshot_error_maps_to_core_completion() {
        assert_eq!(
            conversation_snapshot_completion(
                conversation_correlation("thread-1"),
                Err(anyhow::anyhow!("thread unavailable")),
            ),
            CoreEffectCompletion::ConversationLoaded {
                correlation: conversation_correlation("thread-1"),
                result: Err("thread unavailable".to_string()),
            }
        );
    }

    #[test]
    fn conversation_completion_rejects_provider_thread_mismatch() {
        let snapshot = LoadedConversationThreadSnapshot {
            conversation: crate::domain::conversation::ConversationSnapshot {
                thread_id: "thread-other".to_string(),
                title: "Wrong thread".to_string(),
                cwd: "/tmp/workspace".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
                item_lifecycle: Default::default(),
            },
            thread_review: Vec::new(),
        };

        let CoreEffectCompletion::ConversationLoaded { result, .. } =
            conversation_snapshot_completion(conversation_correlation("thread-1"), Ok(snapshot))
        else {
            panic!("general conversation load should use its correlated completion variant");
        };
        assert_eq!(
            result,
            Err("conversation provider returned a different thread".to_string())
        );
    }

    #[test]
    fn parallel_peek_completion_keeps_correlation_and_validates_thread() {
        let conversation = crate::domain::conversation::ConversationSnapshot {
            thread_id: "thread-peek".to_string(),
            title: "Peek thread".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        };

        assert_eq!(
            parallel_peek_conversation_completion(
                ParallelPeekLoadCorrelation::new(7, "thread-peek"),
                Ok(conversation.clone()),
            ),
            CoreEffectCompletion::ParallelPeekConversationLoaded {
                correlation: ParallelPeekLoadCorrelation::new(7, "thread-peek"),
                result: Ok(Box::new(ConversationReadySnapshot::from(conversation))),
            }
        );

        let mismatched = crate::domain::conversation::ConversationSnapshot {
            thread_id: "wrong-thread".to_string(),
            title: "Wrong thread".to_string(),
            cwd: "/tmp/workspace".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
            item_lifecycle: Default::default(),
        };
        let CoreEffectCompletion::ParallelPeekConversationLoaded { result, .. } =
            parallel_peek_conversation_completion(
                ParallelPeekLoadCorrelation::new(8, "thread-peek"),
                Ok(mismatched),
            )
        else {
            panic!("parallel peek completion must keep its dedicated variant");
        };
        assert_eq!(
            result,
            Err("conversation provider returned a different thread".to_string())
        );
    }
}
