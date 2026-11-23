//! 고정한 integration base부터 전달 source까지의 실제 diff를 판정한다.
//! 정책의 보호 경로를 좁히거나 실행 허가를 만들지 않는다.
use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::policy::IssueAutomationPolicy;
use super::triage::matches_glob;

mod assessment;
pub use assessment::IssueDeliveryAssessment;

/// 자동 전달의 고정 상한이다. 정확히 이 값인 diff는 크기 때문에 사람을 요구하지 않는다.
pub const MAX_AUTOMATIC_DELIVERY_FILES: u64 = 50;
pub const MAX_AUTOMATIC_DELIVERY_CHANGED_LINES: u64 = 1_000;
/// 수집 한계를 넘은 결과를 부분적인 정상 diff로 사용하지 않는다.
pub const MAX_ISSUE_DELIVERY_DIFF_FILES: usize = 4_096;
pub const MAX_ISSUE_DELIVERY_DIFF_PATH_BYTES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDeliveryDiff {
    pub base_commit_sha: String,
    pub source_commit_sha: String,
    /// Rename은 old/delete와 new/add 두 경로로 보존한다.
    pub files: Vec<IssueDeliveryDiffFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDeliveryDiffFile {
    pub path: String,
    pub lines: IssueDeliveryDiffLines,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IssueDeliveryDiffLines {
    Text {
        added: u64,
        deleted: u64,
    },
    /// Git numstat의 두 '-'는 0줄이 아니다. 항상 별도 사람 검토 사유다.
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueDeliveryRiskReason {
    ProtectedPath,
    DiffTooLarge,
    BinaryDiff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueDeliveryDiffVerdict {
    pub changed_files: u64,
    /// Binary가 포함되면 전체 줄 수를 안다고 표시하지 않는다.
    pub changed_lines: Option<u64>,
    pub reasons: Vec<IssueDeliveryRiskReason>,
}

impl IssueDeliveryDiffVerdict {
    pub fn requires_human(&self) -> bool {
        !self.reasons.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.changed_files == 0
            || !usize::try_from(self.changed_files)
                .is_ok_and(|count| count <= MAX_ISSUE_DELIVERY_DIFF_FILES)
            || self
                .reasons
                .iter()
                .enumerate()
                .any(|(index, reason)| self.reasons[..index].contains(reason))
            || (self.reasons.contains(&IssueDeliveryRiskReason::BinaryDiff)
                != self.changed_lines.is_none())
        {
            return Err("the delivery diff verdict is incomplete or inconsistent".into());
        }
        let large = self
            .reasons
            .contains(&IssueDeliveryRiskReason::DiffTooLarge);
        let known_large = self.changed_files > MAX_AUTOMATIC_DELIVERY_FILES
            || self
                .changed_lines
                .is_some_and(|lines| lines > MAX_AUTOMATIC_DELIVERY_CHANGED_LINES);
        let known_small = self.changed_files <= MAX_AUTOMATIC_DELIVERY_FILES
            && self
                .changed_lines
                .is_some_and(|lines| lines <= MAX_AUTOMATIC_DELIVERY_CHANGED_LINES);
        if (known_large && !large) || (known_small && large) {
            return Err("the delivery diff verdict does not match its size bounds".into());
        }
        Ok(())
    }
}

impl IssueDeliveryDiff {
    pub fn validate(&self) -> Result<(), String> {
        validate_commit_range(&self.base_commit_sha, &self.source_commit_sha)?;
        if self.files.is_empty() || self.files.len() > MAX_ISSUE_DELIVERY_DIFF_FILES {
            return Err("the complete delivery diff exceeds its file bound or is empty".into());
        }
        let mut paths = BTreeSet::new();
        let mut changed_lines = 0u64;
        for file in &self.files {
            validate_delivery_path(&file.path)?;
            if !paths.insert(file.path.as_str()) {
                return Err("the delivery diff contains a repeated path".into());
            }
            if let IssueDeliveryDiffLines::Text { added, deleted } = file.lines {
                changed_lines = add_changed_lines(changed_lines, added, deleted)?;
            }
        }
        Ok(())
    }
}

pub fn assess_delivery_diff(
    policy: &IssueAutomationPolicy,
    diff: &IssueDeliveryDiff,
) -> Result<IssueDeliveryDiffVerdict, String> {
    policy
        .validate()
        .map_err(|_| "delivery diff assessment requires a valid effective policy".to_string())?;
    diff.validate()?;
    let changed_files = u64::try_from(diff.files.len())
        .map_err(|_| "the delivery diff file count is invalid".to_string())?;
    let mut changed_lines = 0u64;
    let mut binary = false;
    let mut protected = false;
    for file in &diff.files {
        for pattern in &policy.protection.paths {
            protected |= matches_glob(pattern, &file.path).ok_or_else(|| {
                "a delivery protection pattern could not be evaluated".to_string()
            })?;
        }
        match file.lines {
            IssueDeliveryDiffLines::Text { added, deleted } => {
                changed_lines = add_changed_lines(changed_lines, added, deleted)?;
            }
            IssueDeliveryDiffLines::Binary => binary = true,
        }
    }
    let mut reasons = Vec::new();
    if protected {
        reasons.push(IssueDeliveryRiskReason::ProtectedPath);
    }
    if changed_files > MAX_AUTOMATIC_DELIVERY_FILES
        || changed_lines > MAX_AUTOMATIC_DELIVERY_CHANGED_LINES
    {
        reasons.push(IssueDeliveryRiskReason::DiffTooLarge);
    }
    if binary {
        reasons.push(IssueDeliveryRiskReason::BinaryDiff);
    }
    let verdict = IssueDeliveryDiffVerdict {
        changed_files,
        changed_lines: (!binary).then_some(changed_lines),
        reasons,
    };
    verdict.validate()?;
    Ok(verdict)
}

pub(crate) fn validate_commit_range(base: &str, source: &str) -> Result<(), String> {
    let exact_sha = |sha: &str| {
        matches!(sha.len(), 40 | 64)
            && sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if !exact_sha(base) || !exact_sha(source) || base.len() != source.len() || base == source {
        return Err("the delivery diff requires distinct exact commit object IDs".into());
    }
    Ok(())
}

fn validate_delivery_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > MAX_ISSUE_DELIVERY_DIFF_PATH_BYTES
        || path.starts_with('/')
        || path
            .chars()
            .any(|character| character.is_control() || matches!(character, '\\' | ':'))
        || path
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err("the delivery diff contains an unsupported repository path".into());
    }
    Ok(())
}

fn add_changed_lines(total: u64, added: u64, deleted: u64) -> Result<u64, String> {
    total
        .checked_add(added)
        .and_then(|value| value.checked_add(deleted))
        .ok_or_else(|| "the delivery diff line count overflowed".into())
}

#[cfg(test)]
mod tests;
