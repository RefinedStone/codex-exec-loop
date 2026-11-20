use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::application::port::outbound::issue_automation_policy_source_port::{
    IssueAutomationPolicySourceDocument, IssueAutomationPolicySourcePort,
    IssueAutomationPolicySourceRead, IssueAutomationPolicySourceSnapshot,
};
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
        read_config_file_if_present(&project_policy_path(workspace_dir), ConfigScope::Project)
    }

    fn read_policy_snapshot(&self, workspace_dir: &str) -> IssueAutomationPolicySourceSnapshot {
        read_snapshot_from_paths(
            akra_home_dir().map(|home| home.join(ISSUE_AUTOMATION_POLICY_FILE_NAME)),
            project_policy_path(workspace_dir),
        )
    }
}

fn project_policy_path(workspace_dir: &str) -> PathBuf {
    Path::new(workspace_dir)
        .join(PROJECT_POLICY_DIRECTORY)
        .join(ISSUE_AUTOMATION_POLICY_FILE_NAME)
}

fn read_snapshot_from_paths(
    user_path: Result<PathBuf>,
    project_path: PathBuf,
) -> IssueAutomationPolicySourceSnapshot {
    let user = match user_path {
        Ok(path) => read_document(path, ConfigScope::Global),
        Err(_) => IssueAutomationPolicySourceDocument {
            location: None,
            read: IssueAutomationPolicySourceRead::Unavailable,
        },
    };
    IssueAutomationPolicySourceSnapshot {
        user,
        project: read_document(project_path, ConfigScope::Project),
    }
}

fn read_document(path: PathBuf, scope: ConfigScope) -> IssueAutomationPolicySourceDocument {
    // production의 workspace/home는 절대 경로다. 위치를 추측하거나 별도 재조회하지 않는다.
    if !path.is_absolute() {
        return IssueAutomationPolicySourceDocument {
            location: None,
            read: IssueAutomationPolicySourceRead::Unavailable,
        };
    }
    let result = read_config_file_if_present(&path, scope);
    IssueAutomationPolicySourceDocument::from_read(Some(path), result)
}

fn read_user_policy(akra_home: &Path) -> Result<Option<String>> {
    let path = akra_home.join(ISSUE_AUTOMATION_POLICY_FILE_NAME);
    read_config_file_if_present(&path, ConfigScope::Global)
}

#[cfg(test)]
mod tests;
