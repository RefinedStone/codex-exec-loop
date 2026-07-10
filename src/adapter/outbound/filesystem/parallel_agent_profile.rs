#[cfg(unix)]
use std::path::Path;

#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;

#[cfg(unix)]
use super::secure_fs;
use crate::application::port::outbound::parallel_agent_profile_repository_port::ParallelAgentProfileRepositoryPort;

#[cfg(unix)]
const PROFILE_CONFIG_RELATIVE_PATH: &str = ".akra/parallel-agent-profiles.json";
#[cfg(unix)]
pub(crate) const MAX_PROFILE_CONFIG_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default)]
pub struct FilesystemParallelAgentProfileRepositoryAdapter;

impl FilesystemParallelAgentProfileRepositoryAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ParallelAgentProfileRepositoryPort for FilesystemParallelAgentProfileRepositoryAdapter {
    fn load_profile_config_json(&self, workspace_dir: &str) -> Result<Option<String>> {
        #[cfg(windows)]
        {
            let _ = workspace_dir;
            // Do not inspect an unanchored workspace path on Windows. Returning
            // no override keeps the built-in, validated profiles available
            // without trusting a file we cannot open relative to a pinned NT
            // directory handle.
            Ok(None)
        }
        #[cfg(unix)]
        {
            secure_fs::read_optional_file_with_limit(
                Path::new(workspace_dir),
                Path::new(PROFILE_CONFIG_RELATIVE_PATH),
                MAX_PROFILE_CONFIG_BYTES,
            )
            .with_context(|| {
                format!(
                    "failed to securely read parallel agent profile config beneath `{workspace_dir}`"
                )
            })
        }
    }

    fn save_profile_config_json(&self, workspace_dir: &str, body: &str) -> Result<()> {
        #[cfg(windows)]
        {
            let _ = (workspace_dir, body);
            anyhow::bail!(
                "custom parallel agent profile file writes are unsupported on Windows; built-in profiles remain active"
            );
        }
        #[cfg(unix)]
        {
            secure_fs::write_file_atomic_with_limit(
                Path::new(workspace_dir),
                Path::new(PROFILE_CONFIG_RELATIVE_PATH),
                body.as_bytes(),
                MAX_PROFILE_CONFIG_BYTES,
            )
            .with_context(|| {
                format!(
                    "failed to securely write parallel agent profile config beneath `{workspace_dir}`"
                )
            })
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::FilesystemParallelAgentProfileRepositoryAdapter;
    use crate::application::port::outbound::parallel_agent_profile_repository_port::ParallelAgentProfileRepositoryPort;

    #[test]
    fn windows_uses_built_in_profiles_without_reading_workspace_files() {
        let adapter = FilesystemParallelAgentProfileRepositoryAdapter::new();

        assert_eq!(
            adapter
                .load_profile_config_json(r"C:\unsafe-workspace")
                .expect("built-in profile fallback should remain available"),
            None
        );
        let error = adapter
            .save_profile_config_json(r"C:\unsafe-workspace", "{}")
            .expect_err("custom profile writes must stay disabled");
        assert!(
            error
                .to_string()
                .contains("built-in profiles remain active")
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        FilesystemParallelAgentProfileRepositoryAdapter, MAX_PROFILE_CONFIG_BYTES,
        PROFILE_CONFIG_RELATIVE_PATH,
    };
    use crate::adapter::outbound::filesystem::secure_fs;
    use crate::application::port::outbound::parallel_agent_profile_repository_port::ParallelAgentProfileRepositoryPort;

    fn temp_directory(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "akra-profile-repository-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp directory");
        path
    }

    fn write_private(path: &Path, body: &[u8]) {
        fs::write(path, body).expect("fixture file");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("private fixture mode");
    }

    #[test]
    fn repository_round_trips_json_with_private_atomic_destination() {
        let workspace = temp_directory("round-trip");
        let adapter = FilesystemParallelAgentProfileRepositoryAdapter::new();
        let body = "{\"profiles\":[]}\n";

        adapter
            .save_profile_config_json(workspace.to_str().unwrap(), body)
            .expect("profile should save");
        let loaded = adapter
            .load_profile_config_json(workspace.to_str().unwrap())
            .expect("profile should load");

        assert_eq!(loaded.as_deref(), Some(body));
        let metadata = fs::metadata(workspace.join(PROFILE_CONFIG_RELATIVE_PATH)).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn ancestor_symlink_is_rejected_for_read_and_write() {
        let workspace = temp_directory("ancestor-workspace");
        let outside = temp_directory("ancestor-outside");
        let outside_config = outside.join("parallel-agent-profiles.json");
        write_private(&outside_config, b"outside");
        symlink(&outside, workspace.join(".akra")).expect("ancestor symlink");
        let adapter = FilesystemParallelAgentProfileRepositoryAdapter::new();

        assert!(
            adapter
                .load_profile_config_json(workspace.to_str().unwrap())
                .is_err()
        );
        assert!(
            adapter
                .save_profile_config_json(workspace.to_str().unwrap(), "replacement")
                .is_err()
        );
        assert_eq!(fs::read(&outside_config).unwrap(), b"outside");

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn leaf_symlink_is_rejected_for_read_and_write() {
        let workspace = temp_directory("leaf-workspace");
        let outside = temp_directory("leaf-outside");
        fs::create_dir(workspace.join(".akra")).expect("profile directory");
        fs::set_permissions(workspace.join(".akra"), fs::Permissions::from_mode(0o700)).unwrap();
        let outside_config = outside.join("outside.json");
        write_private(&outside_config, b"outside");
        symlink(
            &outside_config,
            workspace.join(PROFILE_CONFIG_RELATIVE_PATH),
        )
        .expect("leaf symlink");
        let adapter = FilesystemParallelAgentProfileRepositoryAdapter::new();

        assert!(
            adapter
                .load_profile_config_json(workspace.to_str().unwrap())
                .is_err()
        );
        assert!(
            adapter
                .save_profile_config_json(workspace.to_str().unwrap(), "replacement")
                .is_err()
        );
        assert_eq!(fs::read(&outside_config).unwrap(), b"outside");

        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn oversized_profile_is_rejected_on_read_and_write() {
        let workspace = temp_directory("oversized");
        let adapter = FilesystemParallelAgentProfileRepositoryAdapter::new();
        let oversized = "x".repeat(MAX_PROFILE_CONFIG_BYTES + 1);

        let write_error = adapter
            .save_profile_config_json(workspace.to_str().unwrap(), &oversized)
            .expect_err("oversized profile write must fail");
        assert!(format!("{write_error:#}").contains("exceeds"));
        assert!(!workspace.join(PROFILE_CONFIG_RELATIVE_PATH).exists());

        fs::create_dir(workspace.join(".akra")).expect("profile directory");
        fs::set_permissions(workspace.join(".akra"), fs::Permissions::from_mode(0o700)).unwrap();
        write_private(
            &workspace.join(PROFILE_CONFIG_RELATIVE_PATH),
            oversized.as_bytes(),
        );
        let read_error = adapter
            .load_profile_config_json(workspace.to_str().unwrap())
            .expect_err("oversized profile read must fail");
        assert!(format!("{read_error:#}").contains("exceeds"));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn ancestor_replacement_race_fails_without_writing_through_symlink() {
        let workspace = temp_directory("race-workspace");
        let outside = temp_directory("race-outside");
        let adapter = FilesystemParallelAgentProfileRepositoryAdapter::new();
        adapter
            .save_profile_config_json(workspace.to_str().unwrap(), "old")
            .expect("initial profile");
        let outside_config = outside.join("parallel-agent-profiles.json");
        write_private(&outside_config, b"outside");
        let hook_workspace = workspace.clone();
        let hook_outside = outside.clone();
        secure_fs::install_before_atomic_replace_hook(move || {
            fs::rename(
                hook_workspace.join(".akra"),
                hook_workspace.join(".akra-held"),
            )
            .expect("replace profile ancestor");
            symlink(&hook_outside, hook_workspace.join(".akra"))
                .expect("install raced ancestor symlink");
        });

        let error = adapter
            .save_profile_config_json(workspace.to_str().unwrap(), "new")
            .expect_err("ancestor replacement must be detected");

        assert!(
            format!("{error:#}").contains("ancestor identity changed"),
            "unexpected race error: {error:#}"
        );
        assert_eq!(fs::read(&outside_config).unwrap(), b"outside");
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(outside);
    }
}
