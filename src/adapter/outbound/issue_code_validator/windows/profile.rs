use std::fs;
use std::io;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::time::Duration;

use super::security::{Identity, wide};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeleteAppContainerProfile, GetAppContainerFolderPath,
};
use windows_sys::Win32::Security::{EqualSid, FreeSid};
use windows_sys::Win32::System::Com::CoTaskMemFree;

/// Owns only a fresh profile for this invocation. Existing profiles are never adopted or deleted.
pub(super) struct Profile {
    name: Vec<u16>,
    root: Option<PathBuf>,
    active: bool,
}
impl Profile {
    pub(super) fn create(identity: &Identity) -> io::Result<Self> {
        let name = wide(identity.name.as_ref())?;
        let mut sid = null_mut();
        // SAFETY: a unique random name, no capabilities, and a valid SID output.
        // Only S_OK establishes ownership; ERROR_ALREADY_EXISTS is a hard failure.
        let status = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                name.as_ptr(),
                name.as_ptr(),
                null(),
                0,
                &mut sid,
            )
        };
        if status != 0 {
            return Err(io::Error::other(
                "fresh validator profile could not be created",
            ));
        }
        let mut profile = Self {
            name,
            root: None,
            active: true,
        };
        // SAFETY: both SIDs are allocated by successful Windows calls.
        let same = !sid.is_null() && unsafe { EqualSid(identity.sid(), sid) } != 0;
        if !sid.is_null() {
            unsafe { FreeSid(sid) };
        }
        if !same {
            return Err(io::Error::other("validator profile identity differs"));
        }
        let folder = folder_path(&identity.sid_text)?;
        let expected = crate::private_fs::windows_local_app_data_path()
            .map_err(io::Error::other)?
            .join("Packages")
            .join(&identity.name);
        if folder.canonicalize()? != expected.join("AC").canonicalize()? {
            return Err(io::Error::other(
                "validator profile is outside its owned location",
            ));
        }
        profile.root = Some(expected);
        Ok(profile)
    }
    pub(super) fn cleanup(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        for attempt in 0..3 {
            // SAFETY: this exact random name was created by this instance with S_OK.
            let result = unsafe { DeleteAppContainerProfile(self.name.as_ptr()) };
            let removed = self.root.as_ref().is_none_or(|root| {
                matches!(fs::symlink_metadata(root), Err(error) if error.kind() == io::ErrorKind::NotFound)
            });
            if result == 0 && removed {
                self.active = false;
                return Ok(());
            }
            if attempt != 2 {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Err(io::Error::other(
            "validator profile cleanup did not complete",
        ))
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
fn folder_path(sid: &str) -> io::Result<PathBuf> {
    let text = wide(sid.as_ref())?;
    let mut pointer = null_mut();
    // SAFETY: terminated SID string; the returned string uses CoTaskMemAlloc.
    let status = unsafe { GetAppContainerFolderPath(text.as_ptr(), &mut pointer) };
    if status != 0 || pointer.is_null() {
        return Err(io::Error::other("validator profile folder is unavailable"));
    }
    let path = (|| {
        // SAFETY: success returns a terminated path, bounded by the platform path limit.
        let length = (0..32_768)
            .find(|index| unsafe { *pointer.add(*index) == 0 })
            .ok_or_else(|| io::Error::other("validator profile path is too long"))?;
        // SAFETY: the string length was established above.
        String::from_utf16(unsafe { std::slice::from_raw_parts(pointer, length) })
            .map(PathBuf::from)
            .map_err(io::Error::other)
    })();
    // SAFETY: the allocated string belongs to this caller.
    unsafe { CoTaskMemFree(pointer.cast()) };
    path
}
