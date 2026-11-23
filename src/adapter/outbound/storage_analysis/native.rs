use crate::domain::storage_analysis::{StorageIssue, StorageVolume};
use anyhow::{Result, bail};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

pub(super) struct NativeMetadata {
    pub logical: u64,
    pub allocated: Option<u64>,
    pub identity: Option<String>,
    pub volume: String,
    pub directory: bool,
    pub issue: Option<StorageIssue>,
    // A directory handle disallows its replacement while enumerating it on Windows.
    pub _handle: Option<File>,
}

pub(super) enum RootVolumeBinding {
    SelectedVolume,
    // This mode is supplied only by the explicitly constructed test adapter.
    #[cfg(test)]
    Fixture,
}

pub(super) fn verify_root_volume(
    root: &NativeMetadata,
    requested_volume: &str,
    binding: RootVolumeBinding,
) -> Result<()> {
    match binding {
        #[cfg(test)]
        RootVolumeBinding::Fixture => {
            if requested_volume != "fixture-volume" {
                bail!("storage_volume_unverified");
            }
            Ok(())
        }
        RootVolumeBinding::SelectedVolume => {
            if volume_guid_root(requested_volume) != Some(requested_volume) {
                bail!("storage_volume_unverified");
            }
            #[cfg(windows)]
            {
                // Bind the request to the same open object that supplied its serial/file ID.
                // Re-querying a drive letter here would introduce a second replacement race.
                let observed = root_volume_id(root)?;
                if !observed.eq_ignore_ascii_case(requested_volume) {
                    bail!("storage_volume_changed");
                }
                Ok(())
            }
            #[cfg(not(windows))]
            {
                let _ = root;
                bail!("storage_unsupported_os");
            }
        }
    }
}

pub(super) fn volume_guid_root(path: &str) -> Option<&str> {
    let root = path.get(..49)?;
    if !root.get(..11)?.eq_ignore_ascii_case(r"\\?\Volume{")
        || root.as_bytes()[47] != b'}'
        || root.as_bytes()[48] != b'\\'
    {
        return None;
    }
    let guid = root.get(11..47)?;
    if !guid.bytes().enumerate().all(|(i, byte)| {
        if matches!(i, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    }) {
        return None;
    }
    Some(root)
}

#[cfg(windows)]
pub(super) fn root_volume_id(root: &NativeMetadata) -> Result<String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_GUID,
    };
    let handle = root
        ._handle
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("storage_volume_unverified"))?;
    let mut name = vec![0_u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            handle.as_raw_handle(),
            name.as_mut_ptr(),
            name.len() as u32,
            FILE_NAME_NORMALIZED | VOLUME_NAME_GUID,
        )
    };
    if length == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    if length as usize >= name.len() {
        bail!("storage_volume_unverified");
    }
    let path = String::from_utf16_lossy(&name[..length as usize]);
    volume_guid_root(&path)
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("storage_volume_unverified"))
}

pub(super) fn list_volumes() -> Result<Vec<StorageVolume>> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
        use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};
        let drives = unsafe { GetLogicalDrives() };
        let mut volumes = Vec::new();
        for letter in b'A'..=b'Z' {
            if drives & (1 << (letter - b'A')) == 0 {
                continue;
            }
            let path = PathBuf::from(format!("{}:\\", letter as char));
            let wide = wide(&path);
            let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
            let supported = kind == DRIVE_FIXED || kind == DRIVE_REMOVABLE;
            let id = volume_id(&path).unwrap_or_else(|_| format!("unsupported-{}", letter as char));
            volumes.push(StorageVolume {
                id,
                label: path.to_string_lossy().into_owned(),
                root_path: path.to_string_lossy().into_owned(),
                kind: if kind == DRIVE_FIXED {
                    "local"
                } else if kind == DRIVE_REMOVABLE {
                    "removable"
                } else {
                    "unsupported"
                }
                .into(),
                supported,
            });
        }
        Ok(volumes)
    }
    #[cfg(not(windows))]
    {
        Ok(vec![])
    }
}
#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
#[cfg(windows)]
fn volume_id(path: &Path) -> Result<String> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
    let mut buffer = [0_u16; 512];
    let input = wide(path);
    if unsafe {
        GetVolumeNameForVolumeMountPointW(input.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32)
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(String::from_utf16_lossy(
        &buffer[..buffer.iter().position(|v| *v == 0).unwrap_or(buffer.len())],
    ))
}
#[cfg(test)]
pub(super) fn fixture_volume(root: &Path) -> Result<StorageVolume> {
    let root = fs::canonicalize(root)?;
    Ok(StorageVolume {
        id: "fixture-volume".into(),
        label: "fixture".into(),
        root_path: root.to_string_lossy().into_owned(),
        kind: "fixture".into(),
        supported: true,
    })
}

pub(super) fn metadata(
    path: &Path,
    root: &Path,
    expected_volume: Option<&str>,
) -> Result<NativeMetadata> {
    let entry = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::{
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        };
        use windows_sys::Win32::Storage::FileSystem::*;
        let attributes = entry.file_attributes();
        if attributes
            & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_OFFLINE | 0x00400000 | 0x00040000)
            != 0
        {
            return Ok(NativeMetadata {
                logical: entry.len(),
                allocated: None,
                identity: None,
                volume: String::new(),
                directory: attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                issue: Some(StorageIssue::one(
                    "reparse_or_placeholder",
                    "Reparse points and offline placeholders are not followed or hydrated",
                )),
                _handle: None,
            });
        }
        let handle = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let raw = handle.as_raw_handle();
        let mut standard = std::mem::MaybeUninit::<FILE_STANDARD_INFO>::uninit();
        let standard_known = unsafe {
            GetFileInformationByHandleEx(
                raw,
                FileStandardInfo,
                standard.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        } != 0;
        let standard = standard_known.then(|| unsafe { standard.assume_init() });
        let current = handle.metadata()?;
        if current.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            bail!("storage_reparse_changed");
        }
        let mut file_id = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
        let identified = unsafe {
            GetFileInformationByHandleEx(
                raw,
                FileIdInfo,
                file_id.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_ID_INFO>() as u32,
            )
        } != 0;
        let (volume, identity) = if identified {
            let info = unsafe { file_id.assume_init() };
            (
                format!("{:016x}", info.VolumeSerialNumber),
                Some(format!(
                    "{:016x}-{:02x?}",
                    info.VolumeSerialNumber, info.FileId.Identifier
                )),
            )
        } else {
            (String::new(), None)
        };
        if identified && expected_volume.is_some_and(|v| v != volume) {
            bail!("storage_volume_changed");
        }
        let mut final_name = vec![0_u16; 32768];
        let len = unsafe {
            GetFinalPathNameByHandleW(raw, final_name.as_mut_ptr(), final_name.len() as u32, 0)
        };
        if len == 0 || len as usize >= final_name.len() {
            bail!("storage_path_unverified");
        }
        let actual = PathBuf::from(String::from_utf16_lossy(&final_name[..len as usize]));
        if !actual.starts_with(root) {
            bail!("storage_outside_root");
        }
        let issue = if !identified {
            Some(StorageIssue::one(
                "identity_unknown",
                "File identity could not be verified; allocation is excluded from unique totals",
            ))
        } else if !standard_known {
            Some(StorageIssue::one(
                "allocation_unknown",
                "Allocation metadata was unavailable; logical size remains an observation",
            ))
        } else {
            None
        };
        Ok(NativeMetadata {
            logical: standard
                .as_ref()
                .and_then(|s| u64::try_from(s.EndOfFile).ok())
                .unwrap_or(entry.len()),
            allocated: standard
                .as_ref()
                .and_then(|s| u64::try_from(s.AllocationSize).ok()),
            identity,
            volume,
            directory: standard
                .as_ref()
                .map(|s| s.Directory)
                .unwrap_or(entry.is_dir()),
            issue,
            _handle: Some(handle),
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if entry.file_type().is_symlink() {
            return Ok(NativeMetadata {
                logical: entry.len(),
                allocated: None,
                identity: None,
                volume: String::new(),
                directory: false,
                issue: Some(StorageIssue::one(
                    "symlink",
                    "Symbolic links are not followed",
                )),
                _handle: None,
            });
        }
        let actual = fs::canonicalize(path)?;
        if !actual.starts_with(root) {
            bail!("storage_outside_root");
        }
        let volume = entry.dev().to_string();
        if expected_volume.is_some_and(|v| v != volume) {
            bail!("storage_volume_changed");
        }
        Ok(NativeMetadata {
            logical: if entry.is_dir() { 0 } else { entry.len() },
            allocated: entry.blocks().checked_mul(512),
            identity: Some(format!("{}:{}", entry.dev(), entry.ino())),
            volume,
            directory: entry.is_dir(),
            issue: None,
            _handle: None,
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (entry, root, expected_volume);
        Err(anyhow::anyhow!("storage_unsupported_os"))
    }
}
pub(super) fn classify_error(error: &anyhow::Error) -> StorageIssue {
    if error.to_string() == "storage_volume_changed" {
        return StorageIssue::one("volume_changed", "The selected volume identity changed");
    }
    if let Some(io) = error.downcast_ref::<std::io::Error>() {
        if matches!(io.raw_os_error(), Some(15 | 21 | 55 | 1167)) {
            return StorageIssue::one("drive_disconnected", "The selected device is unavailable");
        }
        let code = match io.kind() {
            std::io::ErrorKind::PermissionDenied => "access_denied",
            std::io::ErrorKind::NotFound => "disappeared",
            _ => "metadata_unavailable",
        };
        return StorageIssue::one(code, io.to_string());
    }
    StorageIssue::one("unverified_path", error.to_string())
}
