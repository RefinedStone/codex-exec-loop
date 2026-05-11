use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::PlanningTurnExecutionSnapshotCapture;
use crate::application::service::post_turn_evaluation::PostTurnEvaluationExecution;
use crate::domain::conversation::{ConversationApprovalReview, ConversationToolActivity};
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

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

    pub fn apply_stream_event(&mut self, event: ConversationStreamEvent) -> TurnStreamSnapshot {
        let update = match event {
            ConversationStreamEvent::AttachmentObserved { profile } => {
                TurnStreamUpdate::AttachmentObserved { profile }
            }
            ConversationStreamEvent::ThreadPrepared {
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
            ConversationStreamEvent::TurnStarted { turn_id } => {
                self.active_turn_id = Some(turn_id.clone());
                self.terminal = None;
                self.last_applied_post_turn_evaluation_id = None;
                self.status_text = Some("turn started".to_string());
                TurnStreamUpdate::TurnStarted {
                    turn_id,
                    status_text: "turn started".to_string(),
                }
            }
            ConversationStreamEvent::StatusUpdated { text } => {
                self.status_text = Some(text.clone());
                TurnStreamUpdate::StatusUpdated { text }
            }
            ConversationStreamEvent::AgentMessageDelta {
                item_id,
                phase,
                delta,
            } => TurnStreamUpdate::AgentMessageDelta {
                item_id,
                phase,
                delta,
            },
            ConversationStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
            } => TurnStreamUpdate::AgentMessageCompleted {
                item_id,
                phase,
                text,
            },
            ConversationStreamEvent::ToolActivity { activity } => {
                TurnStreamUpdate::ToolActivity { activity }
            }
            ConversationStreamEvent::ApprovalReviewUpdated { review } => {
                TurnStreamUpdate::ApprovalReviewUpdated { review }
            }
            ConversationStreamEvent::TurnCompleted {
                turn_id,
                changed_planning_file_paths,
            } => self.turn_completed_update(turn_id, changed_planning_file_paths, None),
            ConversationStreamEvent::Failed { message } => {
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
        };

        self.snapshot(update)
    }

    pub fn apply_turn_completed(
        &mut self,
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: PlanningTurnExecutionSnapshotCapture,
    ) -> TurnStreamSnapshot {
        let update = self.turn_completed_update(
            turn_id,
            changed_planning_file_paths,
            Some(execution_snapshot_capture),
        );
        self.snapshot(update)
    }

    pub fn apply_runtime_notice(&mut self, notice: String) -> TurnStreamSnapshot {
        self.snapshot(TurnStreamUpdate::RuntimeNotice { notice })
    }

    pub fn accept_post_turn_evaluation_completion(
        &mut self,
        execution: &PostTurnEvaluationExecution,
    ) -> bool {
        if !self.post_turn_evaluation_matches_latest_completed_turn(execution) {
            return false;
        }
        self.last_applied_post_turn_evaluation_id = Some(execution.completed_turn_id.clone());
        true
    }

    fn post_turn_evaluation_matches_latest_completed_turn(
        &self,
        execution: &PostTurnEvaluationExecution,
    ) -> bool {
        self.thread_id.as_deref() == Some(execution.thread_id.as_str())
            && self.active_turn_id.is_none()
            && self.last_applied_post_turn_evaluation_id.as_deref()
                != Some(execution.completed_turn_id.as_str())
            && matches!(
                &self.terminal,
                Some(TurnStreamTerminalSnapshot::Completed { turn_id, .. })
                    if turn_id == &execution.completed_turn_id
            )
    }

    fn turn_completed_update(
        &mut self,
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: Option<PlanningTurnExecutionSnapshotCapture>,
    ) -> TurnStreamUpdate {
        self.active_turn_id = None;
        self.status_text = Some("turn completed".to_string());
        self.terminal = Some(TurnStreamTerminalSnapshot::Completed {
            turn_id: turn_id.clone(),
            changed_planning_file_paths: changed_planning_file_paths.clone(),
        });
        TurnStreamUpdate::TurnCompleted {
            turn_id,
            changed_planning_file_paths,
            execution_snapshot_capture,
            status_text: "turn completed".to_string(),
        }
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
pub enum TurnStreamTerminalSnapshot {
    Completed {
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
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
    TurnCompleted {
        turn_id: String,
        changed_planning_file_paths: Vec<String>,
        execution_snapshot_capture: Option<PlanningTurnExecutionSnapshotCapture>,
        status_text: String,
    },
    Failed {
        message: String,
        status_text: String,
    },
    RuntimeNotice {
        notice: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::service::planning::{
        PlanningExecutionSnapshot, PlanningTurnExecutionSnapshotCapture,
    };
    use crate::domain::conversation::{
        ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationToolActivity,
        ConversationToolActivityKind,
    };

    #[test]
    fn thread_prepared_updates_stream_identity() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(ConversationStreamEvent::ThreadPrepared {
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
    fn turn_started_records_active_turn() {
        let mut state = TurnStreamState::new();

        let snapshot = state.apply_stream_event(ConversationStreamEvent::TurnStarted {
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

        let snapshot = state.apply_stream_event(ConversationStreamEvent::AgentMessageDelta {
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

        let snapshot = state.apply_stream_event(ConversationStreamEvent::AgentMessageCompleted {
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
    fn tool_activity_projects_domain_activity() {
        let mut state = TurnStreamState::new();
        let activity = ConversationToolActivity {
            kind: ConversationToolActivityKind::FileChange,
            text: "edited src/lib.rs".to_string(),
            file_change_count: 1,
        };

        let snapshot = state.apply_stream_event(ConversationStreamEvent::ToolActivity {
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

        let snapshot = state.apply_stream_event(ConversationStreamEvent::ApprovalReviewUpdated {
            review: review.clone(),
        });

        assert_eq!(
            snapshot.update,
            TurnStreamUpdate::ApprovalReviewUpdated { review }
        );
    }

    #[test]
    fn turn_completed_records_terminal_snapshot_with_execution_capture() {
        let mut state = TurnStreamState::new();
        let execution_snapshot_capture = PlanningTurnExecutionSnapshotCapture::ready(
            "/tmp/workspace",
            PlanningExecutionSnapshot::default(),
        );

        let snapshot = state.apply_turn_completed(
            "turn-1".to_string(),
            vec!["docs/plan.md".to_string()],
            execution_snapshot_capture.clone(),
        );

        assert_eq!(snapshot.active_turn_id, None);
        assert_eq!(snapshot.status_text.as_deref(), Some("turn completed"));
        assert_eq!(
            snapshot.terminal,
            Some(TurnStreamTerminalSnapshot::Completed {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec!["docs/plan.md".to_string()]
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
    fn failed_records_terminal_snapshot() {
        let mut state = TurnStreamState::new();
        state.apply_stream_event(ConversationStreamEvent::TurnStarted {
            turn_id: "turn-1".to_string(),
        });

        let snapshot = state.apply_stream_event(ConversationStreamEvent::Failed {
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
