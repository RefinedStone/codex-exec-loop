use super::super::readiness::{assessed, current_text, value, wire};
use super::*;
use crate::application::port::inbound::issue_draft_port::{
    IssueDraftPreviewRequest, IssueDraftReadiness, IssueDraftReadinessBlocker,
};

#[test]
fn creation_and_update_preview_have_the_same_preparation_for_the_same_current_text() {
    let mut fixture = CreationFixture::new(false);
    *fixture.files.policy.lock().unwrap() = String::new();
    let (service, source) = wire(fixture.service, "");
    fixture.service = service;
    let mut creation_target = target();
    creation_target.cwd = "/work/project-linked".into();
    let opened = fixture
        .service
        .creation_open(creation_target.clone())
        .unwrap();
    let text = current_text(
        "## 완료 기준\n- 현재 문장을 확인한다.\n\npassword=synthetic-draft-private-value",
    );
    let labels = vec!["docs".to_owned()];
    let created = fixture
        .service
        .creation_preview(IssueCreationPreviewRequest {
            target: creation_target,
            intent_id: opened.intent.intent_id,
            text: text.clone(),
            expected_labels: labels.clone(),
        })
        .unwrap();
    let existing = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: super::super::target(),
            text,
            expected_labels: labels,
        })
        .unwrap();
    assert_eq!(value(&created.preview), value(&existing));
    assert!(assessed(&created.preview).preview_only);
    assert!(created.approval.is_some());
    assert!(
        !serde_json::to_string(value(&created.preview))
            .unwrap()
            .contains("synthetic-draft-private-value")
    );
    assert_eq!(
        *source.workspaces.lock().unwrap(),
        vec!["/work/project", "/work/project"]
    );
    assert_eq!(fixture.creates(), 0);
    assert!(fixture.remote.updates.lock().unwrap().is_empty());
    assert!(fixture.remote.reads.lock().unwrap().is_empty());
    assert!(fixture.vault.staged.lock().unwrap().is_empty());
}

#[test]
fn a_frozen_creation_copy_never_substitutes_for_current_draft_readiness() {
    for state in [
        IssueCreationIntentState::Held,
        IssueCreationIntentState::Recorded,
    ] {
        let mut fixture = CreationFixture::new(false);
        fixture.service = wire(fixture.service, "").0;
        let (_, request) = freeze(&fixture, state);
        let preview = fixture
            .service
            .creation_preview(IssueCreationPreviewRequest {
                target: target(),
                intent_id: request.intent_id.clone(),
                text: current_text("## 완료 기준\n- 새로 보낸 유효한 문장이다."),
                expected_labels: vec!["akra:auto".into()],
            })
            .unwrap();
        assert!(matches!(
            value(&preview.preview),
            IssueDraftReadiness::CannotAssess {
                reason: IssueDraftReadinessBlocker::FrozenDraft,
                ..
            }
        ));
        assert_eq!(preview.approval, Some(request.approval));
        assert_eq!(fixture.creates(), 0);
    }
}

#[test]
fn malformed_automation_policy_does_not_block_creation_or_its_result_recovery() {
    for lose_answer in [false, true] {
        let mut fixture = CreationFixture::new(lose_answer);
        *fixture.files.policy.lock().unwrap() = String::new();
        let (service, source) = wire(fixture.service, "private-automation-canary = [");
        fixture.service = service;
        let opened = fixture.service.creation_open(target()).unwrap();
        let text = current_text("## 완료 기준\n- 생성 결과를 확인한다.");
        let preview = fixture
            .service
            .creation_preview(IssueCreationPreviewRequest {
                target: target(),
                intent_id: opened.intent.intent_id.clone(),
                text: text.clone(),
                expected_labels: Vec::new(),
            })
            .unwrap();
        assert!(matches!(
            value(&preview.preview),
            IssueDraftReadiness::CannotAssess {
                reason: IssueDraftReadinessBlocker::PolicyBlocked,
                ..
            }
        ));
        assert!(
            !serde_json::to_string(&preview)
                .unwrap()
                .contains("private-automation-canary")
        );
        let lookups = source.workspaces.lock().unwrap().len();
        let published = fixture
            .service
            .creation_publish(IssueCreationPublishRequest {
                target: target(),
                intent_id: opened.intent.intent_id.clone(),
                text,
                approval: preview.approval.expect("Publish still has an approval"),
                shared_copy_only: false,
                validation_id: None,
            })
            .unwrap();
        assert!(matches!(
            published,
            IssueCreationPublished::Confirmed { .. }
                | IssueCreationPublished::NeedsConfirmation { .. }
        ));
        let recovered = fixture.service.creation_open(target()).unwrap();
        assert_eq!(recovered.intent.intent_id, opened.intent.intent_id);
        fixture
            .service
            .creation_recheck(
                crate::application::port::inbound::issue_draft_port::IssueCreationIntentRequest {
                    target: target(),
                    intent_id: opened.intent.intent_id,
                },
            )
            .unwrap();
        assert_eq!(fixture.creates(), 1);
        assert_eq!(
            source.workspaces.lock().unwrap().len(),
            lookups,
            "publication and recovery do not consult the automation policy"
        );
    }
}
