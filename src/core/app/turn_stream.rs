use crate::domain::conversation::{
    ConversationApprovalRequest, ConversationApprovalResolution, ConversationApprovalReview,
    ConversationToolActivity,
};
use crate::domain::planning::{PostTurnExecution, TurnSnapshotCapture};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
use crate::domain::turn_terminal::{
    ConversationTurnApplicationDelivery, ConversationTurnError, ConversationTurnTerminalOutcome,
    ConversationTurnTerminalReceipt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnStreamState {
    revision: u64,
    thread_id: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
    active_turn_id: Option<String>,
    status_text: Option<String>,
    terminal: Option<TurnStreamTerminalSnapshot>,
    last_applied_post_turn_evaluation_id: Option<String>,
}

impl TurnStreamState {
    pub fn new() -> Self {
        Self {
            revision: 0,
            thread_id: None,
            title: None,
            cwd: None,
            active_turn_id: None,
            status_text: None,
            terminal: None,
            last_applied_post_turn_evaluation_id: None,
        }
    }

    pub fn seed_loaded_thread_identity(
        &mut self,
        thread_id: impl Into<String>,
        title: impl Into<String>,
        cwd: impl Into<String>,
    ) {
        self.thread_id = Some(thread_id.into());
        self.title = Some(title.into());
        self.cwd = Some(cwd.into());
        self.active_turn_id = None;
        self.status_text = None;
        self.terminal = None;
        self.last_applied_post_turn_evaluation_id = None;
    }

    pub fn begin_submission(&mut self) {
        self.active_turn_id = None;
        self.status_text = Some("starting turn".to_string());
        self.terminal = None;
        self.last_applied_post_turn_evaluation_id = None;
    }

    pub fn apply_stream_event(&mut self, event: TurnStreamEvent) -> TurnStreamSnapshot {
        let update = match event {
            TurnStreamEvent::AttachmentObserved { profile } => {
                TurnStreamUpdate::AttachmentObserved { profile }
            }
            TurnStreamEvent::ThreadPrepared {
                thread_id,
                title,
                cwd,
            } => {
                self.thread_id = Some(thread_id.clone());
                self.title = Some(title.clone());
                self.cwd = Some(cwd.clone());
                self.active_turn_id = None;
                self.terminal = None;
                self.last_applied_post_turn_evaluation_id = None;
                self.status_text = Some("thread started".to_string());
                TurnStreamUpdate::ThreadPrepared {
                    thread_id,
                    title,
                    cwd,
                    status_text: "thread started".to_string(),
                }
            }
            TurnStreamEvent::TurnStarted { turn_id } => {
                self.active_turn_id = Some(turn_id.clone());
                self.terminal = None;
                self.last_applied_post_turn_evaluation_id = None;
                self.status_text = Some("turn started".to_string());
                TurnStreamUpdate::TurnStarted {
                    turn_id,
                    status_text: "turn started".to_string(),
                }
            }
            TurnStreamEvent::StatusUpdated { text } => {
                self.status_text = Some(text.clone());
                TurnStreamUpdate::StatusUpdated { text }
            }
            TurnStreamEvent::AgentMessageDelta {
                item_id,
                phase,
                delta,
            } => TurnStreamUpdate::AgentMessageDelta {
                item_id,
                phase,
                delta,
            },
            TurnStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
            } => TurnStreamUpdate::AgentMessageCompleted {
                item_id,
                phase,
                text,
            },
            TurnStreamEvent::ToolActivity { activity } => {
                TurnStreamUpdate::ToolActivity { activity }
            }
            TurnStreamEvent::ApprovalReviewUpdated { review } => {
                TurnStreamUpdate::ApprovalReviewUpdated { review }
            }
            TurnStreamEvent::ApprovalRequested { request } => {
                self.status_text = Some("approval required".to_string());
                TurnStreamUpdate::ApprovalRequested { request }
            }
            TurnStreamEvent::ApprovalResolved {
                approval_id,
                resolution,
            } => TurnStreamUpdate::ApprovalResolved {
                approval_id,
                resolution,
            },
            TurnStreamEvent::TurnInterruptRequestFailed { message } => {
                self.status_text = Some(message.clone());
                TurnStreamUpdate::TurnInterruptRequestFailed { message }
            }
            TurnStreamEvent::TurnRetrying {
                thread_id,
                turn_id,
                error,
            } => self.turn_retrying_update(thread_id, turn_id, error),
            TurnStreamEvent::TurnTerminal {
                receipt,
                execution_snapshot_capture,
            } => self.turn_terminal_update(receipt, execution_snapshot_capture),
            TurnStreamEvent::Failed { message } => {
                if self.terminal.is_some() {
                    TurnStreamUpdate::RuntimeFailureIgnored { message }
                } else {
                    self.active_turn_id = None;
                    self.status_text = Some("turn failed".to_string());
                    self.terminal = Some(TurnStreamTerminalSnapshot::Failed {
                        message: message.clone(),
                    });
                    TurnStreamUpdate::Failed {
                        message,
                        status_text: "turn failed".to_string(),
                    }
                }
            }
        };

        self.snapshot(update)
    }

    #[cfg(test)]
    pub fn apply_turn_completed(
        &mut self,
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: TurnSnapshotCapture,
    ) -> TurnStreamSnapshot {
        let thread_id = self.thread_id.clone().unwrap_or_default();
        let receipt = ConversationTurnTerminalReceipt::completed(
            thread_id,
            turn_id,
            changed_planning_file_paths,
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        let update = if self.thread_id.is_none() && self.active_turn_id.is_none() {
            self.apply_terminal_receipt(receipt, Some(execution_snapshot_capture))
        } else {
            self.turn_terminal_update(receipt, Some(execution_snapshot_capture))
        };
        self.snapshot(update)
    }

    pub fn apply_runtime_notice(&mut self, notice: String) -> TurnStreamSnapshot {
        self.snapshot(TurnStreamUpdate::RuntimeNotice { notice })
    }

    pub fn accept_post_turn_evaluation_completion(
        &mut self,
        execution: &PostTurnExecution,
    ) -> bool {
        if !self.post_turn_evaluation_matches_latest_completed_turn(execution) {
            return false;
        }
        self.last_applied_post_turn_evaluation_id = Some(execution.completed_turn_id.clone());
        true
    }

    fn post_turn_evaluation_matches_latest_completed_turn(
        &self,
        execution: &PostTurnExecution,
    ) -> bool {
        self.thread_id.as_deref() == Some(execution.thread_id.as_str())
            && self.active_turn_id.is_none()
            && self.last_applied_post_turn_evaluation_id.as_deref()
                != Some(execution.completed_turn_id.as_str())
            && matches!(
                &self.terminal,
                Some(TurnStreamTerminalSnapshot::Turn { receipt })
                    if receipt.turn_id == execution.completed_turn_id
                        && receipt.is_completed_and_confirmed()
            )
    }

    fn turn_retrying_update(
        &mut self,
        thread_id: String,
        turn_id: String,
        error: ConversationTurnError,
    ) -> TurnStreamUpdate {
        let correlation_failure = self.correlation_failure(&thread_id, &turn_id);
        if correlation_failure.is_none() && self.terminal.is_none() {
            self.status_text = Some("turn retrying".to_string());
        }
        TurnStreamUpdate::TurnRetrying {
            thread_id,
            turn_id,
            error,
            correlation_failure,
            status_text: "turn retrying".to_string(),
        }
    }

    fn turn_terminal_update(
        &mut self,
        receipt: ConversationTurnTerminalReceipt,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
    ) -> TurnStreamUpdate {
        if self.terminal.is_some() {
            return TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(receipt),
                reason: TurnStreamTerminalRejection::TerminalAlreadyApplied,
            };
        }
        if let Some(reason) = self.correlation_failure(&receipt.thread_id, &receipt.turn_id) {
            return TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(receipt),
                reason,
            };
        }
        self.apply_terminal_receipt(receipt, execution_snapshot_capture)
    }

    fn apply_terminal_receipt(
        &mut self,
        receipt: ConversationTurnTerminalReceipt,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
    ) -> TurnStreamUpdate {
        self.active_turn_id = None;
        let status_text = terminal_status_text(&receipt).to_string();
        self.status_text = Some(status_text.clone());
        self.terminal = Some(TurnStreamTerminalSnapshot::Turn {
            receipt: Box::new(receipt.clone()),
        });
        if receipt.is_completed_and_confirmed() {
            TurnStreamUpdate::TurnCompleted {
                turn_id: receipt.turn_id.clone(),
                changed_planning_file_paths: receipt
                    .observations
                    .changed_planning_file_paths
                    .clone(),
                execution_snapshot_capture,
                status_text,
            }
        } else {
            TurnStreamUpdate::TurnTerminal {
                receipt: Box::new(receipt),
                execution_snapshot_capture,
                status_text,
            }
        }
    }

    fn correlation_failure(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Option<TurnStreamTerminalRejection> {
        if self.thread_id.as_deref() != Some(thread_id) {
            return Some(TurnStreamTerminalRejection::ThreadMismatch {
                expected_thread_id: self.thread_id.clone(),
            });
        }
        if self.active_turn_id.as_deref() != Some(turn_id) {
            return Some(TurnStreamTerminalRejection::TurnMismatch {
                expected_turn_id: self.active_turn_id.clone(),
            });
        }
        None
    }

    fn snapshot(&mut self, update: TurnStreamUpdate) -> TurnStreamSnapshot {
        self.revision += 1;
        TurnStreamSnapshot {
            revision: self.revision,
            thread_id: self.thread_id.clone(),
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            active_turn_id: self.active_turn_id.clone(),
            status_text: self.status_text.clone(),
            terminal: self.terminal.clone(),
            update,
        }
    }
}

impl Default for TurnStreamState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnStreamSnapshot {
    pub revision: u64,
    pub thread_id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub active_turn_id: Option<String>,
    pub status_text: Option<String>,
    pub terminal: Option<TurnStreamTerminalSnapshot>,
    pub update: TurnStreamUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamEvent {
    AttachmentObserved {
        profile: TerminalBridgeAttachmentProfile,
    },
    ThreadPrepared {
        thread_id: String,
        title: String,
        cwd: String,
    },
    TurnStarted {
        turn_id: String,
    },
    StatusUpdated {
        text: String,
    },
    AgentMessageDelta {
        item_id: String,
        phase: Option<String>,
        delta: String,
    },
    AgentMessageCompleted {
        item_id: String,
        phase: Option<String>,
        text: String,
    },
    ToolActivity {
        activity: ConversationToolActivity,
    },
    ApprovalReviewUpdated {
        review: ConversationApprovalReview,
    },
    ApprovalRequested {
        request: ConversationApprovalRequest,
    },
    ApprovalResolved {
        approval_id: String,
        resolution: ConversationApprovalResolution,
    },
    TurnInterruptRequestFailed {
        message: String,
    },
    TurnRetrying {
        thread_id: String,
        turn_id: String,
        error: ConversationTurnError,
    },
    TurnTerminal {
        receipt: ConversationTurnTerminalReceipt,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamTerminalSnapshot {
    Turn {
        receipt: Box<ConversationTurnTerminalReceipt>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamUpdate {
    AttachmentObserved {
        profile: TerminalBridgeAttachmentProfile,
    },
    ThreadPrepared {
        thread_id: String,
        title: String,
        cwd: String,
        status_text: String,
    },
    TurnStarted {
        turn_id: String,
        status_text: String,
    },
    StatusUpdated {
        text: String,
    },
    AgentMessageDelta {
        item_id: String,
        phase: Option<String>,
        delta: String,
    },
    AgentMessageCompleted {
        item_id: String,
        phase: Option<String>,
        text: String,
    },
    ToolActivity {
        activity: ConversationToolActivity,
    },
    ApprovalReviewUpdated {
        review: ConversationApprovalReview,
    },
    ApprovalRequested {
        request: ConversationApprovalRequest,
    },
    ApprovalResolved {
        approval_id: String,
        resolution: ConversationApprovalResolution,
    },
    TurnInterruptRequestFailed {
        message: String,
    },
    TurnRetrying {
        thread_id: String,
        turn_id: String,
        error: ConversationTurnError,
        correlation_failure: Option<TurnStreamTerminalRejection>,
        status_text: String,
    },
    TurnCompleted {
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
        status_text: String,
    },
    TurnTerminal {
        receipt: Box<ConversationTurnTerminalReceipt>,
        execution_snapshot_capture: Option<TurnSnapshotCapture>,
        status_text: String,
    },
    TurnTerminalIgnored {
        receipt: Box<ConversationTurnTerminalReceipt>,
        reason: TurnStreamTerminalRejection,
    },
    Failed {
        message: String,
        status_text: String,
    },
    RuntimeFailureIgnored {
        message: String,
    },
    RuntimeNotice {
        notice: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnStreamTerminalRejection {
    ThreadMismatch { expected_thread_id: Option<String> },
    TurnMismatch { expected_turn_id: Option<String> },
    TerminalAlreadyApplied,
}

fn terminal_status_text(receipt: &ConversationTurnTerminalReceipt) -> &'static str {
    if receipt.application_delivery != ConversationTurnApplicationDelivery::Confirmed {
        return "turn recovery pending";
    }
    match &receipt.outcome {
        ConversationTurnTerminalOutcome::Completed => "turn completed",
        ConversationTurnTerminalOutcome::Interrupted => "turn interrupted",
        ConversationTurnTerminalOutcome::Failed { .. } => "turn failed",
        ConversationTurnTerminalOutcome::Unknown { .. } => "turn outcome unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
        ConversationApprovalResolution, ConversationApprovalReview,
        ConversationApprovalReviewStatus, ConversationToolActivity, ConversationToolActivityKind,
    };
    use crate::domain::planning::{ExecutionSnapshot, TurnSnapshotCapture};
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDeliveryFailure, ConversationTurnTerminalUncertainty,
    };

    fn prepared_turn_state() -> TurnStreamState {
        let mut state = TurnStreamState::new();
        state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-1".to_string(),
            title: "Core stream".to_string(),
            cwd: "/tmp/workspace".to_string(),
        });
        state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
        });
        state
    }

    fn confirmed_receipt(
        outcome: ConversationTurnTerminalOutcome,
    ) -> ConversationTurnTerminalReceipt {
        ConversationTurnTerminalReceipt::new("thread-1", "turn-1", outcome)
            .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed)
    }

    #[test]
    fn thread_prepared_updates_stream_identity() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(TurnStreamEvent::ThreadPrepared {
            thread_id: "thread-1".to_string(),
            title: "Core stream".to_string(),
            cwd: "/tmp/workspace".to_string(),
        });

        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(snapshot.title.as_deref(), Some("Core stream"));
        assert_eq!(snapshot.cwd.as_deref(), Some("/tmp/workspace"));
        assert_eq!(snapshot.status_text.as_deref(), Some("thread started"));
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Core stream".to_string(),
                cwd: "/tmp/workspace".to_string(),
                status_text: "thread started".to_string(),
            }
        );
    }

    #[test]
    fn loaded_thread_identity_is_seeded_without_emitting_a_stream_revision() {
        let mut state = TurnStreamState::new();

        state.seed_loaded_thread_identity("thread-loaded", "Loaded thread", "/tmp/loaded");
        let snapshot = state.apply_runtime_notice("reattached".to_string());

        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.thread_id.as_deref(), Some("thread-loaded"));
        assert_eq!(snapshot.title.as_deref(), Some("Loaded thread"));
        assert_eq!(snapshot.cwd.as_deref(), Some("/tmp/loaded"));
        assert_eq!(snapshot.status_text, None);
    }

    #[test]
    fn turn_started_records_active_turn() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
        });

        assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.status_text.as_deref(), Some("turn started"));
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnStarted {
                turn_id: "turn-1".to_string(),
                status_text: "turn started".to_string(),
            }
        );
    }

    #[test]
    fn agent_delta_projects_snapshot_update() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(TurnStreamEvent::AgentMessageDelta {
            item_id: "item-1".to_string(),
            phase: Some("output".to_string()),
            delta: "hello".to_string(),
        });

        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::AgentMessageDelta {
                item_id: "item-1".to_string(),
                phase: Some("output".to_string()),
                delta: "hello".to_string(),
            }
        );
    }

    #[test]
    fn completed_message_projects_snapshot_update() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(TurnStreamEvent::AgentMessageCompleted {
            item_id: "item-1".to_string(),
            phase: None,
            text: "final answer".to_string(),
        });

        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::AgentMessageCompleted {
                item_id: "item-1".to_string(),
                phase: None,
                text: "final answer".to_string(),
            }
        );
    }

    #[test]
    fn default_state_projects_attachment_status_and_runtime_notice_updates() {
        let mut state = TurnStreamState::default();
        let profile = TerminalBridgeAttachmentProfile::default();

        let attachment = state.apply_stream_event(TurnStreamEvent::AttachmentObserved { profile });
        assert_eq!(attachment.revision, 1);
        assert_eq!(
            attachment.update,
            TurnStreamUpdate::AttachmentObserved { profile }
        );

        let status = state.apply_stream_event(TurnStreamEvent::StatusUpdated {
            text: "running tools".to_string(),
        });
        assert_eq!(status.status_text.as_deref(), Some("running tools"));
        assert_eq!(
            status.update,
            TurnStreamUpdate::StatusUpdated {
                text: "running tools".to_string(),
            }
        );

        let notice = state.apply_runtime_notice("runtime reattached".to_string());
        assert_eq!(notice.status_text.as_deref(), Some("running tools"));
        assert_eq!(
            notice.update,
            TurnStreamUpdate::RuntimeNotice {
                notice: "runtime reattached".to_string(),
            }
        );
    }

    #[test]
    fn tool_activity_projects_domain_activity() {
        let mut state = TurnStreamState::new();
        let activity = ConversationToolActivity {
            kind: ConversationToolActivityKind::FileChange,
            text: "edited src/lib.rs".to_string(),
            file_change_count: 1,
        };

        let snapshot = state.apply_stream_event(TurnStreamEvent::ToolActivity {
            activity: activity.clone(),
        });

        assert_eq!(snapshot.update, TurnStreamUpdate::ToolActivity { activity });
    }

    #[test]
    fn approval_review_projects_domain_review() {
        let mut state = TurnStreamState::new();
        let review = ConversationApprovalReview {
            target_item_id: "review-1".to_string(),
            status: ConversationApprovalReviewStatus::InProgress,
            risk_level: Some("medium".to_string()),
            rationale: Some("approve command".to_string()),
        };

        let snapshot = state.apply_stream_event(TurnStreamEvent::ApprovalReviewUpdated {
            review: review.clone(),
        });

        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::ApprovalReviewUpdated { review }
        );
    }

    #[test]
    fn approval_request_and_resolution_cross_the_core_stream_unchanged() {
        let mut state = TurnStreamState::new();
        let request = ConversationApprovalRequest {
            approval_id: "approval-core".to_string(),
            server_request_id: "server-core".to_string(),
            method: "item/fileChange/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::FileChange,
            summary: "File changes requested.".to_string(),
            details: vec!["Reason: update tests".to_string()],
        };

        let requested = state.apply_stream_event(TurnStreamEvent::ApprovalRequested {
            request: request.clone(),
        });
        assert_eq!(
            requested.update,
            TurnStreamUpdate::ApprovalRequested { request }
        );
        assert_eq!(requested.status_text.as_deref(), Some("approval required"));

        let resolved = state.apply_stream_event(TurnStreamEvent::ApprovalResolved {
            approval_id: "approval-core".to_string(),
            resolution: ConversationApprovalResolution::Declined,
        });
        assert!(matches!(
            resolved.update,
            TurnStreamUpdate::ApprovalResolved {
                resolution: ConversationApprovalResolution::Declined,
                ..
            }
        ));
    }

    #[test]
    fn confirmed_completed_receipt_projects_completed_update() {
        let mut state = prepared_turn_state();
        let receipt = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            vec!["docs/plan.md".to_string()],
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: receipt.clone(),
            execution_snapshot_capture: None,
        });

        assert_eq!(snapshot.active_turn_id, None);
        assert_eq!(snapshot.status_text.as_deref(), Some("turn completed"));
        assert_eq!(
            snapshot.terminal,
            Some(TurnStreamTerminalSnapshot::Turn {
                receipt: Box::new(receipt)
            })
        );
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec!["docs/plan.md".to_string()],
                execution_snapshot_capture: None,
                status_text: "turn completed".to_string(),
            }
        );
    }

    #[test]
    fn turn_completed_records_terminal_snapshot_with_execution_capture() {
        let mut state = TurnStreamState::new();
        let execution_snapshot_capture =
            TurnSnapshotCapture::ready("/tmp/workspace", ExecutionSnapshot::default());

        let snapshot = state.apply_turn_completed(
            "turn-1".to_string(),
            vec!["docs/plan.md".to_string()],
            execution_snapshot_capture.clone(),
        );

        assert_eq!(snapshot.active_turn_id, None);
        assert_eq!(snapshot.status_text.as_deref(), Some("turn completed"));
        assert_eq!(
            snapshot.terminal,
            Some(TurnStreamTerminalSnapshot::Turn {
                receipt: Box::new(
                    ConversationTurnTerminalReceipt::completed(
                        "",
                        "turn-1",
                        vec!["docs/plan.md".to_string()]
                    )
                    .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed)
                )
            })
        );
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec!["docs/plan.md".to_string()],
                execution_snapshot_capture: Some(execution_snapshot_capture),
                status_text: "turn completed".to_string(),
            }
        );
    }

    #[test]
    fn non_completed_and_unconfirmed_receipts_stay_non_completed_updates() {
        let failed_error = ConversationTurnError::new("model failed", None::<&str>, None);
        let receipts = vec![
            confirmed_receipt(ConversationTurnTerminalOutcome::Interrupted),
            confirmed_receipt(ConversationTurnTerminalOutcome::Failed {
                error: failed_error.clone(),
            }),
            confirmed_receipt(ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::NonRetryErrorGraceExpired,
                observed_error: Some(failed_error),
            }),
            ConversationTurnTerminalReceipt::completed("thread-1", "turn-1", Vec::new())
                .with_application_delivery(ConversationTurnApplicationDelivery::Unconfirmed(
                    ConversationTurnApplicationDeliveryFailure::Disconnected,
                )),
        ];

        for receipt in receipts {
            let mut state = prepared_turn_state();
            let snapshot = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
                receipt: receipt.clone(),
                execution_snapshot_capture: None,
            });

            assert!(matches!(
                snapshot.update,
                TurnStreamUpdate::TurnTerminal {
                    receipt: projected,
                    ..
                } if projected.as_ref() == &receipt
            ));
            assert_eq!(
                snapshot.terminal,
                Some(TurnStreamTerminalSnapshot::Turn {
                    receipt: Box::new(receipt)
                })
            );
        }
    }

    #[test]
    fn terminal_receipt_must_match_active_thread_and_turn() {
        let mut state = prepared_turn_state();
        let receipt =
            ConversationTurnTerminalReceipt::completed("thread-other", "turn-1", Vec::new())
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: receipt.clone(),
            execution_snapshot_capture: None,
        });

        assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.terminal, None);
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(receipt),
                reason: TurnStreamTerminalRejection::ThreadMismatch {
                    expected_thread_id: Some("thread-1".to_string())
                }
            }
        );
    }

    #[test]
    fn terminal_receipt_for_another_turn_keeps_the_active_turn_running() {
        let mut state = prepared_turn_state();
        let receipt =
            ConversationTurnTerminalReceipt::completed("thread-1", "turn-other", Vec::new())
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: receipt.clone(),
            execution_snapshot_capture: None,
        });

        assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.terminal, None);
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(receipt),
                reason: TurnStreamTerminalRejection::TurnMismatch {
                    expected_turn_id: Some("turn-1".to_string())
                }
            }
        );
    }

    #[test]
    fn first_terminal_receipt_cannot_be_promoted_by_later_observations() {
        let mut state = prepared_turn_state();
        let interrupted = confirmed_receipt(ConversationTurnTerminalOutcome::Interrupted);
        state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: interrupted.clone(),
            execution_snapshot_capture: None,
        });
        let later_completed = ConversationTurnTerminalReceipt::completed(
            "thread-1",
            "turn-1",
            vec!["docs/plan.md".to_string()],
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);

        let duplicate = state.apply_stream_event(TurnStreamEvent::TurnTerminal {
            receipt: later_completed.clone(),
            execution_snapshot_capture: None,
        });

        assert_eq!(
            duplicate.terminal,
            Some(TurnStreamTerminalSnapshot::Turn {
                receipt: Box::new(interrupted)
            })
        );
        assert_eq!(
            duplicate.update,
            TurnStreamUpdate::TurnTerminalIgnored {
                receipt: Box::new(later_completed),
                reason: TurnStreamTerminalRejection::TerminalAlreadyApplied
            }
        );
    }

    #[test]
    fn retry_update_records_correlation_without_ending_the_turn() {
        let mut state = prepared_turn_state();
        let error = ConversationTurnError::new("server overloaded", None::<&str>, None);

        let snapshot = state.apply_stream_event(TurnStreamEvent::TurnRetrying {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            error: error.clone(),
        });

        assert_eq!(snapshot.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.terminal, None);
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::TurnRetrying {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                error,
                correlation_failure: None,
                status_text: "turn retrying".to_string()
            }
        );
    }

    #[test]
    fn failed_records_terminal_snapshot() {
        let mut state = TurnStreamState::new();
        state.apply_stream_event(TurnStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
        });

        let snapshot = state.apply_stream_event(TurnStreamEvent::Failed {
            message: "transport closed".to_string(),
        });

        assert_eq!(snapshot.active_turn_id, None);
        assert_eq!(snapshot.status_text.as_deref(), Some("turn failed"));
        assert_eq!(
            snapshot.terminal,
            Some(TurnStreamTerminalSnapshot::Failed {
                message: "transport closed".to_string()
            })
        );
        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::Failed {
                message: "transport closed".to_string(),
                status_text: "turn failed".to_string(),
            }
        );
    }
}
