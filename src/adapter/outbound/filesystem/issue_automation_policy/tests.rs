use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{
    FilesystemIssueAutomationPolicySourceAdapter, ISSUE_AUTOMATION_POLICY_FILE_NAME,
    PROJECT_POLICY_DIRECTORY, read_user_policy,
};
use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;

const AUTOMATION_POLICY: &str = "schema_version = 1\n[binding.github]\nmode = \"observe\"\n";
const DISCLOSURE_POLICY: &str = "schema_version = 1\n";

struct Fixture(PathBuf);

impl Fixture {
    fn new(label: &str) -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-automation-policy-{label}-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        crate::private_directory::ensure_private_directory(&path)
            .expect("private temporary directory");
        Self(path)
    }

    fn workspace_dir(&self) -> &str {
        self.0.to_str().expect("UTF-8 temporary path")
    }

    fn project_directory(&self) -> PathBuf {
        let path = self.0.join(PROJECT_POLICY_DIRECTORY);
        crate::private_directory::ensure_private_directory(&path)
            .expect("private project configuration directory");
        path
    }

    fn project_policy_path(&self) -> PathBuf {
        self.project_directory()
            .join(ISSUE_AUTOMATION_POLICY_FILE_NAME)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_private_file(path: &Path, content: &[u8]) {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use crate::private_fs::{
            WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
            WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC,
        };
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_GENERIC_WRITE
                    | WINDOWS_READ_CONTROL
                    | WINDOWS_WRITE_DAC,
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let mut file = options.open(path).expect("new private policy fixture");
    #[cfg(windows)]
    crate::private_fs::set_windows_private_acl(&file, false)
        .expect("protect the fixture file ACL, not just its parent");
    file.write_all(content).expect("policy fixture contents");
}

#[test]
fn missing_project_policy_does_not_create_configuration_files() {
    let fixture = Fixture::new("missing");
    let adapter = FilesystemIssueAutomationPolicySourceAdapter::new();

    assert_eq!(
        adapter
            .load_project_policy_toml(fixture.workspace_dir())
            .unwrap(),
        None
    );
    assert!(!fixture.0.join(PROJECT_POLICY_DIRECTORY).exists());

    let project = fixture.project_directory();
    assert_eq!(
        adapter
            .load_project_policy_toml(fixture.workspace_dir())
            .unwrap(),
        None
    );
    assert!(!project.join(ISSUE_AUTOMATION_POLICY_FILE_NAME).exists());
}

#[test]
fn project_policy_returns_the_exact_file_without_parsing_toml() {
    let fixture = Fixture::new("project");
    let contents = "# 원문 설정은 application에서 해석한다.\nnot valid TOML = [\n";
    write_private_file(&fixture.project_policy_path(), contents.as_bytes());

    assert_eq!(
        FilesystemIssueAutomationPolicySourceAdapter::new()
            .load_project_policy_toml(fixture.workspace_dir())
            .unwrap(),
        Some(contents.to_owned())
    );
}

#[test]
fn user_policy_uses_the_home_root_and_ignores_other_policy_paths() {
    let fixture = Fixture::new("user");
    assert_eq!(read_user_policy(&fixture.0).unwrap(), None);

    write_private_file(&fixture.project_policy_path(), b"project-only");
    write_private_file(
        &fixture.0.join("issue-policy.toml"),
        DISCLOSURE_POLICY.as_bytes(),
    );
    assert_eq!(read_user_policy(&fixture.0).unwrap(), None);

    write_private_file(
        &fixture.0.join(ISSUE_AUTOMATION_POLICY_FILE_NAME),
        AUTOMATION_POLICY.as_bytes(),
    );
    assert_eq!(
        read_user_policy(&fixture.0).unwrap(),
        Some(AUTOMATION_POLICY.to_owned())
    );
}

#[test]
fn invalid_files_are_errors_instead_of_missing_policy() {
    let directory = Fixture::new("non-file");
    fs::create_dir(directory.project_policy_path()).unwrap();
    let adapter = FilesystemIssueAutomationPolicySourceAdapter::new();
    assert!(
        adapter
            .load_project_policy_toml(directory.workspace_dir())
            .is_err()
    );

    for (label, contents) in [
        ("invalid-utf8", vec![0xff]),
        ("oversized", vec![b'x'; 1024 * 1024 + 1]),
    ] {
        let fixture = Fixture::new(label);
        write_private_file(&fixture.project_policy_path(), &contents);
        assert!(
            adapter
                .load_project_policy_toml(fixture.workspace_dir())
                .is_err(),
            "{label} must not fall back to an absent policy"
        );
    }
}

#[test]
fn automation_and_disclosure_files_fail_independently() {
    let fixture = Fixture::new("independent");
    let automation_path = fixture.project_policy_path();
    let disclosure_path = fixture.project_directory().join("issue-policy.toml");
    write_private_file(&automation_path, AUTOMATION_POLICY.as_bytes());
    write_private_file(&disclosure_path, &[0xff]);
    let automation = FilesystemIssueAutomationPolicySourceAdapter::new();
    let disclosure = FilesystemIssuePolicySourceAdapter::new();

    assert_eq!(
        automation
            .load_project_policy_toml(fixture.workspace_dir())
            .unwrap(),
        Some(AUTOMATION_POLICY.to_owned())
    );
    assert!(
        disclosure
            .load_project_policy_toml(fixture.workspace_dir())
            .is_err()
    );

    fs::write(&disclosure_path, DISCLOSURE_POLICY).unwrap();
    fs::write(&automation_path, [0xff]).unwrap();
    assert_eq!(
        disclosure
            .load_project_policy_toml(fixture.workspace_dir())
            .unwrap(),
        Some(DISCLOSURE_POLICY.to_owned())
    );
    assert!(
        automation
            .load_project_policy_toml(fixture.workspace_dir())
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn linked_project_policy_is_an_error_even_when_the_target_is_valid() {
    let fixture = Fixture::new("linked-file");
    let target = fixture.0.join("target.toml");
    write_private_file(&target, AUTOMATION_POLICY.as_bytes());
    std::os::unix::fs::symlink(&target, fixture.project_policy_path()).unwrap();

    assert!(
        FilesystemIssueAutomationPolicySourceAdapter::new()
            .load_project_policy_toml(fixture.workspace_dir())
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn linked_project_directory_is_an_error_even_when_the_policy_is_missing() {
    let fixture = Fixture::new("linked-parent");
    let target = Fixture::new("parent-target");
    std::os::unix::fs::symlink(&target.0, fixture.0.join(PROJECT_POLICY_DIRECTORY)).unwrap();

    assert!(
        FilesystemIssueAutomationPolicySourceAdapter::new()
            .load_project_policy_toml(fixture.workspace_dir())
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn user_policy_keeps_global_file_and_parent_permission_checks() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("user-permissions");
    let path = fixture.0.join(ISSUE_AUTOMATION_POLICY_FILE_NAME);
    write_private_file(&path, AUTOMATION_POLICY.as_bytes());
    assert_eq!(
        read_user_policy(&fixture.0).unwrap(),
        Some(AUTOMATION_POLICY.to_owned())
    );

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_user_policy(&fixture.0).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(read_user_policy(&fixture.0).is_err());
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        read_user_policy(&fixture.0).unwrap(),
        Some(AUTOMATION_POLICY.to_owned())
    );
}
