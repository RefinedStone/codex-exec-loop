use std::path::Path;

use anyhow::Result;

use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::configuration::{ConfigScope, akra_home_dir, read_config_file_if_present};

const ISSUE_AUTOMATION_POLICY_FILE_NAME: &str = "issue-automation.toml";
const PROJECT_POLICY_DIRECTORY: &str = ".akra";

/// 이슈 공개 정책과 독립된 자동화 정책 파일을 기존 설정 안전 규칙으로 읽는다.
#[derive(Debug, Default)]
pub struct FilesystemIssueAutomationPolicySourceAdapter;

impl FilesystemIssueAutomationPolicySourceAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl IssueAutomationPolicySourcePort for FilesystemIssueAutomationPolicySourceAdapter {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        read_user_policy(&akra_home_dir()?)
    }

    fn load_project_policy_toml(&self, workspace_dir: &str) -> Result<Option<String>> {
        let path = Path::new(workspace_dir)
            .join(PROJECT_POLICY_DIRECTORY)
            .join(ISSUE_AUTOMATION_POLICY_FILE_NAME);
        read_config_file_if_present(&path, ConfigScope::Project)
    }
}

fn read_user_policy(akra_home: &Path) -> Result<Option<String>> {
    let path = akra_home.join(ISSUE_AUTOMATION_POLICY_FILE_NAME);
    read_config_file_if_present(&path, ConfigScope::Global)
}

#[cfg(test)]
mod tests;
