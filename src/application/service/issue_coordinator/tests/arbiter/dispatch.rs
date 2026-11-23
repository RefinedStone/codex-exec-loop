use super::*;
use crate::application::port::outbound::issue_assessment_model_port::{
    IssueAssessmentModelFailure, IssueAssessmentModelStatus,
};

#[test]
fn arbiter_dispatch_failure_consumes_its_claim_and_never_retries() {
    for (failure, reason) in [
        (
            IssueAssessmentModelFailure::Unavailable,
            IssueArbiterWaitReason::ModelUnavailable,
        ),
        (
            IssueAssessmentModelFailure::Uncertain,
            IssueArbiterWaitReason::OutcomeUnknown,
        ),
    ] {
        let a = ArbiterHarness::new();
        a.model
            .set_dispatch_status(IssueAssessmentModelStatus::Unusable(failure));
        let finished = events(&a.track(a.due));
        a.assert_wait(reason);
        assert_eq!(finished.len(), 1);
        let attempt_id = a.pending_attempt();
        assert_eq!(a.model.forgotten(), vec![attempt_id]);
        a.coordinator()
            .acknowledge_event(&finished[0].event_id)
            .unwrap();
        assert!(events(&a.track(a.due + 1)).is_empty());
        assert!(events(&a.track(a.due + 86_400_000)).is_empty());
        assert_eq!(a.model.requests().len(), 1);
        assert!(a.model.observations().is_empty());
        assert!(
            a.h.fixture
                .store()
                .unfinished_issue_arbiter_attempts(&a.h.fixture.workspace, None, 256)
                .unwrap()
                .is_empty()
        );
    }
}
