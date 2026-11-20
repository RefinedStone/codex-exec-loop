use serde::{Deserialize, Serialize};

use crate::domain::issue_work::{IssueComparisonBasis, IssueReference, IssueSharedCopy};

use super::state::{next_state, validate_resume, validate_state_replacement};
use super::{
    IssueAutomationBinding, IssueAutomationBindingId, IssueAutomationError, IssueAutomationFlag,
    IssueAutomationFlags, IssueAutomationState, IssueDecisionResumePhase, IssueTransition,
    MAX_ISSUE_AUTOMATION_REVISION, validate_reference, validate_text,
};

/// 원격 관찰의 원본 지문과 저장 가능한 공유본은 서로 다른 값이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueAutomationSnapshot {
    /// provider 원본 제목/본문의 해시와 provider updated_at이다.
    /// 마스킹으로 원본이 달라도 공유본은 같을 수 있으므로 공유본 해시와 비교하지 않는다.
    pub basis: IssueComparisonBasis,
    pub shared_copy: IssueSharedCopy,
    /// 이 공유본을 만든 공개 정책의 판이다.
    pub policy_version: String,
}

impl IssueAutomationSnapshot {
    fn validate(&self) -> Result<(), IssueAutomationError> {
        let hash = &self.basis.field_hash;
        if hash.version == 0
            || hash.sha256.len() != 64
            || !hash
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(IssueAutomationError::InvalidValue {
                field: "snapshot.basis.field_hash",
            });
        }
        validate_text(
            &self.basis.provider_updated_at,
            "snapshot.basis.provider_updated_at",
        )?;
        validate_text(&self.policy_version, "snapshot.policy_version")
    }
}

/// DB codec용 값이다. 도메인 객체에 직접 Deserialize를 제공하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersistedTrackedIssue {
    pub reference: IssueReference,
    pub binding_id: IssueAutomationBindingId,
    pub snapshot: IssueAutomationSnapshot,
    pub state: IssueAutomationState,
    pub flags: IssueAutomationFlags,
    pub decision_resume: Option<IssueDecisionResumePhase>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedIssue {
    reference: IssueReference,
    binding_id: IssueAutomationBindingId,
    snapshot: IssueAutomationSnapshot,
    state: IssueAutomationState,
    flags: IssueAutomationFlags,
    decision_resume: Option<IssueDecisionResumePhase>,
    // 저장소가 소유하는 기준 revision이다. 도메인 변경만으로 증가시키지 않는다.
    revision: u64,
}

impl TrackedIssue {
    pub fn discovered(
        reference: IssueReference,
        binding: &IssueAutomationBinding,
        snapshot: IssueAutomationSnapshot,
    ) -> Result<Self, IssueAutomationError> {
        validate_reference(&reference)?;
        snapshot.validate()?;
        if !binding.matches_reference(&reference) {
            return Err(IssueAutomationError::BindingChanged);
        }
        Ok(Self {
            reference,
            binding_id: binding.id().clone(),
            snapshot,
            state: IssueAutomationState::Discovered,
            flags: IssueAutomationFlags::default(),
            decision_resume: None,
            revision: 0,
        })
    }

    pub fn hydrate(record: PersistedTrackedIssue) -> Result<Self, IssueAutomationError> {
        validate_reference(&record.reference)?;
        validate_text(record.binding_id.as_str(), "binding.id")?;
        record.snapshot.validate()?;
        validate_resume(record.state, record.decision_resume)?;
        if record.revision == 0 || record.revision > MAX_ISSUE_AUTOMATION_REVISION {
            return Err(IssueAutomationError::InvalidRevision);
        }
        Ok(Self {
            reference: record.reference,
            binding_id: record.binding_id,
            snapshot: record.snapshot,
            state: record.state,
            flags: record.flags,
            decision_resume: record.decision_resume,
            revision: record.revision,
        })
    }

    /// 저장 전 발견 객체의 revision은 0이다. store가 insert/CAS로 판을 할당한 뒤 hydrate한다.
    pub fn to_record(&self) -> PersistedTrackedIssue {
        PersistedTrackedIssue {
            reference: self.reference.clone(),
            binding_id: self.binding_id.clone(),
            snapshot: self.snapshot.clone(),
            state: self.state,
            flags: self.flags,
            decision_resume: self.decision_resume,
            revision: self.revision,
        }
    }

    pub fn reference(&self) -> &IssueReference {
        &self.reference
    }

    pub fn binding_id(&self) -> &IssueAutomationBindingId {
        &self.binding_id
    }

    pub fn snapshot(&self) -> &IssueAutomationSnapshot {
        &self.snapshot
    }

    pub fn state(&self) -> IssueAutomationState {
        self.state
    }

    pub fn flags(&self) -> IssueAutomationFlags {
        self.flags
    }

    pub fn decision_resume(&self) -> Option<IssueDecisionResumePhase> {
        self.decision_resume
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn transition(&mut self, change: IssueTransition) -> Result<bool, IssueAutomationError> {
        let (state, decision_resume) = next_state(self.state, self.decision_resume, change)?;
        if (self.state, self.decision_resume) == (state, decision_resume) {
            return Ok(false);
        }
        self.state = state;
        self.decision_resume = decision_resume;
        Ok(true)
    }

    /// 판정/재동기화 규칙을 대신하지 않는다. snapshot 갱신은 상태/표시/바인딩을 초기화하지 않는다.
    pub fn replace_snapshot(
        &mut self,
        snapshot: IssueAutomationSnapshot,
    ) -> Result<bool, IssueAutomationError> {
        snapshot.validate()?;
        if self.snapshot == snapshot {
            return Ok(false);
        }
        self.snapshot = snapshot;
        Ok(true)
    }

    pub fn set_flag(&mut self, flag: IssueAutomationFlag, enabled: bool) -> bool {
        let current = match flag {
            IssueAutomationFlag::Held => &mut self.flags.held,
            IssueAutomationFlag::Stale => &mut self.flags.stale,
            IssueAutomationFlag::Unknown => &mut self.flags.unknown,
        };
        if *current == enabled {
            return false;
        }
        *current = enabled;
        true
    }

    /// 응답 유실 재시도용 비교다. revision만 제외하고 시각, 정책, 재개 단계도 비교한다.
    pub fn same_content(&self, other: &Self) -> bool {
        self.reference == other.reference
            && self.binding_id == other.binding_id
            && self.snapshot == other.snapshot
            && self.state == other.state
            && self.flags == other.flags
            && self.decision_resume == other.decision_resume
    }

    /// adapter는 현재 행과 후보를 이 함수로 검사한 뒤 같은 revision으로 CAS한다.
    /// 한 번의 CAS에는 상태 전이 한 단계만 저장할 수 있다.
    pub fn validate_replacement(&self, candidate: &Self) -> Result<(), IssueAutomationError> {
        if self.reference != candidate.reference {
            return Err(IssueAutomationError::IdentityChanged);
        }
        if self.binding_id != candidate.binding_id {
            return Err(IssueAutomationError::BindingChanged);
        }
        if self.revision != candidate.revision {
            return Err(IssueAutomationError::RevisionChanged);
        }
        candidate.snapshot.validate()?;
        validate_state_replacement(
            self.state,
            self.decision_resume,
            candidate.state,
            candidate.decision_resume,
        )
    }
}
