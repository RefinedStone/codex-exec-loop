use super::*;
use crate::application::service::issue_interactions::IssueInteractionsService;
use crate::domain::issue_automation::interaction::{
    IssueActionCommand, IssueConversationTarget, IssueDecisionResolutionCode,
};

fn project(
    h: &Harness,
) -> crate::domain::issue_automation::interaction::IssueInteractionProjection {
    let service = IssueInteractionsService::new(
        h.fixture.workspace.clone(),
        h.fixture.store(),
        h.execution.clone(),
        h.triage.clone(),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    );
    service.project(&page(&h.triage).items[0]).unwrap()
}

#[test]
fn detail_projects_answered_history_and_shared_draft_without_any_worker_action() {
    let h = Harness::new();
    h.tick(false).unwrap();
    let request_id = h.decision().request.id().clone();
    h.answer("reject");
    let state = project(&h);
    assert_eq!(state.decision_history.len(), 1);
    assert_eq!(state.decision_history[0].decision.decision_id, request_id);
    assert_eq!(
        state.decision_history[0].resolution,
        IssueDecisionResolutionCode::Answered
    );
    assert!(matches!(
        state.conversation,
        IssueConversationTarget::Draft { .. }
    ));
    assert_eq!(state.actions.len(), 8);
    assert!(
        !state
            .actions
            .iter()
            .find(|action| action.command == IssueActionCommand::SteerIssue)
            .unwrap()
            .enabled
    );
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    assert!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control_commands
            .is_empty()
    );
    assert!(h.execution.state.lock().unwrap().grants.is_empty());
    assert_eq!(h.source.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn stale_disclosure_keeps_resolution_metadata_but_hides_history_text_and_draft() {
    let h = Harness::new();
    h.tick(false).unwrap();
    h.answer("reject");
    let mut observation = h.fixture.observation("I_001");
    let IssueSourceContent::Shared(snapshot) = &mut observation.content else {
        unreachable!()
    };
    snapshot.policy_version = "b".repeat(64);
    h.fixture.seed(vec![observation], &"b".repeat(64));
    let state = project(&h);
    assert_eq!(state.decision_history.len(), 1);
    assert_eq!(
        state.decision_history[0].resolution,
        IssueDecisionResolutionCode::Answered
    );
    assert!(state.decision_history[0].request.is_none());
    assert_eq!(state.conversation, IssueConversationTarget::Unavailable);
    assert!(
        h.execution
            .state
            .lock()
            .unwrap()
            .control_commands
            .is_empty()
    );
}

#[test]
fn history_query_is_bounded_at_twenty() {
    let h = Harness::new();
    let store = h.fixture.store();
    let issue = h.fixture.reference("I_001");
    assert!(
        store
            .closed_issue_decisions(&h.fixture.workspace, &issue, 0)
            .is_err()
    );
    assert!(
        store
            .closed_issue_decisions(&h.fixture.workspace, &issue, 21)
            .is_err()
    );
    assert!(
        store
            .closed_issue_decisions(&h.fixture.workspace, &issue, 20)
            .unwrap()
            .is_empty()
    );
}
