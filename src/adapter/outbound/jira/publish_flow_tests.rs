//! Jira Cloud 이슈의 게시와 로컬 복원을 실제 서비스와 Jira 어댑터로 잇는 시험이다 (#2434). Jira 는 메모리 가짜이다.
//! 네트워크가 없다. 값은 시험용으로 만든 것이다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::{JiraIssueSnapshotAdapter, JiraIssueUpdateAdapter};
use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultKey, VaultPlaintext, VaultRecordLoadRequest,
    VaultRecordSaveRequest, VaultRecordScope, VaultRestoreResponse, VaultStoreError,
};
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotReadRequest,
};
use crate::application::service::evidence_vault_restore::{
    EvidenceVaultRestoreService, VaultRestoreHold, restorable_fields,
};
use crate::application::service::evidence_vault_unlock::{
    EvidenceVaultUnlockService, VaultUnlockOutcome, VaultUnlockRequest, VaultUserDecision,
    VaultViewingSessionId,
};
use crate::application::service::issue_masking::{
    IssueMaskingOutcome, IssueSensitiveSpan, IssueTextField, mask_issue_text,
};
use crate::application::service::issue_publish::IssueApprovedPublishRequest;
use crate::application::service::issue_publish_journal::{
    IssueJournaledPublishOutcome, IssueJournaledPublishRequest, IssueJournaledPublishService,
};
use crate::domain::issue_policy::{IssueDisclosurePolicy, IssueSensitiveSpanAction};
use crate::domain::issue_publish_intent::{IssuePublishIntentRecord, IssuePublishIntentState};
use crate::domain::issue_validation::{IssueRequirements, validate_issue};
use crate::domain::issue_work::{
    IssueFieldHash, IssueProjectScope, IssueProvider, IssuePublishApproval, IssueReference,
    IssueWriteApproval, IssueWriteOperation,
};

const WORKSPACE: &str = "/work/project";
const CLOUD_ID: &str = "11111111-2222-3333-4444-555555555555";
const OTHER_CLOUD_ID: &str = "99999999-2222-3333-4444-555555555555";
const TITLE: &str = "Build fails for alice";
const BODY: &str =
    "## Steps\n\nalice ran **the tool** in /home/alice/app.\n\n1. open *.log\n2. run `make`";
const MASKED_TITLE: &str = "Build fails for [[akra-mask:1]]";
const MASKED_BODY: &str = "## Steps\n\n[[akra-mask:2]] ran **the tool** in [[akra-mask:3]].\n\n1. open \\*.log\n2. run `make`";

#[derive(Clone, Copy, PartialEq)]
enum PutBehavior {
    /// 쓰고 답을 준다.
    Answer,
    /// 쓰고 답을 잃는다.
    ApplyAndLoseAnswer,
    /// 쓰지 못하고 오류를 준다. 부르는 쪽은 앞의 경우와 구별하지 못한다.
    FailWithoutApply,
}

/// 메모리에 이슈 한 건을 가진 Jira 사이트이다.
struct FakeJira {
    cloud_id: &'static str,
    issue: Mutex<Value>,
    puts: Mutex<Vec<Value>>,
    put_behavior: PutBehavior,
}

impl FakeJira {
    fn new(cloud_id: &'static str, description: Value, put_behavior: PutBehavior) -> Arc<Self> {
        Arc::new(Self {
            cloud_id,
            issue: Mutex::new(json!({
                "id": "10002",
                "key": "DEMO-7",
                "fields": {
                    "summary": "Old title",
                    "description": description,
                    "updated": "2026-11-19T07:19:56.000+0000",
                    "project": { "id": "10000", "key": "DEMO" }
                }
            })),
            puts: Mutex::default(),
            put_behavior,
        })
    }

    fn get(&self, path: &str) -> Result<Vec<u8>> {
        let answer = if path == "/_edge/tenant_info" {
            json!({ "cloudId": self.cloud_id })
        } else if path == "/rest/api/3/issue/10002/editmeta" {
            json!({ "fields": {
                "summary": { "required": true, "schema": { "type": "string" } },
                "description": { "schema": { "type": "string" } }
            } })
        } else if path.starts_with("/rest/api/3/issue/10002?") {
            self.issue.lock().unwrap().clone()
        } else {
            bail!("404")
        };
        Ok(answer.to_string().into_bytes())
    }

    fn put(&self, path: &str, body: &[u8]) -> Result<()> {
        assert_eq!(path, "/rest/api/3/issue/10002");
        let fields = serde_json::from_slice::<Value>(body)?["fields"].clone();
        self.puts.lock().unwrap().push(fields.clone());
        if self.put_behavior != PutBehavior::FailWithoutApply {
            self.store(&fields);
        }
        if self.put_behavior != PutBehavior::Answer {
            bail!("the connection closed before the answer")
        }
        Ok(())
    }

    /// 제목과 본문을 바꾼다. 다른 사람의 편집과 같다. 본문은 공급자의 방식으로 다듬어 둔다.
    fn store(&self, fields: &Value) {
        let mut description = fields["description"].clone();
        normalize(&mut description);
        let mut issue = self.issue.lock().unwrap();
        issue["fields"]["summary"] = fields["summary"].clone();
        issue["fields"]["description"] = description;
        issue["fields"]["updated"] = json!("2026-11-20T01:02:03.000+0000");
    }

    fn put_count(&self) -> usize {
        self.puts.lock().unwrap().len()
    }

    fn stored(&self) -> String {
        self.issue.lock().unwrap().to_string()
    }
}

/// 공급자가 문서를 저장하며 바꾸는 것을 흉내 낸다: 표시의 순서, 기본값인 속성, 노드마다 붙는 ID.
fn normalize(node: &mut Value) {
    if let Some(marks) = node.get_mut("marks").and_then(Value::as_array_mut) {
        marks.reverse();
    }
    if node["type"] == "orderedList" && node["attrs"]["order"] == 1 {
        node.as_object_mut().unwrap().remove("attrs");
    }
    if node["type"] == "paragraph" {
        node["attrs"] = json!({ "localId": "generated" });
    }
    if let Some(children) = node.get_mut("content").and_then(Value::as_array_mut) {
        children.iter_mut().for_each(normalize);
    }
}

fn paragraph_document(text: &str) -> Value {
    json!({ "type": "doc", "version": 1, "content": [
        { "type": "paragraph", "content": [{ "type": "text", "text": text }] }
    ] })
}

fn snapshot_adapter(jira: &Arc<FakeJira>) -> JiraIssueSnapshotAdapter {
    let jira = jira.clone();
    JiraIssueSnapshotAdapter::new(
        "https://jira.example.invalid".to_string(),
        Box::new(move |path| jira.get(path)),
    )
}

#[derive(Default)]
struct FakeJournal {
    records: Mutex<Vec<IssuePublishIntentRecord>>,
}

impl IssuePublishIntentJournalPort for FakeJournal {
    fn save(&self, _: &str, record: &IssuePublishIntentRecord) -> Result<()> {
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
}

impl EvidenceVaultStorePort for FakeVault {
    fn begin_binding(
        &self,
        request: VaultRecordLoadRequest,
    ) -> Result<
        Box<
            dyn crate::application::port::outbound::evidence_vault_port::VaultBindingTransaction
                + '_,
        >,
        VaultStoreError,
    > {
        crate::test_utils::vault_binding::begin_test_binding(
            self.records.lock().unwrap(),
            request,
            crate::test_utils::vault_binding::TestBindingHooks::default(),
        )
    }

    fn save(&self, request: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
        let mut records = self.records.lock().unwrap();
        records.insert(request.scope, request.plaintext);
        Ok(())
    }

    fn load(&self, request: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
        let records = self.records.lock().unwrap();
        records
            .get(&request.scope)
            .cloned()
            .ok_or(VaultStoreError::RecordNotFound)
    }
}

struct Fixture {
    jira: Arc<FakeJira>,
    journal: Arc<FakeJournal>,
    vault: Arc<FakeVault>,
    unlock: Arc<EvidenceVaultUnlockService>,
    session: VaultViewingSessionId,
}

impl Fixture {
    /// 원격은 `Old title` / `old body` 이다.
    fn new(put_behavior: PutBehavior) -> Self {
        Self::with_description(paragraph_document("old body"), put_behavior)
    }

    fn with_description(description: Value, put_behavior: PutBehavior) -> Self {
        let unlock = Arc::new(EvidenceVaultUnlockService::new(Arc::new(
            FakeVaultKeyStoreAdapter::with_key(VaultKey::new(vec![7; 32])),
        )));
        Self {
            jira: FakeJira::new(CLOUD_ID, description, put_behavior),
            journal: Arc::default(),
            vault: Arc::default(),
            session: unlock.begin_session(),
            unlock,
        }
    }

    /// 같은 일지, 원격, Vault 로 만든 새 서비스이다. 데몬의 재시작과 같다.
    fn publisher(&self) -> IssueJournaledPublishService {
        let jira = self.jira.clone();
        IssueJournaledPublishService::new(
            Arc::new(snapshot_adapter(&self.jira)),
            Arc::new(JiraIssueUpdateAdapter::new(
                snapshot_adapter(&self.jira),
                Box::new(move |path, body| jira.put(path, body)),
            )),
            self.journal.clone(),
            self.vault.clone(),
        )
    }

    /// 지금 원격 글의 지문이다. 사용자가 편집을 시작한 기준이다.
    fn remote_hash(&self) -> IssueFieldHash {
        snapshot_adapter(&self.jira)
            .read(IssueSnapshotReadRequest { issue: issue() })
            .unwrap()
            .comparison_basis
            .field_hash
    }

    /// `TITLE` / `BODY` 를 가린 공유본을 승인한 요청이다.
    fn request(&self, edit_basis: IssueFieldHash) -> IssueJournaledPublishRequest {
        let masked = masked();
        let requirements = IssueRequirements {
            template_fields: Vec::new(),
            required_sections: Vec::new(),
        };
        IssueJournaledPublishRequest {
            publish: IssueApprovedPublishRequest {
                issue: issue(),
                validated: validate_issue(&masked.shared_copy, &requirements).expect("valid"),
                approval: IssuePublishApproval {
                    write: IssueWriteApproval {
                        issue: issue(),
                        operation: IssueWriteOperation::Body,
                    },
                    shared_copy_hash: IssueFieldHash::of_fields(MASKED_TITLE, MASKED_BODY),
                    policy_version: "policy-1".to_string(),
                    template_version: None,
                },
                edit_basis,
                policy_version: "policy-1".to_string(),
                template_version: None,
            },
            restorable: restorable_fields(&masked.masked_spans),
            vault_key: Some(self.unlock.sealing_key().unwrap()),
            shared_copy_only: false,
        }
    }

    fn publish(&self) -> IssueJournaledPublishOutcome {
        self.publisher()
            .publish(self.request(self.remote_hash()))
            .unwrap()
    }

    fn finalize(&self, intent_id: &str) -> Result<IssuePublishIntentRecord> {
        let key = self.unlock.sealing_key().unwrap();
        self.publisher()
            .finalize_restore_link(WORKSPACE, intent_id, key)
    }

    /// `jira` 사이트에서 읽은 `issue` 의 로컬 복원본이다.
    fn restore(
        &self,
        jira: &Arc<FakeJira>,
        issue: IssueReference,
    ) -> Result<VaultRestoreResponse, VaultRestoreHold> {
        let service = EvidenceVaultRestoreService::new(
            Arc::new(snapshot_adapter(jira)),
            self.vault.clone(),
            self.unlock.clone(),
        );
        let VaultUnlockOutcome::Unlocked(grant) = self.unlock.unlock(VaultUnlockRequest {
            issue,
            session: self.session.clone(),
            decision: VaultUserDecision::Confirmed,
        }) else {
            panic!("not unlocked")
        };
        service.restore_for_grant(&grant)
    }

    /// 게시를 확인하고 복원 링크를 확정했다.
    fn published() -> Self {
        let fixture = Self::new(PutBehavior::Answer);
        let IssueJournaledPublishOutcome::Confirmed(record) = fixture.publish() else {
            panic!("not confirmed")
        };
        fixture.finalize(&record.intent_id).unwrap();
        fixture
    }
}

fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::Jira,
        server_id: CLOUD_ID.to_string(),
        repository_id: "10000".to_string(),
        issue_id: "10002".to_string(),
        project_scope: IssueProjectScope {
            workspace_dir: WORKSPACE.to_string(),
        },
    }
}

fn masked() -> IssueMaskingOutcome {
    let sensitive = |field, text: &str, needle: &str| {
        let start = text.find(needle).unwrap();
        IssueSensitiveSpan {
            field,
            range: start..start + needle.len(),
            class: None,
            user_decision: Some(IssueSensitiveSpanAction::RestorableMask),
        }
    };
    // 본문은 Jira 표현의 Markdown 이다: 글자 그대로의 `*` 에는 `\` 가 붙는다.
    let body = BODY.replace("*.log", "\\*.log");
    let outcome = mask_issue_text(
        TITLE,
        &body,
        &[
            sensitive(IssueTextField::Title, TITLE, "alice"),
            sensitive(IssueTextField::Body, &body, "alice"),
            sensitive(IssueTextField::Body, &body, "/home/alice/app"),
        ],
        &IssueDisclosurePolicy::builtin(),
    )
    .unwrap();
    assert_eq!(outcome.shared_copy.title, MASKED_TITLE);
    assert_eq!(outcome.shared_copy.body, MASKED_BODY);
    outcome
}

fn state_of(outcome: &IssueJournaledPublishOutcome) -> Option<IssuePublishIntentState> {
    match outcome {
        IssueJournaledPublishOutcome::Confirmed(record)
        | IssueJournaledPublishOutcome::NeedsConfirmation(record)
        | IssueJournaledPublishOutcome::AlreadyPending(record)
        | IssueJournaledPublishOutcome::Held(record) => Some(record.state),
        _ => None,
    }
}

#[test]
fn the_approved_copy_is_sent_once_confirmed_after_normalization_and_restored_locally() {
    let fixture = Fixture::published();

    // 보낸 값은 승인한 공유본의 제목과 본문뿐이다. 가린 평문은 원격에 없다.
    let puts = fixture.jira.puts.lock().unwrap().clone();
    assert_eq!(puts.len(), 1);
    let sent: Vec<&String> = puts[0].as_object().unwrap().keys().collect();
    assert_eq!(sent, ["description", "summary"]);
    assert_eq!(puts[0]["summary"], MASKED_TITLE);
    assert!(!fixture.jira.stored().contains("alice"));
    // 공급자가 문서를 다듬었어도 다시 읽은 글의 해시는 승인한 글과 같다.
    assert!(fixture.jira.stored().contains("localId"));
    assert_eq!(
        fixture.remote_hash(),
        IssueFieldHash::of_fields(MASKED_TITLE, MASKED_BODY)
    );

    let restored = fixture.restore(&fixture.jira, issue()).unwrap();
    assert_eq!(restored.title.expose(), TITLE);
    assert_eq!(
        restored.body.expose(),
        BODY.replace("*.log", "\\*.log").as_str()
    );
}

#[test]
fn a_lost_answer_is_settled_by_a_reread_and_never_by_a_second_write() {
    // 쓰기는 되었고 답을 잃었다. 재조회가 승인한 글을 본다.
    let applied = Fixture::new(PutBehavior::ApplyAndLoseAnswer);
    let outcome = applied.publish();
    assert_eq!(state_of(&outcome), Some(IssuePublishIntentState::Confirmed));
    assert_eq!(applied.jira.put_count(), 1);

    // 쓰기가 되지 않았다. 결과를 모르는 상태로 남고, 복원 링크는 없다.
    let lost = Fixture::new(PutBehavior::FailWithoutApply);
    let IssueJournaledPublishOutcome::NeedsConfirmation(record) = lost.publish() else {
        panic!("not waiting for confirmation")
    };
    assert!(lost.finalize(&record.intent_id).is_err());
    assert_eq!(
        lost.restore(&lost.jira, issue()),
        Err(VaultRestoreHold::NotBound)
    );
    // 재시작 뒤의 재확인과 중복 게시는 읽기만 한다.
    let reconciled = lost.publisher().reconcile(WORKSPACE).unwrap();
    assert_eq!(
        reconciled[0].state,
        IssuePublishIntentState::NeedsConfirmation
    );
    assert_eq!(
        state_of(&lost.publish()),
        Some(IssuePublishIntentState::NeedsConfirmation)
    );
    assert_eq!(lost.jira.put_count(), 1);

    // 늦게 반영된 쓰기는 다음 재확인에서 확인된다. 그 뒤에 복원 링크를 확정한다.
    lost.jira.store(&lost.jira.puts.lock().unwrap()[0]);
    let reconciled = lost.publisher().reconcile(WORKSPACE).unwrap();
    assert_eq!(reconciled[0].state, IssuePublishIntentState::Confirmed);
    lost.finalize(&record.intent_id).unwrap();
    assert_eq!(
        lost.restore(&lost.jira, issue()).unwrap().title.expose(),
        TITLE
    );
    assert_eq!(lost.jira.put_count(), 1);
}

#[test]
fn an_external_edit_stops_the_write_and_later_holds_the_restore() {
    // 사용자가 편집하는 동안 다른 사람이 고쳤다. 쓰지 않는다.
    let fixture = Fixture::new(PutBehavior::Answer);
    let edit_basis = fixture.remote_hash();
    fixture.jira.store(&json!({
        "summary": "Old title",
        "description": paragraph_document("edited by someone else")
    }));
    let outcome = fixture
        .publisher()
        .publish(fixture.request(edit_basis))
        .unwrap();
    assert!(matches!(
        outcome,
        IssueJournaledPublishOutcome::Conflict { .. }
    ));
    assert_eq!(fixture.jira.put_count(), 0);

    // 게시한 뒤에 다른 사람이 본문을 고쳤다. 표식은 그대로 있지만 복원을 보류한다.
    let fixture = Fixture::published();
    let mut edited = fixture.jira.puts.lock().unwrap()[0].clone();
    edited["description"]["content"]
        .as_array_mut()
        .unwrap()
        .push(paragraph_document("One more line.")["content"][0].clone());
    fixture.jira.store(&edited);
    assert_eq!(
        fixture.restore(&fixture.jira, issue()),
        Err(VaultRestoreHold::PublishedCopyChanged)
    );
}

#[test]
fn a_copy_on_another_jira_site_has_no_restore_binding() {
    let fixture = Fixture::published();
    // 같은 제목과 본문(표식 포함)을 다른 사이트의 같은 번호 이슈에 복사했다.
    let other_site = FakeJira::new(OTHER_CLOUD_ID, Value::Null, PutBehavior::Answer);
    other_site.store(&fixture.jira.puts.lock().unwrap()[0]);
    let copied = IssueReference {
        server_id: OTHER_CLOUD_ID.to_string(),
        ..issue()
    };

    assert_eq!(
        fixture.restore(&other_site, copied.clone()),
        Err(VaultRestoreHold::NotBound)
    );
    // 사이트 주소가 가리키는 cloud ID 가 참조와 다르면 읽기부터 실패한다.
    assert_eq!(
        fixture.restore(&fixture.jira, copied),
        Err(VaultRestoreHold::LatestReadFailed)
    );
    assert_eq!(
        fixture.restore(&other_site, issue()),
        Err(VaultRestoreHold::LatestReadFailed)
    );
    // 원래 이슈의 복원은 그대로이다.
    assert!(fixture.restore(&fixture.jira, issue()).is_ok());
}

#[test]
fn a_remote_body_with_unsupported_content_goes_to_review_and_is_not_overwritten() {
    let description = json!({ "type": "doc", "version": 1, "content": [
        { "type": "paragraph", "content": [{ "type": "text", "text": "old body" }] },
        { "type": "panel", "content": [] }
    ] });
    let fixture = Fixture::with_description(description, PutBehavior::Answer);
    let before = fixture.jira.stored();

    let IssueJournaledPublishOutcome::NeedsConfirmation(record) = fixture.publish() else {
        panic!("not sent to review")
    };

    assert_eq!(fixture.jira.put_count(), 0);
    assert_eq!(fixture.jira.stored(), before);
    assert_eq!(record.observed_hash, Some(record.previous_hash.clone()));
    assert!(fixture.finalize(&record.intent_id).is_err());
    // 사용자가 검토를 끝내면 이 의도를 버린다. 그 뒤의 게시는 새 승인으로 한다.
    let abandoned = fixture
        .publisher()
        .abandon(WORKSPACE, &record.intent_id)
        .unwrap();
    assert_eq!(abandoned.state, IssuePublishIntentState::Abandoned);
}
