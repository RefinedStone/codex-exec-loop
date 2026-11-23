use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::application::service::evidence_vault_restore::{
    EvidenceVaultRestoreService, VaultApprovedMapping, VaultReconnectApproval, VaultRestoreHold,
    restorable_fields,
};
use crate::application::service::evidence_vault_unlock::{
    EvidenceVaultUnlockService, VaultSharedCopyReason, VaultUnlockGrant, VaultUnlockOutcome,
    VaultUnlockRequest, VaultUserDecision,
};
use crate::application::service::issue_masking::{IssueMaskedSpan, IssueTextField};
use crate::domain::issue_publish_intent::{IssuePublishVaultField, confirmed_version_name};
use crate::domain::issue_work::IssueComparisonBasis;

const MARKER: &str = "[synthetic-mask-1]";
const SHARED_BODY: &str = "before [synthetic-mask-1] after";
const ORIGINAL: &str = "synthetic-private-원문";

struct Snapshot {
    issue: IssueReference,
    fields: Mutex<IssueRawFields>,
    reads: AtomicUsize,
}

impl IssueSnapshotPort for Snapshot {
    fn resolve(&self, _: IssueSnapshotResolveRequest) -> anyhow::Result<IssueSnapshotResponse> {
        anyhow::bail!("the synthetic restore fixture does not resolve locators")
    }

    fn read(&self, request: IssueSnapshotReadRequest) -> anyhow::Result<IssueSnapshotResponse> {
        assert_eq!(request.issue, self.issue);
        self.reads.fetch_add(1, Ordering::SeqCst);
        let fields = self.fields.lock().unwrap().clone();
        Ok(IssueSnapshotResponse {
            issue: request.issue,
            comparison_basis: IssueComparisonBasis {
                field_hash: fields.field_hash(),
                provider_updated_at: "2026-11-23T00:00:00Z".to_string(),
            },
            raw_fields: fields,
        })
    }
}

struct RestoreFixture {
    root: PathBuf,
    store: Arc<FilesystemEvidenceVaultStoreAdapter>,
    snapshots: Arc<Snapshot>,
    key_store: Arc<FakeVaultKeyStoreAdapter>,
    unlock: Arc<EvidenceVaultUnlockService>,
    grant: VaultUnlockGrant,
}

impl RestoreFixture {
    fn new() -> Self {
        let root = temp_vault("service-restore");
        let issue = scope(r"C:\Synthetic\복원").issue;
        let snapshots = Arc::new(Snapshot {
            issue: issue.clone(),
            fields: Mutex::new(IssueRawFields::new(
                "synthetic title".to_string(),
                SHARED_BODY.to_string(),
            )),
            reads: AtomicUsize::new(0),
        });
        let key_store = Arc::new(FakeVaultKeyStoreAdapter::with_key(VaultKey::new(vec![
            7;
            32
        ])));
        let unlock = Arc::new(EvidenceVaultUnlockService::new(key_store.clone()));
        let grant = match unlock.unlock(VaultUnlockRequest {
            issue,
            session: unlock.begin_session(),
            decision: VaultUserDecision::Confirmed,
        }) {
            VaultUnlockOutcome::Unlocked(grant) => grant,
            outcome => panic!("synthetic grant unavailable: {outcome:?}"),
        };
        let fixture = Self {
            store: Arc::new(FilesystemEvidenceVaultStoreAdapter::new(root.clone())),
            root,
            snapshots,
            key_store,
            unlock,
            grant,
        };
        fixture.seed_legacy();
        fixture
    }

    fn key(&self) -> VaultRecordKey {
        self.unlock.access(&self.grant).unwrap().key
    }

    fn service(&self) -> EvidenceVaultRestoreService {
        // A new store instance must restore the on-disk v1 records without cached bindings.
        EvidenceVaultRestoreService::new(
            self.snapshots.clone(),
            Arc::new(FilesystemEvidenceVaultStoreAdapter::new(self.root.clone())),
            self.unlock.clone(),
        )
    }

    fn seed_legacy(&self) {
        let version = confirmed_version_name(&self.snapshots.fields.lock().unwrap().field_hash());
        let fields = restorable_fields(&[IssueMaskedSpan {
            marker: MARKER.to_string(),
            field: IssueTextField::Body,
            position: "before ".len(),
            plaintext: VaultPlaintext::new(ORIGINAL.to_string()),
        }]);
        for field in fields {
            self.store
                .seed_record_for_test(VaultRecordSaveRequest {
                    scope: VaultRecordScope {
                        issue: self.grant.issue().clone(),
                        field: match field.field {
                            IssuePublishVaultField::Title => VaultRecordField::Title,
                            IssuePublishVaultField::Body => VaultRecordField::Body,
                        },
                        binding: VaultRecordBinding::ConfirmedVersion(version.clone()),
                    },
                    key: self.key(),
                    plaintext: field.plaintext,
                })
                .unwrap();
        }
        self.store
            .seed_record_for_test(VaultRecordSaveRequest {
                scope: VaultRecordScope::current_version(self.grant.issue().clone()),
                key: self.key(),
                plaintext: VaultPlaintext::new(version),
            })
            .unwrap();
    }

    fn publish(&self, body: &str) {
        *self.snapshots.fields.lock().unwrap() =
            IssueRawFields::new("synthetic title".to_string(), body.to_string());
    }

    fn records(&self) -> BTreeMap<String, Vec<u8>> {
        fs::read_dir(self.root.join("records"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                (
                    entry.file_name().to_str().unwrap().to_string(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect()
    }
}

impl Drop for RestoreFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn on_disk_legacy_binding_restores_reconnects_and_replays_without_rewriting_ciphertext() {
    let fixture = RestoreFixture::new();
    let service = fixture.service();
    let restored = service.restore_for_grant(&fixture.grant).unwrap();
    assert_eq!(restored.title.expose(), "synthetic title");
    assert_eq!(restored.body.expose(), format!("before {ORIGINAL} after"));
    assert_eq!(fixture.key_store.initialization_count(), 0);

    let edited = format!("edited {SHARED_BODY}");
    fixture.publish(&edited);
    assert_eq!(
        service.restore_for_grant(&fixture.grant),
        Err(VaultRestoreHold::PublishedCopyChanged)
    );
    let review = service.review_reconnect(&fixture.grant).unwrap();
    let mappings = review
        .markers
        .iter()
        .map(|marker| VaultApprovedMapping {
            marker: marker.marker.clone(),
            place: marker
                .candidates
                .first()
                .expect("one synthetic marker")
                .clone(),
        })
        .collect();
    let approval = VaultReconnectApproval::from_review(&review, mappings).unwrap();
    let before = fixture.records();

    fixture.publish(&format!("changed after review {SHARED_BODY}"));
    assert_eq!(
        service.reconnect(&fixture.grant, &approval),
        Err(VaultRestoreHold::ChangedDuringReview)
    );
    assert_eq!(fixture.records(), before);

    fixture.publish(&edited);
    service.reconnect(&fixture.grant, &approval).unwrap();
    let after = fixture.records();
    assert_ne!(after, before);
    assert_eq!(
        fixture
            .service()
            .restore_for_grant(&fixture.grant)
            .unwrap()
            .body
            .expose(),
        format!("edited before {ORIGINAL} after")
    );

    fixture
        .service()
        .reconnect(&fixture.grant, &approval)
        .unwrap();
    assert_eq!(fixture.records(), after);
    for record in after.values() {
        assert!(
            !record
                .windows(ORIGINAL.len())
                .any(|part| part == ORIGINAL.as_bytes())
        );
    }
    fixture.unlock.end_session(fixture.grant.session());
    assert_eq!(
        fixture.service().restore_for_grant(&fixture.grant),
        Err(VaultRestoreHold::Locked(
            VaultSharedCopyReason::SessionExpired
        ))
    );
    assert_eq!(fixture.key_store.initialization_count(), 0);
}

#[test]
fn restore_never_recreates_a_missing_root_or_a_lost_device_key() {
    let fixture = RestoreFixture::new();
    fs::remove_dir_all(&fixture.root).unwrap();
    assert!(matches!(
        fixture.service().restore_for_grant(&fixture.grant),
        Err(VaultRestoreHold::RecordUnavailable(
            VaultStoreError::Storage(_)
        ))
    ));
    assert!(!fixture.root.exists());
    assert_eq!(fixture.key_store.initialization_count(), 0);

    fixture.key_store.lose_key();
    let reads = fixture.snapshots.reads.load(Ordering::SeqCst);
    assert_eq!(
        fixture.service().restore_for_grant(&fixture.grant),
        Err(VaultRestoreHold::Locked(VaultSharedCopyReason::KeyLost))
    );
    assert_eq!(fixture.snapshots.reads.load(Ordering::SeqCst), reads);
    assert_eq!(fixture.key_store.initialization_count(), 0);
    assert!(fixture.key_store.stored_key().is_none());
    assert!(!fixture.root.exists());
}
