//! 자동화 정책만 읽고 해석한다 (#2540). 기존 issue-policy, 수집 설정, 실행 허가는 건드리지 않는다.
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::domain::issue_automation::policy::{
    IssueAutomationPolicy, IssueAutomationPolicyValueError,
};

mod inspection;
mod layer;
mod merge;
mod provenance;

pub use crate::application::port::inbound::issue_automation_policy_query_port::{
    IssueAutomationPolicyFile, IssueAutomationPolicyFileScope, IssueAutomationPolicyFileStatus,
    IssueAutomationPolicyInspection, IssueAutomationPolicyInspectionBlocked,
    IssueAutomationPolicyOrigin, IssueAutomationPolicyOriginScope, IssueAutomationPolicyResolution,
    IssueAutomationPolicyScope, IssueAutomationPolicySetting,
};

/// 원문, 파일 경로, 공급자가 보낸 오류는 이 값에 넣지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueAutomationPolicyBlocked {
    pub scope: IssueAutomationPolicyScope,
    pub field: &'static str,
    pub reason: &'static str,
}

impl IssueAutomationPolicyBlocked {
    fn invalid(scope: IssueAutomationPolicyScope, error: IssueAutomationPolicyValueError) -> Self {
        Self {
            scope,
            field: error.field,
            reason: error.reason,
        }
    }

    fn unreadable(scope: IssueAutomationPolicyScope) -> Self {
        Self {
            scope,
            field: "policy",
            reason: "source_unavailable",
        }
    }
}

impl std::fmt::Display for IssueAutomationPolicyBlocked {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "policy_blocked: {} {}: {}",
            self.scope.label(),
            self.field,
            self.reason
        )
    }
}

impl std::error::Error for IssueAutomationPolicyBlocked {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIssueAutomationPolicy {
    pub policy: IssueAutomationPolicy,
    /// 정규화한 효과값의 JSON을 SHA-256으로 계산한 소문자 16진수다.
    pub version: String,
}

pub struct IssueAutomationPolicyService {
    source: Arc<dyn IssueAutomationPolicySourcePort>,
}

impl IssueAutomationPolicyService {
    pub fn new(source: Arc<dyn IssueAutomationPolicySourcePort>) -> Self {
        Self { source }
    }

    /// 파일을 바꾸지 않고 같은 source snapshot의 효과값·출처·파일 상태를 표시한다.
    /// workspace_dir는 composition이 정한 대표 workspace이며 이 값에서 경로를 추론하지 않는다.
    pub fn inspect(&self, workspace_dir: &str) -> IssueAutomationPolicyInspection {
        inspection::inspect(
            workspace_dir,
            self.source.read_policy_snapshot(workspace_dir),
        )
    }

    pub fn resolve(
        &self,
        workspace_dir: &str,
    ) -> Result<ResolvedIssueAutomationPolicy, IssueAutomationPolicyBlocked> {
        let user = self.source.load_user_policy_toml().map_err(|_| {
            IssueAutomationPolicyBlocked::unreadable(IssueAutomationPolicyScope::User)
        })?;
        let project = self
            .source
            .load_project_policy_toml(workspace_dir)
            .map_err(|_| {
                IssueAutomationPolicyBlocked::unreadable(IssueAutomationPolicyScope::Project)
            })?;
        resolve_issue_automation_policy(user.as_deref(), project.as_deref())
    }
}

impl crate::application::port::inbound::issue_automation_policy_query_port::IssueAutomationPolicyQueryPort
    for IssueAutomationPolicyService
{
    fn inspect(&self, workspace_dir: &str) -> IssueAutomationPolicyInspection {
        IssueAutomationPolicyService::inspect(self, workspace_dir)
    }
}

/// 파일이 없을 때만 None이다. 하나라도 잘못되면 기본값이나 다른 파일로 대신하지 않는다.
pub fn resolve_issue_automation_policy(
    user_toml: Option<&str>,
    project_toml: Option<&str>,
) -> Result<ResolvedIssueAutomationPolicy, IssueAutomationPolicyBlocked> {
    resolve_with_provenance(user_toml, project_toml).map(|(resolved, _)| resolved)
}

fn resolve_with_provenance(
    user_toml: Option<&str>,
    project_toml: Option<&str>,
) -> Result<
    (ResolvedIssueAutomationPolicy, provenance::PolicyProvenance),
    IssueAutomationPolicyBlocked,
> {
    let user = layer::parse(user_toml, IssueAutomationPolicyScope::User)?;
    let project = layer::parse(project_toml, IssueAutomationPolicyScope::Project)?;
    layer::validate_jira_layers(&user, &project)?;
    let (mut policy, origins) = merge::merge(user, project);
    policy.normalize();
    policy.validate().map_err(|error| {
        IssueAutomationPolicyBlocked::invalid(IssueAutomationPolicyScope::Effective, error)
    })?;
    // 구조체의 고정 필드 순서와 정렬된 집합만 직렬화한다. HashMap, 원문, Debug 표현은 사용하지 않는다.
    let canonical_json = serde_json::to_vec(&policy).map_err(|_| IssueAutomationPolicyBlocked {
        scope: IssueAutomationPolicyScope::Effective,
        field: "policy",
        reason: "canonical_encoding_failed",
    })?;
    let version = format!("{:x}", Sha256::digest(canonical_json));
    Ok((ResolvedIssueAutomationPolicy { policy, version }, origins))
}

#[cfg(test)]
mod tests;
