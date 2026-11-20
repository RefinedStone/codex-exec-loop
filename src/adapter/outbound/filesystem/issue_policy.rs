use std::path::Path;

use anyhow::Result;

use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::configuration::{ConfigScope, akra_home_dir, read_config_file_if_present};

/// 사용자 파일은 `AKRA_HOME` 바로 아래에, 프로젝트 파일은 작업 폴더의 `.akra` 아래에 있다.
const ISSUE_POLICY_FILE_NAME: &str = "issue-policy.toml";
const PROJECT_POLICY_DIRECTORY: &str = ".akra";

/// 이슈 정책 파일을 `config.toml` 과 같은 안전 규칙으로 읽는다 (링크, 소유자, 크기 검사).
#[derive(Debug, Default)]
pub struct FilesystemIssuePolicySourceAdapter;

impl FilesystemIssuePolicySourceAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl IssuePolicySourcePort for FilesystemIssuePolicySourceAdapter {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        let path = akra_home_dir()?.join(ISSUE_POLICY_FILE_NAME);
        read_config_file_if_present(&path, ConfigScope::Global)
    }

    fn load_project_policy_toml(&self, workspace_dir: &str) -> Result<Option<String>> {
        let path = Path::new(workspace_dir)
            .join(PROJECT_POLICY_DIRECTORY)
            .join(ISSUE_POLICY_FILE_NAME);
        read_config_file_if_present(&path, ConfigScope::Project)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::FilesystemIssuePolicySourceAdapter;
    use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;

    fn temp_workspace(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "akra-issue-policy-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(path.join(".akra")).expect("temp workspace");
        path
    }

    #[test]
    fn project_policy_is_read_from_the_workspace_akra_directory() {
        let workspace = temp_workspace("read");
        let workspace_dir = workspace.to_str().expect("utf-8 temp path");
        let adapter = FilesystemIssuePolicySourceAdapter::new();

        assert_eq!(
            adapter
                .load_project_policy_toml(workspace_dir)
                .expect("a missing file is not an error"),
            None
        );

        fs::write(
            workspace.join(".akra/issue-policy.toml"),
            "schema_version = 1\n",
        )
        .expect("policy fixture");

        assert_eq!(
            adapter
                .load_project_policy_toml(workspace_dir)
                .expect("policy file should load"),
            Some("schema_version = 1\n".to_string())
        );
        fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_project_policy_file_is_an_error_and_not_a_missing_file() {
        let workspace = temp_workspace("link");
        fs::write(workspace.join("elsewhere.toml"), "").expect("link target");
        std::os::unix::fs::symlink(
            workspace.join("elsewhere.toml"),
            workspace.join(".akra/issue-policy.toml"),
        )
        .expect("symlink fixture");

        let result = FilesystemIssuePolicySourceAdapter::new()
            .load_project_policy_toml(workspace.to_str().expect("utf-8 temp path"));

        assert!(result.is_err());
        fs::remove_dir_all(workspace).expect("cleanup");
    }
}
