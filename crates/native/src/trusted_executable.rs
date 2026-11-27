//! Native executable format, identity and path trust checks.
//! Product launcher/configuration policy is supplied explicitly by the caller.

use anyhow::{Context, Result, bail};
use std::ffi::{OsStr, OsString};
use std::fs;
#[cfg(unix)]
use std::io::{BufRead, BufReader};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

mod fingerprint;
pub use fingerprint::PinnedNativeExecutable;
const MAX_NATIVE_EXECUTABLE_METADATA_BYTES: u64 = 16 * 1024 * 1024;
#[cfg(target_os = "macos")]
const MACOS_ADMIN_GROUP_ID: u32 = 80;

/// Permissive lookup is an explicit consumer policy; pinned native fingerprints always use strict validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutableTrustPolicy {
    AllowUntrusted,
    RequireTrusted,
}

#[cfg(windows)]
pub fn windows_process_argument_path(path: &Path) -> Result<OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    const EXTENDED_PREFIX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const EXTENDED_UNC_PREFIX: [u16; 8] = [
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];

    let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if let Some(suffix) = encoded.strip_prefix(&EXTENDED_UNC_PREFIX) {
        let mut normalized = vec![b'\\' as u16, b'\\' as u16];
        normalized.extend_from_slice(suffix);
        return Ok(OsString::from_wide(&normalized));
    }
    if let Some(suffix) = encoded.strip_prefix(&EXTENDED_PREFIX) {
        let drive = suffix.first().copied().unwrap_or_default();
        if suffix.len() < 3
            || !((b'A' as u16..=b'Z' as u16).contains(&drive)
                || (b'a' as u16..=b'z' as u16).contains(&drive))
            || suffix[1] != b':' as u16
            || suffix[2] != b'\\' as u16
        {
            bail!("Windows Codex npm target uses an unsupported extended path form")
        }
        return Ok(OsString::from_wide(suffix));
    }
    Ok(path.as_os_str().to_os_string())
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

#[cfg(windows)]
fn read_untrusted_bytes(path: &Path, maximum: usize, label: &str) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open {label} `{}`", path.display()))?;
    if !file
        .metadata()
        .with_context(|| format!("failed to inspect {label} `{}`", path.display()))?
        .is_file()
    {
        bail!("{label} is not a regular file")
    }
    let read_limit = maximum
        .checked_add(1)
        .context("file read limit overflowed")?;
    let mut bytes = Vec::with_capacity(read_limit);
    file.by_ref()
        .take(read_limit as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {label} from `{}`", path.display()))?;
    if bytes.len() > maximum {
        bail!("{label} exceeds the {maximum}-byte security limit")
    }
    Ok(bytes)
}

fn validate_native_executable_format(file: &mut fs::File, file_length: u64) -> Result<()> {
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
}

pub fn validate_native_executable(path: &Path) -> Result<()> {
    inspect_validated_file(path, validate_native_executable_format).with_context(|| {
        format!(
            "security-sensitive executable failed native format validation: {}",
            path.display()
        )
    })
}

fn validate_untrusted_native_executable(path: &Path) -> Result<()> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open native executable `{}`", path.display()))?;
    let metadata = file
        .metadata()
        .context("failed to inspect native executable")?;
    if !metadata.is_file() {
        bail!("native executable is not a regular file")
    }
    validate_native_executable_format(&mut file, metadata.len()).with_context(|| {
        format!(
            "executable failed native format validation: {}",
            path.display()
        )
    })
}

pub fn validate_native_executable_with_policy(
    path: &Path,
    policy: ExecutableTrustPolicy,
) -> Result<()> {
    match policy {
        ExecutableTrustPolicy::AllowUntrusted => validate_untrusted_native_executable(path),
        ExecutableTrustPolicy::RequireTrusted => validate_native_executable(path),
    }
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

pub fn resolve_from_path(program: &str, path: &OsStr, cwd: &Path) -> Result<PathBuf> {
    resolve_from_path_with_policy(program, path, cwd, ExecutableTrustPolicy::RequireTrusted)
}

fn resolve_from_path_with_policy(
    program: &str,
    path: &OsStr,
    cwd: &Path,
    policy: ExecutableTrustPolicy,
) -> Result<PathBuf> {
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
                    return validate_candidate_with_policy(&candidate, &untrusted_roots, policy)
                        .with_context(|| {
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
    let qualifier = if policy == ExecutableTrustPolicy::RequireTrusted {
        "trusted "
    } else {
        ""
    };
    bail!("`{program}` was not found on the {qualifier}absolute PATH")
}

pub fn sanitized_path(path: &OsStr, cwd: &Path) -> Result<OsString> {
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

pub fn validate_absolute(path: &Path, cwd: &Path) -> Result<PathBuf> {
    validate_absolute_with_policy(path, cwd, ExecutableTrustPolicy::RequireTrusted)
}

pub fn validate_absolute_with_policy(
    path: &Path,
    cwd: &Path,
    policy: ExecutableTrustPolicy,
) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!("pinned executable path must be absolute")
    }
    validate_candidate_with_policy(path, &untrusted_roots(cwd), policy)
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

#[cfg(any(unix, test))]
fn unix_executable_mode_is_set(mode: u32) -> bool {
    mode & 0o111 != 0
}

fn validate_candidate_with_policy(
    candidate: &Path,
    untrusted_roots: &[PathBuf],
    policy: ExecutableTrustPolicy,
) -> Result<PathBuf> {
    if policy == ExecutableTrustPolicy::RequireTrusted {
        return validate_candidate(candidate, untrusted_roots);
    }
    let canonical = fs::canonicalize(candidate)
        .with_context(|| format!("failed to canonicalize `{}`", candidate.display()))?;
    let metadata = fs::metadata(&canonical)
        .with_context(|| format!("failed to inspect executable `{}`", canonical.display()))?;
    if !metadata.is_file() {
        bail!("pinned executable target is not a regular file")
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if !unix_executable_mode_is_set(metadata.permissions().mode()) {
            bail!("pinned executable target is not executable")
        }
    }
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
    // This private workspace crate lives at crates/native. Keep the original
    // workspace source root excluded when caller cwd is outside the checkout.
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    push_root(&mut roots, &source_root);

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
pub fn executable_names(program: &str) -> Vec<String> {
    [".exe", ".cmd", "", ".bat"]
        .into_iter()
        .map(|suffix| format!("{program}{suffix}"))
        .collect()
}

#[cfg(not(windows))]
pub fn executable_names(program: &str) -> Vec<String> {
    vec![program.to_string()]
}

pub fn resolve_native_from_current_path(program: &str, cwd: &Path) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH is unavailable")?;
    resolve_native_from_path(program, &path, cwd)
}

pub fn resolve_native_from_path(program: &str, path: &OsStr, cwd: &Path) -> Result<PathBuf> {
    let executable = resolve_from_path(program, path, cwd)?;
    validate_native_executable(&executable)?;
    Ok(executable)
}

pub fn resolve_native_from_path_with_policy(
    program: &str,
    path: &OsStr,
    cwd: &Path,
    policy: ExecutableTrustPolicy,
) -> Result<PathBuf> {
    if policy == ExecutableTrustPolicy::RequireTrusted {
        return resolve_native_from_path(program, path, cwd);
    }
    let executable = resolve_from_path_with_policy(program, path, cwd, policy)?;
    validate_native_executable_with_policy(&executable, policy)?;
    Ok(executable)
}

#[cfg(unix)]
fn read_validated_first_line(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    let (file, identity) = open_validated_file(path)?;
    let read_limit = maximum
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
    if line.len() > maximum {
        bail!("Codex launcher first line exceeds the security limit")
    }
    line.pop();
    finish_validated_file_read(path, &file, identity)?;
    Ok(line)
}

#[cfg(unix)]
fn read_untrusted_first_line(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open Codex launcher `{}`", path.display()))?;
    if !file
        .metadata()
        .context("failed to inspect Codex launcher")?
        .is_file()
    {
        bail!("Codex launcher is not a regular file")
    }
    let read_limit = maximum
        .checked_add(1)
        .context("Codex shebang read limit overflowed")?;
    let mut reader = BufReader::new(file.take(read_limit as u64));
    let mut line = Vec::with_capacity(128);
    let read = reader
        .read_until(b'\n', &mut line)
        .with_context(|| format!("failed to read Codex launcher `{}`", path.display()))?;
    if read == 0 || !line.ends_with(b"\n") {
        bail!("Codex launcher first line is missing a bounded newline")
    }
    if line.len() > maximum {
        bail!("Codex launcher first line exceeds the security limit")
    }
    line.pop();
    Ok(line)
}

pub fn validate_trusted_configuration_path(
    value: &OsStr,
    cwd: &Path,
    directory: bool,
) -> Result<PathBuf> {
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

pub fn path_is_repository_controlled(path: &Path, cwd: &Path) -> bool {
    path_is_untrusted(path, &untrusted_roots(cwd))
}

#[cfg(windows)]
pub fn read_launcher_bytes(
    path: &Path,
    maximum: usize,
    label: &str,
    policy: ExecutableTrustPolicy,
) -> Result<Vec<u8>> {
    match policy {
        ExecutableTrustPolicy::AllowUntrusted => read_untrusted_bytes(path, maximum, label),
        ExecutableTrustPolicy::RequireTrusted => read_validated_bytes(path, maximum, label),
    }
}

#[cfg(unix)]
pub fn read_launcher_first_line(
    path: &Path,
    maximum: usize,
    policy: ExecutableTrustPolicy,
) -> Result<Vec<u8>> {
    match policy {
        ExecutableTrustPolicy::AllowUntrusted => read_untrusted_first_line(path, maximum),
        ExecutableTrustPolicy::RequireTrusted => read_validated_first_line(path, maximum),
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

#[cfg(test)]
mod tests;
