use super::*;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::configuration::{ConfigScope, read_config_file_if_present};

const POLICY: &str = r#"
[binding.github]
mode = "propose"
[authority]
autonomy_cap = "propose"
[select]
mode = "all_open"
trusted_actors = [{ provider = "github", server_id = "github.com", actor_id = "U_flow_author" }]
[feedback]
comment = true
labels = false
transition = false
[close]
when = "never"
"#;

/// Only path selection differs from production. Real files pass through the same
/// safe reader, TOML parsing, policy composition and validation as application files.
pub(super) struct FlowPolicySource {
    user_dir: PathBuf,
}
impl FlowPolicySource {
    pub fn new(root: &Path) -> Self {
        let user_dir = root.join("isolated-user-policy");
        fs::create_dir_all(&user_dir).expect("isolated policy directory should exist");
        protect_policy_path(&user_dir, true)
            .expect("global policy directory should satisfy the real private-owner contract");
        let path = user_dir.join("issue-automation.toml");
        fs::write(&path, POLICY).expect("real automation policy file should be written");
        protect_policy_path(&path, false)
            .expect("global policy file should satisfy the real private-owner contract");
        Self { user_dir }
    }

    pub fn require_selection_label(&self, label: &str) -> Result<()> {
        ensure!(
            !label.trim().is_empty(),
            "selection label must not be empty"
        );
        let path = self.user_dir.join("issue-automation.toml");
        let current = fs::read_to_string(&path)?;
        let previous = "mode = \"all_open\"";
        ensure!(
            current.matches(previous).count() == 1,
            "selection label must be configured once before collection"
        );
        let labels = toml::Value::Array(vec![toml::Value::String(label.into())]);
        let next = current.replace(
            previous,
            &format!("mode = \"labels_any\"\nlabels_any = {labels}"),
        );
        fs::write(path, next)?;
        Ok(())
    }
}

#[cfg(unix)]
fn protect_policy_path(path: &Path, directory: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
    )?;
    Ok(())
}

#[cfg(windows)]
fn protect_policy_path(path: &Path, directory: bool) -> Result<()> {
    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC,
        set_windows_private_acl, validate_windows_private_owner_and_acl,
    };
    use std::os::windows::fs::OpenOptionsExt;

    let flags = WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
        | if directory {
            WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
        } else {
            0
        };
    let handle = fs::OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(flags)
        .open(path)?;
    set_windows_private_acl(&handle, directory)?;
    validate_windows_private_owner_and_acl(path, &handle)?;
    Ok(())
}

impl IssueAutomationPolicySourcePort for FlowPolicySource {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        read_config_file_if_present(
            &self.user_dir.join("issue-automation.toml"),
            ConfigScope::Global,
        )
    }
    fn load_project_policy_toml(&self, workspace: &str) -> Result<Option<String>> {
        read_config_file_if_present(
            &Path::new(workspace)
                .join(".akra")
                .join("issue-automation.toml"),
            ConfigScope::Project,
        )
    }
}
impl IssuePolicySourcePort for FlowPolicySource {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        read_config_file_if_present(
            &self.user_dir.join("issue-policy.toml"),
            ConfigScope::Global,
        )
    }
    fn load_project_policy_toml(&self, workspace: &str) -> Result<Option<String>> {
        read_config_file_if_present(
            &Path::new(workspace).join(".akra").join("issue-policy.toml"),
            ConfigScope::Project,
        )
    }
}
