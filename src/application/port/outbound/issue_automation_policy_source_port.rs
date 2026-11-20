use anyhow::Result;

/// 자동화 정책 파일만 읽는 경계다 (#2540). 기존 issue-policy와 별도로 실패한다.
/// 파일 없음은 None, 안전하게 읽지 못한 파일은 Err이다. application은 경로를 열지 않는다.
pub trait IssueAutomationPolicySourcePort: Send + Sync {
    fn load_user_policy_toml(&self) -> Result<Option<String>>;
    fn load_project_policy_toml(&self, workspace_dir: &str) -> Result<Option<String>>;
}
