use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

#[cfg(unix)]
const MAX_CODEX_SHEBANG_BYTES: usize = 4 * 1024;
#[cfg(any(windows, test))]
const MAX_WINDOWS_CODEX_SHIM_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustedCommand {
    pub(crate) program: PathBuf,
    pub(crate) prefix_args: Vec<OsString>,
    pub(crate) source_executable: PathBuf,
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowsAceMutationAction {
    Ignore,
    InspectStandardAllowSid,
    RejectUnrecognizedAllow,
}

#[cfg(any(windows, test))]
pub(crate) fn classify_windows_ace_mutation(
    ace_type: u8,
    mask: u32,
    directory: bool,
) -> WindowsAceMutationAction {
    const DELETE: u32 = 0x0001_0000;
    const FILE_DELETE_CHILD: u32 = 0x0000_0040;
    const FILE_WRITE_DATA: u32 = 0x0000_0002;
    const FILE_APPEND_DATA: u32 = 0x0000_0004;
    const FILE_WRITE_EA: u32 = 0x0000_0010;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
    const WRITE_DAC: u32 = 0x0004_0000;
    const WRITE_OWNER: u32 = 0x0008_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const GENERIC_ALL: u32 = 0x1000_0000;
    let dangerous = if directory {
        DELETE
            | FILE_DELETE_CHILD
            | FILE_WRITE_DATA
            | FILE_APPEND_DATA
            | FILE_WRITE_EA
            | FILE_WRITE_ATTRIBUTES
            | WRITE_DAC
            | WRITE_OWNER
            | GENERIC_WRITE
            | GENERIC_ALL
    } else {
        DELETE
            | FILE_WRITE_DATA
            | FILE_APPEND_DATA
            | FILE_WRITE_EA
            | FILE_WRITE_ATTRIBUTES
            | WRITE_DAC
            | WRITE_OWNER
            | GENERIC_WRITE
            | GENERIC_ALL
    };
    if mask & dangerous == 0 {
        return WindowsAceMutationAction::Ignore;
    }
    match ace_type {
        0 => WindowsAceMutationAction::InspectStandardAllowSid,
        // Compound, object, and callback allow ACEs need type-specific SID/condition parsing.
        4 | 5 | 9 | 11 => WindowsAceMutationAction::RejectUnrecognizedAllow,
        // Known deny/audit/alarm/mandatory/resource/trust/filter ACEs do not grant access.
        1..=3 | 6..=8 | 10 | 12..=15 | 17..=21 => WindowsAceMutationAction::Ignore,
        _ => WindowsAceMutationAction::RejectUnrecognizedAllow,
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
                resolve_codex_command_from_path(&path, &cwd).map_err(|error| error.to_string())
            })
            .clone()
            .map_err(anyhow::Error::msg)
    }
}

pub(crate) fn resolve_native_from_current_path(program: &str, cwd: &Path) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH is unavailable")?;
    resolve_native_from_path(program, &path, cwd)
}

pub(crate) fn resolve_native_from_path(program: &str, path: &OsStr, cwd: &Path) -> Result<PathBuf> {
    let executable = resolve_from_path(program, path, cwd)?;
    validate_native_executable(&executable)?;
    Ok(executable)
}

pub(crate) fn resolve_codex_command_from_path(path: &OsStr, cwd: &Path) -> Result<TrustedCommand> {
    let source_executable = resolve_from_path("codex", path, cwd)?;
    if validate_native_executable(&source_executable).is_ok() {
        return Ok(TrustedCommand {
            program: source_executable.clone(),
            prefix_args: Vec::new(),
            source_executable,
        });
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
        let shim = read_validated_bytes(
            &source_executable,
            MAX_WINDOWS_CODEX_SHIM_BYTES,
            "Windows Codex npm shim",
        )?;
        let shim = std::str::from_utf8(&shim).context("Windows Codex npm shim is not UTF-8")?;
        let relative_target = parse_windows_npm_codex_cmd_shim(shim)?;
        let target = source_executable
            .parent()
            .context("Windows Codex npm shim has no parent directory")?
            .join(relative_target);
        let target = validate_absolute(&target, cwd)
            .context("Windows Codex npm target failed executable trust validation")?;
        if !has_codex_package_script_suffix(&target) {
            bail!("Windows Codex npm target is outside @openai/codex/bin/codex.js")
        }
        let node = resolve_native_from_path("node", path, cwd)?;
        return Ok(TrustedCommand {
            program: node,
            prefix_args: vec![target.as_os_str().to_os_string()],
            source_executable,
        });
    }
    #[cfg(unix)]
    {
        let first_line = read_validated_first_line(&source_executable)?;
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
            "#!/usr/bin/env node" => resolve_native_from_path("node", path, cwd)?,
            value if value.starts_with("#!") && !value[2..].contains(char::is_whitespace) => {
                let interpreter = Path::new(&value[2..]);
                if !interpreter.is_absolute() {
                    bail!("Codex launcher interpreter must be absolute")
                }
                let interpreter = validate_absolute(interpreter, cwd)?;
                validate_native_executable(&interpreter)?;
                let name = interpreter
                    .file_name()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default();
                if !matches!(name, "node" | "nodejs") {
                    bail!("Codex launcher interpreter is not trusted Node.js")
                }
                interpreter
            }
            _ => bail!("Codex launcher uses an unsupported shebang"),
        };
        Ok(TrustedCommand {
            program: node,
            prefix_args: vec![source_executable.as_os_str().to_os_string()],
            source_executable,
        })
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InspectedFileIdentity {
    device: u64,
    inode: u64,
    length: u64,
    mode: u32,
    owner: u32,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(windows)]
type InspectedFileIdentity = crate::private_fs::WindowsFileIdentitySnapshot;

#[cfg(unix)]
fn inspected_file_identity(metadata: &fs::Metadata) -> InspectedFileIdentity {
    use std::os::unix::fs::MetadataExt;

    InspectedFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        mode: metadata.mode(),
        owner: metadata.uid(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

fn open_validated_file(path: &Path) -> Result<(fs::File, InspectedFileIdentity)> {
    #[cfg(unix)]
    {
        let file = fs::File::open(path)
            .with_context(|| format!("failed to open trusted file `{}`", path.display()))?;
        let metadata = file
            .metadata()
            .context("failed to inspect trusted file handle")?;
        if !metadata.is_file() {
            bail!("trusted file handle is not a regular file")
        }
        validate_owner_and_mode(&metadata, false)?;
        let path_metadata = fs::metadata(path)
            .with_context(|| format!("failed to re-open trusted file `{}`", path.display()))?;
        let identity = inspected_file_identity(&metadata);
        if identity != inspected_file_identity(&path_metadata) {
            bail!("trusted file path changed while it was opened")
        }
        Ok((file, identity))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;

        use crate::private_fs::{
            WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
            WINDOWS_READ_CONTROL, validate_windows_path_identity_only,
            validate_windows_trusted_executable_acl, windows_file_identity_snapshot,
        };
        let file = fs::OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .with_context(|| format!("failed to open trusted Windows file `{}`", path.display()))?;
        validate_windows_path_identity_only(path, &file, false)?;
        validate_windows_trusted_executable_acl(path, &file, false)?;
        let identity = windows_file_identity_snapshot(&file)?;
        Ok((file, identity))
    }
}

fn finish_validated_file_read(
    path: &Path,
    file: &fs::File,
    initial: InspectedFileIdentity,
) -> Result<()> {
    #[cfg(unix)]
    {
        let metadata = file
            .metadata()
            .context("failed to re-inspect trusted file handle")?;
        validate_owner_and_mode(&metadata, false)?;
        let path_metadata = fs::metadata(path)
            .with_context(|| format!("failed to re-open trusted file `{}`", path.display()))?;
        if initial != inspected_file_identity(&metadata)
            || initial != inspected_file_identity(&path_metadata)
        {
            bail!("trusted file identity or contents changed while being inspected")
        }
    }
    #[cfg(windows)]
    {
        use crate::private_fs::{
            validate_windows_path_identity_only, validate_windows_trusted_executable_acl,
            windows_file_identity_snapshot,
        };
        if initial != windows_file_identity_snapshot(file)? {
            bail!("trusted Windows file changed while being inspected")
        }
        validate_windows_path_identity_only(path, file, false)?;
        validate_windows_trusted_executable_acl(path, file, false)?;
    }
    Ok(())
}

fn read_validated_prefix(path: &Path, length: usize, label: &str) -> Result<Vec<u8>> {
    let (mut file, identity) = open_validated_file(path)?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)
        .with_context(|| format!("failed to read {label} from `{}`", path.display()))?;
    finish_validated_file_read(path, &file, identity)?;
    Ok(bytes)
}

#[cfg(windows)]
fn read_validated_bytes(path: &Path, maximum: usize, label: &str) -> Result<Vec<u8>> {
    let (mut file, identity) = open_validated_file(path)?;
    let read_limit = maximum
        .checked_add(1)
        .context("trusted file read limit overflowed")?;
    let mut bytes = Vec::with_capacity(read_limit);
    file.by_ref()
        .take(read_limit as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {label} from `{}`", path.display()))?;
    if bytes.len() > maximum {
        bail!("{label} exceeds the {maximum}-byte security limit")
    }
    finish_validated_file_read(path, &file, identity)?;
    Ok(bytes)
}

#[cfg(unix)]
fn read_validated_first_line(path: &Path) -> Result<Vec<u8>> {
    let (file, identity) = open_validated_file(path)?;
    let read_limit = MAX_CODEX_SHEBANG_BYTES
        .checked_add(1)
        .context("Codex shebang read limit overflowed")?;
    let mut reader = BufReader::new(file.take(read_limit as u64));
    let mut line = Vec::with_capacity(128);
    let read = reader
        .read_until(b'\n', &mut line)
        .with_context(|| format!("failed to read Codex launcher `{}`", path.display()))?;
    let file = reader.into_inner().into_inner();
    if read == 0 || !line.ends_with(b"\n") {
        bail!("Codex launcher first line is missing a bounded newline")
    }
    if line.len() > MAX_CODEX_SHEBANG_BYTES {
        bail!("Codex launcher first line exceeds the security limit")
    }
    line.pop();
    finish_validated_file_read(path, &file, identity)?;
    Ok(line)
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

pub(crate) fn validate_native_executable(path: &Path) -> Result<()> {
    let bytes = read_validated_prefix(path, 4, "native executable header")?;
    #[cfg(windows)]
    if bytes.starts_with(b"MZ") {
        return Ok(());
    }
    #[cfg(unix)]
    if bytes.starts_with(b"\x7fELF")
        || matches!(
            bytes.as_slice(),
            [0xfe, 0xed, 0xfa, 0xce]
                | [0xce, 0xfa, 0xed, 0xfe]
                | [0xfe, 0xed, 0xfa, 0xcf]
                | [0xcf, 0xfa, 0xed, 0xfe]
                | [0xca, 0xfe, 0xba, 0xbe]
                | [0xbe, 0xba, 0xfe, 0xca]
        )
    {
        return Ok(());
    }
    bail!("security-sensitive executable must be a native binary")
}

pub(crate) fn resolve_from_path(program: &str, path: &OsStr, cwd: &Path) -> Result<PathBuf> {
    if program.is_empty()
        || Path::new(program).components().count() != 1
        || program.contains(['/', '\\'])
    {
        bail!("trusted executable name must be a single path component")
    }
    let untrusted_roots = untrusted_roots(cwd);
    let entries = std::env::split_paths(path).collect::<Vec<_>>();
    if entries.is_empty() {
        bail!("PATH contains no executable directories")
    }
    for directory in entries {
        if !directory.is_absolute() {
            bail!(
                "PATH contains a relative executable directory `{}`",
                directory.display()
            )
        }
        for name in executable_names(program) {
            let candidate = directory.join(name);
            match fs::symlink_metadata(&candidate) {
                Ok(_) => {
                    return validate_candidate(&candidate, &untrusted_roots).with_context(|| {
                        format!(
                            "PATH selected an unsafe `{program}` executable at `{}`",
                            candidate.display()
                        )
                    });
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
    bail!("`{program}` was not found on the trusted absolute PATH")
}

pub(crate) fn sanitized_path(path: &OsStr, cwd: &Path) -> Result<OsString> {
    let untrusted_roots = untrusted_roots(cwd);
    let mut trusted = Vec::new();
    for directory in std::env::split_paths(path) {
        if !directory.is_absolute() {
            continue;
        }
        let Ok(canonical) = fs::canonicalize(&directory) else {
            continue;
        };
        if path_is_untrusted(&canonical, &untrusted_roots)
            || validate_directory_chain(&canonical).is_err()
            || trusted.contains(&canonical)
        {
            continue;
        }
        trusted.push(canonical);
    }
    if trusted.is_empty() {
        bail!("PATH contains no trusted absolute executable directories")
    }
    std::env::join_paths(trusted).context("trusted executable PATH could not be constructed")
}

pub(crate) fn validate_absolute(path: &Path, cwd: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("pinned executable path must be absolute")
    }
    validate_candidate(path, &untrusted_roots(cwd))
}

pub(crate) fn configure_credential_command_environment(
    command: &mut Command,
    cwd: &Path,
    include_user_config: bool,
) -> Result<()> {
    command.env_clear();
    let inherited_path = std::env::var_os("PATH").context("PATH is unavailable")?;
    command.env("PATH", sanitized_path(&inherited_path, cwd)?);

    for name in ["LANG", "LC_ALL", "LC_CTYPE"] {
        if let Some(value) = std::env::var_os(name).filter(safe_plain_value) {
            command.env(name, value);
        }
    }
    if include_user_config {
        for name in ["HOME", "USERPROFILE", "XDG_CONFIG_HOME", "GH_CONFIG_DIR"] {
            let Some(value) = std::env::var_os(name).filter(|value| !value.is_empty()) else {
                continue;
            };
            if let Ok(path) = validate_configuration_path(&value, cwd, true) {
                command.env(name, path);
            }
        }
    }
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

fn validate_configuration_path(value: &OsStr, cwd: &Path, directory: bool) -> Result<PathBuf> {
    let path = Path::new(value);
    if !path.is_absolute() {
        bail!("credential subprocess configuration path must be absolute")
    }
    let canonical = fs::canonicalize(path).with_context(|| {
        format!(
            "failed to canonicalize credential configuration path `{}`",
            path.display()
        )
    })?;
    if path_is_untrusted(&canonical, &untrusted_roots(cwd)) {
        bail!("credential configuration path is repository- or pool-controlled")
    }
    let metadata = fs::metadata(&canonical)?;
    if metadata.is_dir() != directory || metadata.is_file() == directory {
        bail!("credential configuration path has the wrong filesystem type")
    }
    validate_directory_chain(if directory {
        &canonical
    } else {
        canonical
            .parent()
            .context("credential configuration file has no parent")?
    })?;
    #[cfg(unix)]
    if !directory {
        validate_owner_and_mode(&metadata, false)?;
    }
    #[cfg(windows)]
    if !directory {
        validate_windows_path(&canonical, false)?;
    }
    Ok(canonical)
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

fn validate_candidate(candidate: &Path, untrusted_roots: &[PathBuf]) -> Result<PathBuf> {
    let canonical = fs::canonicalize(candidate)
        .with_context(|| format!("failed to canonicalize `{}`", candidate.display()))?;
    if path_is_untrusted(&canonical, untrusted_roots) {
        bail!("executable resolves inside a repository, worktree, or pool boundary")
    }
    validate_directory_chain(
        canonical
            .parent()
            .context("trusted executable has no parent directory")?,
    )?;
    validate_file(&canonical)?;
    Ok(canonical)
}

fn path_is_untrusted(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| path == root || path.starts_with(root))
}

fn untrusted_roots(cwd: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    push_root(&mut roots, cwd);
    push_root(&mut roots, Path::new(env!("CARGO_MANIFEST_DIR")));

    let mut cursor = cwd;
    loop {
        let git_marker = cursor.join(".git");
        if git_marker.exists() {
            push_root(&mut roots, cursor);
            if git_marker.is_file()
                && let Some(parent) = cursor.parent()
            {
                push_root(&mut roots, parent);
            }
            break;
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
    }
    roots
}

fn push_root(roots: &mut Vec<PathBuf>, path: &Path) {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !roots.contains(&canonical) {
        roots.push(canonical);
    }
}

#[cfg(unix)]
fn validate_directory_chain(directory: &Path) -> Result<()> {
    for ancestor in directory.ancestors() {
        let metadata = fs::metadata(ancestor).with_context(|| {
            format!(
                "failed to inspect executable ancestor `{}`",
                ancestor.display()
            )
        })?;
        if !metadata.is_dir() {
            bail!("executable ancestor is not a directory")
        }
        validate_owner_and_mode(&metadata, false)
            .with_context(|| format!("unsafe executable ancestor `{}`", ancestor.display()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn validate_file(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect executable `{}`", path.display()))?;
    if !metadata.is_file() {
        bail!("trusted executable target is not a regular file")
    }
    validate_owner_and_mode(&metadata, true)
}

#[cfg(unix)]
fn validate_owner_and_mode(metadata: &fs::Metadata, executable: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let owner = metadata.uid();
    // SAFETY: geteuid has no preconditions and does not retain pointers.
    let current = unsafe { libc::geteuid() };
    if owner != 0 && owner != current {
        bail!("path owner is neither root nor the current user")
    }
    if metadata.mode() & 0o022 != 0 {
        bail!("path is group- or world-writable")
    }
    if executable && metadata.mode() & 0o111 == 0 {
        bail!("executable target has no execute bit")
    }
    Ok(())
}

#[cfg(windows)]
fn validate_directory_chain(directory: &Path) -> Result<()> {
    for ancestor in directory.ancestors() {
        validate_windows_path(ancestor, true)?;
    }
    Ok(())
}

#[cfg(windows)]
fn validate_file(path: &Path) -> Result<()> {
    validate_windows_path(path, false)
}

#[cfg(windows)]
fn validate_windows_path(path: &Path, directory: bool) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_path_identity_only, validate_windows_trusted_executable_acl,
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(
            WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        )
        .open(path)
        .with_context(|| format!("failed to open trusted Windows path `{}`", path.display()))?;
    validate_windows_path_identity_only(path, &file, directory)?;
    validate_windows_trusted_executable_acl(path, &file, directory)
}

#[cfg(windows)]
fn executable_names(program: &str) -> Vec<String> {
    [".exe", ".cmd", "", ".bat"]
        .into_iter()
        .map(|suffix| format!("{program}{suffix}"))
        .collect()
}

#[cfg(not(windows))]
fn executable_names(program: &str) -> Vec<String> {
    vec![program.to_string()]
}

#[cfg(test)]
mod tests {
    use super::{
        WindowsAceMutationAction, classify_windows_ace_mutation, parse_windows_npm_codex_cmd_shim,
    };
    #[cfg(unix)]
    use super::{
        resolve_codex_command_from_path, resolve_from_path, sanitized_path, validate_absolute,
    };
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    #[cfg(unix)]
    use std::path::Path;
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn resolves_safe_user_local_and_canonical_symlink_installations() {
        let workspace_root = fixture_root("trusted-executable-workspace");
        let install_root = fixture_root("trusted-executable-install");
        let workspace = workspace_root.join("workspace");
        let install = install_root.join("user-local");
        fs::create_dir_all(&workspace).expect("workspace should be created");
        fs::create_dir_all(&install).expect("user install should be created");
        make_safe_directory_chain(&workspace_root);
        make_safe_directory_chain(&install_root);
        let target = install.join("codex-real");
        write_executable(&target, "#!/bin/sh\nexit 0\n");
        let link = install.join("codex");
        symlink(&target, &link).expect("npm-style executable symlink should be created");

        let resolved = resolve_from_path("codex", install.as_os_str(), &workspace)
            .expect("safe user-local symlink should resolve to its canonical target");
        assert_eq!(
            resolved,
            fs::canonicalize(target).expect("target should canonicalize")
        );
        assert_eq!(
            validate_absolute(&resolved, &workspace)
                .expect("a pinned canonical executable should remain valid"),
            resolved
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_relative_repo_and_writable_path_candidates_without_falling_through() {
        let root = fixture_root("trusted-executable-reject");
        let safe_root = fixture_root("trusted-executable-safe-fallback");
        let workspace = root.join("workspace");
        let repo_bin = workspace.join("bin");
        let safe_bin = safe_root.join("safe-bin");
        fs::create_dir_all(&repo_bin).expect("repo bin should be created");
        fs::create_dir_all(&safe_bin).expect("safe bin should be created");
        make_safe_directory_chain(&root);
        make_safe_directory_chain(&safe_root);
        write_executable(&repo_bin.join("codex"), "#!/bin/sh\nexit 91\n");
        write_executable(&safe_bin.join("codex"), "#!/bin/sh\nexit 0\n");

        let relative = std::env::join_paths([PathBuf::from("."), safe_bin.clone()])
            .expect("relative PATH fixture should join");
        assert!(
            resolve_from_path("codex", &relative, &workspace)
                .expect_err("relative PATH entry must fail closed")
                .to_string()
                .contains("relative")
        );

        let repo_first = std::env::join_paths([repo_bin, safe_bin.clone()])
            .expect("repo-first PATH should join");
        let repo_error = resolve_from_path("codex", &repo_first, &workspace)
            .expect_err("repo executable must not fall through to a later safe binary");
        assert!(
            repo_error
                .chain()
                .map(ToString::to_string)
                .any(|detail| detail.contains("repository"))
        );

        let mut permissions = fs::metadata(&safe_bin)
            .expect("safe bin metadata should exist")
            .permissions();
        permissions.set_mode(0o777);
        fs::set_permissions(&safe_bin, permissions).expect("unsafe mode should be applied");
        assert!(resolve_from_path("codex", safe_bin.as_os_str(), &workspace).is_err());
        assert!(
            sanitized_path(safe_bin.as_os_str(), &workspace)
                .expect_err("writable PATH directory should be removed")
                .to_string()
                .contains("no trusted")
        );
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

    #[test]
    fn windows_ace_classification_distinguishes_file_and_directory_mutation_rights() {
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0002, false),
            WindowsAceMutationAction::InspectStandardAllowSid
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0002, true),
            WindowsAceMutationAction::InspectStandardAllowSid,
            "untrusted create-child rights can plant a PATH candidate"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0004, true),
            WindowsAceMutationAction::InspectStandardAllowSid,
            "untrusted create-subdirectory rights can plant a PATH subtree"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x4000_0000, true),
            WindowsAceMutationAction::InspectStandardAllowSid,
            "generic directory write rights include mutation primitives"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0040, true),
            WindowsAceMutationAction::InspectStandardAllowSid
        );
        assert_eq!(
            classify_windows_ace_mutation(9, 0x4000_0000, false),
            WindowsAceMutationAction::RejectUnrecognizedAllow
        );
        assert_eq!(
            classify_windows_ace_mutation(1, 0x1000_0000, false),
            WindowsAceMutationAction::Ignore,
            "deny ACEs never grant mutation rights"
        );
        assert_eq!(
            classify_windows_ace_mutation(0xff, 0x1000_0000, false),
            WindowsAceMutationAction::RejectUnrecognizedAllow
        );
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
        write_executable(path, "\x7fELF");
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
}
