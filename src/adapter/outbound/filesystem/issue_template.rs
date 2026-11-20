use std::path::{Component, Path};

use anyhow::{Result, bail};

use crate::application::port::outbound::issue_template_source_port::IssueTemplateSourcePort;
use crate::configuration::{ConfigScope, read_config_file_if_present};

/// 이슈 템플릿을 작업 폴더에서 `config.toml` 과 같은 안전 규칙으로 읽는다 (링크, 크기 검사).
#[derive(Debug, Default)]
pub struct FilesystemIssueTemplateSourceAdapter;

impl FilesystemIssueTemplateSourceAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl IssueTemplateSourcePort for FilesystemIssueTemplateSourceAdapter {
    fn load_template(&self, workspace_dir: &str, template_path: &str) -> Result<Option<String>> {
        // 정책 검증(#2416)과 같은 규칙이다. 이 경계에서도 저장소 밖의 경로를 막는다.
        let inside_repository = Path::new(template_path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        if template_path.trim().is_empty() || !inside_repository {
            bail!("issue template must be a relative path inside the repository: `{template_path}`")
        }
        read_config_file_if_present(
            &Path::new(workspace_dir).join(template_path),
            ConfigScope::Project,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::FilesystemIssueTemplateSourceAdapter;
    use crate::application::port::outbound::issue_template_source_port::IssueTemplateSourcePort;

    const TEMPLATE_PATH: &str = ".github/ISSUE_TEMPLATE/bug.md";

    fn temp_workspace(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "akra-issue-template-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(path.join(".github/ISSUE_TEMPLATE")).expect("temp workspace");
        path
    }

    #[test]
    fn template_is_read_from_the_workspace_and_a_missing_file_is_none() {
        let workspace = temp_workspace("read");
        let workspace_dir = workspace.to_str().expect("utf-8 temp path");
        let adapter = FilesystemIssueTemplateSourceAdapter::new();

        assert_eq!(
            adapter
                .load_template(workspace_dir, TEMPLATE_PATH)
                .expect("a missing file is not an error"),
            None
        );

        fs::write(workspace.join(TEMPLATE_PATH), "## Summary\n").expect("template fixture");

        assert_eq!(
            adapter
                .load_template(workspace_dir, TEMPLATE_PATH)
                .expect("template should load"),
            Some("## Summary\n".to_string())
        );
        fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[test]
    fn a_path_outside_the_repository_is_an_error() {
        let workspace = temp_workspace("outside");
        let workspace_dir = workspace.to_str().expect("utf-8 temp path");
        let adapter = FilesystemIssueTemplateSourceAdapter::new();

        assert!(adapter.load_template(workspace_dir, "../bug.md").is_err());
        assert!(adapter.load_template(workspace_dir, "/etc/hosts").is_err());
        fs::remove_dir_all(workspace).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_template_file_is_an_error_and_not_a_missing_file() {
        let workspace = temp_workspace("link");
        fs::write(workspace.join("elsewhere.md"), "").expect("link target");
        std::os::unix::fs::symlink(
            workspace.join("elsewhere.md"),
            workspace.join(TEMPLATE_PATH),
        )
        .expect("symlink fixture");

        let result = FilesystemIssueTemplateSourceAdapter::new()
            .load_template(workspace.to_str().expect("utf-8 temp path"), TEMPLATE_PATH);

        assert!(result.is_err());
        fs::remove_dir_all(workspace).expect("cleanup");
    }
}
