use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn changed_review(fixture: &Fixture) -> (VaultUnlockGrant, VaultReconnectApproval) {
    let changed_body = format!("외부 수정: {MASKED_BODY}");
    fixture.publish(&issue(), MASKED_TITLE, &changed_body);
    let grant = fixture.grant(&issue());
    let review = fixture.service.review_reconnect(&grant).unwrap();
    let approval = VaultReconnectApproval::from_review(
        &review,
        vec![mapping(
            VaultRecordField::Body,
            &changed_body,
            "[[akra-mask:2]]",
        )],
    )
    .unwrap();
    (grant, approval)
}

#[test]
fn a_declined_commit_gate_preserves_every_record_without_writes() {
    let fixture = Fixture::published();
    let (grant, approval) = changed_review(&fixture);
    let before = fixture.vault.records.lock().unwrap().clone();
    let calls = AtomicUsize::new(0);

    let result = fixture
        .service
        .reconnect_with_commit_gate(&grant, &approval, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(VaultRestoreHold::Locked(
                VaultSharedCopyReason::SessionExpired,
            ))
        });

    assert_eq!(
        result,
        Err(VaultRestoreHold::Locked(
            VaultSharedCopyReason::SessionExpired
        ))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), 0);
    assert_eq!(*fixture.vault.records.lock().unwrap(), before);
}

#[test]
fn a_grant_revoked_during_remote_recheck_never_reaches_the_commit_gate() {
    let fixture = Fixture::published();
    let (grant, approval) = changed_review(&fixture);
    let before = fixture.vault.records.lock().unwrap().clone();
    let (unlock, session) = (fixture.unlock.clone(), fixture.session.clone());
    *fixture.remote.during_read.lock().unwrap() =
        Some(Box::new(move || unlock.end_session(&session)));
    let calls = AtomicUsize::new(0);

    let result = fixture
        .service
        .reconnect_with_commit_gate(&grant, &approval, || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

    assert_eq!(
        result,
        Err(VaultRestoreHold::Locked(
            VaultSharedCopyReason::SessionExpired
        ))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), 0);
    assert_eq!(*fixture.vault.records.lock().unwrap(), before);
}

#[test]
fn revocation_after_commit_admission_does_not_claim_to_undo_the_binding() {
    let fixture = Fixture::published();
    let (grant, approval) = changed_review(&fixture);
    let before = fixture.bound_version();
    let calls = AtomicUsize::new(0);

    let result = fixture
        .service
        .reconnect_with_commit_gate(&grant, &approval, || {
            calls.fetch_add(1, Ordering::SeqCst);
            // The permit was admitted immediately before the owner closed its session.
            fixture.unlock.end_session(&fixture.session);
            Ok(())
        });

    assert_eq!(result, Ok(()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_ne!(fixture.bound_version(), before);
    assert_eq!(*fixture.vault.writes.lock().unwrap(), 3);
    assert_eq!(
        fixture.service.restore_for_grant(&grant),
        Err(VaultRestoreHold::Locked(
            VaultSharedCopyReason::SessionExpired
        ))
    );
}
