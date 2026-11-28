use super::*;
use crate::adapter::inbound::kernel_protocol::AttentionCause;
use crate::composition::attention::stored_items;

#[test]
fn attention_requires_an_active_wait_and_disappears_when_it_is_rearmed() {
    for (reason, cause) in [
        (
            TaskDeliveryEvidenceReason::EvidenceStale,
            AttentionCause::EvidenceStale,
        ),
        (
            TaskDeliveryEvidenceReason::CriteriaNotMet,
            AttentionCause::CriteriaNotMet,
        ),
    ] {
        let f = DeliveryFixture::new(&format!("attention-{}", reason.label()));
        f.claim();
        let mut blocked = f.observation(TaskEvidenceDeliveryMode::Enforce);
        blocked.reasons = vec![reason];
        for criterion in &mut blocked.criteria {
            criterion.reason = Some(reason);
        }
        assert!(!f.commit(blocked, f.effect()).unwrap().effect_allowed);
        let items = stored_items(&f.base.workspace, vec![]).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].cause, cause);
        assert_eq!(items[0].subject.id, f.base.lease.task_id);
        f.release();
        f.commit(f.waive_all(), TaskDeliveryEvidenceOperation::Recheck)
            .unwrap();
        assert!(stored_items(&f.base.workspace, vec![]).unwrap().is_empty());
    }
}
