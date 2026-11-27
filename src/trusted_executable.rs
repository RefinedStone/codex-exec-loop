#[cfg(all(test, unix))]
pub(crate) use akra_native::trusted_executable::resolve_from_path;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

#[cfg(windows)]
use akra_native::trusted_executable::read_launcher_bytes;
#[cfg(unix)]
use akra_native::trusted_executable::read_launcher_first_line;
#[cfg(windows)]
pub(crate) use akra_native::trusted_executable::windows_process_argument_path;
use akra_native::trusted_executable::{
    ExecutableTrustPolicy as CodexExecutableTrustPolicy, executable_names,
    path_is_repository_controlled, resolve_native_from_path_with_policy,
    validate_absolute_with_policy, validate_native_executable_with_policy,
    validate_trusted_configuration_path as validate_configuration_path,
};
pub(crate) use akra_native::trusted_executable::{
    resolve_native_from_current_path, resolve_native_from_path, sanitized_path, validate_absolute,
    validate_native_executable,
};

#[cfg(unix)]
const MAX_CODEX_SHEBANG_BYTES: usize = 4 * 1024;
#[cfg(any(windows, test))]
const MAX_WINDOWS_CODEX_SHIM_BYTES: usize = 16 * 1024;
const REQUIRE_TRUSTED_CODEX_EXECUTABLE_ENV_VAR: &str = "AKRA_REQUIRE_TRUSTED_CODEX_EXECUTABLE";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustedCommand {
    pub(crate) program: PathBuf,
    pub(crate) prefix_args: Vec<OsString>,
    pub(crate) source_executable: PathBuf,
    pub(crate) trust_warning: Option<String>,
}

fn resolve_codex_executable_trust_policy(
    value: Option<&str>,
) -> Result<CodexExecutableTrustPolicy> {
    match value {
        None | Some("0") => Ok(CodexExecutableTrustPolicy::AllowUntrusted),
        Some("1") => Ok(CodexExecutableTrustPolicy::RequireTrusted),
        Some(_) => bail!("{REQUIRE_TRUSTED_CODEX_EXECUTABLE_ENV_VAR} must be exactly `1` or `0`"),
    }
}

fn configured_codex_executable_trust_policy() -> Result<CodexExecutableTrustPolicy> {
    match std::env::var(REQUIRE_TRUSTED_CODEX_EXECUTABLE_ENV_VAR) {
        Ok(value) => resolve_codex_executable_trust_policy(Some(&value)),
        Err(std::env::VarError::NotPresent) => resolve_codex_executable_trust_policy(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            bail!("{REQUIRE_TRUSTED_CODEX_EXECUTABLE_ENV_VAR} must be exactly `1` or `0`")
        }
    }
}

pub(crate) fn pinned_codex_command() -> Result<TrustedCommand> {
    let cwd = std::env::current_dir().context("failed to resolve cwd for Codex executable pin")?;
    #[cfg(test)]
    {
        resolve_codex_command_from_path(
            &std::env::var_os("PATH").context("PATH is unavailable")?,
            &cwd,
        )
    }
    #[cfg(not(test))]
    {
        use std::sync::OnceLock;

        static PINNED_CODEX: OnceLock<Result<TrustedCommand, String>> = OnceLock::new();
        PINNED_CODEX
            .get_or_init(|| {
                let path =
                    std::env::var_os("PATH").ok_or_else(|| "PATH is unavailable".to_string())?;
                resolve_codex_command_from_path(&path, &cwd).map_err(|error| format!("{error:#}"))
            })
            .clone()
            .map_err(anyhow::Error::msg)
    }
}

/// Optional runtimes installed by the operator can use the same strict setting as Codex.
pub(crate) fn resolve_blog_program(program: &str, cwd: &Path) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH is unavailable")?;
    let executable = resolve_native_from_path_with_policy(
        program,
        &path,
        cwd,
        configured_codex_executable_trust_policy()?,
    )?;
    if path_is_repository_controlled(&executable, cwd) {
        bail!("blog runtime must be installed outside the repository")
    }
    // Git for Windows cannot read its config when its mingw64 executable uses
    // an extended path. Change only the spelling after canonical trust checks.
    #[cfg(windows)]
    let executable = PathBuf::from(windows_process_argument_path(&executable)?);
    Ok(executable)
}

pub(crate) fn resolve_codex_command_from_path(path: &OsStr, cwd: &Path) -> Result<TrustedCommand> {
    resolve_codex_command_from_path_with_policy(
        path,
        cwd,
        configured_codex_executable_trust_policy()?,
    )
}

fn resolve_codex_command_from_path_with_policy(
    path: &OsStr,
    cwd: &Path,
    policy: CodexExecutableTrustPolicy,
) -> Result<TrustedCommand> {
    let trust_error = match resolve_codex_command_from_path_for_policy(
        path,
        cwd,
        CodexExecutableTrustPolicy::RequireTrusted,
    ) {
        Ok(command) => return Ok(command),
        Err(error) => error,
    };
    if policy == CodexExecutableTrustPolicy::RequireTrusted {
        return Err(trust_error);
    }

    let mut command =
        resolve_codex_command_from_path_for_policy(path, cwd, policy).with_context(|| {
            format!(
                "trusted Codex executable resolution failed ({trust_error:#}); permissive resolution also failed"
            )
    })?;
    command.trust_warning = Some(format!(
        "Codex executable trust checks failed and a mutation-capable principal may replace the pinned path before spawn; startup continued because {REQUIRE_TRUSTED_CODEX_EXECUTABLE_ENV_VAR}=1 is not set: {trust_error:#}"
    ));
    Ok(command)
}

fn resolve_codex_command_from_path_for_policy(
    path: &OsStr,
    cwd: &Path,
    policy: CodexExecutableTrustPolicy,
) -> Result<TrustedCommand> {
    let entries = std::env::split_paths(path).collect::<Vec<_>>();
    if entries.is_empty() {
        bail!("PATH contains no executable directories")
    }
    let mut unsupported_launcher = None;
    for directory in entries {
        if !directory.is_absolute() {
            bail!(
                "PATH contains a relative executable directory `{}`",
                directory.display()
            )
        }
        for name in executable_names("codex") {
            let candidate = directory.join(name);
            match fs::symlink_metadata(&candidate) {
                Ok(_) => {
                    let source_executable = validate_absolute_with_policy(&candidate, cwd, policy)
                        .with_context(|| {
                            format!(
                                "PATH selected an unsafe `codex` executable at `{}`",
                                candidate.display()
                            )
                        })?;
                    match resolve_codex_command(source_executable.clone(), path, cwd, policy)? {
                        Some(command) => return Ok(command),
                        None => unsupported_launcher = Some(source_executable),
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to inspect `{}` from PATH", candidate.display())
                    });
                }
            }
        }
    }
    if let Some(launcher) = unsupported_launcher {
        bail!(
            "no supported Codex launcher was found after `{}` used an unsupported shebang",
            launcher.display()
        )
    }
    let qualifier = if policy == CodexExecutableTrustPolicy::RequireTrusted {
        "trusted "
    } else {
        ""
    };
    bail!("`codex` was not found on the {qualifier}absolute PATH")
}

fn resolve_codex_command(
    source_executable: PathBuf,
    path: &OsStr,
    cwd: &Path,
    policy: CodexExecutableTrustPolicy,
) -> Result<Option<TrustedCommand>> {
    if validate_native_executable_with_policy(&source_executable, policy).is_ok() {
        return Ok(Some(TrustedCommand {
            program: source_executable.clone(),
            prefix_args: Vec::new(),
            source_executable,
            trust_warning: None,
        }));
    }
    #[cfg(windows)]
    {
        if !source_executable
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
        {
            bail!("Windows Codex launcher must be a native executable or an npm .cmd shim")
        }
        let shim = read_codex_launcher_bytes(
            &source_executable,
            MAX_WINDOWS_CODEX_SHIM_BYTES,
            "Windows Codex npm shim",
            policy,
        )?;
        let shim = std::str::from_utf8(&shim).context("Windows Codex npm shim is not UTF-8")?;
        let relative_target = parse_windows_npm_codex_cmd_shim(shim)?;
        let target = source_executable
            .parent()
            .context("Windows Codex npm shim has no parent directory")?
            .join(relative_target);
        let target = validate_absolute_with_policy(&target, cwd, policy)
            .context("Windows Codex npm target failed executable trust validation")?;
        if !has_codex_package_script_suffix(&target) {
            bail!("Windows Codex npm target is outside @openai/codex/bin/codex.js")
        }
        let node = resolve_native_from_path_with_policy("node", path, cwd, policy)?;
        Ok(Some(TrustedCommand {
            program: node,
            prefix_args: vec![windows_process_argument_path(&target)?],
            source_executable,
            trust_warning: None,
        }))
    }
    #[cfg(unix)]
    {
        let first_line = read_codex_launcher_first_line(&source_executable, policy)?;
        if first_line.len() > 128
            || first_line
                .iter()
                .any(|byte| byte.is_ascii_control() && *byte != b'\r')
        {
            bail!("Codex launcher shebang is malformed")
        }
        let shebang = std::str::from_utf8(&first_line)
            .context("Codex launcher shebang is not UTF-8")?
            .trim_end_matches('\r');
        let node = match shebang {
            "#!/usr/bin/env node" => {
                resolve_native_from_path_with_policy("node", path, cwd, policy)?
            }
            value if value.starts_with("#!") && !value[2..].contains(char::is_whitespace) => {
                let interpreter = Path::new(&value[2..]);
                if !interpreter.is_absolute() {
                    bail!("Codex launcher interpreter must be absolute")
                }
                let interpreter = validate_absolute_with_policy(interpreter, cwd, policy)?;
                validate_native_executable_with_policy(&interpreter, policy)?;
                let name = interpreter
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default();
                if !matches!(name, "node" | "nodejs") {
                    return Ok(None);
                }
                interpreter
            }
            _ => return Ok(None),
        };
        Ok(Some(TrustedCommand {
            program: node,
            prefix_args: vec![source_executable.as_os_str().to_os_string()],
            source_executable,
            trust_warning: None,
        }))
    }
}

#[cfg(windows)]
fn read_codex_launcher_bytes(
    path: &Path,
    maximum: usize,
    label: &str,
    policy: CodexExecutableTrustPolicy,
) -> Result<Vec<u8>> {
    read_launcher_bytes(path, maximum, label, policy)
}

#[cfg(unix)]
fn read_codex_launcher_first_line(
    path: &Path,
    policy: CodexExecutableTrustPolicy,
) -> Result<Vec<u8>> {
    read_launcher_first_line(path, MAX_CODEX_SHEBANG_BYTES, policy)
}

#[cfg(any(windows, test))]
fn parse_windows_npm_codex_cmd_shim(contents: &str) -> Result<PathBuf> {
    if contents.len() > MAX_WINDOWS_CODEX_SHIM_BYTES
        || !contents.is_ascii()
        || contents.as_bytes().contains(&0)
    {
        bail!("Windows Codex npm shim is not bounded ASCII text")
    }
    let lines = contents
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .collect::<Vec<_>>();
    let normalized = lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let expected_prefix = [
        "@ECHO off",
        "GOTO start",
        ":find_dp0",
        "SET dp0=%~dp0",
        "EXIT /b",
        ":start",
        "SETLOCAL",
        "CALL :find_dp0",
        "IF EXIST \"%dp0%\\node.exe\" (",
        "SET \"_prog=%dp0%\\node.exe\"",
        ") ELSE (",
        "SET \"_prog=node\"",
        "SET PATHEXT=%PATHEXT:;.JS;=;%",
        ")",
    ];
    if normalized.len() != expected_prefix.len() + 1
        || !normalized
            .iter()
            .zip(expected_prefix)
            .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    {
        bail!("Windows Codex launcher is not a standard npm cmd shim")
    }

    const TARGET_MARKER: &str = "\"%dp0%\\";
    let mut targets = Vec::new();
    for line in normalized.into_iter().skip(expected_prefix.len()) {
        let lower = line.to_ascii_lowercase();
        let Some(start) = lower.find(TARGET_MARKER) else {
            continue;
        };
        let target_start = start + TARGET_MARKER.len();
        let Some(relative_end) = line[target_start..].find('"') else {
            bail!("Windows Codex npm target quote is unterminated")
        };
        let target_end = target_start + relative_end;
        let prefix = line[..start].trim_end();
        let suffix = line[target_end + 1..].trim();
        if !prefix.to_ascii_lowercase().ends_with("\"%_prog%\"") || suffix != "%*" {
            continue;
        }
        let target = &line[target_start..target_end];
        if target.is_empty()
            || target.starts_with(['\\', '/'])
            || target.chars().any(|character| {
                !character.is_ascii_alphanumeric()
                    && !matches!(character, '\\' | '/' | '.' | '_' | '-' | '@' | '+')
            })
            || !relative_codex_script_suffix_matches(target)
        {
            bail!("Windows Codex npm target is not the canonical package script")
        }
        targets.push(PathBuf::from(target));
    }
    if targets.len() != 1 {
        bail!("Windows Codex npm shim must contain exactly one package invocation")
    }
    Ok(targets.remove(0))
}

#[cfg(any(windows, test))]
fn relative_codex_script_suffix_matches(target: &str) -> bool {
    let components = target.split(['\\', '/']).collect::<Vec<_>>();
    let expected = ["node_modules", "@openai", "codex", "bin", "codex.js"];
    components.len() == expected.len()
        && components
            .iter()
            .zip(expected)
            .all(|(actual, expected)| !actual.is_empty() && actual.eq_ignore_ascii_case(expected))
}

#[cfg(windows)]
fn has_codex_package_script_suffix(target: &Path) -> bool {
    let components = target
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(component) => component.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = ["node_modules", "@openai", "codex", "bin", "codex.js"];
    components.len() >= expected.len()
        && components[components.len() - expected.len()..]
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
pub(crate) fn copy_native_executable_fixture(path: &Path) -> Result<()> {
    #[cfg(unix)]
    let source = [Path::new("/usr/bin/true"), Path::new("/bin/true")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_exe().context("failed to resolve test executable")?);
    #[cfg(windows)]
    let source = std::env::current_exe().context("failed to resolve test executable")?;
    fs::copy(&source, path).with_context(|| {
        format!(
            "failed to copy native executable fixture from {} to {}",
            source.display(),
            path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .context("failed to secure native executable fixture mode")?;
    }
    Ok(())
}

pub(crate) fn configure_credential_command_environment(
    command: &mut Command,
    cwd: &Path,
    include_user_config: bool,
) -> Result<()> {
    command.env_clear();
    let inherited_path = std::env::var_os("PATH").context("PATH is unavailable")?;
    command.env("PATH", sanitized_path(&inherited_path, cwd)?);
    // Windows 의 Winsock 은 `SystemRoot` 가 없으면 이름을 풀지 못한다 (curl 종료 코드 6). git 과 clipboard 도 넘긴다.
    #[cfg(windows)]
    if let Some(value) = std::env::var_os("SystemRoot").filter(|value| !value.is_empty()) {
        command.env("SystemRoot", value);
    }

    for name in ["LANG", "LC_ALL", "LC_CTYPE"] {
        if let Some(value) = std::env::var_os(name).filter(safe_plain_value) {
            command.env(name, value);
        }
    }
    configure_user_config_environment(
        command,
        cwd,
        include_user_config,
        |name| std::env::var_os(name),
        |value, cwd| validate_configuration_path(value, cwd, true),
    );
    for (upper, lower) in [("HTTPS_PROXY", "https_proxy"), ("NO_PROXY", "no_proxy")] {
        if let Some(value) = resolve_alias(upper, lower)? {
            if upper == "HTTPS_PROXY" {
                validate_proxy(&value)?;
            } else {
                validate_no_proxy(&value)?;
            }
            command.env(upper, value);
        }
    }
    for (name, directory) in [
        ("SSL_CERT_FILE", false),
        ("CURL_CA_BUNDLE", false),
        ("SSL_CERT_DIR", true),
    ] {
        if let Some(value) = std::env::var_os(name).filter(|value| !value.is_empty()) {
            command.env(name, validate_configuration_path(&value, cwd, directory)?);
        }
    }
    Ok(())
}

// 조회와 검증 함수를 분리해 전역 환경이나 사용자 폴더 권한에 의존하지 않고 전달 규칙을 검증한다.
fn configure_user_config_environment(
    command: &mut Command,
    cwd: &Path,
    include_user_config: bool,
    mut lookup: impl FnMut(&str) -> Option<OsString>,
    mut validate: impl FnMut(&OsStr, &Path) -> Result<PathBuf>,
) {
    if !include_user_config {
        return;
    }
    for name in [
        "HOME",
        "USERPROFILE",
        "XDG_CONFIG_HOME",
        "GH_CONFIG_DIR",
        // Windows gh는 이 경로에서 host 설정을 찾는다. keyring에 토큰만 있어도 API 로그인이 되지는 않는다.
        #[cfg(windows)]
        "APPDATA",
    ] {
        let Some(value) = lookup(name).filter(|value| !value.is_empty()) else {
            continue;
        };
        if let Ok(path) = validate(&value, cwd) {
            command.env(name, path);
        }
    }
}

pub(crate) fn neutral_user_config_directory(cwd: &Path) -> PathBuf {
    for name in ["HOME", "USERPROFILE", "XDG_CONFIG_HOME"] {
        if let Some(value) = std::env::var_os(name)
            && let Ok(path) = validate_configuration_path(&value, cwd, true)
        {
            return path;
        }
    }
    #[cfg(unix)]
    return PathBuf::from("/");
    #[cfg(windows)]
    return PathBuf::from(r"C:\");
}

fn resolve_alias(upper: &str, lower: &str) -> Result<Option<OsString>> {
    let upper = std::env::var_os(upper).filter(|value| !value.is_empty());
    let lower = std::env::var_os(lower).filter(|value| !value.is_empty());
    match (upper, lower) {
        (Some(upper), Some(lower)) if upper != lower => {
            bail!("conflicting upper/lower-case network environment values")
        }
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

fn validate_proxy(value: &OsStr) -> Result<()> {
    let value = value.to_str().context("HTTPS proxy must be UTF-8")?;
    if !(value.starts_with("http://") || value.starts_with("https://"))
        || value.contains('@')
        || value.chars().any(char::is_whitespace)
        || value.chars().any(char::is_control)
    {
        bail!("HTTPS proxy must be an absolute credential-free HTTP(S) URL")
    }
    Ok(())
}

fn validate_no_proxy(value: &OsStr) -> Result<()> {
    let value = value.to_str().context("NO_PROXY must be UTF-8")?;
    if value.is_empty()
        || value.chars().any(char::is_whitespace)
        || value.chars().any(char::is_control)
    {
        bail!("NO_PROXY contains unsafe characters")
    }
    Ok(())
}

fn safe_plain_value(value: &OsString) -> bool {
    value
        .to_str()
        .is_some_and(|value| !value.is_empty() && !value.chars().any(char::is_control))
}

#[cfg(test)]
mod tests {
    use super::{copy_native_executable_fixture, parse_windows_npm_codex_cmd_shim};
    #[cfg(unix)]
    use super::{resolve_codex_command_from_path, sanitized_path};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    #[cfg(windows)]
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn credential_config_opt_out_does_not_consult_user_environment() {
        let mut command = std::process::Command::new("not-executed");
        command.env_clear();
        super::configure_user_config_environment(
            &mut command,
            Path::new("."),
            false,
            |_| panic!("disabled user configuration must not consult its environment"),
            |_, _| panic!("disabled user configuration must not validate a path"),
        );
        assert_eq!(command.get_envs().count(), 0);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_credential_configuration_does_not_consult_appdata() {
        let mut command = std::process::Command::new("not-executed");
        command.env_clear();
        super::configure_user_config_environment(
            &mut command,
            Path::new("."),
            true,
            |name| {
                assert_ne!(name, "APPDATA");
                None
            },
            |_, _| panic!("no user configuration value should reach validation"),
        );
        assert_eq!(command.get_envs().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_configuration_passes_only_validated_appdata() {
        // 이 시험은 전달 경계를 검증한다. 실제 사용자 폴더를 만들거나 그 ACL을 가정하지 않는다.
        let workspace = Path::new("C:/synthetic-workspace");
        let supplied = Path::new("C:/supplied-appdata");
        let validated = PathBuf::from("C:/validated-appdata");
        let mut command = Command::new("not-executed");
        command.env_clear();
        super::configure_user_config_environment(
            &mut command,
            workspace,
            true,
            |name| (name == "APPDATA").then(|| supplied.as_os_str().to_owned()),
            |value, cwd| {
                assert_eq!(value, supplied.as_os_str());
                assert_eq!(cwd, workspace);
                Ok(validated.clone())
            },
        );
        let passed = command
            .get_envs()
            .find_map(|(name, value)| (name == std::ffi::OsStr::new("APPDATA")).then_some(value));
        assert_eq!(passed, Some(Some(validated.as_os_str())));
        assert_eq!(command.get_envs().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_configuration_never_passes_a_path_after_validation_failure() {
        let supplied = Path::new("C:/supplied-appdata");
        let mut command = Command::new("not-executed");
        command.env_clear();
        let mut validations = 0;
        super::configure_user_config_environment(
            &mut command,
            Path::new("C:/synthetic-workspace"),
            true,
            |name| (name == "APPDATA").then(|| supplied.as_os_str().to_owned()),
            |value, _| {
                validations += 1;
                assert_eq!(value, supplied.as_os_str());
                Err(anyhow::anyhow!(
                    "synthetic configuration validation failure"
                ))
            },
        );
        assert_eq!(validations, 1);
        assert_eq!(
            command.get_envs().count(),
            0,
            "failed validation must not fall back to the supplied path"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_credential_configuration_rejects_relative_and_repository_appdata() {
        let workspace = fixture_root("credential-appdata-untrusted-workspace");
        let repository_config = workspace.join("credential-config");
        fs::create_dir_all(&repository_config).expect("repository config fixture should exist");
        for config in [
            PathBuf::from("relative-credential-config"),
            repository_config,
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        ] {
            let expected_boundary = if config.is_absolute() {
                "repository- or pool-controlled"
            } else {
                "must be absolute"
            };
            let mut command = Command::new("not-executed");
            command.env_clear();
            super::configure_user_config_environment(
                &mut command,
                &workspace,
                true,
                |name| (name == "APPDATA").then(|| config.as_os_str().to_owned()),
                |value, cwd| {
                    let result = super::validate_configuration_path(value, cwd, true);
                    assert!(
                        result.as_ref().err().is_some_and(|error| error
                            .chain()
                            .any(|cause| cause.to_string().contains(expected_boundary))),
                        "configuration must fail at the expected path boundary"
                    );
                    result
                },
            );
            assert_eq!(
                command.get_envs().count(),
                0,
                "untrusted configuration must not fall back to the original path"
            );
        }
        fs::remove_dir_all(workspace).expect("workspace fixture should be removed");
    }

    #[test]
    fn codex_executable_trust_policy_defaults_to_permissive_and_accepts_exact_strict_opt_in() {
        assert_eq!(
            super::resolve_codex_executable_trust_policy(None)
                .expect("missing policy should use the default"),
            super::CodexExecutableTrustPolicy::AllowUntrusted
        );
        assert_eq!(
            super::resolve_codex_executable_trust_policy(Some("0"))
                .expect("zero should retain the permissive default"),
            super::CodexExecutableTrustPolicy::AllowUntrusted
        );
        assert_eq!(
            super::resolve_codex_executable_trust_policy(Some("1"))
                .expect("one should require trusted executables"),
            super::CodexExecutableTrustPolicy::RequireTrusted
        );
        assert!(super::resolve_codex_executable_trust_policy(Some("true")).is_err());
        assert_eq!(
            super::REQUIRE_TRUSTED_CODEX_EXECUTABLE_ENV_VAR,
            "AKRA_REQUIRE_TRUSTED_CODEX_EXECUTABLE"
        );
    }

    #[test]
    fn permissive_codex_resolution_pins_repository_binary_and_reports_trust_warning() {
        let root = fixture_root("permissive-codex-resolution");
        let workspace = root.join("workspace");
        let bin = workspace.join("bin");
        fs::create_dir_all(&bin).expect("repository bin should be created");
        let codex = bin.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        copy_native_executable_fixture(&codex).expect("native Codex fixture should copy");

        let command = super::resolve_codex_command_from_path_with_policy(
            bin.as_os_str(),
            &workspace,
            super::CodexExecutableTrustPolicy::AllowUntrusted,
        )
        .expect("permissive policy should pin a repository-controlled Codex binary");

        assert_eq!(
            command.program,
            fs::canonicalize(&codex).expect("Codex fixture should canonicalize")
        );
        assert_eq!(command.source_executable, command.program);
        assert!(command.prefix_args.is_empty());
        assert!(command.trust_warning.is_some());
        assert!(
            super::resolve_codex_command_from_path_with_policy(
                bin.as_os_str(),
                &workspace,
                super::CodexExecutableTrustPolicy::RequireTrusted,
            )
            .is_err()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn permissive_codex_resolution_rejects_native_binary_without_execute_bits() {
        let root = fixture_root("permissive-non-executable-codex");
        let workspace = root.join("workspace");
        let bin = workspace.join("bin");
        fs::create_dir_all(&bin).expect("repository bin should be created");
        let codex = bin.join("codex");
        copy_native_executable_fixture(&codex).expect("native Codex fixture should copy");
        let mut permissions = fs::metadata(&codex)
            .expect("native Codex fixture should have metadata")
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&codex, permissions)
            .expect("native Codex fixture should lose execute bits");

        let error = super::resolve_codex_command_from_path_with_policy(
            bin.as_os_str(),
            &workspace,
            super::CodexExecutableTrustPolicy::AllowUntrusted,
        )
        .expect_err("permissive policy must retain native execute-bit validation");

        assert!(
            error
                .chain()
                .map(ToString::to_string)
                .any(|detail| detail.contains("not executable"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_macos_homebrew_style_codex_launcher_is_trusted() {
        let root = fixture_root("native-macos-homebrew-codex");
        let workspace = root.join("workspace");
        let prefix = root.join("homebrew");
        let bin = prefix.join("bin");
        let library = prefix.join("lib");
        let cellar = prefix.join("Cellar");
        let launcher = library
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin")
            .join("codex.js");
        let node = cellar.join("node").join("1.0.0").join("bin").join("node");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::create_dir_all(&bin).expect("Homebrew bin should be created");
        fs::create_dir_all(
            launcher
                .parent()
                .expect("Codex launcher should have a parent"),
        )
        .expect("Codex package directory should be created");
        fs::create_dir_all(node.parent().expect("Node should have a parent"))
            .expect("Node Cellar directory should be created");
        make_safe_directory_chain(&root);
        make_safe_directory_ancestors(&launcher, &root);
        make_safe_directory_ancestors(&node, &root);
        write_executable(&launcher, "#!/usr/bin/env node\nprocess.exit(0);\n");
        write_native_executable(&node);
        symlink(&launcher, bin.join("codex")).expect("Codex launcher should be linked");
        symlink(&node, bin.join("node")).expect("Node should be linked");

        for directory in [bin.as_path(), library.as_path(), cellar.as_path()] {
            if let Err(error) =
                std::os::unix::fs::chown(directory, None, Some(super::MACOS_ADMIN_GROUP_ID))
            {
                if error.kind() == std::io::ErrorKind::PermissionDenied {
                    eprintln!(
                        "skipping Homebrew path integration: cannot assign the macOS admin group"
                    );
                    let _ = fs::remove_dir_all(root);
                    return;
                }
                panic!("Homebrew directory should use the macOS admin group: {error}");
            }
            fs::set_permissions(directory, fs::Permissions::from_mode(0o775))
                .expect("Homebrew directory should be group-writable");
        }

        let plan = resolve_codex_command_from_path(bin.as_os_str(), &workspace)
            .expect("standard Homebrew Codex and Node paths should resolve");
        assert_eq!(
            plan.source_executable,
            fs::canonicalize(&launcher).expect("launcher should canonicalize")
        );
        assert_eq!(
            plan.program,
            fs::canonicalize(&node).expect("Node should canonicalize")
        );
        sanitized_path(bin.as_os_str(), &workspace)
            .expect("standard Homebrew bin should remain on the sanitized PATH");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn native_windows_codex_shim_pins_node_and_package_script() {
        let root = fixture_root("native-windows-codex-shim");
        let workspace = root.join("workspace");
        let install = root.join("install");
        let target = install
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin")
            .join("codex.js");
        fs::create_dir_all(&workspace).expect("Windows workspace fixture should create");
        fs::create_dir_all(
            target
                .parent()
                .expect("package target should have a parent"),
        )
        .expect("Windows Codex package fixture should create");
        fs::write(&target, "process.exit(0);\n").expect("Codex package script should write");
        let node = install.join("node.exe");
        copy_native_executable_fixture(&node).expect("native Windows Node fixture should copy");
        let shim = install.join("codex.cmd");
        fs::write(
            &shim,
            standard_windows_codex_shim(r"node_modules\@openai\codex\bin\codex.js"),
        )
        .expect("Windows Codex shim should write");

        let plan = super::resolve_codex_command_from_path(install.as_os_str(), &workspace)
            .expect("standard Windows npm Codex launcher should resolve");

        assert_eq!(
            plan.program,
            fs::canonicalize(node).expect("Node fixture should canonicalize")
        );
        assert_eq!(
            plan.source_executable,
            fs::canonicalize(shim).expect("Codex shim should canonicalize")
        );
        assert_eq!(plan.prefix_args, vec![target.into_os_string()]);
        assert!(!plan.prefix_args[0].to_string_lossy().starts_with(r"\\?\"));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn codex_launcher_pins_its_trusted_node_interpreter_and_script() {
        let workspace_root = fixture_root("trusted-codex-workspace");
        let install_root = fixture_root("trusted-codex-install");
        let workspace = workspace_root.join("workspace");
        let install = install_root.join("user-local");
        let node_directory = install_root.join("safe-bin");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::create_dir_all(&install).expect("Codex install should be created");
        fs::create_dir_all(&node_directory).expect("Node.js fixture directory should be created");
        make_safe_directory_chain(&workspace_root);
        make_safe_directory_chain(&install_root);
        let launcher = install.join("codex.js");
        write_executable(&launcher, "#!/usr/bin/env node\nprocess.exit(0);\n");
        symlink(&launcher, install.join("codex")).expect("Codex shim should be linked");
        let node = node_directory.join("node");
        write_native_executable(&node);
        let path = std::env::join_paths([install.clone(), node_directory])
            .expect("Codex PATH should join");

        let plan = resolve_codex_command_from_path(&path, &workspace)
            .expect("standard npm Codex launcher should resolve");

        assert_eq!(
            plan.program,
            fs::canonicalize(node).expect("Node.js fixture should canonicalize")
        );
        assert_eq!(
            plan.source_executable,
            fs::canonicalize(&launcher).expect("launcher should canonicalize")
        );
        assert_eq!(
            plan.prefix_args,
            vec![plan.source_executable.as_os_str().to_os_string()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_launcher_skips_supported_path_wrappers_with_unsupported_shebangs() {
        let workspace_root = fixture_root("wrapped-codex-workspace");
        let install_root = fixture_root("wrapped-codex-install");
        let workspace = workspace_root.join("workspace");
        let wrapper_directory = install_root.join("wrapper-bin");
        let install = install_root.join("user-local");
        let node_directory = install_root.join("safe-bin");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::create_dir_all(&wrapper_directory).expect("wrapper directory should be created");
        fs::create_dir_all(&install).expect("Codex install should be created");
        fs::create_dir_all(&node_directory).expect("Node.js fixture directory should be created");
        make_safe_directory_chain(&workspace_root);
        make_safe_directory_chain(&install_root);
        write_executable(
            &wrapper_directory.join("codex"),
            "#!/usr/bin/env bash\nexec codex \"$@\"\n",
        );
        let launcher = install.join("codex.js");
        write_executable(&launcher, "#!/usr/bin/env node\nprocess.exit(0);\n");
        symlink(&launcher, install.join("codex")).expect("Codex shim should be linked");
        let node = node_directory.join("node");
        write_native_executable(&node);
        let path = std::env::join_paths([&wrapper_directory, &install, &node_directory])
            .expect("wrapped Codex PATH should join");

        let plan = resolve_codex_command_from_path(&path, &workspace)
            .expect("supported Codex after a shell wrapper should resolve");

        assert_eq!(
            plan.source_executable,
            fs::canonicalize(&launcher).expect("launcher should canonicalize")
        );
        assert_eq!(
            plan.program,
            fs::canonicalize(&node).expect("Node.js fixture should canonicalize")
        );

        write_executable(
            &wrapper_directory.join("codex"),
            "#!/bin/bash\nexec codex \"$@\"\n",
        );
        let plan = resolve_codex_command_from_path(&path, &workspace)
            .expect("supported Codex after an absolute shell wrapper should resolve");
        assert_eq!(
            plan.program,
            fs::canonicalize(node).expect("Node.js fixture should canonicalize")
        );
    }

    #[cfg(unix)]
    #[test]
    fn codex_launcher_rejects_repository_node_and_unbounded_shebangs() {
        let workspace_root = fixture_root("hostile-codex-workspace");
        let install_root = fixture_root("hostile-codex-install");
        let workspace = workspace_root.join("workspace");
        let repo_bin = workspace.join("bin");
        let install = install_root.join("user-local");
        let node_directory = install_root.join("safe-bin");
        fs::create_dir_all(&repo_bin).expect("repository bin should be created");
        fs::create_dir_all(&install).expect("Codex install should be created");
        fs::create_dir_all(&node_directory).expect("Node.js fixture directory should be created");
        make_safe_directory_chain(&workspace_root);
        make_safe_directory_chain(&install_root);
        let launcher = install.join("codex.js");
        write_executable(&launcher, "#!/usr/bin/env node\nprocess.exit(0);\n");
        symlink(&launcher, install.join("codex")).expect("Codex shim should be linked");
        write_executable(&repo_bin.join("node"), "#!/bin/sh\nexit 91\n");
        write_native_executable(&node_directory.join("node"));
        let path = std::env::join_paths([install.clone(), repo_bin, node_directory])
            .expect("hostile PATH should join");

        let error = resolve_codex_command_from_path(&path, &workspace)
            .expect_err("repository Node.js must fail closed");
        assert!(
            error
                .chain()
                .map(ToString::to_string)
                .any(|detail| detail.contains("repository"))
        );

        fs::remove_file(install.join("codex")).expect("Codex shim should be replaced");
        write_executable(
            &launcher,
            &format!("#!{}", "x".repeat(super::MAX_CODEX_SHEBANG_BYTES)),
        );
        symlink(&launcher, install.join("codex")).expect("Codex shim should be relinked");
        let error = resolve_codex_command_from_path(&path, &workspace)
            .expect_err("a launcher without a bounded newline must fail closed");
        assert!(
            error.to_string().contains("bounded newline") || error.to_string().contains("limit")
        );
    }

    #[test]
    fn parses_only_the_standard_windows_npm_codex_cmd_shim() {
        let target = r"node_modules\@openai\codex\bin\codex.js";
        let standard = standard_windows_codex_shim(target);
        assert_eq!(
            parse_windows_npm_codex_cmd_shim(&standard).expect("standard npm shim should parse"),
            PathBuf::from(target)
        );

        let arbitrary_batch = format!("@ECHO off\r\ncalc.exe\r\n{standard}");
        assert!(parse_windows_npm_codex_cmd_shim(&arbitrary_batch).is_err());
        let wrong_package = standard_windows_codex_shim(r"node_modules\other\bin\codex.js");
        assert!(parse_windows_npm_codex_cmd_shim(&wrong_package).is_err());
        let parent_traversal =
            standard_windows_codex_shim(r"..\node_modules\@openai\codex\bin\codex.js");
        assert!(parse_windows_npm_codex_cmd_shim(&parent_traversal).is_err());
        let injected =
            standard_windows_codex_shim(r"node_modules\@openai\codex\bin\codex.js&calc.exe");
        assert!(parse_windows_npm_codex_cmd_shim(&injected).is_err());
        let duplicate = format!(
            "{}\r\n{}",
            standard.trim_end(),
            standard
                .lines()
                .last()
                .expect("standard shim should contain an invocation")
        );
        assert!(parse_windows_npm_codex_cmd_shim(&duplicate).is_err());
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).expect("executable fixture should write");
        let mut permissions = fs::metadata(path)
            .expect("fixture metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("fixture should become executable");
    }

    #[cfg(unix)]
    fn write_native_executable(path: &Path) {
        copy_native_executable_fixture(path).expect("native executable fixture should copy");
    }

    fn standard_windows_codex_shim(target: &str) -> String {
        format!(
            "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\{target}\" %*\r\n"
        )
    }

    #[cfg(unix)]
    fn make_safe_directory_chain(root: &Path) {
        let mut root_permissions = fs::metadata(root)
            .expect("fixture root metadata should exist")
            .permissions();
        root_permissions.set_mode(0o755);
        fs::set_permissions(root, root_permissions).expect("fixture root should be safe");
        for child in [
            root.join("workspace"),
            root.join("user-local"),
            root.join("safe-bin"),
        ] {
            if child.exists() {
                let mut permissions = fs::metadata(&child)
                    .expect("child metadata should exist")
                    .permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(child, permissions).expect("child should be safe");
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn make_safe_directory_ancestors(path: &Path, root: &Path) {
        for directory in path
            .parent()
            .expect("fixture path should have a parent")
            .ancestors()
            .take_while(|directory| directory.starts_with(root))
        {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755))
                .expect("fixture directory should have deterministic permissions");
        }
    }

    #[cfg(unix)]
    fn fixture_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch")
            .as_nanos();
        let root = PathBuf::from(std::env::var_os("HOME").expect("HOME should be available"))
            .join(".cache")
            .join(format!("akra-{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture root should be created");
        root
    }

    #[cfg(windows)]
    fn fixture_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch")
            .as_nanos();
        let root = crate::private_fs::windows_local_app_data_path()
            .expect("Windows LocalAppData should resolve")
            .join("Akra")
            .join("tests")
            .join(format!("{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture root should be created");
        root
    }
}
