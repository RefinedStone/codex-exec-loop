//! Resolve the shared user data root from the OS account, without creating files.
//! Environment overrides and the current directory must not split process-wide coordination.

use std::path::PathBuf;

use anyhow::Result;
#[cfg(not(windows))]
use anyhow::bail;

#[cfg(unix)]
pub(crate) fn stable_user_data_root() -> Result<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStringExt;

    // SAFETY: these OS account queries take no pointers and retain no state.
    let uid = unsafe { libc::geteuid() };
    let buffer_size = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let buffer_size = if buffer_size <= 0 {
        16 * 1024
    } else {
        usize::try_from(buffer_size)
            .unwrap_or(16 * 1024)
            .min(1024 * 1024)
    };
    let mut record = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0_u8; buffer_size];
    // SAFETY: the record, output pointer, and scratch buffer remain live for the call.
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            record.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() {
        bail!("failed to resolve the stable operating-system home for private runtime data");
    }
    // SAFETY: success and a non-null result mean getpwuid_r initialized the record.
    let record = unsafe { record.assume_init() };
    if record.pw_dir.is_null() {
        bail!("operating-system account has no home directory for private runtime data");
    }
    // SAFETY: pw_dir points into the still-live buffer and is NUL-terminated by getpwuid_r.
    let home = PathBuf::from(std::ffi::OsString::from_vec(
        unsafe { CStr::from_ptr(record.pw_dir) }.to_bytes().to_vec(),
    ));
    if !home.is_absolute() {
        bail!("operating-system account home must be absolute for private runtime data");
    }
    Ok(home.join(".akra"))
}

#[cfg(windows)]
pub(crate) fn stable_user_data_root() -> Result<PathBuf> {
    Ok(crate::private_fs::windows_local_app_data_path()?.join("Akra"))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn stable_user_data_root() -> Result<PathBuf> {
    bail!("stable operating-system runtime storage is unsupported on this platform")
}

#[cfg(all(test, any(unix, windows)))]
mod tests;
