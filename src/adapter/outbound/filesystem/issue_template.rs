use std::path::{Component, Path};

use anyhow::{Result, bail};

use crate::application::port::outbound::issue_template_source_port::IssueTemplateSourcePort;

/// 이슈 템플릿을 루트에 고정한 경로에서 읽는다 (모든 부모 링크와 크기 검사).
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
        read_template(Path::new(workspace_dir), Path::new(template_path))
    }
}

#[cfg(windows)]
mod windows;

const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;

#[cfg(not(windows))]
fn read_template(root: &Path, relative: &Path) -> Result<Option<String>> {
    // Keep the established descriptor-anchored reader and its platform boundary.
    super::secure_fs::read_optional_file_with_limit(root, relative, MAX_TEMPLATE_BYTES)
}

#[cfg(windows)]
fn read_template(root: &Path, relative: &Path) -> Result<Option<String>> {
    windows::read_template(root, relative, MAX_TEMPLATE_BYTES)
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
    #[cfg(unix)]
    fn directory_link(target: &std::path::Path, link: &std::path::Path) {
        std::os::unix::fs::symlink(target, link).expect("directory symlink");
    }

    #[cfg(unix)]
    #[test]
    fn every_template_directory_rejects_links_outside_the_workspace() {
        for ancestor in [".github", ".github/ISSUE_TEMPLATE"] {
            let workspace = temp_workspace("ancestor");
            let outside = temp_workspace("outside-fixture");
            fs::write(outside.join(TEMPLATE_PATH), "synthetic outside contents").unwrap();
            let link = workspace.join(ancestor);
            fs::remove_dir_all(&link).unwrap();
            directory_link(&outside.join(ancestor), &link);
            let result = FilesystemIssueTemplateSourceAdapter::new()
                .load_template(workspace.to_str().unwrap(), TEMPLATE_PATH);
            assert!(result.is_err(), "{ancestor}: {result:?}");
            #[cfg(unix)]
            fs::remove_file(&link).unwrap();
            #[cfg(windows)]
            fs::remove_dir(&link).unwrap();
            fs::remove_dir_all(workspace).unwrap();
            fs::remove_dir_all(outside).unwrap();
        }
    }

    #[test]
    fn template_reads_keep_the_existing_one_mebibyte_limit() {
        let workspace = temp_workspace("size");
        fs::write(
            workspace.join(TEMPLATE_PATH),
            "x".repeat(super::MAX_TEMPLATE_BYTES + 1),
        )
        .unwrap();
        assert!(
            FilesystemIssueTemplateSourceAdapter::new()
                .load_template(workspace.to_str().unwrap(), TEMPLATE_PATH)
                .is_err()
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn empty_and_exact_limit_templates_are_readable_but_invalid_utf8_is_not() {
        let workspace = temp_workspace("boundary");
        let adapter = FilesystemIssueTemplateSourceAdapter::new();
        for body in [String::new(), "x".repeat(super::MAX_TEMPLATE_BYTES)] {
            fs::write(workspace.join(TEMPLATE_PATH), body.as_bytes()).unwrap();
            assert_eq!(
                adapter
                    .load_template(workspace.to_str().unwrap(), TEMPLATE_PATH)
                    .unwrap(),
                Some(body)
            );
        }
        fs::write(workspace.join(TEMPLATE_PATH), [0xff, 0xfe]).unwrap();
        assert!(
            adapter
                .load_template(workspace.to_str().unwrap(), TEMPLATE_PATH)
                .is_err()
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn filesystem_templates_resolve_required_fields_and_reject_stale_versions() {
        use std::sync::Arc;

        use crate::application::service::issue_template::IssueTemplateService;
        use crate::domain::issue_template::{
            IssueTemplateFormat, IssueTemplateInputKind, IssueTemplateResolution,
            IssueTemplateVersionCheck,
        };

        let workspace = temp_workspace("service");
        let cwd = workspace.to_str().unwrap();
        let source =
            IssueTemplateService::new(Arc::new(FilesystemIssueTemplateSourceAdapter::new()));
        for (path, body, expected_format, expected_label, expected_kind) in [
            (
                TEMPLATE_PATH,
                "## 재현 순서\n",
                IssueTemplateFormat::Markdown,
                "재현 순서",
                IssueTemplateInputKind::Section,
            ),
            (
                ".github/ISSUE_TEMPLATE/bug.yaml",
                "name: Bug\nbody:\n  - type: textarea\n    id: steps\n    attributes:\n      label: 재현 순서\n    validations:\n      required: true\n",
                IssueTemplateFormat::IssueForm,
                "재현 순서",
                IssueTemplateInputKind::Textarea,
            ),
        ] {
            fs::write(workspace.join(path), body).unwrap();
            let IssueTemplateResolution::Resolved(convention) = source.resolve(cwd, Some(path))
            else {
                panic!("filesystem template should resolve: {path}");
            };
            assert_eq!(convention.format, expected_format);
            assert_eq!(convention.required_inputs.len(), 1);
            assert_eq!(convention.required_inputs[0].label, expected_label);
            assert_eq!(convention.required_inputs[0].kind, expected_kind);
            assert_eq!(
                source.check_draft_version(cwd, Some(path), Some(&convention.version)),
                IssueTemplateVersionCheck::Unchanged
            );
            fs::write(workspace.join(path), format!("{body}\n")).unwrap();
            assert!(matches!(
                source.check_draft_version(cwd, Some(path), Some(&convention.version)),
                IssueTemplateVersionCheck::ReviewRequired { .. }
            ));
        }
        fs::write(workspace.join(TEMPLATE_PATH), [0xff]).unwrap();
        assert!(matches!(
            source.resolve(cwd, Some(TEMPLATE_PATH)),
            IssueTemplateResolution::Unreadable { .. }
        ));
        fs::remove_file(workspace.join(TEMPLATE_PATH)).unwrap();
        assert!(matches!(
            source.resolve(cwd, Some(TEMPLATE_PATH)),
            IssueTemplateResolution::Missing { .. }
        ));
        fs::remove_dir_all(workspace).unwrap();
    }
}
