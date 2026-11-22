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

impl crate::domain::kernel_event::KernelEvent {
    pub fn issue_feedback_posted(
        event: &crate::domain::issue_automation::feedback::IssueFeedbackEvent,
    ) -> Self {
        use crate::domain::issue_automation::feedback::{
            IssueFeedbackAction, IssueFeedbackEvidence,
        };
        let action = match event.action {
            IssueFeedbackAction::CommentCreated => IssueFeedbackRecordAction::CommentCreated,
            IssueFeedbackAction::CommentUpdated => IssueFeedbackRecordAction::CommentUpdated,
            IssueFeedbackAction::LabelAdded => IssueFeedbackRecordAction::LabelAdded,
            IssueFeedbackAction::LabelRemoved => IssueFeedbackRecordAction::LabelRemoved,
        };
        let (comment_id, label_id) = match &event.receipt.evidence {
            IssueFeedbackEvidence::Comment { comment_id, .. } => (Some(comment_id.clone()), None),
            IssueFeedbackEvidence::Label { label, .. } => (None, Some(label.id.clone())),
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
}
