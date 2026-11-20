//! 로컬 생성 기록에서 얻은 안정 ID만 출처 관찰에 보탠다. 사람의 승인 증거가 아니다.
use std::collections::BTreeSet;

use anyhow::Result;

use crate::domain::issue_automation::source::{IssueCreationProvenance, IssueSourceRepository};
use crate::domain::issue_work::IssueReference;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCreationProvenanceSnapshot {
    pub repository: IssueSourceRepository,
    pub created_issue_ids: BTreeSet<String>,
    /// 원격 전송 뒤 안정 ID를 확인하지 못한 시도가 이 저장소에 남아 있다.
    pub unresolved: bool,
}

impl IssueCreationProvenanceSnapshot {
    pub fn classify(&self, issue: &IssueReference) -> IssueCreationProvenance {
        if !self.repository.matches_reference(issue) {
            return IssueCreationProvenance::Unresolved;
        }
        if self.created_issue_ids.contains(&issue.issue_id) {
            IssueCreationProvenance::AkraCreated
        } else if self.unresolved {
            IssueCreationProvenance::Unresolved
        } else {
            // 로컬 기록이 없다는 뜻이며 사람이 만들었다는 증명은 아니다.
            IssueCreationProvenance::Unattributed
        }
    }
}

pub trait IssueCreationProvenancePort: Send + Sync {
    /// 원격 응답을 받은 뒤 한 번 읽는다. 승인 공유본이나 Vault 내용은 반환하지 않는다.
    fn load(&self, repository: &IssueSourceRepository) -> Result<IssueCreationProvenanceSnapshot>;
}
