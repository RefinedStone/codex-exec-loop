use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::io::{BufRead, BufReader};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

#[cfg(unix)]
const MAX_CODEX_SHEBANG_BYTES: usize = 4 * 1024;
#[cfg(any(windows, test))]
const MAX_WINDOWS_CODEX_SHIM_BYTES: usize = 16 * 1024;
const MAX_NATIVE_EXECUTABLE_METADATA_BYTES: u64 = 16 * 1024 * 1024;
#[cfg(target_os = "macos")]
const MACOS_ADMIN_GROUP_ID: u32 = 80;

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
        DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER | GENERIC_ALL
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
                resolve_codex_command_from_path(&path, &cwd).map_err(|error| format!("{error:#}"))
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
            WINDOWS_READ_CONTROL, validate_windows_trusted_executable_acl,
            validate_windows_trusted_executable_path_identity, windows_file_identity_snapshot,
        };
        let file = fs::OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .with_context(|| format!("failed to open trusted Windows file `{}`", path.display()))?;
        validate_windows_trusted_executable_path_identity(path, &file, false)?;
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
            validate_windows_trusted_executable_acl,
            validate_windows_trusted_executable_path_identity, windows_file_identity_snapshot,
        };
        if initial != windows_file_identity_snapshot(file)? {
            bail!("trusted Windows file changed while being inspected")
        }
        validate_windows_trusted_executable_path_identity(path, file, false)?;
        validate_windows_trusted_executable_acl(path, file, false)?;
    }
    Ok(())
}

fn inspect_validated_file<T>(
    path: &Path,
    inspect: impl FnOnce(&mut fs::File, u64) -> Result<T>,
) -> Result<T> {
    let (mut file, identity) = open_validated_file(path)?;
    let length = file
        .metadata()
        .context("failed to inspect trusted file length")?
        .len();
    let result = inspect(&mut file, length);
    finish_validated_file_read(path, &file, identity)?;
    result
}

fn read_file_range(
    file: &mut fs::File,
    offset: u64,
    length: u64,
    file_length: u64,
    label: &str,
) -> Result<Vec<u8>> {
    let end = offset
        .checked_add(length)
        .with_context(|| format!("{label} range overflowed"))?;
    if end > file_length || length > MAX_NATIVE_EXECUTABLE_METADATA_BYTES {
        bail!("{label} exceeds the trusted executable inspection boundary")
    }
    let length = usize::try_from(length).context("trusted executable range is too large")?;
    file.seek(SeekFrom::Start(offset))
        .with_context(|| format!("failed to seek to {label}"))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)
        .with_context(|| format!("failed to read {label}"))?;
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
    inspect_validated_file(path, |file, file_length| {
        let prefix_length = file_length.min(64);
        let bytes = read_file_range(
            file,
            0,
            prefix_length,
            file_length,
            "native executable header",
        )?;
        #[cfg(windows)]
        validate_pe_executable(file, &bytes, file_length)?;
        #[cfg(target_os = "macos")]
        validate_macho_executable(file, &bytes, file_length)?;
        #[cfg(all(unix, not(target_os = "macos")))]
        validate_elf_executable(file, &bytes, file_length)?;
        #[cfg(not(any(unix, windows)))]
        bail!("native executable validation is unsupported on this platform");
        Ok(())
    })
    .with_context(|| {
        format!(
            "security-sensitive executable failed native format validation: {}",
            path.display()
        )
    })
}

fn checked_bytes<'a>(
    bytes: &'a [u8],
    offset: usize,
    length: usize,
    label: &str,
) -> Result<&'a [u8]> {
    let end = offset
        .checked_add(length)
        .with_context(|| format!("{label} offset overflowed"))?;
    bytes
        .get(offset..end)
        .with_context(|| format!("{label} is truncated"))
}

#[cfg(windows)]
fn read_le_u16(bytes: &[u8], offset: usize, label: &str) -> Result<u16> {
    let value: [u8; 2] = checked_bytes(bytes, offset, 2, label)?
        .try_into()
        .expect("checked two-byte slice");
    Ok(u16::from_le_bytes(value))
}

#[cfg(windows)]
fn read_le_u32(bytes: &[u8], offset: usize, label: &str) -> Result<u32> {
    let value: [u8; 4] = checked_bytes(bytes, offset, 4, label)?
        .try_into()
        .expect("checked four-byte slice");
    Ok(u32::from_le_bytes(value))
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum ByteOrder {
    Little,
    Big,
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_u16(bytes: &[u8], offset: usize, order: ByteOrder, label: &str) -> Result<u16> {
    let value: [u8; 2] = checked_bytes(bytes, offset, 2, label)?
        .try_into()
        .expect("checked two-byte slice");
    Ok(match order {
        ByteOrder::Little => u16::from_le_bytes(value),
        ByteOrder::Big => u16::from_be_bytes(value),
    })
}

#[cfg(unix)]
fn read_u32(bytes: &[u8], offset: usize, order: ByteOrder, label: &str) -> Result<u32> {
    let value: [u8; 4] = checked_bytes(bytes, offset, 4, label)?
        .try_into()
        .expect("checked four-byte slice");
    Ok(match order {
        ByteOrder::Little => u32::from_le_bytes(value),
        ByteOrder::Big => u32::from_be_bytes(value),
    })
}

#[cfg(unix)]
fn read_u64(bytes: &[u8], offset: usize, order: ByteOrder, label: &str) -> Result<u64> {
    let value: [u8; 8] = checked_bytes(bytes, offset, 8, label)?
        .try_into()
        .expect("checked eight-byte slice");
    Ok(match order {
        ByteOrder::Little => u64::from_le_bytes(value),
        ByteOrder::Big => u64::from_be_bytes(value),
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn validate_elf_executable(file: &mut fs::File, bytes: &[u8], file_length: u64) -> Result<()> {
    const ELF64_HEADER_SIZE: usize = 64;
    const ELF64_PROGRAM_HEADER_SIZE: u16 = 56;
    const PT_LOAD: u32 = 1;
    let header = checked_bytes(bytes, 0, ELF64_HEADER_SIZE, "ELF64 header")?;
    if &header[..4] != b"\x7fELF" {
        bail!("native executable is not an ELF binary")
    }
    if usize::BITS != 64 || header[4] != 2 {
        bail!("ELF executable class does not match the 64-bit host")
    }
    let order = match header[5] {
        1 if cfg!(target_endian = "little") => ByteOrder::Little,
        2 if cfg!(target_endian = "big") => ByteOrder::Big,
        _ => bail!("ELF executable byte order does not match the host"),
    };
    if header[6] != 1 {
        bail!("ELF executable identification version is invalid")
    }
    let executable_type = read_u16(header, 16, order, "ELF type")?;
    if !matches!(executable_type, 2 | 3) {
        bail!("ELF binary is not an executable or position-independent executable")
    }
    let machine = read_u16(header, 18, order, "ELF machine")?;
    if machine != native_elf_machine()? {
        bail!("ELF executable architecture does not match the host")
    }
    if read_u32(header, 20, order, "ELF version")? != 1 {
        bail!("ELF executable version is invalid")
    }
    if read_u16(header, 52, order, "ELF header size")? != ELF64_HEADER_SIZE as u16 {
        bail!("ELF executable header size is invalid")
    }
    let program_offset = read_u64(header, 32, order, "ELF program table offset")?;
    let entry_point = read_u64(header, 24, order, "ELF entry point")?;
    let program_entry_size = read_u16(header, 54, order, "ELF program entry size")?;
    let program_count = read_u16(header, 56, order, "ELF program entry count")?;
    if program_entry_size != ELF64_PROGRAM_HEADER_SIZE || program_count == 0 {
        bail!("ELF executable program table shape is invalid")
    }
    if program_count == u16::MAX {
        bail!("ELF extended program table counts are not accepted")
    }
    let table_length = u64::from(program_entry_size)
        .checked_mul(u64::from(program_count))
        .context("ELF program table length overflowed")?;
    let table_end = program_offset
        .checked_add(table_length)
        .context("ELF program table boundary overflowed")?;
    if program_offset < ELF64_HEADER_SIZE as u64 || table_end > file_length {
        bail!("ELF program table is outside the executable")
    }
    let table = read_file_range(
        file,
        program_offset,
        table_length,
        file_length,
        "ELF program table",
    )?;
    let mut executable_entry_segment = false;
    for index in 0..usize::from(program_count) {
        let entry = index
            .checked_mul(usize::from(program_entry_size))
            .context("ELF program entry offset overflowed")?;
        let segment_type = read_u32(&table, entry, order, "ELF program type")?;
        let segment_flags = read_u32(&table, entry + 4, order, "ELF segment flags")?;
        let file_offset = read_u64(&table, entry + 8, order, "ELF segment offset")?;
        let virtual_address = read_u64(&table, entry + 16, order, "ELF segment address")?;
        let file_size = read_u64(&table, entry + 32, order, "ELF segment file size")?;
        let memory_size = read_u64(&table, entry + 40, order, "ELF segment memory size")?;
        if segment_type == PT_LOAD {
            if file_size > memory_size
                || file_offset
                    .checked_add(file_size)
                    .is_none_or(|end| end > file_length)
            {
                bail!("ELF load segment exceeds the executable boundary")
            }
            let memory_end = virtual_address
                .checked_add(memory_size)
                .context("ELF load segment memory boundary overflowed")?;
            if segment_flags & 1 != 0
                && file_size > 0
                && entry_point >= virtual_address
                && entry_point < memory_end
            {
                executable_entry_segment = true;
            }
        }
    }
    if entry_point == 0 || !executable_entry_segment {
        bail!("ELF entry point is not contained in an executable load segment")
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn native_elf_machine() -> Result<u16> {
    match std::env::consts::ARCH {
        "x86_64" => Ok(62),
        "aarch64" => Ok(183),
        architecture => bail!("ELF executable validation does not support {architecture}"),
    }
}

#[cfg(target_os = "macos")]
fn validate_macho_executable(file: &mut fs::File, bytes: &[u8], file_length: u64) -> Result<()> {
    let magic = checked_bytes(bytes, 0, 4, "Mach-O magic")?;
    if matches!(magic, [0xcf, 0xfa, 0xed, 0xfe] | [0xfe, 0xed, 0xfa, 0xcf]) {
        return validate_thin_macho(file, bytes, 0, file_length, file_length, None);
    }
    let (order, fat64) = match magic {
        [0xca, 0xfe, 0xba, 0xbe] => (ByteOrder::Big, false),
        [0xbe, 0xba, 0xfe, 0xca] => (ByteOrder::Little, false),
        [0xca, 0xfe, 0xba, 0xbf] => (ByteOrder::Big, true),
        [0xbf, 0xba, 0xfe, 0xca] => (ByteOrder::Little, true),
        _ => bail!("native executable is not a Mach-O binary"),
    };
    let architecture_count = read_u32(bytes, 4, order, "fat Mach-O architecture count")?;
    if architecture_count == 0 || architecture_count > 64 {
        bail!("fat Mach-O architecture count is outside the security limit")
    }
    let entry_size = if fat64 { 32_usize } else { 20_usize };
    let table_end = 8_usize
        .checked_add(
            usize::try_from(architecture_count)
                .context("fat Mach-O architecture count is too large")?
                .checked_mul(entry_size)
                .context("fat Mach-O table length overflowed")?,
        )
        .context("fat Mach-O table boundary overflowed")?;
    let table = read_file_range(
        file,
        0,
        table_end as u64,
        file_length,
        "fat Mach-O architecture table",
    )?;
    let expected_cpu = native_macho_cpu()?;
    let preferred_subtype = native_macho_preferred_subtype()?;
    let mut native_slices = Vec::new();
    let mut slice_ranges = Vec::with_capacity(
        usize::try_from(architecture_count).expect("bounded architecture count"),
    );
    for index in 0..usize::try_from(architecture_count).expect("bounded architecture count") {
        let entry = 8 + index * entry_size;
        let cpu = read_u32(&table, entry, order, "fat Mach-O CPU type")?;
        let subtype = read_u32(&table, entry + 4, order, "fat Mach-O CPU subtype")?;
        let (offset, size, alignment) = if fat64 {
            (
                read_u64(&table, entry + 8, order, "fat Mach-O slice offset")?,
                read_u64(&table, entry + 16, order, "fat Mach-O slice size")?,
                read_u32(&table, entry + 24, order, "fat Mach-O slice alignment")?,
            )
        } else {
            (
                u64::from(read_u32(
                    &table,
                    entry + 8,
                    order,
                    "fat Mach-O slice offset",
                )?),
                u64::from(read_u32(
                    &table,
                    entry + 12,
                    order,
                    "fat Mach-O slice size",
                )?),
                read_u32(&table, entry + 16, order, "fat Mach-O slice alignment")?,
            )
        };
        let end = offset
            .checked_add(size)
            .context("fat Mach-O slice boundary overflowed")?;
        if offset < table_end as u64 || size < 32 || end > file_length || alignment > 63 {
            bail!("fat Mach-O slice is outside the executable boundary")
        }
        let alignment_bytes = 1_u64
            .checked_shl(alignment)
            .context("fat Mach-O slice alignment overflowed")?;
        if offset % alignment_bytes != 0 {
            bail!("fat Mach-O slice offset violates its declared alignment")
        }
        slice_ranges.push((offset, end));
        if cpu == expected_cpu {
            let masked_subtype = subtype & 0x00ff_ffff;
            if native_slices
                .iter()
                .any(|(_, existing, _, _)| *existing == subtype)
            {
                bail!("fat Mach-O contains duplicate native CPU subtype slices")
            }
            let compatibility = if subtype == preferred_subtype {
                0_u8
            } else if masked_subtype == preferred_subtype {
                1
            } else {
                2
            };
            native_slices.push((compatibility, subtype, offset, size));
        }
    }
    slice_ranges.sort_unstable_by_key(|(offset, _)| *offset);
    if slice_ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        bail!("fat Mach-O architecture slices overlap")
    }
    let best_compatibility = native_slices
        .iter()
        .map(|(compatibility, _, _, _)| *compatibility)
        .min()
        .context("fat Mach-O contains no native architecture slice")?;
    let mut compatible = native_slices
        .iter()
        .filter(|(compatibility, _, _, _)| *compatibility == best_compatibility);
    let (_, subtype, offset, size) = *compatible
        .next()
        .context("fat Mach-O contains no compatible native architecture slice")?;
    if compatible.next().is_some() {
        bail!("fat Mach-O has no unambiguous compatible CPU subtype slice")
    }
    let header = read_file_range(
        file,
        offset,
        32,
        file_length,
        "native fat Mach-O slice header",
    )?;
    validate_thin_macho(file, &header, offset, size, file_length, Some(subtype))
}

#[cfg(target_os = "macos")]
fn validate_thin_macho(
    file: &mut fs::File,
    header: &[u8],
    slice_offset: u64,
    slice_length: u64,
    file_length: u64,
    expected_subtype: Option<u32>,
) -> Result<()> {
    const MACHO64_HEADER_SIZE: usize = 32;
    const LC_UNIXTHREAD: u32 = 0x5;
    const LC_SEGMENT_64: u32 = 0x19;
    const LC_MAIN: u32 = 0x8000_0028;
    let magic = checked_bytes(header, 0, 4, "Mach-O header")?;
    let order = match magic {
        [0xcf, 0xfa, 0xed, 0xfe] if cfg!(target_endian = "little") => ByteOrder::Little,
        [0xfe, 0xed, 0xfa, 0xcf] if cfg!(target_endian = "big") => ByteOrder::Big,
        _ => bail!("Mach-O executable byte order or class does not match the host"),
    };
    let header = checked_bytes(header, 0, MACHO64_HEADER_SIZE, "Mach-O 64-bit header")?;
    if usize::BITS != 64 || read_u32(header, 4, order, "Mach-O CPU type")? != native_macho_cpu()? {
        bail!("Mach-O executable architecture does not match the host")
    }
    let subtype = read_u32(header, 8, order, "Mach-O CPU subtype")?;
    if expected_subtype.is_some_and(|expected| subtype != expected) {
        bail!("fat Mach-O CPU subtype does not match its native slice header")
    }
    if read_u32(header, 12, order, "Mach-O file type")? != 2 {
        bail!("Mach-O binary is not an executable image")
    }
    let command_count = read_u32(header, 16, order, "Mach-O load command count")?;
    let command_bytes = read_u32(header, 20, order, "Mach-O load command bytes")?;
    if command_count == 0 || command_count > 4_096 || command_bytes == 0 {
        bail!("Mach-O load command table shape is invalid")
    }
    let table_length = u64::from(command_bytes);
    if (MACHO64_HEADER_SIZE as u64)
        .checked_add(table_length)
        .is_none_or(|end| end > slice_length)
    {
        bail!("Mach-O load command table exceeds its executable slice")
    }
    let command_offset = slice_offset
        .checked_add(MACHO64_HEADER_SIZE as u64)
        .context("Mach-O load command offset overflowed")?;
    let commands = read_file_range(
        file,
        command_offset,
        table_length,
        file_length,
        "Mach-O load command table",
    )?;
    let table_end = commands.len();
    let mut cursor = 0_usize;
    let mut executable_ranges = Vec::new();
    let mut main_entry = None;
    for _ in 0..command_count {
        if cursor.checked_add(8).is_none_or(|end| end > table_end) {
            bail!("Mach-O load command header exceeds the declared table")
        }
        let command = read_u32(&commands, cursor, order, "Mach-O load command")?;
        let command_size = usize::try_from(read_u32(
            &commands,
            cursor + 4,
            order,
            "Mach-O load command size",
        )?)
        .context("Mach-O load command is too large")?;
        if command_size < 8 || command_size % 8 != 0 {
            bail!("Mach-O load command size is invalid")
        }
        let command_end = cursor
            .checked_add(command_size)
            .context("Mach-O load command boundary overflowed")?;
        if command_end > table_end {
            bail!("Mach-O load command exceeds the declared table")
        }
        if command == LC_SEGMENT_64 {
            if command_size < 72 {
                bail!("Mach-O segment command is truncated")
            }
            let section_count = usize::try_from(read_u32(
                &commands,
                cursor + 64,
                order,
                "Mach-O section count",
            )?)
            .context("Mach-O section count is too large")?;
            let expected_size = 72_usize
                .checked_add(
                    section_count
                        .checked_mul(80)
                        .context("Mach-O section table length overflowed")?,
                )
                .context("Mach-O segment command length overflowed")?;
            if expected_size != command_size {
                bail!("Mach-O segment command does not match its section table")
            }
            let file_offset = read_u64(&commands, cursor + 40, order, "Mach-O segment offset")?;
            let file_size = read_u64(&commands, cursor + 48, order, "Mach-O segment file size")?;
            let initial_protection =
                read_u32(&commands, cursor + 60, order, "Mach-O segment protection")?;
            if file_offset
                .checked_add(file_size)
                .is_none_or(|end| end > slice_length)
            {
                bail!("Mach-O segment exceeds its executable slice")
            }
            if file_size > 0 && initial_protection & 4 != 0 {
                executable_ranges.push((file_offset, file_offset + file_size));
            }
        } else if command == LC_MAIN {
            if command_size != 24 || main_entry.is_some() {
                bail!("Mach-O main entry command is invalid or duplicated")
            }
            main_entry = Some(read_u64(
                &commands,
                cursor + 8,
                order,
                "Mach-O main entry offset",
            )?);
        } else if command == LC_UNIXTHREAD {
            bail!("legacy Mach-O Unix thread entry commands are not supported")
        }
        cursor = command_end;
    }
    let main_entry_is_executable = main_entry.is_some_and(|entry| {
        executable_ranges
            .iter()
            .any(|(start, end)| entry >= *start && entry < *end)
    });
    if cursor != table_end || executable_ranges.is_empty() {
        bail!("Mach-O executable load command table is incomplete")
    }
    if !main_entry_is_executable {
        bail!("Mach-O entry point is not contained in an executable segment")
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn native_macho_cpu() -> Result<u32> {
    match std::env::consts::ARCH {
        "aarch64" => Ok(0x0100_000c),
        "x86_64" => Ok(0x0100_0007),
        architecture => bail!("Mach-O executable validation does not support {architecture}"),
    }
}

#[cfg(target_os = "macos")]
fn native_macho_preferred_subtype() -> Result<u32> {
    match std::env::consts::ARCH {
        "aarch64" => Ok(0),
        "x86_64" => Ok(3),
        architecture => bail!("Mach-O executable validation does not support {architecture}"),
    }
}

#[cfg(windows)]
fn validate_pe_executable(file: &mut fs::File, bytes: &[u8], file_length: u64) -> Result<()> {
    const DOS_HEADER_SIZE: usize = 64;
    const COFF_HEADER_SIZE: usize = 20;
    const SECTION_HEADER_SIZE: usize = 40;
    const IMAGE_FILE_EXECUTABLE_IMAGE: u16 = 0x0002;
    const IMAGE_FILE_DLL: u16 = 0x2000;
    const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
    let dos = checked_bytes(bytes, 0, DOS_HEADER_SIZE, "PE DOS header")?;
    if &dos[..2] != b"MZ" {
        bail!("native executable is not a PE binary")
    }
    let pe_offset = u64::from(read_le_u32(dos, 0x3c, "PE header offset")?);
    if pe_offset < DOS_HEADER_SIZE as u64 {
        bail!("PE header overlaps the DOS header")
    }
    let coff_prefix = read_file_range(
        file,
        pe_offset,
        4 + COFF_HEADER_SIZE as u64,
        file_length,
        "PE signature and COFF header",
    )?;
    if checked_bytes(&coff_prefix, 0, 4, "PE signature")? != b"PE\0\0" {
        bail!("PE executable signature is invalid")
    }
    let coff = 4_usize;
    if read_le_u16(&coff_prefix, coff, "PE machine")? != native_pe_machine()? {
        bail!("PE executable architecture does not match the host")
    }
    let section_count = usize::from(read_le_u16(&coff_prefix, coff + 2, "PE section count")?);
    if section_count == 0 || section_count > 96 {
        bail!("PE section count is outside the security limit")
    }
    let optional_size = usize::from(read_le_u16(
        &coff_prefix,
        coff + 16,
        "PE optional header size",
    )?);
    let characteristics = read_le_u16(&coff_prefix, coff + 18, "PE characteristics")?;
    if characteristics & IMAGE_FILE_EXECUTABLE_IMAGE == 0 || characteristics & IMAGE_FILE_DLL != 0 {
        bail!("PE binary is not a standalone executable image")
    }
    let optional = 4_usize + COFF_HEADER_SIZE;
    let section_table = optional
        .checked_add(optional_size)
        .context("PE section table offset overflowed")?;
    let section_table_length = section_count
        .checked_mul(SECTION_HEADER_SIZE)
        .context("PE section table length overflowed")?;
    let header_length = section_table
        .checked_add(section_table_length)
        .context("PE section table boundary overflowed")?;
    let headers = read_file_range(
        file,
        pe_offset,
        u64::try_from(header_length).context("PE headers are too large")?,
        file_length,
        "PE headers",
    )?;
    if optional_size < 112 || read_le_u16(&headers, optional, "PE optional magic")? != 0x020b {
        bail!("PE executable is not a complete PE32+ image")
    }
    checked_bytes(&headers, optional, optional_size, "PE optional header")?;
    let entry_point = read_le_u32(&headers, optional + 16, "PE entry point")?;
    if entry_point == 0 || read_le_u32(&headers, optional + 56, "PE image size")? == 0 {
        bail!("PE executable image has no entry point or image size")
    }
    let size_of_headers = u64::from(read_le_u32(&headers, optional + 60, "PE header size")?);
    let absolute_header_end = pe_offset
        .checked_add(u64::try_from(header_length).context("PE headers are too large")?)
        .context("PE absolute header boundary overflowed")?;
    if absolute_header_end > size_of_headers || size_of_headers > file_length {
        bail!("PE headers exceed the executable boundary")
    }
    let mut executable_entry_section = false;
    for index in 0..section_count {
        let section = section_table + index * SECTION_HEADER_SIZE;
        let virtual_size = read_le_u32(&headers, section + 8, "PE section virtual size")?;
        let virtual_address = read_le_u32(&headers, section + 12, "PE section virtual address")?;
        let raw_size = u64::from(read_le_u32(&headers, section + 16, "PE section raw size")?);
        let raw_offset = u64::from(read_le_u32(
            &headers,
            section + 20,
            "PE section raw offset",
        )?);
        let section_characteristics =
            read_le_u32(&headers, section + 36, "PE section characteristics")?;
        if raw_size > 0
            && (raw_offset < size_of_headers
                || raw_offset
                    .checked_add(raw_size)
                    .is_none_or(|end| end > file_length))
        {
            bail!("PE section exceeds the executable boundary")
        }
        if raw_size > 0 && section_characteristics & IMAGE_SCN_MEM_EXECUTE != 0 {
            let raw_size = u32::try_from(raw_size)
                .context("PE section raw size exceeds the virtual address space")?;
            let virtual_end = virtual_address.checked_add(virtual_size.max(raw_size));
            if virtual_end.is_some_and(|end| entry_point >= virtual_address && entry_point < end) {
                executable_entry_section = true;
            }
        }
    }
    if !executable_entry_section {
        bail!("PE entry point is not contained in an executable file section")
    }
    Ok(())
}

#[cfg(windows)]
fn native_pe_machine() -> Result<u16> {
    match std::env::consts::ARCH {
        "x86_64" => Ok(0x8664),
        "aarch64" => Ok(0xaa64),
        architecture => bail!("PE executable validation does not support {architecture}"),
    }
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
    validate_unix_mode(
        metadata.gid(),
        metadata.mode(),
        metadata.is_dir(),
        executable,
    )?;
    Ok(())
}

#[cfg(unix)]
fn validate_unix_mode(group: u32, mode: u32, directory: bool, executable: bool) -> Result<()> {
    if mode & 0o002 != 0 {
        bail!("path is world-writable")
    }
    if mode & 0o020 != 0 && !(directory && unix_group_can_mutate_trusted_directory(group)) {
        bail!("path is group-writable by an untrusted group")
    }
    if executable && mode & 0o111 == 0 {
        bail!("executable target has no execute bit")
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn unix_group_can_mutate_trusted_directory(group: u32) -> bool {
    // Homebrew's standard prefix is current-user/root owned while its shared directories are
    // writable by macOS's privileged admin group. Treat that well-known group like the Windows
    // Administrators SID, but keep group-writable executable files and ordinary groups rejected.
    group == MACOS_ADMIN_GROUP_ID
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unix_group_can_mutate_trusted_directory(_group: u32) -> bool {
    false
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
        validate_windows_trusted_executable_acl, validate_windows_trusted_executable_path_identity,
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
    validate_windows_trusted_executable_path_identity(path, &file, directory)?;
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
    #[cfg(windows)]
    use super::resolve_native_from_current_path;
    use super::{
        WindowsAceMutationAction, classify_windows_ace_mutation, copy_native_executable_fixture,
        parse_windows_npm_codex_cmd_shim, validate_native_executable,
    };
    #[cfg(unix)]
    use super::{
        resolve_codex_command_from_path, resolve_from_path, sanitized_path, validate_absolute,
        validate_unix_mode,
    };
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    #[cfg(windows)]
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn native_executable_validation_accepts_platform_binary_and_rejects_invalid_formats() {
        let root = fixture_root("native-format-validation");
        #[cfg(unix)]
        make_safe_directory_chain(&root);
        let native = root.join(if cfg!(windows) {
            "native.exe"
        } else {
            "native"
        });
        copy_native_executable_fixture(&native).expect("native executable fixture should copy");
        validate_native_executable(&native).expect("current-platform executable should validate");

        let malformed = root.join(if cfg!(windows) {
            "malformed.exe"
        } else {
            "malformed"
        });
        fs::copy(&native, &malformed).expect("malformed native fixture should copy");
        corrupt_native_structure(&malformed);
        let malformed_error = validate_native_executable(&malformed)
            .expect_err("malformed native structure must fail closed");
        assert!(
            malformed_error
                .chain()
                .map(ToString::to_string)
                .any(|detail| detail.contains("table")
                    || detail.contains("boundary")
                    || detail.contains("security limit"))
        );

        let invalid_entry = root.join(if cfg!(windows) {
            "invalid-entry.exe"
        } else {
            "invalid-entry"
        });
        fs::copy(&native, &invalid_entry).expect("invalid entry fixture should copy");
        corrupt_native_entry_relationship(&invalid_entry);
        let entry_error = validate_native_executable(&invalid_entry)
            .expect_err("native entry outside executable code must fail closed");
        assert!(
            entry_error
                .chain()
                .map(ToString::to_string)
                .any(|detail| detail.contains("entry") || detail.contains("subtype"))
        );

        let truncated = root.join(if cfg!(windows) {
            "truncated.exe"
        } else {
            "truncated"
        });
        #[cfg(windows)]
        write_executable_bytes(&truncated, b"MZ");
        #[cfg(target_os = "macos")]
        write_executable_bytes(&truncated, b"\xcf\xfa\xed\xfe");
        #[cfg(all(unix, not(target_os = "macos")))]
        write_executable_bytes(&truncated, b"\x7fELF");
        let truncated_error = validate_native_executable(&truncated)
            .expect_err("magic-only executable must fail closed");
        assert!(
            truncated_error
                .chain()
                .map(ToString::to_string)
                .any(|detail| detail.contains("truncated"))
        );

        let foreign = root.join(if cfg!(windows) {
            "foreign.exe"
        } else {
            "foreign"
        });
        let mut foreign_header = vec![0_u8; 64];
        #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
        foreign_header[..4].copy_from_slice(b"\xcf\xfa\xed\xfe");
        #[cfg(target_os = "macos")]
        foreign_header[..4].copy_from_slice(b"\x7fELF");
        write_executable_bytes(&foreign, &foreign_header);
        let foreign_error = validate_native_executable(&foreign)
            .expect_err("foreign executable format must fail closed");
        assert!(
            foreign_error
                .chain()
                .map(ToString::to_string)
                .any(|detail| detail.contains("not a"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn native_unix_mode_policy_limits_group_writes_to_privileged_macos_directories() {
        assert!(validate_unix_mode(20, 0o755, true, false).is_ok());
        assert!(validate_unix_mode(20, 0o755, false, true).is_ok());
        assert!(validate_unix_mode(20, 0o777, true, false).is_err());
        assert!(validate_unix_mode(20, 0o775, false, true).is_err());
        assert!(validate_unix_mode(20, 0o644, false, true).is_err());

        #[cfg(target_os = "macos")]
        {
            assert!(validate_unix_mode(super::MACOS_ADMIN_GROUP_ID, 0o775, true, false).is_ok());
            assert!(validate_unix_mode(super::MACOS_ADMIN_GROUP_ID, 0o775, false, true).is_err());
        }
        #[cfg(not(target_os = "macos"))]
        assert!(validate_unix_mode(80, 0o775, true, false).is_err());
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
        write_executable(&launcher, "#!/usr/bin/env node\nprocess.exit(0);\n");
        write_native_executable(&node);
        symlink(&launcher, bin.join("codex")).expect("Codex launcher should be linked");
        symlink(&node, bin.join("node")).expect("Node should be linked");

        for directory in [bin.as_path(), library.as_path(), cellar.as_path()] {
            std::os::unix::fs::chown(directory, None, Some(super::MACOS_ADMIN_GROUP_ID))
                .expect("Homebrew directory should use the macOS admin group");
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

    #[test]
    fn native_host_git_and_node_formats_are_accepted() {
        let root = fixture_root("native-host-tools");
        #[cfg(unix)]
        make_safe_directory_chain(&root);
        for (index, program) in ["git", "node"].into_iter().enumerate() {
            #[cfg(windows)]
            let source = resolve_native_from_current_path(
                program,
                &std::env::current_dir().expect("test current directory should resolve"),
            )
            .unwrap_or_else(|error| {
                panic!("installed native {program} should resolve safely: {error:#}")
            });
            #[cfg(not(windows))]
            let source = find_host_tool(program)
                .unwrap_or_else(|| panic!("test host should provide native {program}"));
            #[cfg(windows)]
            assert!(
                Command::new(&source)
                    .arg("--version")
                    .output()
                    .is_ok_and(|output| output.status.success()),
                "resolved native {program} should execute"
            );
            let destination = root.join(if cfg!(windows) {
                format!("tool-{index}.exe")
            } else {
                format!("tool-{index}")
            });
            fs::copy(&source, &destination).unwrap_or_else(|error| {
                panic!(
                    "host tool {} should copy to fixture: {error}",
                    source.display()
                )
            });
            #[cfg(unix)]
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))
                .expect("host tool fixture should become executable");
            validate_native_executable(&destination)
                .unwrap_or_else(|error| panic!("native {program} should validate: {error:#}"));
            #[cfg(windows)]
            {
                let hardlink = root.join(format!("tool-{index}-hardlink.exe"));
                fs::hard_link(&destination, &hardlink)
                    .expect("trusted executable hardlink fixture should create");
                validate_native_executable(&hardlink).unwrap_or_else(|error| {
                    panic!("hard-linked native {program} should validate: {error:#}")
                });
            }
        }
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
        assert_eq!(
            plan.prefix_args,
            vec![
                fs::canonicalize(target)
                    .expect("Codex package target should canonicalize")
                    .into_os_string()
            ]
        );
        let _ = fs::remove_dir_all(root);
    }

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
            WindowsAceMutationAction::Ignore,
            "creating a sibling cannot replace an existing executable path"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0004, true),
            WindowsAceMutationAction::Ignore,
            "creating a sibling directory cannot replace an existing path component"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x4000_0000, true),
            WindowsAceMutationAction::Ignore,
            "generic directory write does not grant delete-child or ACL replacement"
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0000_0040, true),
            WindowsAceMutationAction::InspectStandardAllowSid
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x0004_0000, true),
            WindowsAceMutationAction::InspectStandardAllowSid
        );
        assert_eq!(
            classify_windows_ace_mutation(0, 0x1000_0000, true),
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
        copy_native_executable_fixture(path).expect("native executable fixture should copy");
    }

    fn write_executable_bytes(path: &Path, body: &[u8]) {
        fs::write(path, body).expect("executable byte fixture should write");
        #[cfg(unix)]
        {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .expect("executable byte fixture should become executable");
        }
    }

    #[cfg(not(windows))]
    fn find_host_tool(program: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).find_map(|directory| {
            super::executable_names(program)
                .into_iter()
                .map(|name| directory.join(name))
                .find(|candidate| candidate.is_file())
                .and_then(|candidate| fs::canonicalize(candidate).ok())
        })
    }

    fn corrupt_native_structure(path: &Path) {
        let mut bytes = fs::read(path).expect("native fixture should remain readable");
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let invalid_offset = u64::try_from(bytes.len()).expect("fixture length should fit u64");
            let encoded = if cfg!(target_endian = "little") {
                invalid_offset.to_le_bytes()
            } else {
                invalid_offset.to_be_bytes()
            };
            bytes[32..40].copy_from_slice(&encoded);
        }
        #[cfg(target_os = "macos")]
        match &bytes[..4] {
            [0xcf, 0xfa, 0xed, 0xfe] => bytes[20..24].copy_from_slice(&u32::MAX.to_le_bytes()),
            [0xfe, 0xed, 0xfa, 0xcf] => bytes[20..24].copy_from_slice(&u32::MAX.to_be_bytes()),
            [0xca, 0xfe, 0xba, 0xbe] | [0xca, 0xfe, 0xba, 0xbf] => {
                bytes[4..8].copy_from_slice(&65_u32.to_be_bytes());
            }
            [0xbe, 0xba, 0xfe, 0xca] | [0xbf, 0xba, 0xfe, 0xca] => {
                bytes[4..8].copy_from_slice(&65_u32.to_le_bytes());
            }
            magic => panic!("unexpected native Mach-O magic: {magic:?}"),
        }
        #[cfg(windows)]
        bytes[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        write_executable_bytes(path, &bytes);
    }

    fn corrupt_native_entry_relationship(path: &Path) {
        let mut bytes = fs::read(path).expect("native fixture should remain readable");
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let encoded = if cfg!(target_endian = "little") {
                u64::MAX.to_le_bytes()
            } else {
                u64::MAX.to_be_bytes()
            };
            bytes[24..32].copy_from_slice(&encoded);
        }
        #[cfg(target_os = "macos")]
        corrupt_macho_entry_or_subtype(&mut bytes);
        #[cfg(windows)]
        {
            let pe_offset = usize::try_from(u32::from_le_bytes(
                bytes[0x3c..0x40]
                    .try_into()
                    .expect("PE header offset bytes should exist"),
            ))
            .expect("PE header offset should fit usize");
            let entry_offset = pe_offset + 4 + 20 + 16;
            bytes[entry_offset..entry_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        write_executable_bytes(path, &bytes);
    }

    #[cfg(target_os = "macos")]
    fn corrupt_macho_entry_or_subtype(bytes: &mut [u8]) {
        let magic: [u8; 4] = bytes[..4].try_into().expect("Mach-O magic should exist");
        match magic {
            [0xcf, 0xfa, 0xed, 0xfe] => corrupt_thin_macho_main_entry(bytes, 0, true),
            [0xfe, 0xed, 0xfa, 0xcf] => corrupt_thin_macho_main_entry(bytes, 0, false),
            [0xca, 0xfe, 0xba, 0xbe]
            | [0xca, 0xfe, 0xba, 0xbf]
            | [0xbe, 0xba, 0xfe, 0xca]
            | [0xbf, 0xba, 0xfe, 0xca] => {
                let little = matches!(magic, [0xbe, 0xba, 0xfe, 0xca] | [0xbf, 0xba, 0xfe, 0xca]);
                let fat64 = matches!(magic, [0xca, 0xfe, 0xba, 0xbf] | [0xbf, 0xba, 0xfe, 0xca]);
                let count = read_test_u32(bytes, 4, little);
                let entry_size = if fat64 { 32 } else { 20 };
                let expected_cpu = if cfg!(target_arch = "aarch64") {
                    0x0100_000c
                } else {
                    0x0100_0007
                };
                let mut changed = false;
                for index in 0..usize::try_from(count).expect("fat count should fit usize") {
                    let entry = 8 + index * entry_size;
                    if read_test_u32(bytes, entry, little) != expected_cpu {
                        continue;
                    }
                    let subtype = read_test_u32(bytes, entry + 4, little);
                    let offset = if fat64 {
                        read_test_u64(bytes, entry + 8, little)
                    } else {
                        u64::from(read_test_u32(bytes, entry + 8, little))
                    };
                    let offset =
                        usize::try_from(offset).expect("fat slice offset should fit usize");
                    let inner_little = match &bytes[offset..offset + 4] {
                        [0xcf, 0xfa, 0xed, 0xfe] => true,
                        [0xfe, 0xed, 0xfa, 0xcf] => false,
                        value => panic!("unexpected thin Mach-O magic: {value:?}"),
                    };
                    write_test_u32(bytes, offset + 8, subtype ^ 1, inner_little);
                    changed = true;
                }
                assert!(changed, "fat Mach-O fixture should contain a native slice");
            }
            value => panic!("unexpected native Mach-O magic: {value:?}"),
        }
    }

    #[cfg(target_os = "macos")]
    fn corrupt_thin_macho_main_entry(bytes: &mut [u8], base: usize, little: bool) {
        let command_count = read_test_u32(bytes, base + 16, little);
        let mut cursor = base + 32;
        for _ in 0..command_count {
            let command = read_test_u32(bytes, cursor, little);
            let command_size = usize::try_from(read_test_u32(bytes, cursor + 4, little))
                .expect("Mach-O command size should fit usize");
            if command == 0x8000_0028 {
                let encoded = if little {
                    u64::MAX.to_le_bytes()
                } else {
                    u64::MAX.to_be_bytes()
                };
                bytes[cursor + 8..cursor + 16].copy_from_slice(&encoded);
                return;
            }
            cursor += command_size;
        }
        panic!("thin Mach-O fixture should contain LC_MAIN");
    }

    #[cfg(target_os = "macos")]
    fn read_test_u32(bytes: &[u8], offset: usize, little: bool) -> u32 {
        let value = bytes[offset..offset + 4]
            .try_into()
            .expect("test u32 bytes should exist");
        if little {
            u32::from_le_bytes(value)
        } else {
            u32::from_be_bytes(value)
        }
    }

    #[cfg(target_os = "macos")]
    fn read_test_u64(bytes: &[u8], offset: usize, little: bool) -> u64 {
        let value = bytes[offset..offset + 8]
            .try_into()
            .expect("test u64 bytes should exist");
        if little {
            u64::from_le_bytes(value)
        } else {
            u64::from_be_bytes(value)
        }
    }

    #[cfg(target_os = "macos")]
    fn write_test_u32(bytes: &mut [u8], offset: usize, value: u32, little: bool) {
        let encoded = if little {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        bytes[offset..offset + 4].copy_from_slice(&encoded);
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
