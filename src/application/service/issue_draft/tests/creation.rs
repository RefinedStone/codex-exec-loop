//! Exercise the public creation facade, including its policy and masking orchestration.
mod support;

use std::sync::atomic::Ordering;

use super::{FAKE_SECRET, ready, text_with_span};
use crate::application::port::inbound::issue_creation_draft::{
    IssueCreationOpened, IssueCreationPreviewRequest, IssueCreationPublishRequest,
    IssueCreationPublished,
};
use crate::application::port::inbound::issue_draft_port::{IssueDraftPort, IssueDraftText};
use crate::domain::issue_creation::IssueCreationIntentState;
use crate::domain::issue_policy::IssueSensitiveSpanClass;
use crate::domain::issue_work::{IssueFieldHash, IssueSharedCopy};
use support::{CreationFixture, target};

fn prepare(
    fixture: &CreationFixture,
    class: IssueSensitiveSpanClass,
) -> (
    IssueCreationOpened,
    IssueSharedCopy,
    IssueCreationPublishRequest,
) {
    let opened = fixture.service.creation_open(target()).unwrap();
    let text = text_with_span(Some(class));
    let preview = fixture
        .service
        .creation_preview(IssueCreationPreviewRequest {
            target: target(),
            intent_id: opened.intent.intent_id.clone(),
            text: text.clone(),
        })
        .unwrap();
    let approval = preview
        .approval
        .expect("a valid prepared draft has an approval");
    let (shared_copy, _, publishable) = ready(preview.preview);
    assert!(publishable);
    let request = IssueCreationPublishRequest {
        target: target(),
        intent_id: opened.intent.intent_id.clone(),
        text,
        approval,
        shared_copy_only: false,
    };
    (opened, shared_copy, request)
}

fn freeze(
    fixture: &CreationFixture,
    state: IssueCreationIntentState,
) -> (IssueSharedCopy, IssueCreationPublishRequest) {
    let (_, shared_copy, request) = prepare(fixture, IssueSensitiveSpanClass::Log);
    if state == IssueCreationIntentState::Held {
        fixture
            .vault
            .lose_stage_acknowledgement
            .store(true, Ordering::SeqCst);
    } else {
        assert_eq!(state, IssueCreationIntentState::Recorded);
        *fixture.journal.fail_before.lock().unwrap() = Some(IssueCreationIntentState::Sent);
    }
    let result = fixture.service.creation_publish(request.clone());
    if state == IssueCreationIntentState::Held {
        assert!(matches!(
            result.unwrap(),
            IssueCreationPublished::Held { .. }
        ));
    } else {
        assert!(result.is_err());
    }
    assert_eq!(
        fixture
            .journal
            .records
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .state,
        state
    );
    assert_eq!(fixture.creates(), 0);
    let staged = fixture.staged();
    assert_eq!(staged.len(), 1);
    assert!(
        staged
            .values()
            .any(|(_, value)| value.expose().contains(FAKE_SECRET))
    );
    (shared_copy, request)
}

fn different_raw_text(mut request: IssueCreationPublishRequest) -> IssueCreationPublishRequest {
    // Invalid ranges and missing required sections would fail if the frozen facade
    // accidentally reevaluated this replacement input instead of the approved shared copy.
    request.text = IssueDraftText {
        title: "Changed unapproved title".to_string(),
        body: "different-private-sentinel".to_string(),
        spans: request.text.spans,
    };
    request.text.spans[0].start = usize::MAX;
    request.text.spans[0].end = usize::MAX;
    request
}

#[test]
fn creation_preview_binds_the_repository_intent_and_masked_copy_before_publication() {
    let mutations: [fn(&mut IssueCreationPublishRequest); 6] = [
        |request| request.approval.repository.repository_id = "R_OTHER".to_string(),
        |request| request.approval.intent_id = "another-intent".to_string(),
        |request| request.approval.shared_copy_hash = IssueFieldHash::of_fields("other", "copy"),
        |request| request.approval.policy_version = "older-policy".to_string(),
        |request| request.approval.template_version = None,
        |request| request.target.owner = "different-owner".to_string(),
    ];
    for change in mutations {
        let fixture = CreationFixture::new(false);
        let (opened, shared_copy, mut request) = prepare(&fixture, IssueSensitiveSpanClass::Secret);
        assert_eq!(request.approval.repository, opened.repository);
        assert_eq!(request.approval.intent_id, opened.intent.intent_id);
        assert_eq!(
            request.approval.shared_copy_hash,
            IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body)
        );
        assert!(!shared_copy.body.contains(FAKE_SECRET));
        change(&mut request);

        assert!(matches!(
            fixture.service.creation_publish(request).unwrap(),
            IssueCreationPublished::Refused { .. },
        ));

        assert_eq!(fixture.creates(), 0);
        assert!(fixture.staged().is_empty());
    }
}

#[test]
fn creation_publish_rechecks_changed_text_policy_and_template_before_the_first_post() {
    for change in 0..4 {
        let fixture = CreationFixture::new(false);
        let (_, _, mut request) = prepare(&fixture, IssueSensitiveSpanClass::Secret);
        match change {
            0 => request.text.title = "A different title".to_string(),
            1 => fixture
                .files
                .policy
                .lock()
                .unwrap()
                .push_str("\n[publish.spans]\nlog = \"remove\"\n"),
            2 => fixture
                .files
                .template
                .lock()
                .unwrap()
                .push_str("\n<!-- changed -->\n"),
            _ => request.text.title = " ".to_string(),
        }

        let result = fixture.service.creation_publish(request).unwrap();

        if change == 3 {
            assert!(matches!(result, IssueCreationPublished::Blocked { .. }));
        } else {
            assert!(matches!(
                result,
                IssueCreationPublished::Refused {
                    reason: "approval_mismatch"
                }
            ));
        }
        assert_eq!(fixture.creates(), 0);
        assert!(fixture.staged().is_empty());
    }
}

#[test]
fn frozen_held_and_recorded_creation_ignore_replacement_raw_text_and_spans() {
    for state in [
        IssueCreationIntentState::Held,
        IssueCreationIntentState::Recorded,
    ] {
        let fixture = CreationFixture::new(false);
        let (shared_copy, request) = freeze(&fixture, state);
        let staged = fixture.staged();
        let mut replay = different_raw_text(request);
        let preview = fixture
            .service
            .creation_preview(IssueCreationPreviewRequest {
                target: target(),
                intent_id: replay.intent_id.clone(),
                text: replay.text.clone(),
            })
            .unwrap();
        assert_eq!(preview.approval, Some(replay.approval.clone()));
        assert_eq!(ready(preview.preview).0, shared_copy);

        let first = fixture.service.creation_publish(replay.clone()).unwrap();
        if state == IssueCreationIntentState::Held {
            assert!(matches!(first, IssueCreationPublished::Held { .. }));
            assert_eq!(fixture.creates(), 0);
            replay.shared_copy_only = true;
            assert!(matches!(
                fixture.service.creation_publish(replay).unwrap(),
                IssueCreationPublished::Confirmed { .. },
            ));
        } else {
            assert!(matches!(first, IssueCreationPublished::Confirmed { .. }));
        }

        assert_eq!(fixture.creates(), 1);
        assert_eq!(
            fixture.remote.posts.lock().unwrap()[0].shared_copy,
            shared_copy
        );
        assert_eq!(fixture.staged(), staged);
        assert_eq!(fixture.vault.stage_attempts.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn changed_policy_or_template_cannot_reapprove_a_frozen_unsent_creation() {
    for state in [
        IssueCreationIntentState::Held,
        IssueCreationIntentState::Recorded,
    ] {
        for change_policy in [false, true] {
            let fixture = CreationFixture::new(false);
            let (_, request) = freeze(&fixture, state);
            let staged = fixture.staged();
            if change_policy {
                fixture
                    .files
                    .policy
                    .lock()
                    .unwrap()
                    .push_str("\n[publish.spans]\nlog = \"remove\"\n");
            } else {
                fixture
                    .files
                    .template
                    .lock()
                    .unwrap()
                    .push_str("\n<!-- changed -->\n");
            }
            let preview = fixture
                .service
                .creation_preview(IssueCreationPreviewRequest {
                    target: target(),
                    intent_id: request.intent_id.clone(),
                    text: request.text.clone(),
                })
                .unwrap();
            assert!(preview.approval.is_none());

            assert!(matches!(
                fixture.service.creation_publish(request).unwrap(),
                IssueCreationPublished::Refused {
                    reason: "approval_mismatch"
                },
            ));

            assert_eq!(fixture.creates(), 0);
            assert_eq!(fixture.staged(), staged);
            assert_eq!(fixture.vault.stage_attempts.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn sent_and_confirmed_creation_reopen_and_recover_even_when_the_policy_becomes_invalid() {
    for state in [
        IssueCreationIntentState::Sent,
        IssueCreationIntentState::NeedsConfirmation,
        IssueCreationIntentState::Confirmed,
    ] {
        let fixture = CreationFixture::new(state == IssueCreationIntentState::NeedsConfirmation);
        let (opened, shared_copy, request) = prepare(&fixture, IssueSensitiveSpanClass::Secret);
        if state == IssueCreationIntentState::Sent {
            *fixture.journal.fail_before.lock().unwrap() =
                Some(IssueCreationIntentState::NeedsConfirmation);
        }
        let first = fixture.service.creation_publish(request.clone());
        if state == IssueCreationIntentState::Sent {
            assert!(first.is_err());
        } else {
            first.unwrap();
        }
        assert_eq!(fixture.creates(), 1);
        assert_eq!(
            fixture
                .journal
                .records
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .state,
            state
        );
        *fixture.files.policy.lock().unwrap() = "invalid = [".to_string();

        let reopened = fixture.service.creation_open(target()).unwrap();

        assert_eq!(reopened.intent.intent_id, opened.intent.intent_id);
        assert_eq!(reopened.intent.state, state);
        assert_eq!(reopened.template.status, "recovery_only");
        assert_eq!(reopened.title, shared_copy.title);
        assert_eq!(reopened.body, shared_copy.body);
        assert!(
            !serde_json::to_string(&reopened)
                .unwrap()
                .contains(FAKE_SECRET)
        );

        let mut replay = different_raw_text(request);
        for _ in 0..2 {
            let recovered = fixture.service.creation_publish(replay.clone()).unwrap();
            if state == IssueCreationIntentState::Confirmed {
                assert!(matches!(
                    recovered,
                    IssueCreationPublished::Confirmed { .. }
                ));
            } else {
                assert!(matches!(
                    recovered,
                    IssueCreationPublished::NeedsConfirmation { .. }
                ));
            }
        }
        replay.approval.intent_id = "unapproved-intent".to_string();
        assert!(matches!(
            fixture.service.creation_publish(replay).unwrap(),
            IssueCreationPublished::Refused {
                reason: "approval_mismatch"
            },
        ));
        assert_eq!(fixture.creates(), 1);
        assert!(fixture.staged().is_empty());
    }
}

#[test]
fn creator_enabled_updates_use_the_primary_scope_for_linked_and_subdirectory_targets() {
    use crate::application::port::inbound::issue_draft_port::{
        IssueDraftPreviewRequest, IssueDraftPublishRequest, IssueDraftPublished,
    };

    for cwd in ["/work/project-linked", "/work/project/sub"] {
        let fixture = CreationFixture::new(false);
        let mut update_target = super::target();
        update_target.cwd = cwd.to_string();
        let opened = fixture.service.open(update_target.clone()).unwrap();
        let text = text_with_span(Some(IssueSensitiveSpanClass::Log));
        let (_, approval, publishable) = ready(
            fixture
                .service
                .preview(IssueDraftPreviewRequest {
                    target: update_target.clone(),
                    text: text.clone(),
                })
                .unwrap(),
        );
        assert!(publishable);

        let published = fixture
            .service
            .publish(IssueDraftPublishRequest {
                target: update_target.clone(),
                text,
                approval,
                edit_basis: opened.edit_basis,
                shared_copy_only: false,
            })
            .unwrap();
        let IssueDraftPublished::Confirmed { intent: published } = published else {
            panic!("the canonical update should be confirmed: {published:?}");
        };

        assert_eq!(
            *fixture.sources.policy_workspaces.lock().unwrap(),
            vec!["/work/project".to_string(); 3],
            "open, preview and publish must use the primary checkout's policy"
        );
        assert_eq!(
            *fixture.sources.template_workspaces.lock().unwrap(),
            vec!["/work/project".to_string(); 3]
        );
        let resolutions = fixture.remote.resolves.lock().unwrap();
        assert_eq!(resolutions.len(), 2);
        assert!(
            resolutions
                .iter()
                .all(|request| request.project_scope.workspace_dir == "/work/project")
        );
        drop(resolutions);
        let updates = fixture.remote.updates.lock().unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[0].issue.project_scope.workspace_dir,
            "/work/project"
        );
        drop(updates);
        assert!(
            fixture
                .remote
                .reads
                .lock()
                .unwrap()
                .iter()
                .all(|issue| issue.project_scope.workspace_dir == "/work/project")
        );
        let vault_scopes = fixture.regular_scopes();
        assert!(!vault_scopes.is_empty());
        assert!(
            vault_scopes
                .iter()
                .all(|scope| scope.issue.project_scope.workspace_dir == "/work/project")
        );
        assert_eq!(fixture.creates(), 0);
        assert!(fixture.staged().is_empty());

        assert_eq!(
            fixture
                .service
                .recheck(update_target)
                .unwrap()
                .unwrap()
                .intent_id,
            published.intent_id
        );
        assert_eq!(
            fixture
                .service
                .open(super::target())
                .unwrap()
                .intent
                .unwrap()
                .intent_id,
            published.intent_id,
            "reopening the primary checkout must find the same update intent"
        );
    }
}
