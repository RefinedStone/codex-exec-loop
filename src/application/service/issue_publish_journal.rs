use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow, bail};

use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultPlaintext, VaultRecordBinding, VaultRecordField, VaultRecordKey,
    VaultRecordLoadRequest, VaultRecordSaveRequest, VaultRecordScope,
};
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotReadRequest,
};
use crate::application::port::outbound::issue_update_port::{IssueUpdatePort, IssueUpdateRequest};
use crate::application::service::issue_publish::{
    IssueApprovedPublishRequest, IssuePublishRefusal, approval_refusal,
};
use crate::domain::issue_publish_intent::{
    IssuePublishIntentRecord, IssuePublishIntentState, IssuePublishVaultField,
};
use crate::domain::issue_work::{IssueComparisonBasis, IssueFieldHash, IssueReference};

/*
 * 게시 의도를 기록한 뒤에만 이슈를 고치는 경로이다 (#2424). 승인 검사와 충돌 검사는 `issue_publish` (#2423)와
 * 같다. 순서는 고정이다:
 *
 * 1. 승인이 보낼 값과 다르면 거절한다. 원격과 일지를 건드리지 않는다.
 * 2. 이 이슈에 결과를 모르는 의도가 있으면 보내지 않고 그 의도를 준다 (중복 클릭, 재시작 뒤).
 * 3. 복원 가능으로 표시한 구간을 Vault 에 이 의도에 묶어 저장한다. 저장하지 못하면 보류한다. 사용자가
 *    "공유본만 게시"를 확인한 요청은 Vault 를 쓰지 않는다.
 * 4. 의도(`Recorded`)를 저장한다. 원격을 읽는다. 편집 기준과 다르면 충돌이다.
 * 5. `Sent` 를 저장하고 쓰기를 한 번 보낸다.
 * 6. 원격을 다시 읽는다. 쓰기의 답과 재조회가 모두 승인한 글과 같으면 `Confirmed` 이다. 답을 잃었으면 재조회만으로
 *    정한다. 그 밖은 `NeedsConfirmation` 이다.
 *
 * 이 서비스는 쓰기를 다시 보내지 않는다. `reconcile` 은 읽기만 한다. 결과를 모르는 의도는 재조회가 승인한 글을
 * 보거나 사용자가 `abandon` 으로 버릴 때까지 새 쓰기를 막는다.
 */

/// 복원 가능으로 표시한 구간의 평문이다. 필드마다 하나이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRestorableField {
    pub field: IssuePublishVaultField,
    pub plaintext: VaultPlaintext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueJournaledPublishRequest {
    pub publish: IssueApprovedPublishRequest,
    pub restorable: Vec<IssueRestorableField>,
    /// 레코드를 암호화할 키이다 (`EvidenceVaultUnlockService::sealing_key`). Vault 를 쓸 수 없으면 `None` 이다.
    pub vault_key: Option<VaultRecordKey>,
    /// 사용자가 복원 연결 없이 공유본만 게시한다고 확인했으면 `true` 이다.
    pub shared_copy_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueJournaledPublishOutcome {
    /// 원격의 글이 승인한 공유본과 같다.
    Confirmed(IssuePublishIntentRecord),
    /// 게시 결과 확인 필요. 쓰기를 보냈고 결과를 모른다.
    NeedsConfirmation(IssuePublishIntentRecord),
    /// 이 이슈에 결과를 모르는 앞의 의도가 있다. 이번에는 보내지 않았다.
    AlreadyPending(IssuePublishIntentRecord),
    /// 복원 가능 구간을 저장하지 못했다. 보내지 않았다. 공유본만 게시하려면 `shared_copy_only` 로 다시 부른다.
    Held(IssuePublishIntentRecord),
    /// 원격 글이 편집 기준과 다르다. 쓰지 않았다.
    Conflict { remote: IssueComparisonBasis },
    /// 승인이 보낼 값과 다르다. 원격을 부르지 않았다.
    Refused(IssuePublishRefusal),
}

pub struct IssueJournaledPublishService {
    snapshot: Arc<dyn IssueSnapshotPort>,
    update: Arc<dyn IssueUpdatePort>,
    journal: Arc<dyn IssuePublishIntentJournalPort>,
    vault: Arc<dyn EvidenceVaultStorePort>,
    // 의도 확인과 기록 사이에 다른 게시가 끼지 않게 한다. 게시하는 프로세스는 Gateway 하나이다.
    publishing: Mutex<()>,
}

impl IssueJournaledPublishService {
    pub fn new(
        snapshot: Arc<dyn IssueSnapshotPort>,
        update: Arc<dyn IssueUpdatePort>,
        journal: Arc<dyn IssuePublishIntentJournalPort>,
        vault: Arc<dyn EvidenceVaultStorePort>,
    ) -> Self {
        Self {
            snapshot,
            update,
            journal,
            vault,
            publishing: Mutex::new(()),
        }
    }

    /// 승인한 글을 기존 이슈에 쓴다. `Err` 는 일지나 쓰기 전 원격 읽기의 실패이다. `Err` 일 때 쓰기는 보내지
    /// 않았다. 단, 쓰기 뒤의 상태를 일지에 저장하지 못한 `Err` 는 예외이다: 일지에 `Sent` 가 남고 `reconcile` 이 정한다.
    pub fn publish(
        &self,
        request: IssueJournaledPublishRequest,
    ) -> Result<IssueJournaledPublishOutcome> {
        let IssueJournaledPublishRequest {
            publish,
            restorable,
            vault_key,
            shared_copy_only,
        } = request;
        if let Some(refusal) = approval_refusal(&publish) {
            return Ok(IssueJournaledPublishOutcome::Refused(refusal));
        }
        let _publishing = self.lock();
        let workspace_dir = publish.issue.project_scope.workspace_dir.clone();
        let pending = self
            .journal
            .load(&workspace_dir)?
            .into_iter()
            .find(|record| record.issue == publish.issue && record.state.is_unresolved());
        if let Some(pending) = pending {
            return Ok(IssueJournaledPublishOutcome::AlreadyPending(pending));
        }

        let mut record = IssuePublishIntentRecord {
            intent_id: format!("{:032x}", rand::random::<u128>()),
            issue: publish.issue.clone(),
            previous_hash: publish.edit_basis.clone(),
            approved_hash: publish.approval.shared_copy_hash.clone(),
            pending_vault_fields: Vec::new(),
            state: IssuePublishIntentState::Recorded,
            observed_hash: None,
            restore_link_finalized: false,
        };
        if !shared_copy_only && !restorable.is_empty() {
            match self.save_pending_records(&record, restorable, vault_key) {
                Some(fields) => record.pending_vault_fields = fields,
                None => {
                    record.state = IssuePublishIntentState::Held;
                    self.journal.save(&workspace_dir, &record)?;
                    return Ok(IssueJournaledPublishOutcome::Held(record));
                }
            }
        }
        self.journal.save(&workspace_dir, &record)?;

        let remote = match self.snapshot.read(IssueSnapshotReadRequest {
            issue: publish.issue.clone(),
        }) {
            Ok(snapshot) => snapshot.comparison_basis,
            Err(error) => {
                record.state = IssuePublishIntentState::Abandoned;
                self.journal.save(&workspace_dir, &record)?;
                return Err(error);
            }
        };
        if remote.field_hash != publish.edit_basis {
            record.state = IssuePublishIntentState::Conflict;
            record.observed_hash = Some(remote.field_hash.clone());
            self.journal.save(&workspace_dir, &record)?;
            return Ok(IssueJournaledPublishOutcome::Conflict { remote });
        }

        record.state = IssuePublishIntentState::Sent;
        self.journal.save(&workspace_dir, &record)?;
        let answer = self.update.update_title_and_body(IssueUpdateRequest {
            approval: publish.approval.write,
            issue: publish.issue,
            shared_copy: publish.validated.shared_copy().clone(),
        });
        // 답이 있는데 대상이나 글이 다르면 재조회가 같아도 확인하지 않는다. 답을 잃었으면 재조회가 정한다.
        let answer_differs = answer.is_ok_and(|response| {
            response.issue != record.issue
                || IssueFieldHash::of_fields(
                    &response.published_copy.title,
                    &response.published_copy.body,
                ) != record.approved_hash
        });
        let observed = self.observe(&record.issue);
        let mut record = record.reconciled(observed);
        if answer_differs {
            record.state = IssuePublishIntentState::NeedsConfirmation;
        }
        self.journal.save(&workspace_dir, &record)?;
        Ok(match record.state {
            IssuePublishIntentState::Confirmed => IssueJournaledPublishOutcome::Confirmed(record),
            _ => IssueJournaledPublishOutcome::NeedsConfirmation(record),
        })
    }

    /// 결과를 모르는 의도마다 원격을 다시 읽어 상태를 정한다. 시작할 때와 사용자가 다시 확인할 때 부른다.
    /// 읽기만 한다. 쓰기를 보내지 않는다. 이 작업 폴더의 모든 의도를 돌려준다.
    pub fn reconcile(&self, workspace_dir: &str) -> Result<Vec<IssuePublishIntentRecord>> {
        let _publishing = self.lock();
        let mut records = self.journal.load(workspace_dir)?;
        for record in &mut records {
            if record.state.is_unresolved() {
                let next = record.clone().reconciled(self.observe(&record.issue));
                if next != *record {
                    self.journal.save(workspace_dir, &next)?;
                    *record = next;
                }
            }
        }
        Ok(records)
    }

    /// 사용자가 `NeedsConfirmation` 의도의 결과 확인을 그만둔다. 그 뒤의 게시는 새 승인으로 한다.
    pub fn abandon(
        &self,
        workspace_dir: &str,
        intent_id: &str,
    ) -> Result<IssuePublishIntentRecord> {
        let _publishing = self.lock();
        let mut record = self.intent(workspace_dir, intent_id)?;
        if record.state != IssuePublishIntentState::NeedsConfirmation {
            bail!("only an intent that needs confirmation can be abandoned")
        }
        record.state = IssuePublishIntentState::Abandoned;
        self.journal.save(workspace_dir, &record)?;
        Ok(record)
    }

    /// 복원 링크를 확정한다: 의도에 묶은 Vault 레코드를 확인된 판에 묶는다. 게시가 `Confirmed` 일 때만 한다.
    pub fn finalize_restore_link(
        &self,
        workspace_dir: &str,
        intent_id: &str,
        key: VaultRecordKey,
    ) -> Result<IssuePublishIntentRecord> {
        let _publishing = self.lock();
        let mut record = self.intent(workspace_dir, intent_id)?;
        if record.state != IssuePublishIntentState::Confirmed {
            bail!("the restore link can be finalized only after the publish is confirmed")
        }
        if record.restore_link_finalized {
            return Ok(record);
        }
        for field in &record.pending_vault_fields {
            let scope = |binding| VaultRecordScope {
                issue: record.issue.clone(),
                field: vault_field(*field),
                binding,
            };
            let plaintext = self.vault.load(VaultRecordLoadRequest {
                scope: scope(VaultRecordBinding::PublishIntent(record.intent_id.clone())),
                key: key.clone(),
            })?;
            self.vault.save(VaultRecordSaveRequest {
                scope: scope(VaultRecordBinding::ConfirmedVersion(
                    record.confirmed_version(),
                )),
                key: key.clone(),
                plaintext,
            })?;
        }
        record.restore_link_finalized = true;
        self.journal.save(workspace_dir, &record)?;
        Ok(record)
    }

    /// 복원 가능 구간을 이 의도에 묶어 저장한다. 키가 없거나 저장에 실패하면 `None` 이다.
    fn save_pending_records(
        &self,
        record: &IssuePublishIntentRecord,
        restorable: Vec<IssueRestorableField>,
        vault_key: Option<VaultRecordKey>,
    ) -> Option<Vec<IssuePublishVaultField>> {
        let key = vault_key?;
        let mut fields = Vec::new();
        for item in restorable {
            self.vault
                .save(VaultRecordSaveRequest {
                    scope: VaultRecordScope {
                        issue: record.issue.clone(),
                        field: vault_field(item.field),
                        binding: VaultRecordBinding::PublishIntent(record.intent_id.clone()),
                    },
                    key: key.clone(),
                    plaintext: item.plaintext,
                })
                .ok()?;
            fields.push(item.field);
        }
        Some(fields)
    }

    /// 지금 원격 글의 지문이다. 읽지 못하면 `None` 이다.
    fn observe(&self, issue: &IssueReference) -> Option<IssueFieldHash> {
        self.snapshot
            .read(IssueSnapshotReadRequest {
                issue: issue.clone(),
            })
            .ok()
            .map(|snapshot| snapshot.comparison_basis.field_hash)
    }

    fn intent(&self, workspace_dir: &str, intent_id: &str) -> Result<IssuePublishIntentRecord> {
        self.journal
            .load(workspace_dir)?
            .into_iter()
            .find(|record| record.intent_id == intent_id)
            .ok_or_else(|| anyhow!("the journal has no publish intent `{intent_id}`"))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.publishing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn vault_field(field: IssuePublishVaultField) -> VaultRecordField {
    match field {
        IssuePublishVaultField::Title => VaultRecordField::Title,
        IssuePublishVaultField::Body => VaultRecordField::Body,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::application::port::outbound::evidence_vault_port::{VaultKey, VaultStoreError};
    use crate::application::port::outbound::issue_snapshot_port::{
        IssueRawFields, IssueSnapshotResolveRequest, IssueSnapshotResponse,
    };
    use crate::application::port::outbound::issue_update_port::IssueUpdateResponse;
    use crate::domain::issue_validation::{IssueRequirements, validate_issue};
    use crate::domain::issue_work::{
        IssueProjectScope, IssueProvider, IssuePublishApproval, IssueSharedCopy,
        IssueWriteApproval, IssueWriteOperation,
    };

    const WORKSPACE: &str = "/work/project";

    #[derive(Clone, Copy, PartialEq)]
    enum WriteBehavior {
        /// 쓰고 답을 준다.
        Answer,
        /// 쓰고 답을 잃는다.
        ApplyAndLoseAnswer,
        /// 쓰지 못하고 오류를 준다. 부르는 쪽은 앞의 경우와 구별하지 못한다.
        FailWithoutApply,
    }

    /// 메모리에 원격 이슈 한 건을 가진 가짜이다. 읽기와 쓰기의 횟수를 센다. 네트워크가 없다.
    struct FakeRemoteIssue {
        fields: Mutex<IssueSharedCopy>,
        reads: Mutex<usize>,
        /// 이 번호(0부터)의 읽기부터 실패한다.
        fail_reads_from: Mutex<usize>,
        writes: Mutex<usize>,
        write_behavior: WriteBehavior,
    }

    impl IssueSnapshotPort for FakeRemoteIssue {
        fn resolve(&self, _: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
            unreachable!("the publish path reads by stable ID")
        }

        fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
            let mut reads = self.reads.lock().unwrap();
            *reads += 1;
            if *reads > *self.fail_reads_from.lock().unwrap() {
                bail!("gh could not read the issue")
            }
            let fields = self.fields.lock().unwrap().clone();
            let raw_fields = IssueRawFields::new(fields.title, fields.body);
            Ok(IssueSnapshotResponse {
                issue: request.issue,
                comparison_basis: IssueComparisonBasis {
                    field_hash: raw_fields.field_hash(),
                    provider_updated_at: "2026-11-19T07:19:56Z".to_string(),
                },
                raw_fields,
            })
        }
    }

    impl IssueUpdatePort for FakeRemoteIssue {
        fn update_title_and_body(
            &self,
            request: IssueUpdateRequest,
        ) -> Result<IssueUpdateResponse> {
            request.ensure_authorized()?;
            *self.writes.lock().unwrap() += 1;
            if self.write_behavior != WriteBehavior::FailWithoutApply {
                *self.fields.lock().unwrap() = request.shared_copy.clone();
            }
            if self.write_behavior != WriteBehavior::Answer {
                bail!("the connection closed before the answer")
            }
            Ok(IssueUpdateResponse {
                issue: request.issue,
                published_copy: request.shared_copy,
            })
        }
    }

    #[derive(Default)]
    struct FakeJournal {
        records: Mutex<Vec<IssuePublishIntentRecord>>,
        /// 저장할 때마다 본 상태이다.
        saved_states: Mutex<Vec<IssuePublishIntentState>>,
        fail_save: bool,
    }

    impl IssuePublishIntentJournalPort for FakeJournal {
        fn save(&self, _: &str, record: &IssuePublishIntentRecord) -> Result<()> {
            if self.fail_save {
                bail!("the disk is full")
            }
            self.saved_states.lock().unwrap().push(record.state);
            let mut records = self.records.lock().unwrap();
            records.retain(|existing| existing.intent_id != record.intent_id);
            records.push(record.clone());
            Ok(())
        }

        fn load(&self, _: &str) -> Result<Vec<IssuePublishIntentRecord>> {
            Ok(self.records.lock().unwrap().clone())
        }
    }

    #[derive(Default)]
    struct FakeVault {
        records: Mutex<HashMap<VaultRecordScope, VaultPlaintext>>,
        fail_save: bool,
    }

    impl EvidenceVaultStorePort for FakeVault {
        fn save(&self, request: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
            if self.fail_save {
                return Err(VaultStoreError::Storage("the disk is full".to_string()));
            }
            self.records
                .lock()
                .unwrap()
                .insert(request.scope, request.plaintext);
            Ok(())
        }

        fn load(&self, request: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
            self.records
                .lock()
                .unwrap()
                .get(&request.scope)
                .cloned()
                .ok_or(VaultStoreError::RecordNotFound)
        }
    }

    struct Fixture {
        remote: Arc<FakeRemoteIssue>,
        journal: Arc<FakeJournal>,
        vault: Arc<FakeVault>,
    }

    impl Fixture {
        /// 원격은 `Old title` / `old body` 이다. 사용자는 그 글을 보고 `New title` / `new body` 를 승인했다.
        fn new(write_behavior: WriteBehavior) -> Self {
            Self {
                remote: Arc::new(FakeRemoteIssue {
                    fields: Mutex::new(copy("Old title", "old body")),
                    reads: Mutex::new(0),
                    fail_reads_from: Mutex::new(usize::MAX),
                    writes: Mutex::new(0),
                    write_behavior,
                }),
                journal: Arc::new(FakeJournal::default()),
                vault: Arc::new(FakeVault::default()),
            }
        }

        /// 같은 일지, 원격, Vault 로 만든 새 서비스이다. 데몬의 재시작과 같다.
        fn service(&self) -> IssueJournaledPublishService {
            IssueJournaledPublishService::new(
                self.remote.clone(),
                self.remote.clone(),
                self.journal.clone(),
                self.vault.clone(),
            )
        }

        fn writes(&self) -> usize {
            *self.remote.writes.lock().unwrap()
        }

        fn reads(&self) -> usize {
            *self.remote.reads.lock().unwrap()
        }

        fn fail_reads_from_now(&self) {
            *self.remote.fail_reads_from.lock().unwrap() = self.reads();
        }

        fn saved_states(&self) -> Vec<IssuePublishIntentState> {
            self.journal.saved_states.lock().unwrap().clone()
        }

        fn vault_bindings(&self) -> Vec<VaultRecordBinding> {
            let records = self.vault.records.lock().unwrap();
            records.keys().map(|scope| scope.binding.clone()).collect()
        }
    }

    fn issue() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_1".to_string(),
            issue_id: "I_1".to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: WORKSPACE.to_string(),
            },
        }
    }

    fn copy(title: &str, body: &str) -> IssueSharedCopy {
        IssueSharedCopy {
            title: title.to_string(),
            body: body.to_string(),
        }
    }

    fn key() -> VaultRecordKey {
        VaultRecordKey {
            key_id: "key-1".to_string(),
            key: VaultKey::new(vec![7; 32]),
        }
    }

    /// 복원 가능 구간이 없는 요청이다. Vault 는 없다.
    fn request() -> IssueJournaledPublishRequest {
        let requirements = IssueRequirements {
            template_fields: Vec::new(),
            required_sections: Vec::new(),
        };
        IssueJournaledPublishRequest {
            publish: IssueApprovedPublishRequest {
                issue: issue(),
                validated: validate_issue(&copy("New title", "new body"), &requirements)
                    .expect("valid"),
                approval: IssuePublishApproval {
                    write: IssueWriteApproval {
                        issue: issue(),
                        operation: IssueWriteOperation::Body,
                    },
                    shared_copy_hash: IssueFieldHash::of_fields("New title", "new body"),
                    policy_version: "policy-1".to_string(),
                    template_version: None,
                },
                edit_basis: IssueFieldHash::of_fields("Old title", "old body"),
                policy_version: "policy-1".to_string(),
                template_version: None,
            },
            restorable: Vec::new(),
            vault_key: None,
            shared_copy_only: false,
        }
    }

    /// 본문에 복원 가능 구간이 있는 요청이다.
    fn restorable_request(vault_key: Option<VaultRecordKey>) -> IssueJournaledPublishRequest {
        IssueJournaledPublishRequest {
            restorable: vec![IssueRestorableField {
                field: IssuePublishVaultField::Body,
                plaintext: VaultPlaintext::new("masked log line".to_string()),
            }],
            vault_key,
            ..request()
        }
    }

    fn sent_record() -> IssuePublishIntentRecord {
        IssuePublishIntentRecord {
            intent_id: "intent-before-restart".to_string(),
            issue: issue(),
            previous_hash: IssueFieldHash::of_fields("Old title", "old body"),
            approved_hash: IssueFieldHash::of_fields("New title", "new body"),
            pending_vault_fields: Vec::new(),
            state: IssuePublishIntentState::Sent,
            observed_hash: None,
            restore_link_finalized: false,
        }
    }

    #[test]
    fn the_intent_is_on_disk_before_the_write_and_an_answer_plus_a_reread_confirm_it() {
        use IssuePublishIntentState::*;
        let fixture = Fixture::new(WriteBehavior::Answer);
        let service = fixture.service();

        let outcome = service.publish(restorable_request(Some(key()))).unwrap();

        let IssueJournaledPublishOutcome::Confirmed(record) = outcome else {
            panic!("expected a confirmed publish, got {outcome:?}")
        };
        assert_eq!(record.issue, issue());
        assert_eq!(
            record.previous_hash,
            IssueFieldHash::of_fields("Old title", "old body")
        );
        assert_eq!(
            record.approved_hash,
            IssueFieldHash::of_fields("New title", "new body")
        );
        assert_eq!(record.pending_vault_fields, [IssuePublishVaultField::Body]);
        assert_eq!(fixture.saved_states(), [Recorded, Sent, Confirmed]);
        assert_eq!((fixture.writes(), fixture.reads()), (1, 2));
        assert_eq!(
            fixture.vault_bindings(),
            [VaultRecordBinding::PublishIntent(record.intent_id.clone())]
        );

        // 확인된 게시만 복원 링크를 확정한다. 평문은 확인된 판에 묶인다.
        let finalized = service
            .finalize_restore_link(WORKSPACE, &record.intent_id, key())
            .unwrap();
        assert!(finalized.restore_link_finalized);
        let confirmed_scope = VaultRecordScope {
            issue: issue(),
            field: VaultRecordField::Body,
            binding: VaultRecordBinding::ConfirmedVersion(record.confirmed_version()),
        };
        assert_eq!(
            fixture.vault.records.lock().unwrap()[&confirmed_scope].expose(),
            "masked log line"
        );
    }

    #[test]
    fn a_journal_that_cannot_save_the_intent_stops_the_publish_before_any_remote_call() {
        let mut fixture = Fixture::new(WriteBehavior::Answer);
        fixture.journal = Arc::new(FakeJournal {
            fail_save: true,
            ..FakeJournal::default()
        });

        assert!(fixture.service().publish(request()).is_err());

        assert_eq!((fixture.writes(), fixture.reads()), (0, 0));
    }

    #[test]
    fn a_lost_answer_is_confirmed_when_the_reread_shows_the_approved_copy() {
        let fixture = Fixture::new(WriteBehavior::ApplyAndLoseAnswer);

        let outcome = fixture.service().publish(request()).unwrap();

        assert!(matches!(
            outcome,
            IssueJournaledPublishOutcome::Confirmed(_)
        ));
        assert_eq!(fixture.writes(), 1);
    }

    #[test]
    fn an_unknown_result_blocks_a_duplicate_click_and_the_restore_link_until_the_user_decides() {
        let fixture = Fixture::new(WriteBehavior::FailWithoutApply);
        let service = fixture.service();

        let first = service.publish(restorable_request(Some(key()))).unwrap();
        let IssueJournaledPublishOutcome::NeedsConfirmation(record) = first else {
            panic!("expected an unknown result, got {first:?}")
        };
        assert_eq!(
            record.observed_hash,
            Some(IssueFieldHash::of_fields("Old title", "old body"))
        );

        // 중복 클릭과 다시 확인은 쓰기를 보내지 않는다.
        assert_eq!(
            service.publish(restorable_request(Some(key()))).unwrap(),
            IssueJournaledPublishOutcome::AlreadyPending(record.clone())
        );
        assert_eq!(
            service.reconcile(WORKSPACE).unwrap(),
            std::slice::from_ref(&record)
        );
        assert_eq!(fixture.writes(), 1);

        // 확인되지 않은 게시의 복원 링크는 확정하지 못한다.
        assert!(
            service
                .finalize_restore_link(WORKSPACE, &record.intent_id, key())
                .is_err()
        );
        assert_eq!(
            fixture.vault_bindings(),
            [VaultRecordBinding::PublishIntent(record.intent_id.clone())]
        );

        // 사용자가 버린 뒤의 게시는 새 의도이다.
        let abandoned = service.abandon(WORKSPACE, &record.intent_id).unwrap();
        assert_eq!(abandoned.state, IssuePublishIntentState::Abandoned);
        assert!(service.abandon(WORKSPACE, &record.intent_id).is_err());
        assert!(matches!(
            service.publish(request()).unwrap(),
            IssueJournaledPublishOutcome::NeedsConfirmation(next) if next.intent_id != record.intent_id
        ));
        assert_eq!(fixture.writes(), 2);
    }

    #[test]
    fn a_restart_with_an_unknown_result_rereads_and_never_writes() {
        let fixture = Fixture::new(WriteBehavior::Answer);
        // 데몬이 `Sent` 를 저장한 뒤 답을 받기 전에 죽었다.
        fixture.journal.save(WORKSPACE, &sent_record()).unwrap();

        // 재조회 실패: 확인 필요로 남는다.
        fixture.fail_reads_from_now();
        let service = fixture.service();
        let records = service.reconcile(WORKSPACE).unwrap();
        assert_eq!(records[0].state, IssuePublishIntentState::NeedsConfirmation);
        assert_eq!(
            service.publish(request()).unwrap(),
            IssueJournaledPublishOutcome::AlreadyPending(records[0].clone())
        );

        // 원격이 편집 기준 그대로이다: 쓰기가 늦게 닿을 수 있으므로 확인 필요로 남는다.
        *fixture.remote.fail_reads_from.lock().unwrap() = usize::MAX;
        assert_eq!(
            service.reconcile(WORKSPACE).unwrap()[0].state,
            IssuePublishIntentState::NeedsConfirmation
        );

        // 원격이 승인한 글이다: 확인한다.
        *fixture.remote.fields.lock().unwrap() = copy("New title", "new body");
        assert_eq!(
            service.reconcile(WORKSPACE).unwrap()[0].state,
            IssuePublishIntentState::Confirmed
        );
        assert_eq!(fixture.writes(), 0);
    }

    #[test]
    fn a_failed_reread_after_an_answer_needs_confirmation_and_is_not_resent() {
        let fixture = Fixture::new(WriteBehavior::Answer);
        // 쓰기 전의 읽기는 되고, 쓰기 뒤의 재조회는 실패한다.
        *fixture.remote.fail_reads_from.lock().unwrap() = 1;
        let service = fixture.service();

        let outcome = service.publish(request()).unwrap();

        let IssueJournaledPublishOutcome::NeedsConfirmation(record) = outcome else {
            panic!("expected an unknown result, got {outcome:?}")
        };
        assert_eq!(record.observed_hash, None);
        assert_eq!(
            service.publish(request()).unwrap(),
            IssueJournaledPublishOutcome::AlreadyPending(record)
        );
        assert_eq!(fixture.writes(), 1);
    }

    #[test]
    fn a_failed_read_before_the_write_sends_nothing_and_leaves_no_unresolved_intent() {
        let fixture = Fixture::new(WriteBehavior::Answer);
        fixture.fail_reads_from_now();

        assert!(fixture.service().publish(request()).is_err());

        assert_eq!(fixture.writes(), 0);
        assert_eq!(
            fixture.saved_states(),
            [
                IssuePublishIntentState::Recorded,
                IssuePublishIntentState::Abandoned
            ]
        );
    }

    #[test]
    fn a_remote_copy_that_left_the_edit_basis_is_a_recorded_conflict_and_is_not_overwritten() {
        let fixture = Fixture::new(WriteBehavior::Answer);
        *fixture.remote.fields.lock().unwrap() = copy("Old title", "edited by someone else");

        let outcome = fixture.service().publish(request()).unwrap();

        assert!(matches!(
            outcome,
            IssueJournaledPublishOutcome::Conflict { remote }
                if remote.field_hash == IssueFieldHash::of_fields("Old title", "edited by someone else")
        ));
        assert_eq!(fixture.writes(), 0);
        assert_eq!(
            fixture.saved_states().last(),
            Some(&IssuePublishIntentState::Conflict)
        );
    }

    #[test]
    fn a_shared_copy_without_restorable_spans_is_published_while_the_vault_is_down() {
        let mut fixture = Fixture::new(WriteBehavior::Answer);
        fixture.vault = Arc::new(FakeVault {
            fail_save: true,
            ..FakeVault::default()
        });

        let outcome = fixture.service().publish(request()).unwrap();

        assert!(matches!(
            outcome,
            IssueJournaledPublishOutcome::Confirmed(record) if record.pending_vault_fields.is_empty()
        ));
    }

    #[test]
    fn a_restorable_span_that_cannot_be_saved_holds_the_publish_until_the_user_confirms() {
        let mut save_fails = Fixture::new(WriteBehavior::Answer);
        save_fails.vault = Arc::new(FakeVault {
            fail_save: true,
            ..FakeVault::default()
        });
        // Vault 의 저장 실패와 키가 없는 Vault 는 같은 보류이다.
        for (fixture, vault_key) in [
            (save_fails, Some(key())),
            (Fixture::new(WriteBehavior::Answer), None),
        ] {
            let service = fixture.service();

            let held = service
                .publish(restorable_request(vault_key.clone()))
                .unwrap();

            assert!(matches!(
                held,
                IssueJournaledPublishOutcome::Held(record)
                    if record.state == IssuePublishIntentState::Held
            ));
            assert_eq!((fixture.writes(), fixture.reads()), (0, 0));

            // 사용자가 "공유본만 게시"를 확인했다. 복원 연결이 없는 게시이다.
            let confirmed = service
                .publish(IssueJournaledPublishRequest {
                    shared_copy_only: true,
                    ..restorable_request(vault_key)
                })
                .unwrap();
            assert!(matches!(
                confirmed,
                IssueJournaledPublishOutcome::Confirmed(record)
                    if record.pending_vault_fields.is_empty()
            ));
            assert_eq!(fixture.writes(), 1);
            assert!(fixture.vault_bindings().is_empty());
        }
    }

    #[test]
    fn a_value_that_differs_from_the_approval_is_refused_before_the_journal_and_the_remote() {
        let fixture = Fixture::new(WriteBehavior::Answer);
        let mut request = request();
        request.publish.policy_version = "policy-2".to_string();

        assert_eq!(
            fixture.service().publish(request).unwrap(),
            IssueJournaledPublishOutcome::Refused(IssuePublishRefusal::PolicyVersion)
        );
        assert_eq!((fixture.writes(), fixture.reads()), (0, 0));
        assert!(fixture.saved_states().is_empty());
    }
}
