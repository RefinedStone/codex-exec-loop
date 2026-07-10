use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(not(test))]
use std::sync::OnceLock;

/*
 * Host-owned Git commands must not inherit repository routing or executable configuration from
 * the shell that launched Akra. `git -C` does not override GIT_DIR/GIT_WORK_TREE/GIT_INDEX_FILE,
 * and GIT_CONFIG_COUNT, fsmonitor, hooks, trace targets, askpass, or GIT_SSH_COMMAND can execute
 * attacker-selected programs. Keep this boundary separate from generic subprocess handling so
 * gh, bash, curl, and app-server commands retain their intentional environments. This does not
 * make arbitrary repository-local network configuration trustworthy; pushes and other remote
 * writes must additionally use the frozen, isolated network context owned by the GitHub adapter.
 */
const SAFE_GIT_ENVIRONMENT: &[(&str, &str)] = &[
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_NO_REPLACE_OBJECTS", "1"),
    ("GIT_NO_LAZY_FETCH", "1"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_ATTR_NOSYSTEM", "1"),
    ("GIT_EDITOR", ":"),
    ("GIT_SEQUENCE_EDITOR", ":"),
];

const NON_GIT_EXECUTION_ENVIRONMENT: &[&str] = &["SSH_ASKPASS", "SSH_ASKPASS_REQUIRE"];
const SAFE_CREDENTIAL_MANAGER_ENVIRONMENT: &[(&str, &str)] =
    &[("GCM_INTERACTIVE", "Never"), ("GCM_GUI_PROMPT", "0")];

pub(crate) fn command<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    command_with_program(OsStr::new("git"), args)
}

pub(crate) fn command_with_program<I, S>(program: &OsStr, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let resolved_program = if is_git_program(program) {
        resolve_git_program(program)
    } else {
        Ok(PathBuf::from(program))
    };
    let mut command = match resolved_program {
        Ok(program) => Command::new(program),
        Err(error) => {
            tracing::error!(error = %error, "trusted host Git executable resolution failed");
            Command::new(fail_closed_git_program())
        }
    };
    harden_command(&mut command, std::env::vars_os());
    append_hardening_arguments(&mut command);
    command.args(args).stdin(Stdio::null());
    command
}

fn resolve_git_program(program: &OsStr) -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    if Path::new(program).is_absolute() {
        #[cfg(test)]
        return Ok(PathBuf::from(program));
        #[cfg(not(test))]
        return crate::trusted_executable::validate_absolute(Path::new(program), &cwd);
    }
    trusted_git_from_environment(&cwd)
}

#[cfg(test)]
fn trusted_git_from_environment(cwd: &Path) -> anyhow::Result<PathBuf> {
    crate::trusted_executable::resolve_native_from_current_path("git", cwd)
}

#[cfg(not(test))]
fn trusted_git_from_environment(cwd: &Path) -> anyhow::Result<PathBuf> {
    static PINNED_GIT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    PINNED_GIT
        .get_or_init(|| {
            crate::trusted_executable::resolve_native_from_current_path("git", cwd)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

#[cfg(unix)]
fn fail_closed_git_program() -> &'static Path {
    Path::new("/__akra_untrusted_git_executable__")
}

#[cfg(windows)]
fn fail_closed_git_program() -> &'static Path {
    Path::new(r"C:\__akra_untrusted_git_executable__.exe")
}

pub(crate) fn command_for_program<I, S>(program: &str, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if is_git_program(OsStr::new(program)) {
        command_with_program(OsStr::new(program), args)
    } else {
        #[cfg(not(test))]
        let program = {
            let path = Path::new(program);
            std::env::current_dir()
                .ok()
                .filter(|_| path.is_absolute())
                .and_then(|cwd| crate::trusted_executable::validate_absolute(path, &cwd).ok())
                .filter(|path| crate::trusted_executable::validate_native_executable(path).is_ok())
                .unwrap_or_else(|| fail_closed_generic_program().to_path_buf())
        };
        let mut command = Command::new(program);
        command.args(args);
        command
    }
}

#[cfg(all(not(test), unix))]
fn fail_closed_generic_program() -> &'static Path {
    Path::new("/__akra_untrusted_host_executable__")
}

#[cfg(all(not(test), windows))]
fn fail_closed_generic_program() -> &'static Path {
    Path::new(r"C:\__akra_untrusted_host_executable__.exe")
}

pub(crate) fn is_git_program(program: &OsStr) -> bool {
    let Some(file_name) = Path::new(program).file_name().and_then(OsStr::to_str) else {
        return false;
    };
    file_name.eq_ignore_ascii_case("git") || file_name.eq_ignore_ascii_case("git.exe")
}

fn harden_command(
    command: &mut Command,
    inherited_environment: impl IntoIterator<Item = (OsString, OsString)>,
) {
    let explicit_environment = command
        .get_envs()
        .map(|(name, value)| (name.to_os_string(), value.map(OsStr::to_os_string)))
        .collect::<Vec<_>>();
    command.env_clear();
    let mut inherited_path = None;
    for (name, value) in inherited_environment {
        if name.to_string_lossy().eq_ignore_ascii_case("PATH") {
            inherited_path = Some(value);
            continue;
        }
        if inherited_git_environment_allowed(&name, &value) {
            command.env(name, value);
        }
    }
    for (name, value) in explicit_environment {
        if is_unsafe_inherited_environment_name(&name)
            || value
                .as_ref()
                .is_some_and(|value| !inherited_git_environment_allowed(&name, value))
        {
            continue;
        }
        match value {
            Some(value) => {
                command.env(name, value);
            }
            None => {
                command.env_remove(name);
            }
        }
    }
    for (name, value) in SAFE_GIT_ENVIRONMENT {
        command.env(name, value);
    }
    for (name, value) in SAFE_CREDENTIAL_MANAGER_ENVIRONMENT {
        command.env(name, value);
    }
    command.env("GIT_CONFIG_GLOBAL", null_device());
    if let Some(path) = inherited_path
        && let Ok(cwd) = std::env::current_dir()
        && let Ok(path) = crate::trusted_executable::sanitized_path(&path, &cwd)
    {
        command.env("PATH", path);
    } else {
        command.env_remove("PATH");
    }
}

fn inherited_git_environment_allowed(name: &OsStr, value: &OsStr) -> bool {
    if is_unsafe_inherited_environment_name(name) {
        return false;
    }
    let normalized = name.to_string_lossy().to_ascii_uppercase();
    if normalized == "HOME"
        || normalized == "USERPROFILE"
        || normalized == "XDG_CONFIG_HOME"
        || normalized == "SYSTEMROOT"
        || normalized == "WINDIR"
        || normalized == "COMSPEC"
        || normalized == "PATHEXT"
        || normalized == "SSH_AUTH_SOCK"
        || normalized == "NO_PROXY"
        || normalized == "HTTPS_PROXY"
        || normalized == "SSL_CERT_FILE"
        || normalized == "SSL_CERT_DIR"
        || normalized == "CURL_CA_BUNDLE"
        || normalized == "GIT_ASKPASS_REQUIRE"
        || normalized == "LANG"
        || normalized.starts_with("LC_")
    {
        return safe_inherited_value(&normalized, value);
    }
    false
}

fn safe_inherited_value(name: &str, value: &OsStr) -> bool {
    let Some(value) = value.to_str() else {
        return false;
    };
    if value.is_empty() || value.chars().any(char::is_control) {
        return false;
    }
    match name {
        "HTTPS_PROXY" => {
            (value.starts_with("http://") || value.starts_with("https://"))
                && !value.contains('@')
                && !value.chars().any(char::is_whitespace)
        }
        "SSL_CERT_FILE" | "SSL_CERT_DIR" | "CURL_CA_BUNDLE" | "HOME" | "USERPROFILE"
        | "XDG_CONFIG_HOME" | "SSH_AUTH_SOCK" => Path::new(value).is_absolute(),
        _ => true,
    }
}

fn is_unsafe_inherited_environment_name(name: &OsStr) -> bool {
    let normalized_name = name.to_string_lossy().to_ascii_uppercase();
    normalized_name.starts_with("GIT_")
        || NON_GIT_EXECUTION_ENVIRONMENT.contains(&normalized_name.as_str())
}

fn append_hardening_arguments(command: &mut Command) {
    command
        .arg("--no-pager")
        .arg("--no-replace-objects")
        .arg("-c")
        .arg(format!("core.hooksPath={}", null_device()))
        .args(["-c", "core.fsmonitor=false"])
        .arg("-c")
        .arg(format!("core.attributesFile={}", null_device()))
        .args(["-c", "commit.gpgSign=false"])
        .args(["-c", "tag.gpgSign=false"])
        .args(["-c", "merge.gpgSign=false"])
        .args(["-c", "push.gpgSign=false"])
        .args(["-c", "maintenance.auto=false"])
        .args(["-c", "gc.auto=0"])
        .args(["-c", "gc.autoDetach=false"])
        .args(["-c", "fetch.writeCommitGraph=false"]);
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device() -> &'static str {
    "/dev/null"
}

#[cfg(test)]
mod tests {
    use super::{
        NON_GIT_EXECUTION_ENVIRONMENT, SAFE_GIT_ENVIRONMENT, append_hardening_arguments,
        command_for_program, harden_command, is_git_program,
    };
    use crate::subprocess;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn program_detection_is_limited_to_git_binaries() {
        assert!(is_git_program(OsStr::new("git")));
        assert!(is_git_program(OsStr::new("/usr/bin/git")));
        assert!(is_git_program(OsStr::new("git.exe")));
        assert!(!is_git_program(OsStr::new("gh")));
        assert!(!is_git_program(OsStr::new("git-wrapper")));
    }

    #[test]
    fn generic_program_boundary_preserves_non_git_argv_and_git_caller_suffix() {
        let caller_args = [
            "value with spaces",
            "line one\nline two",
            "--literal=$VALUE",
        ];
        let non_git = command_for_program("gh", caller_args);
        assert_eq!(
            non_git.get_args().collect::<Vec<_>>(),
            caller_args.map(OsStr::new).to_vec()
        );

        let git = command_for_program("git", caller_args);
        let git_args = git.get_args().collect::<Vec<_>>();
        assert_eq!(
            &git_args[git_args.len() - caller_args.len()..],
            caller_args.map(OsStr::new).as_slice()
        );
    }

    #[test]
    fn hardening_removes_routing_config_trace_and_askpass_environment() {
        let dangerous = [
            "GIT_DIR",
            "Git_Work_Tree",
            "GIT_INDEX_FILE",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_KEY_0",
            "GIT_CONFIG_VALUE_0",
            "GIT_TRACE2_EVENT",
            "GIT_SSH_COMMAND",
        ];
        let mut command = Command::new("git");
        let inherited = dangerous
            .into_iter()
            .chain(["SSH_ASKPASS"])
            .map(|name| (OsString::from(name), OsString::from("attacker-controlled")))
            .chain([
                (
                    OsString::from("SSH_AUTH_SOCK"),
                    OsString::from("/tmp/agent.sock"),
                ),
                (OsString::from("HOME"), OsString::from("/home/operator")),
                (
                    OsString::from("AKRA_FROZEN_TARGET_PROOF"),
                    OsString::from("proof-value"),
                ),
            ]);
        harden_command(&mut command, inherited);

        let configured = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().to_ascii_uppercase(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        for name in dangerous {
            assert!(
                !configured.iter().any(|(configured_name, value)| {
                    configured_name == &name.to_ascii_uppercase()
                        && !SAFE_GIT_ENVIRONMENT.iter().any(|(safe_name, safe_value)| {
                            safe_name == configured_name && value.as_deref() == Some(*safe_value)
                        })
                }),
                "{name} should be removed or replaced by a fixed safe value: {configured:?}"
            );
        }
        for name in NON_GIT_EXECUTION_ENVIRONMENT {
            assert!(
                !configured
                    .iter()
                    .any(|(configured_name, _)| configured_name == name),
                "{name} should be removed: {configured:?}"
            );
        }
        for (name, value) in [
            ("SSH_AUTH_SOCK", "/tmp/agent.sock"),
            ("HOME", "/home/operator"),
        ] {
            assert!(
                configured
                    .iter()
                    .any(|(configured_name, configured_value)| {
                        configured_name == name && configured_value.as_deref() == Some(value)
                    }),
                "{name} should remain available to the authenticated transport: {configured:?}"
            );
        }
        assert!(
            !configured
                .iter()
                .any(|(name, value)| { name == "AKRA_FROZEN_TARGET_PROOF" && value.is_some() })
        );
    }

    #[test]
    fn preconfigured_git_environment_is_removed_when_parent_snapshot_omits_the_keys() {
        let mut command = Command::new("git");
        command
            .env("GIT_DIR", "explicit-poison-repository")
            .env("Git_Config_Count", "1")
            .env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .env("GIT_CONFIG_VALUE_0", "explicit-poison-hooks")
            .env("SSH_ASKPASS", "explicit-poison-askpass")
            .env("AKRA_FROZEN_TARGET_PROOF", "explicit-safe-proof")
            .env_remove("HOME");

        harden_command(
            &mut command,
            [
                (OsString::from("HOME"), OsString::from("/home/parent")),
                (
                    OsString::from("SSH_AUTH_SOCK"),
                    OsString::from("/tmp/parent-agent.sock"),
                ),
            ],
        );

        let configured = command
            .get_envs()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().to_ascii_uppercase(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<Vec<_>>();
        for dangerous_name in [
            "GIT_DIR",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_KEY_0",
            "GIT_CONFIG_VALUE_0",
            "SSH_ASKPASS",
        ] {
            assert!(
                !configured.iter().any(|(name, _)| name == dangerous_name),
                "explicit {dangerous_name} must be removed: {configured:?}"
            );
        }
        assert!(
            !configured
                .iter()
                .any(|(name, value)| { name == "AKRA_FROZEN_TARGET_PROOF" && value.is_some() })
        );
        assert!(configured.iter().any(|(name, value)| {
            name == "SSH_AUTH_SOCK" && value.as_deref() == Some("/tmp/parent-agent.sock")
        }));
        assert!(
            !configured
                .iter()
                .any(|(name, value)| name == "HOME" && value.is_some()),
            "explicit HOME removal must override the inherited parent value: {configured:?}"
        );
    }

    #[test]
    fn malicious_git_environment_cannot_route_a_host_mutation_or_write_trace() {
        let root = unique_temp_dir("git-boundary-routing");
        let leased = root.join("leased");
        let poison = root.join("poison");
        fs::create_dir_all(&leased).expect("leased repository directory should be created");
        fs::create_dir_all(&poison).expect("poison repository directory should be created");
        initialize_repository(&leased, "leased.txt");
        initialize_repository(&poison, "poison.txt");

        let leased_head_before = git_stdout(&leased, &["rev-parse", "HEAD"]);
        let poison_head_before = git_stdout(&poison, &["rev-parse", "HEAD"]);
        fs::write(leased.join("leased.txt"), "leased-only change\n")
            .expect("tracked leased file should be updated");
        let trace_marker = root.join("git-trace-marker");
        let injected_hook_marker = root.join("injected-hook-marker");
        let hooks = root.join("hooks");
        fs::create_dir_all(&hooks).expect("hooks directory should be created");
        let marker_hook = hooks.join("pre-commit");
        write_marker_hook(&marker_hook, &injected_hook_marker);
        run_fixture_git(
            &leased,
            &[
                "config",
                "core.hooksPath",
                hooks.to_str().expect("hooks path should be UTF-8"),
            ],
        );
        run_fixture_git(
            &leased,
            &[
                "config",
                "core.fsmonitor",
                marker_hook
                    .to_str()
                    .expect("fsmonitor path should be UTF-8"),
            ],
        );

        let mut command = Command::new("git");
        let poisoned_environment = vec![
            ("GIT_DIR", poison.join(".git").into_os_string()),
            ("GIT_WORK_TREE", poison.clone().into_os_string()),
            (
                "GIT_INDEX_FILE",
                poison.join(".git/akra-poison-index").into_os_string(),
            ),
            ("GIT_CONFIG_COUNT", OsString::from("2")),
            ("GIT_CONFIG_KEY_0", OsString::from("core.hooksPath")),
            ("GIT_CONFIG_VALUE_0", hooks.as_os_str().to_os_string()),
            ("GIT_CONFIG_KEY_1", OsString::from("core.fsmonitor")),
            (
                "GIT_CONFIG_VALUE_1",
                hooks.join("pre-commit").into_os_string(),
            ),
            ("GIT_TRACE", trace_marker.as_os_str().to_os_string()),
        ];
        let poisoned_names = poisoned_environment
            .iter()
            .map(|(name, _)| name.to_ascii_uppercase())
            .collect::<Vec<_>>();
        let inherited_environment = std::env::vars_os()
            .filter(|(name, _)| {
                !poisoned_names.contains(&name.to_string_lossy().to_ascii_uppercase())
            })
            .chain(
                poisoned_environment
                    .iter()
                    .map(|(name, value)| (OsString::from(name), value.clone())),
            );
        harden_command(&mut command, inherited_environment);
        append_hardening_arguments(&mut command);
        command
            .arg("-C")
            .arg(&leased)
            .args(["commit", "-am", "hardened host mutation"])
            .stdin(Stdio::null());
        let output = subprocess::command_output(&mut command, "hardened git commit")
            .expect("hardened git command should finish");
        assert!(
            output.status.success(),
            "hardened commit should target leased repository: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        assert_ne!(
            git_stdout(&leased, &["rev-parse", "HEAD"]),
            leased_head_before
        );
        assert_eq!(
            git_stdout(&poison, &["rev-parse", "HEAD"]),
            poison_head_before
        );
        assert!(
            !trace_marker.exists(),
            "inherited GIT_TRACE must be removed"
        );
        assert!(
            !injected_hook_marker.exists(),
            "injected hook/fsmonitor command must not execute"
        );
        let _ = fs::remove_dir_all(root);
    }

    fn initialize_repository(root: &Path, file_name: &str) {
        run_fixture_git(root, &["init", "-q"]);
        run_fixture_git(root, &["config", "user.name", "Akra Test"]);
        run_fixture_git(root, &["config", "user.email", "akra@example.invalid"]);
        fs::write(root.join(file_name), "seed\n").expect("seed file should be written");
        run_fixture_git(root, &["add", file_name]);
        run_fixture_git(root, &["commit", "-qm", "seed"]);
    }

    fn run_fixture_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("fixture git command should run");
        assert!(
            output.status.success(),
            "fixture git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .expect("fixture git command should run");
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[cfg(unix)]
    fn write_marker_hook(path: &Path, marker: &Path) {
        use std::os::unix::fs::PermissionsExt;

        fs::write(
            path,
            format!("#!/bin/sh\nprintf executed > '{}'\n", marker.display()),
        )
        .expect("marker hook should be written");
        let mut permissions = fs::metadata(path)
            .expect("marker hook metadata should be available")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).expect("marker hook should be executable");
    }

    #[cfg(windows)]
    fn write_marker_hook(path: &Path, marker: &Path) {
        fs::write(
            path,
            format!("#!/bin/sh\nprintf executed > '{}'\n", marker.display()),
        )
        .expect("marker hook should be written");
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "akra-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should follow Unix epoch")
                .as_nanos()
        ))
    }
}
