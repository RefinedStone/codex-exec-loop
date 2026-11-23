//! Read-only filesystem admission precedes every operation that may create an object.
use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_NAME_NORMALIZED, GetDriveTypeW, GetFinalPathNameByHandleW, GetVolumeInformationByHandleW,
    VOLUME_NAME_GUID,
};
use windows_sys::Win32::System::SystemServices::{FILE_PERSISTENT_ACLS, FILE_READ_ONLY_VOLUME};
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};

use crate::private_sqlite::PinnedPrivateDirectory;

#[derive(Debug, PartialEq, Eq)]
struct LocalVolume {
    guid: String,
    serial: u32,
}

pub(super) fn prepare_writer_root(root: &Path) -> Result<()> {
    validate_local_path(root)?;
    let ancestor = existing_ancestor(root)?;
    let volume = local_ntfs_volume(ancestor.handle())?;
    ancestor.verify()?;
    // Every previously missing directory is created with an owner-only descriptor.
    // PinnedPrivateDirectory refuses unsafe existing ACLs instead of repairing them.
    let prepared = PinnedPrivateDirectory::open(root, true)?;
    if local_ntfs_volume(prepared.handle())? != volume {
        bail!("the vault volume changed while preparing its private root");
    }
    ancestor.verify()?;
    prepared.verify()
}

pub(super) fn open_root(root: &Path) -> Result<Option<PinnedPrivateDirectory>> {
    validate_local_path(root)?;
    match PinnedPrivateDirectory::open(root, false) {
        Ok(directory) => {
            local_ntfs_volume(directory.handle())?;
            directory.verify()?;
            Ok(Some(directory))
        }
        Err(error) if is_not_found(&error) => {
            let ancestor = existing_ancestor(root)?;
            local_ntfs_volume(ancestor.handle())?;
            ancestor.verify()?;
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn open_records(
    root: &PinnedPrivateDirectory,
    create: bool,
) -> Result<Option<PinnedPrivateDirectory>> {
    root.verify()?;
    let directory = match root.child("records", create) {
        Ok(directory) => directory,
        Err(error) if !create && is_not_found(&error) => {
            root.verify()?;
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if local_ntfs_volume(directory.handle())? != local_ntfs_volume(root.handle())? {
        bail!("vault records must remain on the admitted local NTFS volume");
    }
    root.verify()?;
    directory.verify()?;
    Ok(Some(directory))
}

pub(super) fn absolute_destination(parent: &PinnedPrivateDirectory, name: &str) -> Result<PathBuf> {
    parent.file_path(name)?;
    parent.verify()?;
    let destination = PathBuf::from(final_volume_path(parent.handle())?).join(name);
    parent.verify()?;
    Ok(destination)
}

fn validate_local_path(root: &Path) -> Result<()> {
    if !root.is_absolute()
        || !matches!(
            root.components().next(),
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        )
    {
        bail!("the vault requires an absolute local disk path");
    }
    Ok(())
}

fn existing_ancestor(root: &Path) -> Result<PinnedPrivateDirectory> {
    let mut candidate = PathBuf::from(root);
    loop {
        match PinnedPrivateDirectory::pin_ancestors(&candidate) {
            Ok(directory) => return Ok(directory),
            Err(error) if is_not_found(&error) && candidate.pop() => {}
            Err(error) => return Err(error).context("the vault parent could not be pinned"),
        }
    }
}

fn local_ntfs_volume(file: &File) -> Result<LocalVolume> {
    #[cfg(test)]
    super::test_hooks::check_volume_admission()?;
    let mut name = [0_u16; 64];
    let mut flags = 0;
    let mut serial = 0;
    // SAFETY: the live directory handle and all output buffers remain valid.
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            &mut serial,
            std::ptr::null_mut(),
            &mut flags,
            name.as_mut_ptr(),
            name.len() as u32,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .context("the vault filesystem could not be verified");
    }
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    let name =
        String::from_utf16(&name[..length]).context("the vault filesystem name was invalid")?;
    if name != "NTFS" || flags & FILE_PERSISTENT_ACLS == 0 || flags & FILE_READ_ONLY_VOLUME != 0 {
        bail!("the vault requires writable local NTFS with persistent ACLs");
    }
    let guid = local_volume_guid(file)?;
    let wide: Vec<u16> = guid.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the volume GUID is a validated, NUL-terminated root path.
    let kind = unsafe { GetDriveTypeW(wide.as_ptr()) };
    if !matches!(kind, DRIVE_FIXED | DRIVE_REMOVABLE) {
        bail!("the vault volume is not a verified local disk");
    }
    Ok(LocalVolume { guid, serial })
}

fn local_volume_guid(file: &File) -> Result<String> {
    let path = final_volume_path(file)?;
    // final_volume_path has validated the complete ASCII volume GUID root.
    Ok(path[..49].to_ascii_lowercase())
}

fn final_volume_path(file: &File) -> Result<String> {
    let mut path = vec![0_u16; 32_768];
    // SAFETY: the live file handle and bounded writable UTF-16 buffer are valid.
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            path.as_mut_ptr(),
            path.len() as u32,
            FILE_NAME_NORMALIZED | VOLUME_NAME_GUID,
        )
    };
    if length == 0 {
        return Err(std::io::Error::last_os_error())
            .context("the vault local volume identity could not be verified");
    }
    if length as usize >= path.len() {
        bail!("the vault local volume identity exceeds its limit");
    }
    let path = String::from_utf16(&path[..length as usize])
        .context("the vault volume path was invalid")?;
    let guid = path
        .get(..49)
        .filter(|value| value.starts_with(r"\\?\Volume{") && value.ends_with("}\\"))
        .context("the vault requires a local volume GUID")?;
    if !guid.as_bytes()[11..47]
        .iter()
        .enumerate()
        .all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        bail!("the vault volume GUID was invalid");
    }
    Ok(path)
}

pub(super) fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|cause| cause.kind() == std::io::ErrorKind::NotFound)
}
