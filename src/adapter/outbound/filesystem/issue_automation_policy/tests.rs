use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{
    FilesystemIssueAutomationPolicySourceAdapter, ISSUE_AUTOMATION_POLICY_FILE_NAME,
    PROJECT_POLICY_DIRECTORY, read_snapshot_from_paths, read_user_policy,
};
use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::{
    IssueAutomationPolicySourcePort, IssueAutomationPolicySourceRead,
};
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

#[test]
fn source_snapshot_keeps_missing_paths_without_creating_files_or_directories() {
    let user = Fixture::new("snapshot-missing-user");
    let project = Fixture::new("snapshot-missing-project");
    let user_path = user.0.join(ISSUE_AUTOMATION_POLICY_FILE_NAME);
    let project_path = project
        .0
        .join(PROJECT_POLICY_DIRECTORY)
        .join(ISSUE_AUTOMATION_POLICY_FILE_NAME);
    let snapshot = read_snapshot_from_paths(Ok(user_path.clone()), project_path.clone());

    assert_eq!(snapshot.user.location, Some(user_path.clone()));
    assert_eq!(snapshot.project.location, Some(project_path.clone()));
    assert!(matches!(
        snapshot.user.read,
        IssueAutomationPolicySourceRead::Missing
    ));
    assert!(matches!(
        snapshot.project.read,
        IssueAutomationPolicySourceRead::Missing
    ));
    assert!(!user_path.exists());
    assert!(!project_path.exists());
    assert!(!project.0.join(PROJECT_POLICY_DIRECTORY).exists());
}

#[test]
fn source_snapshot_pairs_each_location_with_the_exact_file_read() {
    let user = Fixture::new("snapshot-user");
    let project = Fixture::new("snapshot-project");
    let user_path = user.0.join(ISSUE_AUTOMATION_POLICY_FILE_NAME);
    let project_path = project.project_policy_path();
    let user_text = "[budget]\nmax_running=4\n";
    let project_text = "[budget]\nmax_running=2\n";
    write_private_file(&user_path, user_text.as_bytes());
    write_private_file(&project_path, project_text.as_bytes());
    write_private_file(&user.0.join("issue-policy.toml"), b"not automation");
    write_private_file(
        &project.project_directory().join("issue-policy.toml"),
        b"not automation",
    );

    let snapshot = read_snapshot_from_paths(Ok(user_path.clone()), project_path.clone());
    fs::write(&project_path, b"[budget]\nmax_running=1\n").unwrap();

    assert_eq!(snapshot.user.location.as_ref(), Some(&user_path));
    assert_eq!(snapshot.project.location.as_ref(), Some(&project_path));
    let IssueAutomationPolicySourceRead::Contents(user_contents) = snapshot.user.read else {
        panic!("the user file must be readable")
    };
    let IssueAutomationPolicySourceRead::Contents(project_contents) = snapshot.project.read else {
        panic!("the project file must be readable")
    };
    assert_eq!(user_contents, user_text);
    assert_eq!(project_contents, project_text);
    assert_eq!(fs::read(&user_path).unwrap(), user_text.as_bytes());
}

#[test]
fn source_snapshot_retains_known_unreadable_locations_but_never_extracts_a_path_from_errors() {
    let project = Fixture::new("snapshot-failure");
    let project_path = project.project_policy_path();
    write_private_file(&project_path, &[0xff]);
    let snapshot = read_snapshot_from_paths(
        Err(anyhow::anyhow!(
            "synthetic-private-error /invented-policy-path"
        )),
        project_path.clone(),
    );

    assert!(snapshot.user.location.is_none());
    assert!(matches!(
        snapshot.user.read,
        IssueAutomationPolicySourceRead::Unavailable
    ));
    assert_eq!(snapshot.project.location, Some(project_path));
    assert!(matches!(
        snapshot.project.read,
        IssueAutomationPolicySourceRead::Unavailable
    ));
}

#[cfg(unix)]
#[test]
fn source_snapshot_keeps_existing_link_rejection_and_the_logical_policy_location() {
    let user = Fixture::new("snapshot-linked-user");
    let project = Fixture::new("snapshot-linked-project");
    let target = project.0.join("outside.toml");
    let project_path = project.project_policy_path();
    write_private_file(&target, AUTOMATION_POLICY.as_bytes());
    std::os::unix::fs::symlink(&target, &project_path).unwrap();

    let snapshot = read_snapshot_from_paths(
        Ok(user.0.join(ISSUE_AUTOMATION_POLICY_FILE_NAME)),
        project_path.clone(),
    );
    assert!(matches!(
        snapshot.user.read,
        IssueAutomationPolicySourceRead::Missing
    ));
    assert_eq!(snapshot.project.location, Some(project_path));
    assert!(matches!(
        snapshot.project.read,
        IssueAutomationPolicySourceRead::Unavailable
    ));
    assert_eq!(fs::read(&target).unwrap(), AUTOMATION_POLICY.as_bytes());
}
