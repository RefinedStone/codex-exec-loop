use super::*;

pub(super) fn update(
    fixture: &Fixture,
    original: &IssueFeedbackIntent,
    id: &str,
) -> IssueFeedbackIntent {
    let mut next = fixture.intent("next-intent", "### Akra 작업 상태\n\n상태: 통합 확인");
    let IssueFeedbackOperation::CreateComment {
        body,
        body_sha256,
        content_sha256,
    } = next.operation
    else {
        unreachable!()
    };
    let IssueFeedbackOperation::CreateComment {
        body_sha256: old_body,
        content_sha256: old_content,
        ..
    } = &original.operation
    else {
        unreachable!()
    };
    next.operation = IssueFeedbackOperation::UpdateComment {
        owner: Box::new(IssueFeedbackCommentOwner {
            issue: issue(),
            comment_id: id.into(),
            actor_id: ACTOR.into(),
            latest_intent_id: original.intent_id.clone(),
            body_sha256: old_body.clone(),
            content_sha256: old_content.clone(),
        }),
        body,
        body_sha256,
        content_sha256,
    };
    next
}

#[test]
fn comment_property_and_lossless_adf_bind_the_same_owned_comment_across_updates() {
    let fixture = Fixture::new();
    let first = fixture.intent("first-intent", "### Akra 작업 상태\n\n상태: 실행 중");
    assert_eq!(
        fixture.adapter.write(&first).unwrap(),
        IssueFeedbackWriteResult::Acknowledged {
            comment_id: Some("1000".into())
        }
    );
    assert!(matches!(
        fixture.adapter.reconcile(&first).unwrap(),
        IssueFeedbackReadResult::Confirmed(_)
    ));
    let next = update(&fixture, &first, "1000");
    assert_eq!(
        fixture.adapter.write(&next).unwrap(),
        IssueFeedbackWriteResult::Acknowledged {
            comment_id: Some("1000".into())
        }
    );
    assert!(matches!(
        fixture.adapter.reconcile(&next).unwrap(),
        IssueFeedbackReadResult::Confirmed(_)
    ));
    assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.api.remote.lock().unwrap().comments.len(), 1);
}

#[test]
fn lost_ack_and_a_new_same_account_credential_only_reconcile_the_previous_intent() {
    let fixture = Fixture::new();
    let intent = fixture.intent("lost-ack", "Shared status");
    fixture.api.lose_ack.store(true, Ordering::SeqCst);
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    fixture.credentials.rotate();
    assert!(fixture.adapter.write(&intent).is_err());
    for _ in 0..3 {
        assert!(matches!(
            fixture.adapter.reconcile(&intent).unwrap(),
            IssueFeedbackReadResult::Confirmed(_)
        ));
    }
    assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_copied_property_foreign_editor_changed_body_or_duplicate_is_not_owned_proof() {
    for change in 0..4 {
        let fixture = Fixture::new();
        let intent = fixture.intent("intent-1", "Shared status");
        fixture.adapter.write(&intent).unwrap();
        {
            let mut remote = fixture.api.remote.lock().unwrap();
            match change {
                0 => remote.comments[0]["author"]["accountId"] = json!("another-person"),
                1 => remote.comments[0]["updateAuthor"]["accountId"] = json!("another-person"),
                2 => {
                    remote.comments[0]["body"]["content"][0]["content"][0]["text"] =
                        json!("changed by a person")
                }
                _ => {
                    let mut copied = remote.comments[0].clone();
                    copied["id"] = json!("1001");
                    copied["self"] = json!(format!("{SITE}/rest/api/3/issue/100/comment/1001"));
                    remote.comments.push(copied);
                }
            }
        }
        assert_eq!(
            fixture.adapter.reconcile(&intent).unwrap(),
            IssueFeedbackReadResult::Contradictory
        );
        if change < 3 {
            assert_eq!(
                fixture
                    .adapter
                    .write(&update(&fixture, &intent, "1000"))
                    .unwrap(),
                IssueFeedbackWriteResult::Contradictory
            );
        }
        assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn incomplete_comment_pages_never_authorize_create_or_reconcile_as_absence() {
    let fixture = Fixture::new();
    let intent = fixture.intent("bounded", "Shared status");
    fixture.api.remote.lock().unwrap().incomplete = true;
    assert!(fixture.adapter.write(&intent).is_err());
    assert_eq!(
        fixture.adapter.reconcile(&intent).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
    assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 0);
}

#[test]
fn rotation_or_disconnect_at_the_actual_send_boundary_produces_zero_effects() {
    for disconnect in [false, true] {
        let fixture = Fixture::new();
        let intent = fixture.intent("stale-send", "Shared status");
        let credentials = fixture.credentials.clone();
        *fixture.api.before_send.lock().unwrap() = Some(Box::new(move || {
            if disconnect {
                *credentials.active.lock().unwrap() = None;
            } else {
                credentials.rotate();
            }
        }));
        assert_eq!(
            fixture.adapter.write(&intent).unwrap(),
            IssueFeedbackWriteResult::Unconfirmed
        );
        assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 0);
        assert!(fixture.api.remote.lock().unwrap().comments.is_empty());
    }
}

#[test]
fn changed_project_account_and_binding_scope_cannot_fall_back_to_a_display_key() {
    for change in 0..3 {
        let fixture = Fixture::new();
        let intent = fixture.intent("scope-fence", "Shared status");
        match change {
            0 => fixture.api.remote.lock().unwrap().parent = "20".into(),
            1 => fixture.api.remote.lock().unwrap().actor = "other-account".into(),
            _ => fixture.policy.lock().unwrap().cloud_id = "another-cloud".into(),
        }
        assert!(fixture.adapter.write(&intent).is_err());
        assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn all_comment_pages_are_read_and_fixed_update_id_is_not_rebound() {
    let fixture = Fixture::new();
    let first = fixture.intent("first", "First status");
    fixture.adapter.write(&first).unwrap();
    let second = fixture.intent("second", "Second status");
    fixture.adapter.write(&second).unwrap();
    fixture.api.remote.lock().unwrap().page_size = 1;
    assert!(
        matches!(fixture.adapter.reconcile(&second).unwrap(), IssueFeedbackReadResult::Confirmed(receipt)
        if matches!(receipt.evidence, IssueFeedbackEvidence::Comment { ref comment_id, .. } if comment_id == "1001"))
    );
    let wrong_owner = update(&fixture, &first, "1001");
    assert_eq!(
        fixture.adapter.write(&wrong_owner).unwrap(),
        IssueFeedbackWriteResult::Contradictory
    );
    assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 2);
}

#[test]
fn unsupported_owned_adf_and_unbounded_property_metadata_are_not_silently_normalized() {
    for change in 0..2 {
        let fixture = Fixture::new();
        let intent = fixture.intent("owned", "Shared status");
        fixture.adapter.write(&intent).unwrap();
        let mut remote = fixture.api.remote.lock().unwrap();
        if change == 0 {
            remote.comments[0]["body"]["content"] =
                json!([{"type":"extension","attrs":{"extensionKey":"unknown"}}]);
        } else {
            remote.comments[0]["properties"][0]["value"]["body_sha256"] = json!("x".repeat(2048));
        }
        drop(remote);
        assert_eq!(
            fixture.adapter.reconcile(&intent).unwrap(),
            IssueFeedbackReadResult::NeedsConfirmation
        );
        assert!(
            fixture
                .adapter
                .write(&update(&fixture, &intent, "1000"))
                .is_err()
        );
        assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn explicit_jira_http_rejection_is_distinct_from_transport_or_local_errors() {
    for reason in [
        JiraFeedbackRejection::BadRequest,
        JiraFeedbackRejection::Unauthorized,
        JiraFeedbackRejection::Forbidden,
        JiraFeedbackRejection::NotFound,
    ] {
        for transition in [false, true] {
            let fixture = Fixture::new();
            let intent = if transition {
                fixture.close_intent()
            } else {
                fixture.intent("rejected", "Shared status")
            };
            *fixture.api.rejection.lock().unwrap() = Some(reason);
            assert_eq!(
                fixture.adapter.write(&intent).unwrap(),
                IssueFeedbackWriteResult::JiraRejected(reason)
            );
            assert_eq!(fixture.api.writes.load(Ordering::SeqCst), 1);
            let remote = fixture.api.remote.lock().unwrap();
            assert!(remote.comments.is_empty() && remote.history.is_empty());
        }
    }
    for error in [
        ProviderCredentialError::Unavailable,
        ProviderCredentialError::OutcomeUnknown,
        ProviderCredentialError::InvalidRequest,
        ProviderCredentialError::NotFound,
        ProviderCredentialError::Unauthorized,
    ] {
        let fixture = Fixture::new();
        let intent = fixture.intent("unknown", "Shared status");
        *fixture.api.write_error.lock().unwrap() = Some(error);
        assert_eq!(
            fixture.adapter.write(&intent).unwrap(),
            IssueFeedbackWriteResult::Unconfirmed
        );
    }
}
