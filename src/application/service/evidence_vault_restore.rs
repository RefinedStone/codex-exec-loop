/*
 * Evidence Vault 의 복원 서비스이다 (#2425). 복원 직전에 최신 게시본을 읽고, 저장한 연결과 같을 때만 평문을 준다.
 *
 * 연결은 세 값이다:
 * - `CurrentVersion` 레코드: 이 이슈(프로젝트, 서버, 저장소, 이슈)에서 지금 복원에 쓰는 판의 이름이다.
 * - `ConfirmedVersion(판)` 레코드: 필드마다 하나이다. 가림 구간의 묶음(표식, 바이트 위치, 평문)이다.
 * - 판의 이름: 게시본 해시와 저장 transaction의 불변 generation을 함께 식별한다.
 *
 * 최신 게시본의 해시가 `CurrentVersion` 과 다르거나, 표식이 저장한 위치에 없거나, 최신 조회가 실패하면 복원을
 * 보류한다 (`VaultRestoreHold`). 보류는 평문을 주지 않는다. 이 서비스는 게시본을 저장하지 않는다. 이전 조회의
 * 답을 근거로 쓰지 않는다.
 *
 * 재연결은 같은 이슈에서만 한다. 사용자가 검토한 게시본의 해시와 승인한 연결을 새 판에 저장하고, 마지막에
 * `CurrentVersion` 을 바꾼다. 그 저장 전에 멈추면 이전 연결이 남는다.
 *
 * 이 파일의 타입은 `Serialize` 를 구현하지 않는다. Kernel 이벤트와 Gateway 응답에 넣지 않는다.
 */
use std::fmt;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultRestorePort, EvidenceVaultStorePort, VaultBindingCommitPolicy,
    VaultBindingCommitRequest, VaultBindingCommitResult, VaultBindingFields, VaultBindingSnapshot,
    VaultBindingState, VaultBindingToken, VaultBindingTransaction, VaultPlaintext,
    VaultRecordField, VaultRecordKey, VaultRecordLoadRequest, VaultRecordScope,
    VaultRestoreRequest, VaultRestoreResponse, VaultStoreError,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest,
};
use crate::application::service::evidence_vault_unlock::{
    EvidenceVaultUnlockService, VaultSharedCopyReason, VaultUnlockGrant,
};
use crate::application::service::issue_masking::{IssueMaskedSpan, IssueTextField};
use crate::application::service::issue_publish_journal::IssueRestorableField;
use crate::domain::issue_publish_intent::{
    IssuePublishVaultField, confirmed_version_matches_hash, confirmed_version_name,
};
use crate::domain::issue_work::{IssueFieldHash, IssueProvider, IssueReference};

const SPAN_BUNDLE_HEADER: &str = "akra.vault.spans.v1";
/// 검토 화면에 보이는 표식 앞뒤 글의 최대 바이트 수이다.
const REVIEW_CONTEXT_BYTES: usize = 80;
const FIELDS: [VaultRecordField; 2] = [VaultRecordField::Title, VaultRecordField::Body];

/// 가림 구간 하나의 연결이다: 게시본의 필드, 표식, 표식이 시작하는 바이트 위치, 가린 평문.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRestorableSpan {
    pub field: VaultRecordField,
    pub marker: String,
    pub position: usize,
    pub plaintext: VaultPlaintext,
}

/// 복원을 보류한 이유이다. 어떤 값도 평문이나 게시본의 글을 담지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultRestoreHold {
    /// 잠금 해제 권한이 없거나 끝났다.
    Locked(VaultSharedCopyReason),
    /// 최신 게시본을 읽지 못했다. 이전에 읽은 글로 복원하지 않는다.
    LatestReadFailed,
    /// 이 프로젝트, 서버, 저장소, 이슈에 묶인 연결이 없다. 다른 이슈로 복사한 글이 이 상태이다.
    NotBound,
    /// 게시본의 글이 연결한 판과 다르다. 표식은 모두 한 번씩, 같은 순서로 있다.
    PublishedCopyChanged,
    /// 표식이 다른 필드나 다른 순서로 옮겨졌거나, 저장한 위치에 없다.
    MarkerMoved,
    /// 같은 표식이 두 번 이상 있다.
    MarkerDuplicated,
    /// 표식이 게시본에 없다.
    MarkerMissing,
    /// 사용자가 검토한 뒤에 게시본이나 로컬 연결이 바뀌었다. 재연결을 확정하지 않았다.
    ChangedDuringReview,
    /// 승인한 연결이 저장한 표식이나 지금 게시본의 위치와 맞지 않는다. 재연결을 확정하지 않았다.
    MappingRejected,
    /// 연결 레코드를 읽거나 쓰지 못했다.
    RecordUnavailable(VaultStoreError),
}

impl fmt::Display for VaultRestoreHold {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked(reason) => write!(formatter, "the vault is locked: {reason:?}"),
            Self::LatestReadFailed => {
                formatter.write_str("the latest published copy could not be read")
            }
            Self::NotBound => formatter.write_str("this issue has no restore binding"),
            Self::PublishedCopyChanged => {
                formatter.write_str("the published copy differs from the bound version")
            }
            Self::MarkerMoved => formatter.write_str("a mask marker moved"),
            Self::MarkerDuplicated => formatter.write_str("a mask marker is duplicated"),
            Self::MarkerMissing => formatter.write_str("a mask marker is missing"),
            Self::ChangedDuringReview => formatter.write_str(
                "the published copy or local binding changed during the reconnect review",
            ),
            Self::MappingRejected => {
                formatter.write_str("the approved mapping does not match the published copy")
            }
            Self::RecordUnavailable(error) => {
                write!(formatter, "the restore binding record failed: {error}")
            }
        }
    }
}

impl std::error::Error for VaultRestoreHold {}

/// 게시본에서 표식이 있는 자리이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultMarkerPlace {
    pub field: VaultRecordField,
    /// 필드의 글에서 표식이 시작하는 바이트 위치이다.
    pub position: usize,
}

/// 재연결 검토에서 표식 하나의 상태이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultReconnectMarker {
    pub marker: String,
    /// 연결한 판에서의 자리이다.
    pub previous: VaultMarkerPlace,
    /// 지금 게시본에서 이 표식이 있는 모든 자리이다. 없으면 비어 있다.
    pub candidates: Vec<VaultMarkerPlace>,
}

/// 재연결 검토에 보이는 값이다. 평문은 없다. 게시본의 글은 다른 사람이 고친 원시 값이다 (`Debug` 가 가린다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultReconnectReview {
    pub issue: IssueReference,
    /// 검토한 게시본의 필드 해시이다. 승인은 이 값에 묶인다.
    pub reviewed_hash: IssueFieldHash,
    pub published: IssueRawFields,
    pub markers: Vec<VaultReconnectMarker>,
    // 공개 검토 값을 수정해도 저장의 대상, 원본 판, CAS 기준을 바꿀 수 없다.
    basis: VaultReconnectBasis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VaultReconnectBasis {
    issue: IssueReference,
    expected: VaultBindingToken,
    source_version: String,
    reviewed_hash: IssueFieldHash,
}

impl VaultReconnectReview {
    /// 지금 게시본에서 `place` 앞뒤의 글이다. 신뢰된 로컬 검토 화면만 부른다.
    pub fn context(&self, place: &VaultMarkerPlace) -> &str {
        let text = field_text(&self.published, place.field);
        let mut start = place
            .position
            .saturating_sub(REVIEW_CONTEXT_BYTES)
            .min(text.len());
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        let mut end = place
            .position
            .saturating_add(REVIEW_CONTEXT_BYTES)
            .min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        text.get(start..end).unwrap_or("")
    }
}

/// 사용자가 승인한 연결 하나이다: 저장한 표식과 지금 게시본에서의 자리.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultApprovedMapping {
    pub marker: String,
    pub place: VaultMarkerPlace,
}

/// 사용자의 재연결 승인이다. 검토한 게시본의 해시와 승인한 연결 전체를 묶는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultReconnectApproval {
    basis: VaultReconnectBasis,
    // 승인하지 않은 표식은 새 판에 연결하지 않고 복원본에서도 표식으로 남긴다.
    mappings: Vec<VaultApprovedMapping>,
    operation_id: String,
    approval_digest: [u8; 32],
}

impl VaultReconnectApproval {
    /// 한 검토의 대상, 원본 연결, 게시본과 사용자가 고른 전체 mapping을 고정한다.
    pub fn from_review(
        review: &VaultReconnectReview,
        mut mappings: Vec<VaultApprovedMapping>,
    ) -> Result<Self, VaultRestoreHold> {
        if review.issue != review.basis.issue
            || review.reviewed_hash != review.basis.reviewed_hash
            || review.published.field_hash() != review.basis.reviewed_hash
        {
            return Err(VaultRestoreHold::MappingRejected);
        }
        mappings.sort_by(|left, right| left.marker.cmp(&right.marker));
        if mappings
            .windows(2)
            .any(|pair| pair[0].marker == pair[1].marker)
            || mappings.iter().any(|mapping| {
                !review.markers.iter().any(|marker| {
                    marker.marker == mapping.marker && marker.candidates.contains(&mapping.place)
                })
            })
        {
            return Err(VaultRestoreHold::MappingRejected);
        }
        let (operation_id, approval_digest) = reconnect_identity(&review.basis, &mappings);
        Ok(Self {
            basis: review.basis.clone(),
            mappings,
            operation_id,
            approval_digest,
        })
    }
}

fn reconnect_identity(
    basis: &VaultReconnectBasis,
    mappings: &[VaultApprovedMapping],
) -> (String, [u8; 32]) {
    fn append(digest: &mut Sha256, value: &str) {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }

    let mut digest = Sha256::new();
    digest.update(b"akra.vault.reconnect.approval.v1\n");
    for value in [
        match basis.issue.provider {
            IssueProvider::GitHub => "github",
            IssueProvider::Jira => "jira",
        },
        &basis.issue.server_id,
        &basis.issue.repository_id,
        &basis.issue.issue_id,
        &basis.issue.project_scope.workspace_dir,
        &basis.source_version,
        &basis.reviewed_hash.sha256,
    ] {
        append(&mut digest, value);
    }
    digest.update(basis.reviewed_hash.version.to_be_bytes());
    digest.update((mappings.len() as u64).to_be_bytes());
    for mapping in mappings {
        append(&mut digest, &mapping.marker);
        digest.update([match mapping.place.field {
            VaultRecordField::Title => 0,
            VaultRecordField::Body => 1,
        }]);
        digest.update((mapping.place.position as u64).to_be_bytes());
    }
    // expected token은 저장 transaction이 별도로 동일 승인 식별에 포함한다.
    let digest = digest.finalize();
    (format!("reconnect:{digest:x}"), digest.into())
}

/// 가림 결과를 게시 요청의 필드별 묶음으로 만든다 (`IssueJournaledPublishRequest::restorable`).
pub fn restorable_fields(masked_spans: &[IssueMaskedSpan]) -> Vec<IssueRestorableField> {
    [
        (IssueTextField::Title, IssuePublishVaultField::Title),
        (IssueTextField::Body, IssuePublishVaultField::Body),
    ]
    .into_iter()
    .filter_map(|(text_field, field)| {
        let spans = masked_spans
            .iter()
            .filter(|span| span.field == text_field)
            .map(|span| VaultRestorableSpan {
                field: record_field(text_field),
                marker: span.marker.clone(),
                position: span.position,
                plaintext: span.plaintext.clone(),
            })
            .collect::<Vec<_>>();
        (!spans.is_empty()).then(|| IssueRestorableField {
            field,
            plaintext: seal_field_spans(&spans),
        })
    })
    .collect()
}

fn record_field(field: IssueTextField) -> VaultRecordField {
    match field {
        IssueTextField::Title => VaultRecordField::Title,
        IssueTextField::Body => VaultRecordField::Body,
    }
}

/// 필드 하나의 가림 구간을 레코드 한 건의 평문으로 묶는다. 구간마다 `위치 표식길이 평문길이` 한 줄, 그다음
/// 표식과 평문의 바이트이다.
fn seal_field_spans(spans: &[VaultRestorableSpan]) -> VaultPlaintext {
    let mut bundle = format!("{SPAN_BUNDLE_HEADER}\n");
    for span in spans {
        let plaintext = span.plaintext.expose();
        bundle.push_str(&format!(
            "{} {} {}\n{}{}\n",
            span.position,
            span.marker.len(),
            plaintext.len(),
            span.marker,
            plaintext
        ));
    }
    VaultPlaintext::new(bundle)
}

/// 묶음을 구간으로 푼다. 형식이 다르면 `None` 이다.
fn open_field_spans(
    field: VaultRecordField,
    bundle: &VaultPlaintext,
) -> Option<Vec<VaultRestorableSpan>> {
    let mut rest = bundle
        .expose()
        .strip_prefix(SPAN_BUNDLE_HEADER)?
        .strip_prefix('\n')?;
    let mut spans = Vec::new();
    while !rest.is_empty() {
        let (head, tail) = rest.split_once('\n')?;
        let mut numbers = head.split(' ').map(|number| number.parse::<usize>().ok());
        let position = numbers.next()??;
        let marker_end = numbers.next()??;
        let plaintext_end = marker_end.checked_add(numbers.next()??)?;
        if marker_end == 0 || numbers.next().is_some() {
            return None;
        }
        spans.push(VaultRestorableSpan {
            field,
            marker: tail.get(..marker_end)?.to_string(),
            position,
            plaintext: VaultPlaintext::new(tail.get(marker_end..plaintext_end)?.to_string()),
        });
        rest = tail.get(plaintext_end..)?.strip_prefix('\n')?;
    }
    Some(spans)
}

fn field_text(fields: &IssueRawFields, field: VaultRecordField) -> &str {
    match field {
        VaultRecordField::Title => fields.expose_title(),
        VaultRecordField::Body => fields.expose_body(),
    }
}

/// 지금 게시본에서 `marker` 가 있는 모든 자리이다. 제목이 먼저이다.
fn marker_places(marker: &str, published: &IssueRawFields) -> Vec<VaultMarkerPlace> {
    FIELDS
        .into_iter()
        .flat_map(|field| {
            field_text(published, field)
                .match_indices(marker)
                .map(move |(position, _)| VaultMarkerPlace { field, position })
        })
        .collect()
}

/// 표식을 필드와 위치의 순서로 늘어놓는다.
fn marker_order(mut places: Vec<(VaultRecordField, usize, &str)>) -> Vec<(VaultRecordField, &str)> {
    places.sort_by_key(|(field, position, _)| (*field == VaultRecordField::Body, *position));
    places
        .into_iter()
        .map(|(field, _, marker)| (field, marker))
        .collect()
}

/// 해시가 다른 게시본에서 보류의 원인을 정한다.
fn changed_cause(spans: &[VaultRestorableSpan], published: &IssueRawFields) -> VaultRestoreHold {
    let places = spans
        .iter()
        .map(|span| marker_places(&span.marker, published))
        .collect::<Vec<_>>();
    if places.iter().any(|places| places.len() > 1) {
        return VaultRestoreHold::MarkerDuplicated;
    }
    if places.iter().any(Vec::is_empty) {
        return VaultRestoreHold::MarkerMissing;
    }
    let bound = marker_order(
        spans
            .iter()
            .map(|span| (span.field, span.position, span.marker.as_str()))
            .collect(),
    );
    let current = marker_order(
        spans
            .iter()
            .zip(&places)
            .map(|(span, places)| (places[0].field, places[0].position, span.marker.as_str()))
            .collect(),
    );
    if bound != current {
        return VaultRestoreHold::MarkerMoved;
    }
    VaultRestoreHold::PublishedCopyChanged
}

/// 필드의 글에서 표식을 평문으로 바꾼다. 표식이 저장한 위치에 없거나 구간이 겹치면 보류한다.
fn restored_field(
    text: &str,
    field: VaultRecordField,
    spans: &[VaultRestorableSpan],
) -> Result<VaultPlaintext, VaultRestoreHold> {
    let mut spans = spans
        .iter()
        .filter(|span| span.field == field)
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| span.position);
    let mut restored = String::with_capacity(text.len());
    let mut cursor = 0;
    for span in spans {
        let at_marker = text
            .get(span.position..)
            .is_some_and(|rest| rest.starts_with(&span.marker));
        if span.position < cursor || !at_marker {
            return Err(VaultRestoreHold::MarkerMoved);
        }
        restored.push_str(&text[cursor..span.position]);
        restored.push_str(span.plaintext.expose());
        cursor = span.position + span.marker.len();
    }
    restored.push_str(&text[cursor..]);
    Ok(VaultPlaintext::new(restored))
}

pub struct EvidenceVaultRestoreService {
    snapshots: Arc<dyn IssueSnapshotPort>,
    vault: Arc<dyn EvidenceVaultStorePort>,
    unlock: Arc<EvidenceVaultUnlockService>,
}

impl EvidenceVaultRestoreService {
    pub fn new(
        snapshots: Arc<dyn IssueSnapshotPort>,
        vault: Arc<dyn EvidenceVaultStorePort>,
        unlock: Arc<EvidenceVaultUnlockService>,
    ) -> Self {
        Self {
            snapshots,
            vault,
            unlock,
        }
    }

    /// 살아 있는 권한으로 이슈의 로컬 복원본을 준다. 보류하면 평문을 주지 않는다.
    pub fn restore_for_grant(
        &self,
        grant: &VaultUnlockGrant,
    ) -> Result<VaultRestoreResponse, VaultRestoreHold> {
        let key = self.key(grant)?;
        let restored = self.verified_restore(grant.issue(), &key)?;
        // 원격 읽기와 복호화 동안 세션이 끝났으면 늦은 결과를 주지 않는다.
        self.key(grant)?;
        Ok(restored)
    }

    /// 재연결 검토의 값을 만든다: 지금 게시본과, 저장한 표식마다 이전 자리와 지금 있는 자리이다.
    /// 연결이 없는 이슈는 `NotBound` 이다. 다른 이슈의 연결은 쓰지 않는다.
    pub fn review_reconnect(
        &self,
        grant: &VaultUnlockGrant,
    ) -> Result<VaultReconnectReview, VaultRestoreHold> {
        let key = self.key(grant)?;
        let issue = grant.issue();
        let published = self.read_latest(issue)?;
        let mut binding = self.begin_binding(issue, &key)?;
        let snapshot = binding.snapshot().clone();
        let source_version = Self::bound_version(&snapshot)?;
        let spans = Self::bound_spans(binding.as_mut(), &source_version)?;
        self.key(grant)?;
        let reviewed_hash = published.field_hash();
        let markers = spans
            .into_iter()
            .map(|span| VaultReconnectMarker {
                candidates: marker_places(&span.marker, &published),
                previous: VaultMarkerPlace {
                    field: span.field,
                    position: span.position,
                },
                marker: span.marker.clone(),
            })
            .collect();
        Ok(VaultReconnectReview {
            issue: issue.clone(),
            reviewed_hash: reviewed_hash.clone(),
            published,
            markers,
            basis: VaultReconnectBasis {
                issue: issue.clone(),
                expected: snapshot.token,
                source_version,
                reviewed_hash,
            },
        })
    }

    /// 검토 당시 연결에서 사용자가 승인한 묶음만 원자적으로 확정한다.
    pub fn reconnect(
        &self,
        grant: &VaultUnlockGrant,
        approval: &VaultReconnectApproval,
    ) -> Result<(), VaultRestoreHold> {
        self.reconnect_with_commit_gate(grant, approval, || Ok(()))
    }

    /// Native 세션은 마지막 권한 검사 뒤, 저장 시작 직전에 취소와 단회 진입을 선형화한다.
    /// gate가 성공한 뒤에는 취소가 이미 시작한 저장을 되돌린다고 보장하지 않는다.
    pub(crate) fn reconnect_with_commit_gate(
        &self,
        grant: &VaultUnlockGrant,
        approval: &VaultReconnectApproval,
        before_commit: impl FnOnce() -> Result<(), VaultRestoreHold>,
    ) -> Result<(), VaultRestoreHold> {
        let key = self.key(grant)?;
        let issue = grant.issue();
        if issue != &approval.basis.issue {
            return Err(VaultRestoreHold::NotBound);
        }
        let mut binding = self.begin_binding(issue, &key)?;
        self.key(grant)?;
        // 저장 경계를 기다린 뒤 최신 게시본을 읽어 오래 기다린 조회를 재사용하지 않는다.
        let published = self.read_latest(issue)?;
        if published.field_hash() != approval.basis.reviewed_hash {
            return Err(VaultRestoreHold::ChangedDuringReview);
        }
        // 첫 commit이 일부 표식만 남겼더라도 같은 승인은 원래 불변 판에서 재구성한다.
        let source_spans = Self::bound_spans(binding.as_mut(), &approval.basis.source_version)?;
        let mut approved: Vec<VaultRestorableSpan> = Vec::new();
        for mapping in &approval.mappings {
            let span = source_spans
                .iter()
                .find(|span| span.marker == mapping.marker)
                .ok_or(VaultRestoreHold::MappingRejected)?;
            if approved.iter().any(|span| span.marker == mapping.marker) {
                return Err(VaultRestoreHold::MappingRejected);
            }
            approved.push(VaultRestorableSpan {
                field: mapping.place.field,
                marker: mapping.marker.clone(),
                position: mapping.place.position,
                plaintext: span.plaintext.clone(),
            });
        }
        for field in FIELDS {
            restored_field(field_text(&published, field), field, &approved)
                .map_err(|_| VaultRestoreHold::MappingRejected)?;
        }
        let field_bundle = |field| {
            let spans = approved
                .iter()
                .filter(|span| span.field == field)
                .cloned()
                .collect::<Vec<_>>();
            Some(seal_field_spans(&spans))
        };
        let fields = VaultBindingFields {
            title: field_bundle(VaultRecordField::Title),
            body: field_bundle(VaultRecordField::Body),
        };
        self.key(grant)?;
        before_commit()?;
        match binding
            .commit(VaultBindingCommitRequest {
                expected: approval.basis.expected.clone(),
                operation_id: approval.operation_id.clone(),
                base_version: confirmed_version_name(&approval.basis.reviewed_hash),
                approval_digest: approval.approval_digest,
                fields,
                policy: VaultBindingCommitPolicy::ReplaceExpected,
            })
            .map_err(VaultRestoreHold::RecordUnavailable)?
        {
            VaultBindingCommitResult::Committed(_)
            | VaultBindingCommitResult::AlreadyCommitted(_) => Ok(()),
            VaultBindingCommitResult::Conflict => Err(VaultRestoreHold::ChangedDuringReview),
        }
    }

    fn key(&self, grant: &VaultUnlockGrant) -> Result<VaultRecordKey, VaultRestoreHold> {
        self.unlock
            .access(grant)
            .map(|access| access.key)
            .map_err(VaultRestoreHold::Locked)
    }

    fn verified_restore(
        &self,
        issue: &IssueReference,
        key: &VaultRecordKey,
    ) -> Result<VaultRestoreResponse, VaultRestoreHold> {
        let published = self.read_latest(issue)?;
        let mut binding = self.begin_binding(issue, key)?;
        let version = Self::bound_version(binding.snapshot())?;
        let spans = Self::bound_spans(binding.as_mut(), &version)?;
        if !confirmed_version_matches_hash(&version, &published.field_hash()) {
            return Err(changed_cause(&spans, &published));
        }
        Ok(VaultRestoreResponse {
            title: restored_field(published.expose_title(), VaultRecordField::Title, &spans)?,
            body: restored_field(published.expose_body(), VaultRecordField::Body, &spans)?,
        })
    }

    /// 지금 게시본을 읽는다. 답이 다른 이슈의 글이면 읽지 못한 것과 같다.
    fn read_latest(&self, issue: &IssueReference) -> Result<IssueRawFields, VaultRestoreHold> {
        self.snapshots
            .read(IssueSnapshotReadRequest {
                issue: issue.clone(),
            })
            .ok()
            .filter(|snapshot| snapshot.issue == *issue)
            .map(|snapshot| snapshot.raw_fields)
            .ok_or(VaultRestoreHold::LatestReadFailed)
    }

    fn begin_binding(
        &self,
        issue: &IssueReference,
        key: &VaultRecordKey,
    ) -> Result<Box<dyn VaultBindingTransaction + '_>, VaultRestoreHold> {
        self.vault
            .begin_binding(VaultRecordLoadRequest {
                scope: VaultRecordScope::current_version(issue.clone()),
                key: key.clone(),
            })
            .map_err(VaultRestoreHold::RecordUnavailable)
    }

    fn bound_version(snapshot: &VaultBindingSnapshot) -> Result<String, VaultRestoreHold> {
        match &snapshot.state {
            VaultBindingState::Readable { version } => Ok(version.clone()),
            VaultBindingState::Missing => Err(VaultRestoreHold::NotBound),
            VaultBindingState::Unreadable { error } => {
                Err(VaultRestoreHold::RecordUnavailable(error.clone()))
            }
        }
    }

    /// 같은 transaction이 고정한 이슈와 키로 판의 필드들을 읽는다.
    fn bound_spans(
        binding: &mut dyn VaultBindingTransaction,
        version: &str,
    ) -> Result<Vec<VaultRestorableSpan>, VaultRestoreHold> {
        let fields = binding
            .load_version_fields(version)
            .map_err(VaultRestoreHold::RecordUnavailable)?;
        let mut spans = Vec::new();
        for (field, bundle) in [
            (VaultRecordField::Title, fields.title),
            (VaultRecordField::Body, fields.body),
        ] {
            if let Some(bundle) = bundle {
                spans.extend(open_field_spans(field, &bundle).ok_or(
                    VaultRestoreHold::RecordUnavailable(VaultStoreError::CorruptRecord),
                )?);
            }
        }
        Ok(spans)
    }
}

impl EvidenceVaultRestorePort for EvidenceVaultRestoreService {
    /// 보류는 `VaultRestoreHold` 오류이다 (`downcast_ref` 로 꺼낸다).
    fn restore(&self, request: VaultRestoreRequest) -> Result<VaultRestoreResponse> {
        self.unlock
            .validate_confirmation(&request.confirmation)
            .map_err(VaultRestoreHold::Locked)?;
        let restored = self.verified_restore(request.confirmation.issue(), &request.key)?;
        self.unlock
            .validate_confirmation(&request.confirmation)
            .map_err(VaultRestoreHold::Locked)?;
        Ok(restored)
    }
}

#[cfg(test)]
mod tests {
    mod binding_transactions;
    mod native_commit_gate;

    use std::collections::HashMap;

    use serde::Serialize;

    use super::*;
    use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
    use crate::application::port::outbound::evidence_vault_port::{
        VaultKey, VaultRecordBinding, VaultRecordSaveRequest,
    };
    use crate::application::port::outbound::issue_snapshot_port::{
        IssueSnapshotResolveRequest, IssueSnapshotResponse,
    };
    use crate::application::service::evidence_vault_unlock::{
        VaultUnlockOutcome, VaultUnlockRequest, VaultUserDecision, VaultViewingSessionId,
    };
    use crate::application::service::issue_masking::{IssueSensitiveSpan, mask_issue_text};
    use crate::domain::issue_policy::{IssueDisclosurePolicy, IssueSensitiveSpanAction};
    use crate::domain::issue_work::{
        IssueComparisonBasis, IssueProjectScope, IssueProvider, IssueSharedCopy,
    };

    // 시험용으로 만든 값이다. 실제 사람이나 경로가 아니다.
    const TITLE: &str = "alice 의 빌드 실패";
    const BODY: &str = "alice 가 /home/alice/app 에서 실행했다.\r\n로그는 아래에 있다.";
    const MASKED_TITLE: &str = "[[akra-mask:1]] 의 빌드 실패";
    const MASKED_BODY: &str =
        "[[akra-mask:2]] 가 [[akra-mask:3]] 에서 실행했다.\r\n로그는 아래에 있다.";

    type ReadHook = Box<dyn Fn() + Send>;

    #[derive(Default)]
    struct FakeRemote {
        issues: Mutex<HashMap<IssueReference, IssueSharedCopy>>,
        fail_reads: Mutex<bool>,
        /// 읽는 도중에 일어나는 일이다 (예: 세션 종료).
        during_read: Mutex<Option<ReadHook>>,
    }

    impl IssueSnapshotPort for FakeRemote {
        fn resolve(&self, _: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
            anyhow::bail!("the restore path does not resolve locators")
        }

        fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
            if let Some(hook) = self.during_read.lock().unwrap().as_ref() {
                hook();
            }
            if *self.fail_reads.lock().unwrap() {
                anyhow::bail!("the provider is unreachable")
            }
            let issues = self.issues.lock().unwrap();
            let copy = issues
                .get(&request.issue)
                .ok_or_else(|| anyhow::anyhow!("unknown issue"))?;
            let raw_fields = IssueRawFields::new(copy.title.clone(), copy.body.clone());
            Ok(IssueSnapshotResponse {
                issue: request.issue,
                comparison_basis: IssueComparisonBasis {
                    field_hash: raw_fields.field_hash(),
                    provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
                },
                raw_fields,
            })
        }
    }

    #[derive(Default)]
    struct FakeVault {
        records: Mutex<HashMap<VaultRecordScope, VaultPlaintext>>,
        /// 이 수만큼 저장한 뒤의 저장은 실패한다.
        saves_left: Mutex<Option<usize>>,
        writes: Mutex<usize>,
        fail_after_pointer: Mutex<bool>,
        after_current_load: Mutex<Option<ReadHook>>,
    }

    impl EvidenceVaultStorePort for FakeVault {
        fn begin_binding(
            &self,
            request: VaultRecordLoadRequest,
        ) -> Result<Box<dyn VaultBindingTransaction + '_>, VaultStoreError> {
            use crate::test_utils::vault_binding::{TestBindingHooks, begin_test_binding};
            let transaction = begin_test_binding(
                self.records.lock().unwrap(),
                request,
                TestBindingHooks {
                    before_write: Some(Box::new(|_| {
                        *self.writes.lock().unwrap() += 1;
                        if let Some(left) = self.saves_left.lock().unwrap().as_mut() {
                            if *left == 0 {
                                return Err(VaultStoreError::Storage(
                                    "the disk is full".to_string(),
                                ));
                            }
                            *left -= 1;
                        }
                        Ok(())
                    })),
                    after_pointer: Some(Box::new(|| {
                        if std::mem::take(&mut *self.fail_after_pointer.lock().unwrap()) {
                            Err(VaultStoreError::WriteOutcomeUnknown)
                        } else {
                            Ok(())
                        }
                    })),
                },
            )?;
            if let Some(hook) = self.after_current_load.lock().unwrap().take() {
                hook();
            }
            Ok(transaction)
        }

        fn save(&self, request: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
            if let Some(left) = self.saves_left.lock().unwrap().as_mut() {
                if *left == 0 {
                    return Err(VaultStoreError::Storage("the disk is full".to_string()));
                }
                *left -= 1;
            }
            self.records
                .lock()
                .unwrap()
                .insert(request.scope, request.plaintext);
            Ok(())
        }

        fn load(&self, request: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
            let result = self
                .records
                .lock()
                .unwrap()
                .get(&request.scope)
                .cloned()
                .ok_or(VaultStoreError::RecordNotFound);
            if request.scope.binding == VaultRecordBinding::CurrentVersion {
                let hook = self.after_current_load.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            result
        }
    }

    struct Fixture {
        remote: Arc<FakeRemote>,
        vault: Arc<FakeVault>,
        unlock: Arc<EvidenceVaultUnlockService>,
        service: EvidenceVaultRestoreService,
        session: VaultViewingSessionId,
    }

    impl Fixture {
        /// `issue()` 에 `TITLE` / `BODY` 를 가려 게시했고, 그 판에 연결을 확정했다.
        fn published() -> Self {
            let remote = Arc::new(FakeRemote::default());
            let vault = Arc::new(FakeVault::default());
            let unlock = Arc::new(EvidenceVaultUnlockService::new(Arc::new(
                FakeVaultKeyStoreAdapter::with_key(VaultKey::new(vec![7; 32])),
            )));
            let fixture = Self {
                service: EvidenceVaultRestoreService::new(
                    remote.clone(),
                    vault.clone(),
                    unlock.clone(),
                ),
                session: unlock.begin_session(),
                remote,
                vault,
                unlock,
            };

            let outcome = mask_issue_text(
                TITLE,
                BODY,
                &[
                    sensitive(IssueTextField::Title, TITLE, "alice"),
                    sensitive(IssueTextField::Body, BODY, "alice"),
                    sensitive(IssueTextField::Body, BODY, "/home/alice/app"),
                ],
                &IssueDisclosurePolicy::builtin(),
            )
            .unwrap();
            assert_eq!(outcome.shared_copy, copy(MASKED_TITLE, MASKED_BODY));
            let version = version_of(MASKED_TITLE, MASKED_BODY);
            for restorable in restorable_fields(&outcome.masked_spans) {
                let field = match restorable.field {
                    IssuePublishVaultField::Title => VaultRecordField::Title,
                    IssuePublishVaultField::Body => VaultRecordField::Body,
                };
                fixture.put(
                    VaultRecordScope {
                        issue: issue(),
                        field,
                        binding: VaultRecordBinding::ConfirmedVersion(version.clone()),
                    },
                    restorable.plaintext,
                );
            }
            fixture.put(
                VaultRecordScope::current_version(issue()),
                VaultPlaintext::new(version),
            );
            fixture.publish(&issue(), MASKED_TITLE, MASKED_BODY);
            fixture
        }

        fn put(&self, scope: VaultRecordScope, plaintext: VaultPlaintext) {
            self.vault.records.lock().unwrap().insert(scope, plaintext);
        }

        /// 원격의 글을 바꾼다. 다른 사람의 편집과 같다.
        fn publish(&self, issue: &IssueReference, title: &str, body: &str) {
            self.remote
                .issues
                .lock()
                .unwrap()
                .insert(issue.clone(), copy(title, body));
        }

        fn grant(&self, issue: &IssueReference) -> VaultUnlockGrant {
            match self.unlock.unlock(VaultUnlockRequest {
                issue: issue.clone(),
                session: self.session.clone(),
                decision: VaultUserDecision::Confirmed,
            }) {
                VaultUnlockOutcome::Unlocked(grant) => grant,
                VaultUnlockOutcome::SharedCopyOnly(reason) => panic!("not unlocked: {reason:?}"),
            }
        }

        fn restore(&self) -> Result<VaultRestoreResponse, VaultRestoreHold> {
            self.service.restore_for_grant(&self.grant(&issue()))
        }

        fn bound_version(&self) -> String {
            self.vault.records.lock().unwrap()[&VaultRecordScope::current_version(issue())]
                .expose()
                .to_string()
        }
    }

    fn issue() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_1".to_string(),
            issue_id: "I_1".to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn copy(title: &str, body: &str) -> IssueSharedCopy {
        IssueSharedCopy {
            title: title.to_string(),
            body: body.to_string(),
        }
    }

    fn version_of(title: &str, body: &str) -> String {
        confirmed_version_name(&IssueFieldHash::of_fields(title, body))
    }

    fn sensitive(field: IssueTextField, text: &str, needle: &str) -> IssueSensitiveSpan {
        let start = text.find(needle).unwrap();
        IssueSensitiveSpan {
            field,
            range: start..start + needle.len(),
            class: None,
            user_decision: Some(IssueSensitiveSpanAction::RestorableMask),
        }
    }

    fn place(field: VaultRecordField, text: &str, marker: &str) -> VaultMarkerPlace {
        VaultMarkerPlace {
            field,
            position: text.find(marker).unwrap(),
        }
    }

    fn mapping(field: VaultRecordField, text: &str, marker: &str) -> VaultApprovedMapping {
        VaultApprovedMapping {
            marker: marker.to_string(),
            place: place(field, text, marker),
        }
    }

    #[test]
    fn a_matching_published_copy_restores_every_marker_of_both_fields() {
        let fixture = Fixture::published();

        let restored = fixture.restore().unwrap();

        assert_eq!(restored.title.expose(), TITLE);
        assert_eq!(restored.body.expose(), BODY);
        assert!(!format!("{restored:?}").contains("alice"));
    }

    #[test]
    fn a_copy_under_another_project_server_repository_or_issue_is_not_bound() {
        let fixture = Fixture::published();
        let mut other_project = issue();
        other_project.project_scope.workspace_dir = "/work/other".to_string();
        let others = [
            other_project,
            IssueReference {
                server_id: "github.example".to_string(),
                ..issue()
            },
            IssueReference {
                repository_id: "R_2".to_string(),
                ..issue()
            },
            IssueReference {
                issue_id: "I_2".to_string(),
                ..issue()
            },
        ];
        for other in others {
            // 같은 글을 다른 대상에 복사했다. 표식은 같지만 연결은 없다.
            fixture.publish(&other, MASKED_TITLE, MASKED_BODY);
            let grant = fixture.grant(&other);

            assert_eq!(
                fixture.service.restore_for_grant(&grant),
                Err(VaultRestoreHold::NotBound),
                "{other:?}"
            );
            // 다른 이슈는 재연결의 대상이 아니다.
            assert_eq!(
                fixture.service.review_reconnect(&grant),
                Err(VaultRestoreHold::NotBound)
            );
            let original_grant = fixture.grant(&issue());
            let review = fixture.service.review_reconnect(&original_grant).unwrap();
            let approval = VaultReconnectApproval::from_review(
                &review,
                vec![mapping(
                    VaultRecordField::Title,
                    MASKED_TITLE,
                    "[[akra-mask:1]]",
                )],
            )
            .unwrap();
            assert_eq!(
                fixture.service.reconnect(&grant, &approval),
                Err(VaultRestoreHold::NotBound)
            );
        }
        assert_eq!(fixture.vault.records.lock().unwrap().len(), 3);
    }

    #[test]
    fn each_external_change_holds_the_restore_with_its_own_cause() {
        use VaultRestoreHold::*;
        let cases = [
            // 본문 한 글자.
            (
                MASKED_TITLE,
                MASKED_BODY.replace("아래", "위"),
                PublishedCopyChanged,
            ),
            // 줄 끝만 바뀌었다.
            (
                MASKED_TITLE,
                MASKED_BODY.replace("\r\n", "\n"),
                PublishedCopyChanged,
            ),
            (
                MASKED_TITLE,
                format!("{MASKED_BODY} [[akra-mask:2]]"),
                MarkerDuplicated,
            ),
            // 본문의 표식을 제목에도 복사했다.
            (
                "[[akra-mask:1]] [[akra-mask:3]]",
                MASKED_BODY.to_string(),
                MarkerDuplicated,
            ),
            (
                MASKED_TITLE,
                "[[akra-mask:3]] 가 [[akra-mask:2]] 에서 실행했다.".to_string(),
                MarkerMoved,
            ),
            (
                MASKED_TITLE,
                MASKED_BODY.replace("[[akra-mask:3]]", "어딘가"),
                MarkerMissing,
            ),
        ];
        for (title, body, cause) in cases {
            let fixture = Fixture::published();
            fixture.publish(&issue(), title, &body);

            assert_eq!(fixture.restore(), Err(cause), "{title} / {body}");
        }
    }

    #[test]
    fn a_failed_latest_read_holds_the_restore_even_after_an_earlier_success() {
        let fixture = Fixture::published();
        fixture.restore().unwrap();

        *fixture.remote.fail_reads.lock().unwrap() = true;

        assert_eq!(fixture.restore(), Err(VaultRestoreHold::LatestReadFailed));
    }

    #[test]
    fn a_stored_position_without_the_marker_holds_the_restore() {
        let fixture = Fixture::published();
        let moved = seal_field_spans(&[VaultRestorableSpan {
            field: VaultRecordField::Title,
            marker: "[[akra-mask:1]]".to_string(),
            position: 1,
            plaintext: VaultPlaintext::new("alice".to_string()),
        }]);
        fixture.put(
            VaultRecordScope {
                issue: issue(),
                field: VaultRecordField::Title,
                binding: VaultRecordBinding::ConfirmedVersion(fixture.bound_version()),
            },
            moved,
        );

        assert_eq!(fixture.restore(), Err(VaultRestoreHold::MarkerMoved));
    }

    #[test]
    fn a_restore_needs_a_grant_that_is_still_live_when_the_value_is_ready() {
        let fixture = Fixture::published();
        let grant = fixture.grant(&issue());
        let (unlock, session) = (fixture.unlock.clone(), fixture.session.clone());
        // 원격을 읽는 동안 사용자가 화면을 닫았다.
        *fixture.remote.during_read.lock().unwrap() =
            Some(Box::new(move || unlock.end_session(&session)));

        let locked = VaultRestoreHold::Locked(VaultSharedCopyReason::SessionExpired);
        assert_eq!(
            fixture.service.restore_for_grant(&grant),
            Err(locked.clone())
        );
        assert_eq!(
            fixture.service.restore_for_grant(&grant),
            Err(locked.clone())
        );
        assert_eq!(fixture.service.review_reconnect(&grant), Err(locked));
    }

    #[test]
    fn the_restore_port_gives_the_same_result_and_reports_a_hold_as_an_error() {
        let fixture = Fixture::published();
        let request = || {
            let access = fixture.unlock.access(&fixture.grant(&issue())).unwrap();
            VaultRestoreRequest {
                confirmation: access.confirmation,
                key: access.key,
            }
        };
        let port: &dyn EvidenceVaultRestorePort = &fixture.service;

        assert_eq!(port.restore(request()).unwrap().body.expose(), BODY);

        fixture.publish(&issue(), MASKED_TITLE, "다른 글");
        let error = port.restore(request()).unwrap_err();
        assert_eq!(
            error.downcast_ref::<VaultRestoreHold>(),
            Some(&VaultRestoreHold::MarkerMissing)
        );
    }

    #[test]
    fn the_restore_port_rejects_saved_requests_after_session_end() {
        for end_during_read in [false, true] {
            let fixture = Fixture::published();
            let access = fixture.unlock.access(&fixture.grant(&issue())).unwrap();
            let request = VaultRestoreRequest {
                confirmation: access.confirmation,
                key: access.key,
            };
            if end_during_read {
                let (unlock, session) = (fixture.unlock.clone(), fixture.session.clone());
                *fixture.remote.during_read.lock().unwrap() =
                    Some(Box::new(move || unlock.end_session(&session)));
            } else {
                fixture.unlock.end_session(&fixture.session);
            }
            let port: &dyn EvidenceVaultRestorePort = &fixture.service;
            let error = port.restore(request).unwrap_err();
            assert_eq!(
                error.downcast_ref::<VaultRestoreHold>(),
                Some(&VaultRestoreHold::Locked(
                    VaultSharedCopyReason::SessionExpired
                ))
            );
        }
    }

    #[test]
    fn concurrent_reconnects_do_not_interleave_their_record_transactions() {
        use std::sync::mpsc;
        use std::time::Duration;
        let fixture = Arc::new(Fixture::published());
        let edited = format!("changed: {MASKED_BODY}");
        fixture.publish(&issue(), MASKED_TITLE, &edited);
        let first_grant = fixture.grant(&issue());
        let second_grant = fixture.grant(&issue());
        let review = fixture.service.review_reconnect(&first_grant).unwrap();
        let first_approval = VaultReconnectApproval::from_review(
            &review,
            vec![mapping(
                VaultRecordField::Title,
                MASKED_TITLE,
                "[[akra-mask:1]]",
            )],
        )
        .unwrap();
        let second_approval = VaultReconnectApproval::from_review(
            &review,
            vec![mapping(VaultRecordField::Body, &edited, "[[akra-mask:2]]")],
        )
        .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *fixture.vault.after_current_load.lock().unwrap() = Some(Box::new(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }));
        let first_fixture = fixture.clone();
        let first = std::thread::spawn(move || {
            first_fixture
                .service
                .reconnect(&first_grant, &first_approval)
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let second_fixture = fixture.clone();
        let second = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = second_fixture
                .service
                .reconnect(&second_grant, &second_approval);
            finished_tx.send(()).unwrap();
            result
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let waited_for_first = matches!(
            finished_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        assert_eq!(
            second.join().unwrap(),
            Err(VaultRestoreHold::ChangedDuringReview)
        );
        assert!(
            waited_for_first,
            "a second reconnect committed while the first was paused"
        );
        let restored = fixture.restore().unwrap();
        assert_eq!(restored.title.expose(), TITLE);
        assert_eq!(restored.body.expose(), edited);
    }

    #[test]
    fn a_reconnect_rejects_different_mappings_from_the_same_review() {
        let fixture = Fixture::published();
        let grant = fixture.grant(&issue());
        let edited_body = format!("changed: {MASKED_BODY}");
        fixture.publish(&issue(), MASKED_TITLE, &edited_body);
        let review = fixture.service.review_reconnect(&grant).unwrap();
        let first_approval = VaultReconnectApproval::from_review(
            &review,
            vec![mapping(
                VaultRecordField::Title,
                MASKED_TITLE,
                "[[akra-mask:1]]",
            )],
        )
        .unwrap();
        let conflicting_approval = VaultReconnectApproval::from_review(
            &review,
            vec![mapping(
                VaultRecordField::Body,
                &edited_body,
                "[[akra-mask:2]]",
            )],
        )
        .unwrap();

        fixture.service.reconnect(&grant, &first_approval).unwrap();
        let committed_records = fixture.vault.records.lock().unwrap().clone();
        let committed_writes = *fixture.vault.writes.lock().unwrap();
        let restored = fixture.restore().unwrap();
        assert_eq!(restored.title.expose(), TITLE);
        assert_eq!(restored.body.expose(), edited_body);

        // 같은 승인 재시도는 성공해도 레코드를 다시 쓰지 않는다.
        fixture.service.reconnect(&grant, &first_approval).unwrap();
        assert_eq!(*fixture.vault.records.lock().unwrap(), committed_records);

        // 같은 공유본 해시라도 다른 선택은 앞서 확정된 승인의 재시도가 아니다.
        let conflict = fixture.service.reconnect(&grant, &conflicting_approval);
        assert_eq!(*fixture.vault.records.lock().unwrap(), committed_records);
        assert_eq!(fixture.restore().unwrap(), restored);
        assert_eq!(conflict, Err(VaultRestoreHold::ChangedDuringReview));
        assert_eq!(*fixture.vault.writes.lock().unwrap(), committed_writes);
    }

    #[test]
    fn a_reconnect_commits_only_the_approved_mapping_to_the_current_version() {
        let fixture = Fixture::published();
        let grant = fixture.grant(&issue());
        // 다른 사람이 본문을 고쳤고 표식 하나를 한 번 더 붙였다.
        let edited_body =
            "고친 글: [[akra-mask:2]] 가 [[akra-mask:3]] 에서 실행했다. [[akra-mask:3]]";
        fixture.publish(&issue(), MASKED_TITLE, edited_body);
        assert_eq!(fixture.restore(), Err(VaultRestoreHold::MarkerDuplicated));

        let review = fixture.service.review_reconnect(&grant).unwrap();
        assert_eq!(
            review.reviewed_hash,
            IssueFieldHash::of_fields(MASKED_TITLE, edited_body)
        );
        let path_marker = &review.markers[2];
        assert_eq!(path_marker.marker, "[[akra-mask:3]]");
        assert_eq!(
            path_marker.previous,
            place(VaultRecordField::Body, MASKED_BODY, "[[akra-mask:3]]")
        );
        assert_eq!(path_marker.candidates.len(), 2);
        assert!(
            review
                .context(&path_marker.candidates[0])
                .contains("가 [[akra-mask:3]] 에서")
        );
        assert!(!format!("{review:?}").contains("고친 글"));

        // 사용자는 제목의 표식과 본문의 첫 경로 표식만 승인했다.
        let approval = VaultReconnectApproval::from_review(
            &review,
            vec![
                mapping(VaultRecordField::Title, MASKED_TITLE, "[[akra-mask:1]]"),
                VaultApprovedMapping {
                    marker: "[[akra-mask:3]]".to_string(),
                    place: path_marker.candidates[0].clone(),
                },
            ],
        )
        .unwrap();
        fixture.service.reconnect(&grant, &approval).unwrap();

        assert!(confirmed_version_matches_hash(
            &fixture.bound_version(),
            &review.reviewed_hash
        ));
        let restored = fixture.restore().unwrap();
        assert_eq!(restored.title.expose(), TITLE);
        assert_eq!(
            restored.body.expose(),
            "고친 글: [[akra-mask:2]] 가 /home/alice/app 에서 실행했다. [[akra-mask:3]]"
        );
        // 같은 승인을 다시 보내도 연결은 그대로이다.
        fixture.service.reconnect(&grant, &approval).unwrap();
        assert_eq!(fixture.restore().unwrap(), restored);
    }

    #[test]
    fn a_reconnect_is_not_committed_when_the_copy_changes_again_or_the_mapping_is_wrong() {
        let fixture = Fixture::published();
        let grant = fixture.grant(&issue());
        let edited_body = format!("고친 글: {MASKED_BODY}");
        fixture.publish(&issue(), MASKED_TITLE, &edited_body);
        let review = fixture.service.review_reconnect(&grant).unwrap();
        let bound_before = fixture.bound_version();
        let approve = |mappings| VaultReconnectApproval::from_review(&review, mappings);
        let title_mapping = || mapping(VaultRecordField::Title, MASKED_TITLE, "[[akra-mask:1]]");

        let rejected = [
            // 이 이슈에 저장한 적이 없는 표식이다.
            vec![VaultApprovedMapping {
                marker: "[[akra-mask:9]]".to_string(),
                place: title_mapping().place,
            }],
            // 그 자리에 표식이 없다.
            vec![VaultApprovedMapping {
                marker: "[[akra-mask:2]]".to_string(),
                place: VaultMarkerPlace {
                    field: VaultRecordField::Body,
                    position: 0,
                },
            }],
            // 같은 표식을 두 번 승인했다.
            vec![title_mapping(), title_mapping()],
        ];
        for mappings in rejected {
            assert_eq!(approve(mappings), Err(VaultRestoreHold::MappingRejected));
        }

        // 저장이 `CurrentVersion` 전에 멈췄다.
        *fixture.vault.saves_left.lock().unwrap() = Some(2);
        assert!(matches!(
            fixture
                .service
                .reconnect(&grant, &approve(vec![title_mapping()]).unwrap()),
            Err(VaultRestoreHold::RecordUnavailable(_))
        ));
        *fixture.vault.saves_left.lock().unwrap() = None;
        assert_eq!(fixture.bound_version(), bound_before);
        assert_eq!(
            fixture.restore(),
            Err(VaultRestoreHold::PublishedCopyChanged)
        );

        // 검토하는 동안 다른 사람이 다시 고쳤다.
        fixture.publish(&issue(), MASKED_TITLE, &format!("{edited_body}!"));
        assert_eq!(
            fixture
                .service
                .reconnect(&grant, &approve(vec![title_mapping()]).unwrap()),
            Err(VaultRestoreHold::ChangedDuringReview)
        );
        assert_eq!(fixture.bound_version(), bound_before);

        // 원래 글로 돌아오면 이전 연결로 복원한다.
        fixture.publish(&issue(), MASKED_TITLE, MASKED_BODY);
        assert_eq!(fixture.restore().unwrap().body.expose(), BODY);
    }

    #[test]
    fn a_span_bundle_round_trips_and_rejects_a_broken_record() {
        let spans = [
            VaultRestorableSpan {
                field: VaultRecordField::Body,
                marker: "[[akra-mask:1]]".to_string(),
                position: 0,
                plaintext: VaultPlaintext::new("첫 줄\n둘째 줄 3 4 5\n".to_string()),
            },
            VaultRestorableSpan {
                field: VaultRecordField::Body,
                marker: "[[akra-mask:2]]".to_string(),
                position: 40,
                plaintext: VaultPlaintext::new(String::new()),
            },
        ];
        let bundle = seal_field_spans(&spans);

        assert_eq!(
            open_field_spans(VaultRecordField::Body, &bundle).unwrap(),
            spans
        );
        assert!(
            open_field_spans(VaultRecordField::Body, &seal_field_spans(&[]))
                .unwrap()
                .is_empty()
        );
        let cut = &bundle.expose()[..bundle.expose().len() - 4];
        for broken in [cut, "masked log line", ""] {
            assert_eq!(
                open_field_spans(
                    VaultRecordField::Body,
                    &VaultPlaintext::new(broken.to_string())
                ),
                None
            );
        }
    }

    #[test]
    fn restore_and_reconnect_types_are_not_serializable() {
        trait NotSerialize<Marker> {
            fn proven() {}
        }
        impl<T: ?Sized> NotSerialize<()> for T {}
        impl<T: ?Sized + Serialize> NotSerialize<u8> for T {}

        <VaultRestorableSpan as NotSerialize<_>>::proven();
        <VaultReconnectReview as NotSerialize<_>>::proven();
        <VaultReconnectApproval as NotSerialize<_>>::proven();
        <VaultRestoreHold as NotSerialize<_>>::proven();
    }
}
