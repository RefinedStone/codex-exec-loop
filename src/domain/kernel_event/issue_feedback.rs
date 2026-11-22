//! 피드백 감사 기록은 공유본 자체를 포함하지 않는다.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueFeedbackRecordAction {
    CommentCreated,
    CommentUpdated,
    LabelAdded,
    LabelRemoved,
}

/// provider_observation은 사람/PR 연결 등 실제 닫힘 원인을 추정하지 않는다.
/// 응답 유실 뒤 전송 확인이 도착하면 서로 다른 origin의 두 증거가 남을 수 있다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueClosureRecordOrigin {
    ConfirmedFeedback,
    ProviderObservation,
}

impl crate::domain::kernel_event::KernelEvent {
    pub fn issue_feedback_posted(
        event: &crate::domain::issue_automation::feedback::IssueFeedbackEvent,
    ) -> Self {
        use crate::domain::issue_automation::feedback::{
            IssueFeedbackAction, IssueFeedbackEvidence,
        };
        // The durable outbox verifies the operation, action and receipt before publishing.
        let (action, comment_id, label_id) = match &event.receipt.evidence {
            IssueFeedbackEvidence::Closed {
                event_id: closure_event_id,
            } => {
                return Self::IssueClosed {
                    event_id: event.event_id.clone(),
                    issue: (&event.issue).into(),
                    origin: IssueClosureRecordOrigin::ConfirmedFeedback,
                    intent_id: Some(event.intent_id.clone()),
                    actor_id: Some(event.receipt.actor_id.clone()),
                    closure_event_id: Some(closure_event_id.clone()),
                    observation_revision: None,
                    revision: event.revision,
                };
            }
            IssueFeedbackEvidence::Comment { comment_id, .. } => (
                if event.action == IssueFeedbackAction::CommentCreated {
                    IssueFeedbackRecordAction::CommentCreated
                } else {
                    IssueFeedbackRecordAction::CommentUpdated
                },
                Some(comment_id.clone()),
                None,
            ),
            IssueFeedbackEvidence::Label { label, applied, .. } => (
                if *applied {
                    IssueFeedbackRecordAction::LabelAdded
                } else {
                    IssueFeedbackRecordAction::LabelRemoved
                },
                None,
                Some(label.id.clone()),
            ),
        };
        Self::IssueFeedbackPosted {
            event_id: event.event_id.clone(),
            issue: (&event.issue).into(),
            intent_id: event.intent_id.clone(),
            action,
            actor_id: event.receipt.actor_id.clone(),
            comment_id,
            label_id,
            revision: event.revision,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};
    use crate::domain::kernel_event::KernelEvent;

    #[test]
    fn feedback_record_keeps_only_confirmed_metadata_and_the_durable_outbox_id() {
        let issue = IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_stable".into(),
            issue_id: "I_stable".into(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".into(),
            },
        };
        let event = KernelEvent::IssueFeedbackPosted {
            event_id: "feedback-event-1".into(),
            issue: (&issue).into(),
            intent_id: "feedback-intent-1".into(),
            action: IssueFeedbackRecordAction::CommentCreated,
            actor_id: "actor-stable".into(),
            comment_id: Some("comment-stable".into()),
            label_id: None,
            revision: 1,
        };
        assert_eq!(event.kind(), "issue_feedback_posted");
        assert_eq!(event.issue_work_event_id(), Some("feedback-event-1"));
        assert_eq!(event.thread_id(), "");
        assert_eq!(event.op_id(), None);
        let wire = serde_json::to_value(&event).unwrap();
        for field in ["title", "body", "question", "answer", "output", "token"] {
            assert!(wire.get(field).is_none());
        }
        assert_eq!(wire["actor_id"], "actor-stable");
        assert_eq!(serde_json::from_value::<KernelEvent>(wire).unwrap(), event);
    }

    #[test]
    fn confirmed_closure_has_its_own_event_kind_and_the_same_outbox_identity() {
        use crate::domain::issue_automation::feedback::{
            IssueFeedbackAction, IssueFeedbackEvent, IssueFeedbackEvidence, IssueFeedbackReceipt,
        };
        let issue = IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_stable".into(),
            issue_id: "I_stable".into(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".into(),
            },
        };
        let posted = IssueFeedbackEvent {
            event_id: "issue-feedback:close-1".into(),
            issue: issue.clone(),
            intent_id: "intent-close-1".into(),
            action: IssueFeedbackAction::IssueClosed,
            receipt: IssueFeedbackReceipt {
                issue,
                intent_id: "intent-close-1".into(),
                actor_id: "actor-stable".into(),
                evidence: IssueFeedbackEvidence::Closed {
                    event_id: "closure-provider-1".into(),
                },
            },
            revision: 4,
        };
        let event = KernelEvent::issue_feedback_posted(&posted);
        assert_eq!(event.kind(), "issue_closed");
        assert_eq!(event.issue_work_event_id(), Some(posted.event_id.as_str()));
        assert_eq!(event.thread_id(), "");
        assert_eq!(event.op_id(), None);
        assert_eq!(event.operation_step(), None);
        let wire = serde_json::to_value(&event).unwrap();
        assert_eq!(wire["origin"], "confirmed_feedback");
        assert_eq!(wire["closure_event_id"], "closure-provider-1");
        assert_eq!(wire["intent_id"], "intent-close-1");
        assert!(wire.get("integration").is_none());
        assert!(wire.get("body").is_none());
        assert_eq!(serde_json::from_value::<KernelEvent>(wire).unwrap(), event);
    }

    #[test]
    fn provider_closure_observation_does_not_invent_an_actor_or_feedback_intent() {
        let issue = IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_stable".into(),
            issue_id: "I_stable".into(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".into(),
            },
        };
        let event = KernelEvent::IssueClosed {
            event_id: "issue-event-observed-close".into(),
            issue: (&issue).into(),
            origin: IssueClosureRecordOrigin::ProviderObservation,
            intent_id: None,
            actor_id: None,
            closure_event_id: None,
            observation_revision: Some(9),
            revision: 9,
        };
        let wire = serde_json::to_value(&event).unwrap();
        assert_eq!(wire["kind"], "issue_closed");
        assert_eq!(wire["origin"], "provider_observation");
        assert_eq!(wire["observation_revision"], 9);
        for absent in [
            "intent_id",
            "actor_id",
            "closure_event_id",
            "integration",
            "reason",
        ] {
            assert!(wire.get(absent).is_none());
        }
        assert_eq!(
            event.issue_work_event_id(),
            Some("issue-event-observed-close")
        );
        assert_eq!(serde_json::from_value::<KernelEvent>(wire).unwrap(), event);
    }
}
