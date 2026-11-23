use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::domain::issue_automation::source::{
    IssueCreationProvenance, IssueMetadataCompleteness, IssuePrPresence, IssueRemoteState,
    IssueRepositoryVisibility, IssueSourceActor, IssueSourceActorKind, IssueSourceContent,
    IssueSourceFailure, IssueSourceObservation, IssueSourceRead,
};
use crate::domain::issue_validation::{IssueFieldFormat, IssueRequirements};
use crate::domain::issue_work::{IssueFieldHash, IssueReference};

use super::{
    ISSUE_TRIAGE_BASIS_VERSION, IssueTriageBasis, IssueTriageError, IssueTriageInput,
    IssueTriageSourceState,
};

pub(super) struct SourceBasis {
    pub source: IssueTriageSourceState,
    pub disclosure_policy_version: Option<String>,
    pub field_hash: Option<IssueFieldHash>,
    pub evidence_sha256: String,
}

#[derive(Serialize)]
struct SourceEvidence<'a> {
    version: u32,
    issue: &'a IssueReference,
    held: bool,
    current: CanonicalRead<'a>,
}

#[derive(Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum CanonicalRead<'a> {
    NotYetObserved,
    Unknown(IssueSourceFailure),
    Observed(CanonicalObservation<'a>),
}

#[derive(Serialize)]
struct CanonicalObservation<'a> {
    creation_provenance: IssueCreationProvenance,
    remote_state: IssueRemoteState,
    // Missing legacy metadata must retain its exact pre-Auto evidence hash.
    #[serde(skip_serializing_if = "unknown_repository_visibility")]
    repository_visibility: IssueRepositoryVisibility,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository_name_with_owner: Option<&'a str>,
    labels: Vec<(&'a str, &'a str)>,
    labels_completeness: IssueMetadataCompleteness,
    assignees: Vec<CanonicalActor<'a>>,
    assignees_completeness: IssueMetadataCompleteness,
    author: Option<CanonicalActor<'a>>,
    label_actors: Vec<(&'a str, &'a str, &'a str, Option<CanonicalActor<'a>>)>,
    label_actors_completeness: IssueMetadataCompleteness,
    linked_pull_request: IssuePrPresence,
    content: CanonicalContent<'a>,
}

fn unknown_repository_visibility(value: &IssueRepositoryVisibility) -> bool {
    *value == IssueRepositoryVisibility::Unknown
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct CanonicalActor<'a> {
    id: &'a str,
    kind: IssueSourceActorKind,
}

fn canonical_actor(actor: &IssueSourceActor) -> CanonicalActor<'_> {
    CanonicalActor {
        id: &actor.id,
        kind: actor.kind,
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum CanonicalContent<'a> {
    Shared {
        field_hash: &'a IssueFieldHash,
        disclosure_policy_version: &'a str,
        shared_copy_hash: IssueFieldHash,
    },
    Withheld {
        field_hash: &'a IssueFieldHash,
        disclosure_policy_version: &'a str,
        reason: crate::domain::issue_automation::source::IssueContentWithheldReason,
    },
}

pub(super) fn build_basis(
    input: &IssueTriageInput<'_>,
) -> Result<IssueTriageBasis, IssueTriageError> {
    let source = source_basis(input.issue, input.current, input.held)?;
    let result = IssueTriageBasis {
        version: ISSUE_TRIAGE_BASIS_VERSION,
        issue: input.issue.clone(),
        source: source.source,
        automation_policy_version: input.automation_policy_version.to_string(),
        runtime_selection_revision: input.runtime_selection_revision,
        disclosure_policy_version: source.disclosure_policy_version,
        validation_policy_version: input.validation_policy_version.to_string(),
        field_hash: source.field_hash,
        evidence_sha256: source.evidence_sha256,
        requirements_sha256: requirements_hash(input.requirements),
        pinned_source_commit: input.pinned_source_commit.map(str::to_string),
    };
    validate_basis(&result)?;
    Ok(result)
}

/// 사용자 표시용 login/web/updated_at과 관찰 row revision은 판정 근거가 아니다.
/// 공급자가 명시적으로 검증한 저장소 공개 범위와 좌표는 Auto 권한의 판정 근거다.
/// 현재 메타데이터와 공유본의 내용은 모두 포함하되 순서 없는 목록만 정렬한다.
pub(super) fn source_basis(
    issue: &IssueReference,
    current: Option<&IssueSourceRead>,
    held: bool,
) -> Result<SourceBasis, IssueTriageError> {
    super::super::validate_reference(issue).map_err(|_| IssueTriageError::InvalidIdentity)?;
    let (source, disclosure_policy_version, field_hash, canonical) = match current {
        None => (
            IssueTriageSourceState::NotYetObserved,
            None,
            None,
            CanonicalRead::NotYetObserved,
        ),
        Some(IssueSourceRead::Unknown {
            issue: actual,
            reason,
        }) => {
            if actual != issue {
                return Err(IssueTriageError::InvalidIdentity);
            }
            (
                IssueTriageSourceState::Unknown,
                None,
                None,
                CanonicalRead::Unknown(*reason),
            )
        }
        Some(IssueSourceRead::Observed(observation)) => {
            if observation.issue != *issue {
                return Err(IssueTriageError::InvalidIdentity);
            }
            let (source, version, hash) = match &observation.content {
                IssueSourceContent::Shared(snapshot) => (
                    IssueTriageSourceState::Shared,
                    snapshot.policy_version.clone(),
                    snapshot.basis.field_hash.clone(),
                ),
                IssueSourceContent::Withheld {
                    basis,
                    policy_version,
                    ..
                } => (
                    IssueTriageSourceState::Withheld,
                    policy_version.clone(),
                    basis.field_hash.clone(),
                ),
            };
            validate_hash(&hash)?;
            if !is_sha256(&version) {
                return Err(IssueTriageError::InvalidBasis);
            }
            (
                source,
                Some(version),
                Some(hash),
                CanonicalRead::Observed(canonical_observation(observation)),
            )
        }
    };
    let evidence_sha256 = canonical_hash(&SourceEvidence {
        version: ISSUE_TRIAGE_BASIS_VERSION,
        issue,
        held,
        current: canonical,
    });
    Ok(SourceBasis {
        source,
        disclosure_policy_version,
        field_hash,
        evidence_sha256,
    })
}

fn canonical_observation(observation: &IssueSourceObservation) -> CanonicalObservation<'_> {
    let mut labels: Vec<_> = observation
        .labels
        .items
        .iter()
        .map(|label| (label.id.as_str(), label.name.as_str()))
        .collect();
    sort_set(&mut labels);
    let mut assignees: Vec<_> = observation
        .assignees
        .items
        .iter()
        .map(canonical_actor)
        .collect();
    sort_set(&mut assignees);
    let mut label_actors: Vec<_> = observation
        .label_actors
        .items
        .iter()
        .map(|event| {
            (
                event.label.id.as_str(),
                event.label.name.as_str(),
                event.event_id.as_str(),
                event.actor.as_ref().map(canonical_actor),
            )
        })
        .collect();
    sort_set(&mut label_actors);
    let content = match &observation.content {
        IssueSourceContent::Shared(snapshot) => CanonicalContent::Shared {
            field_hash: &snapshot.basis.field_hash,
            disclosure_policy_version: &snapshot.policy_version,
            shared_copy_hash: IssueFieldHash::of_fields(
                &snapshot.shared_copy.title,
                &snapshot.shared_copy.body,
            ),
        },
        IssueSourceContent::Withheld {
            basis,
            policy_version,
            reason,
        } => CanonicalContent::Withheld {
            field_hash: &basis.field_hash,
            disclosure_policy_version: policy_version,
            reason: *reason,
        },
    };
    CanonicalObservation {
        creation_provenance: observation.creation_provenance,
        remote_state: observation.remote_state,
        repository_visibility: observation.repository_visibility,
        repository_name_with_owner: observation.repository_name_with_owner.as_deref(),
        labels,
        labels_completeness: observation.labels.completeness,
        assignees,
        assignees_completeness: observation.assignees.completeness,
        author: observation.author.as_ref().map(canonical_actor),
        label_actors,
        label_actors_completeness: observation.label_actors.completeness,
        linked_pull_request: observation.linked_pull_request,
        content,
    }
}

#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
struct CanonicalRequirement<'a> {
    label: &'a str,
    required: bool,
    format: Option<CanonicalFormat<'a>>,
}

#[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum CanonicalFormat<'a> {
    SingleLine,
    OneOf(Vec<&'a str>),
}

#[derive(Serialize)]
struct CanonicalRequirements<'a> {
    required_sections: Vec<&'a str>,
    template_fields: Vec<CanonicalRequirement<'a>>,
}

fn requirements_hash(requirements: &IssueRequirements) -> String {
    let mut required_sections: Vec<_> = requirements
        .required_sections
        .iter()
        .map(|section| section.trim())
        .collect();
    sort_set(&mut required_sections);
    let mut template_fields: Vec<_> = requirements
        .template_fields
        .iter()
        .map(|field| {
            let format = match &field.format {
                None => None,
                Some(IssueFieldFormat::SingleLine) => Some(CanonicalFormat::SingleLine),
                Some(IssueFieldFormat::OneOf(options)) => {
                    let mut options: Vec<_> = options.iter().map(|value| value.trim()).collect();
                    sort_set(&mut options);
                    Some(CanonicalFormat::OneOf(options))
                }
            };
            CanonicalRequirement {
                label: field.label.trim(),
                required: field.required,
                format,
            }
        })
        .collect();
    sort_set(&mut template_fields);
    canonical_hash(&CanonicalRequirements {
        required_sections,
        template_fields,
    })
}

pub(super) fn canonical_hash<T: Serialize>(value: &T) -> String {
    // 문자열/정수/닫힌 enum/고정 필드만 직렬화하므로 JSON 인코딩은 실패하지 않는다.
    let bytes = serde_json::to_vec(value).expect("typed triage basis serializes");
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn validate_basis(basis: &IssueTriageBasis) -> Result<(), IssueTriageError> {
    if basis.runtime_selection_revision
        > crate::domain::issue_automation::MAX_ISSUE_AUTOMATION_REVISION
    {
        return Err(IssueTriageError::InvalidBasis);
    }
    super::super::validate_reference(&basis.issue)
        .map_err(|_| IssueTriageError::InvalidIdentity)?;
    if basis.version != ISSUE_TRIAGE_BASIS_VERSION
        || !is_sha256(&basis.automation_policy_version)
        || !is_sha256(&basis.validation_policy_version)
        || !is_sha256(&basis.evidence_sha256)
        || !is_sha256(&basis.requirements_sha256)
        || basis
            .disclosure_policy_version
            .as_ref()
            .is_some_and(|version| !is_sha256(version))
        || basis
            .pinned_source_commit
            .as_ref()
            .is_some_and(|commit| !matches!(commit.len(), 40 | 64) || !is_lower_hex(commit))
    {
        return Err(IssueTriageError::InvalidBasis);
    }
    match basis.source {
        IssueTriageSourceState::Shared | IssueTriageSourceState::Withheld => {
            let Some(hash) = &basis.field_hash else {
                return Err(IssueTriageError::InvalidBasis);
            };
            validate_hash(hash)?;
            if basis.disclosure_policy_version.is_none() {
                return Err(IssueTriageError::InvalidBasis);
            }
        }
        IssueTriageSourceState::Unknown | IssueTriageSourceState::NotYetObserved => {
            if basis.field_hash.is_some() || basis.disclosure_policy_version.is_some() {
                return Err(IssueTriageError::InvalidBasis);
            }
        }
    }
    Ok(())
}

fn validate_hash(hash: &IssueFieldHash) -> Result<(), IssueTriageError> {
    if hash.version == 0 || !is_sha256(&hash.sha256) {
        return Err(IssueTriageError::InvalidBasis);
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && is_lower_hex(value)
}
fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn sort_set<T: Ord>(values: &mut Vec<T>) {
    values.sort();
    values.dedup();
}
