use super::*;

fn reviewed_edit(fixture: &Fixture) -> (VaultUnlockGrant, VaultReconnectReview, String) {
    let grant = fixture.grant(&issue());
    let edited_body = format!("changed: {MASKED_BODY}");
    fixture.publish(&issue(), MASKED_TITLE, &edited_body);
    let review = fixture.service.review_reconnect(&grant).unwrap();
    (grant, review, edited_body)
}

fn approve_title(review: &VaultReconnectReview) -> VaultReconnectApproval {
    VaultReconnectApproval::from_review(
        review,
        vec![mapping(
            VaultRecordField::Title,
            MASKED_TITLE,
            "[[akra-mask:1]]",
        )],
    )
    .unwrap()
}

#[test]
fn an_unknown_reconnect_commit_retries_the_original_approval_without_rewriting() {
    let fixture = Fixture::published();
    let (grant, review, edited_body) = reviewed_edit(&fixture);
    let approval = approve_title(&review);
    *fixture.vault.fail_after_pointer.lock().unwrap() = true;

    assert_eq!(
        fixture.service.reconnect(&grant, &approval),
        Err(VaultRestoreHold::RecordUnavailable(
            VaultStoreError::WriteOutcomeUnknown
        ))
    );
    let committed_records = fixture.vault.records.lock().unwrap().clone();
    let committed_writes = *fixture.vault.writes.lock().unwrap();
    let restored = fixture.restore().unwrap();
    assert_eq!(restored.title.expose(), TITLE);
    assert_eq!(restored.body.expose(), edited_body);

    let recreated = EvidenceVaultRestoreService::new(
        fixture.remote.clone(),
        fixture.vault.clone(),
        fixture.unlock.clone(),
    );
    recreated.reconnect(&grant, &approval).unwrap();
    assert_eq!(*fixture.vault.records.lock().unwrap(), committed_records);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), committed_writes);

    // 첫 commit이 제외한 표식도 원래 검토 판에는 있지만 다른 승인의 재시도는 아니다.
    let conflicting = VaultReconnectApproval::from_review(
        &review,
        vec![mapping(
            VaultRecordField::Body,
            &edited_body,
            "[[akra-mask:2]]",
        )],
    )
    .unwrap();
    assert_eq!(
        recreated.reconnect(&grant, &conflicting),
        Err(VaultRestoreHold::ChangedDuringReview)
    );
    assert_eq!(*fixture.vault.records.lock().unwrap(), committed_records);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), committed_writes);
    assert_eq!(fixture.restore().unwrap(), restored);
}

#[test]
fn a_reconnect_rejects_a_new_local_binding_even_when_the_remote_hash_is_unchanged() {
    let fixture = Fixture::published();
    let (grant, review, edited_body) = reviewed_edit(&fixture);
    let approval = approve_title(&review);
    let replacement_version = format!(
        "{}:intent:{}",
        version_of(MASKED_TITLE, &edited_body),
        "b".repeat(64)
    );
    fixture.put(
        VaultRecordScope {
            issue: issue(),
            field: VaultRecordField::Title,
            binding: VaultRecordBinding::ConfirmedVersion(replacement_version.clone()),
        },
        seal_field_spans(&[VaultRestorableSpan {
            field: VaultRecordField::Title,
            marker: "[[akra-mask:1]]".to_string(),
            position: 0,
            plaintext: VaultPlaintext::new("replacement".to_string()),
        }]),
    );
    fixture.put(
        VaultRecordScope::current_version(issue()),
        VaultPlaintext::new(replacement_version),
    );
    let replacement_records = fixture.vault.records.lock().unwrap().clone();
    let replacement_restore = fixture.restore().unwrap();
    assert_eq!(
        replacement_restore.title.expose(),
        "replacement 의 빌드 실패"
    );

    assert_eq!(
        fixture.service.reconnect(&grant, &approval),
        Err(VaultRestoreHold::ChangedDuringReview)
    );
    assert_eq!(*fixture.vault.records.lock().unwrap(), replacement_records);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), 0);
    assert_eq!(fixture.restore().unwrap(), replacement_restore);
}

#[test]
fn reconnect_approval_normalizes_order_and_rejects_changed_review_identity() {
    let fixture = Fixture::published();
    let (grant, review, edited_body) = reviewed_edit(&fixture);
    let mappings = vec![
        mapping(VaultRecordField::Title, MASKED_TITLE, "[[akra-mask:1]]"),
        mapping(VaultRecordField::Body, &edited_body, "[[akra-mask:2]]"),
    ];
    let first = VaultReconnectApproval::from_review(&review, mappings.clone()).unwrap();
    let reversed =
        VaultReconnectApproval::from_review(&review, mappings.iter().cloned().rev().collect())
            .unwrap();
    assert_eq!(first, reversed);

    let mut changed_issue = review.clone();
    changed_issue.issue.issue_id = "I_other".to_string();
    let mut changed_hash = review.clone();
    changed_hash.reviewed_hash = IssueFieldHash::of_fields("different", &edited_body);
    let mut changed_copy = review.clone();
    changed_copy.published = IssueRawFields::new("different".to_string(), edited_body);
    for changed in [changed_issue, changed_hash, changed_copy] {
        assert_eq!(
            VaultReconnectApproval::from_review(&changed, mappings.clone()),
            Err(VaultRestoreHold::MappingRejected)
        );
    }

    fixture.service.reconnect(&grant, &first).unwrap();
    let records = fixture.vault.records.lock().unwrap().clone();
    let writes = *fixture.vault.writes.lock().unwrap();
    fixture.service.reconnect(&grant, &reversed).unwrap();
    assert_eq!(*fixture.vault.records.lock().unwrap(), records);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), writes);
}

#[test]
fn a_reconnect_retry_completes_staged_fields_after_a_pre_pointer_failure() {
    let fixture = Fixture::published();
    let (grant, review, _) = reviewed_edit(&fixture);
    let approval = approve_title(&review);
    let previous_version = fixture.bound_version();
    *fixture.vault.saves_left.lock().unwrap() = Some(2);

    assert!(matches!(
        fixture.service.reconnect(&grant, &approval),
        Err(VaultRestoreHold::RecordUnavailable(_))
    ));
    assert_eq!(fixture.bound_version(), previous_version);
    let attempts = *fixture.vault.writes.lock().unwrap();
    *fixture.vault.saves_left.lock().unwrap() = None;

    fixture.service.reconnect(&grant, &approval).unwrap();
    assert_eq!(*fixture.vault.writes.lock().unwrap(), attempts + 1);
    assert_eq!(fixture.restore().unwrap().title.expose(), TITLE);
    let records = fixture.vault.records.lock().unwrap().clone();
    fixture.service.reconnect(&grant, &approval).unwrap();
    assert_eq!(*fixture.vault.records.lock().unwrap(), records);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), attempts + 1);
}

#[test]
fn a_new_review_can_change_the_selection_for_the_same_published_copy() {
    let fixture = Fixture::published();
    let (grant, review, edited_body) = reviewed_edit(&fixture);
    fixture
        .service
        .reconnect(&grant, &approve_title(&review))
        .unwrap();

    let fresh = fixture.service.review_reconnect(&grant).unwrap();
    assert_eq!(fresh.reviewed_hash, review.reviewed_hash);
    let clear = VaultReconnectApproval::from_review(&fresh, Vec::new()).unwrap();
    fixture.service.reconnect(&grant, &clear).unwrap();
    let restored = fixture.restore().unwrap();
    assert_eq!(restored.title.expose(), MASKED_TITLE);
    assert_eq!(restored.body.expose(), edited_body);
}

#[test]
fn an_approval_cannot_rebind_another_readable_project_server_repository_or_issue() {
    let fixture = Fixture::published();
    let (grant, review, edited_body) = reviewed_edit(&fixture);
    let approval = approve_title(&review);
    assert!(fixture.unlock.access(&grant).is_ok());
    let original_version = fixture.bound_version();
    let mut other_project = issue();
    other_project.project_scope.workspace_dir = "/work/other".to_string();
    let others = [
        other_project,
        IssueReference {
            server_id: "github.example".to_string(),
            ..issue()
        },
        IssueReference {
            repository_id: "R_other".to_string(),
            ..issue()
        },
        IssueReference {
            issue_id: "I_other".to_string(),
            ..issue()
        },
    ];

    for other in others {
        // B도 같은 공개 해시를 가지며 실제 원문을 복원할 수 있어야 한다.
        fixture.publish(&other, MASKED_TITLE, &edited_body);
        let version = version_of(MASKED_TITLE, &edited_body);
        let span = |field, marker: &str, plaintext: &str| VaultRestorableSpan {
            field,
            marker: marker.to_string(),
            position: match field {
                VaultRecordField::Title => MASKED_TITLE.find(marker).unwrap(),
                VaultRecordField::Body => edited_body.find(marker).unwrap(),
            },
            plaintext: VaultPlaintext::new(plaintext.to_string()),
        };
        for (field, spans) in [
            (
                VaultRecordField::Title,
                vec![span(VaultRecordField::Title, "[[akra-mask:1]]", "bob")],
            ),
            (
                VaultRecordField::Body,
                vec![
                    span(VaultRecordField::Body, "[[akra-mask:2]]", "bob"),
                    span(VaultRecordField::Body, "[[akra-mask:3]]", "/work/b"),
                ],
            ),
        ] {
            fixture.put(
                VaultRecordScope {
                    issue: other.clone(),
                    field,
                    binding: VaultRecordBinding::ConfirmedVersion(version.clone()),
                },
                seal_field_spans(&spans),
            );
        }
        fixture.put(
            VaultRecordScope::current_version(other.clone()),
            VaultPlaintext::new(version),
        );
        let other_grant = fixture.grant(&other);
        let other_restored = fixture.service.restore_for_grant(&other_grant).unwrap();
        assert_eq!(other_restored.title.expose(), "bob 의 빌드 실패");
        assert_eq!(
            other_restored.body.expose(),
            "changed: bob 가 /work/b 에서 실행했다.\r\n로그는 아래에 있다."
        );
        let records = fixture.vault.records.lock().unwrap().clone();
        let writes = *fixture.vault.writes.lock().unwrap();

        assert_eq!(
            fixture.service.reconnect(&other_grant, &approval),
            Err(VaultRestoreHold::NotBound),
            "{other:?}"
        );
        assert_eq!(*fixture.vault.records.lock().unwrap(), records);
        assert_eq!(*fixture.vault.writes.lock().unwrap(), writes);
        assert_eq!(fixture.bound_version(), original_version);
        assert_eq!(
            fixture.service.restore_for_grant(&other_grant).unwrap(),
            other_restored
        );
    }
}

#[test]
fn a_reconnect_revoked_during_the_latest_read_writes_nothing() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let fixture = Fixture::published();
    let (grant, review, _) = reviewed_edit(&fixture);
    let approval = approve_title(&review);
    assert!(fixture.unlock.access(&grant).is_ok());
    let previous_version = fixture.bound_version();
    let records = fixture.vault.records.lock().unwrap().clone();
    let writes = *fixture.vault.writes.lock().unwrap();
    let revoked = Arc::new(AtomicBool::new(false));
    let during_read = revoked.clone();
    let unlock = fixture.unlock.clone();
    let session = fixture.session.clone();
    *fixture.remote.during_read.lock().unwrap() = Some(Box::new(move || {
        unlock.end_session(&session);
        during_read.store(true, Ordering::SeqCst);
    }));

    assert!(matches!(
        fixture.service.reconnect(&grant, &approval),
        Err(VaultRestoreHold::Locked(_))
    ));
    assert!(revoked.load(Ordering::SeqCst));
    assert!(fixture.unlock.access(&grant).is_err());
    assert_eq!(*fixture.vault.records.lock().unwrap(), records);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), writes);
    assert_eq!(fixture.bound_version(), previous_version);
}
