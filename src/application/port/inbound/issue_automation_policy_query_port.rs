//! 자동화 정책의 읽기 전용 검사 계약이다. 원문과 가변 정책 객체는 입구에 반환하지 않는다.
pub use crate::application::port::outbound::issue_automation_policy_source_port::{
    IssueAutomationPolicyFileScope, IssueAutomationPolicyFileStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueAutomationPolicyOriginScope {
    Builtin,
    User,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAutomationPolicyResolution {
    Default,
    Override,
    Minimum,
    And,
    Intersection,
    Union,
    Cap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationPolicyOrigin {
    pub scope: IssueAutomationPolicyOriginScope,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationPolicySetting {
    pub key: String,
    pub label: String,
    pub value: String,
    pub origins: Vec<IssueAutomationPolicyOrigin>,
    pub resolution: IssueAutomationPolicyResolution,
    pub resolution_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationPolicyFile {
    pub scope: IssueAutomationPolicyFileScope,
    pub label: String,
    pub path: Option<String>,
    pub status: IssueAutomationPolicyFileStatus,
    pub open_target: Option<IssueAutomationPolicyFileScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueAutomationPolicyInspectionBlocked {
    pub scope: IssueAutomationPolicyScope,
    pub field: &'static str,
    pub reason: &'static str,
    pub summary: String,
    pub next_summary: String,
    pub file_scopes: Vec<IssueAutomationPolicyFileScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAutomationPolicyInspection {
    Ready {
        workspace: String,
        version: String,
        settings: Vec<IssueAutomationPolicySetting>,
        files: Vec<IssueAutomationPolicyFile>,
    },
    PolicyBlocked {
        workspace: String,
        blocked: IssueAutomationPolicyInspectionBlocked,
        files: Vec<IssueAutomationPolicyFile>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAutomationPolicyScope {
    User,
    Project,
    Effective,
}

impl IssueAutomationPolicyScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Effective => "effective",
        }
    }
}

/// 파일 읽기 결과를 효과값·출처·차단 이유로 표시한다. 파일·장부·실행 상태를 바꾸지 않는다.
pub trait IssueAutomationPolicyQueryPort: Send + Sync {
    fn inspect(&self, workspace_dir: &str) -> IssueAutomationPolicyInspection;
}
