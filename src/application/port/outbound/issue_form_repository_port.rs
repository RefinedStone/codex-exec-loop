//! 이슈 폼 설치의 로컬 Git object/ref 경계. checkout/index 파일은 변경하지 않는다.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ISSUE_FORM_PATH: &str = ".github/ISSUE_TEMPLATE/akra-work.yml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFormRepository {
    pub workspace_dir: String,
    pub common_git_dir: String,
    pub push_remote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFormTarget {
    pub repository: String,
    pub push_remote: String,
    pub push_url: String,
    pub base_branch: String,
    pub asset_sha256: String,
}

impl IssueFormTarget {
    pub fn intent_key(&self) -> String {
        // Aliases and HTTPS/SSH choices do not create separate intents for the same repository.
        let material = serde_json::to_vec(&(
            "akra-issue-form-install-v1",
            self.repository.to_ascii_lowercase(),
            &self.base_branch,
            &self.asset_sha256,
        ))
        .expect("a tuple of strings always serializes");
        format!("{:x}", Sha256::digest(material))
    }

    pub fn journal_ref(&self) -> String {
        format!("refs/akra/issue-form/{}/journal", self.intent_key())
    }

    pub fn base_ref(&self) -> String {
        format!("refs/akra/issue-form/{}/base", self.intent_key())
    }

    pub fn commit_ref(&self) -> String {
        format!("refs/akra/issue-form/{}/commit", self.intent_key())
    }

    pub fn head_branch(&self) -> String {
        format!("akra/issue-form/{}", &self.intent_key()[..24])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFormInstallPlan {
    pub target: IssueFormTarget,
    pub base_commit_sha: String,
    pub commit_sha: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueFormInstallPhase {
    Prepared,
    PushSent,
    BranchReady,
    Sent,
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFormPullRequest {
    pub number: u64,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueFormInstallRecord {
    pub version: u32,
    pub plan: IssueFormInstallPlan,
    pub phase: IssueFormInstallPhase,
    pub pull_request: Option<IssueFormPullRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIssueFormInstall {
    pub object_id: String,
    pub record: IssueFormInstallRecord,
}

#[derive(Debug)]
pub enum IssueFormPreparation {
    AlreadyInstalled,
    Conflict,
    Prepared(Box<StoredIssueFormInstall>),
}

#[derive(Debug)]
pub enum IssueFormInstallSaveResult {
    Saved(Box<StoredIssueFormInstall>),
    Conflict,
}

pub trait IssueFormRepositoryPort: Send + Sync {
    fn inspect(&self, workspace_dir: &str, base_branch: &str) -> Result<IssueFormRepository>;

    /// Checks the private ref namespace even when no journal exists. Call before the isolated fetch.
    fn load(
        &self,
        repository: &IssueFormRepository,
        target: &IssueFormTarget,
    ) -> Result<Option<StoredIssueFormInstall>>;

    /// The immutable base object has already been fetched from the frozen target.
    /// Missing form produces one canonical plan; identical form is a no-op; other content is a conflict.
    fn prepare(
        &self,
        repository: &IssueFormRepository,
        target: &IssueFormTarget,
        base_commit_sha: &str,
        contents: &str,
    ) -> Result<IssueFormPreparation>;

    /// CAS is process-shared. A successful Sent save must be durable before the GitHub mutation.
    /// Only the same plan and one allowed phase transition may replace a record.
    fn save(
        &self,
        repository: &IssueFormRepository,
        expected: &StoredIssueFormInstall,
        candidate: &IssueFormInstallRecord,
    ) -> Result<IssueFormInstallSaveResult>;
}
