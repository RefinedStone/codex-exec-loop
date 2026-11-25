//! LPAC identity and access checks. No profile or host ACL is created here.
use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::ptr::null_mut;

use rand::RngCore;
use windows_sys::Win32::Foundation::{HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::Isolation::DeriveAppContainerSidFromAppContainerName;
use windows_sys::Win32::Security::{
    AccessCheck, DuplicateToken, EqualSid, FreeSid, GENERIC_MAPPING, GetTokenInformation,
    IsValidSid, PSID, SECURITY_ATTRIBUTES, SecurityImpersonation, TOKEN_DUPLICATE,
    TOKEN_INFORMATION_CLASS, TOKEN_QUERY, TokenAppContainerSid, TokenCapabilities,
    TokenIsAppContainer,
};
use windows_sys::Win32::System::Threading::OpenProcessToken;

use super::handles::Handle;

pub(super) fn wide(value: &std::ffi::OsStr) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(io::Error::other("embedded NUL in Windows value"));
    }
    value.push(0);
    Ok(value)
}

pub(super) struct Descriptor(*mut c_void);
impl Descriptor {
    pub(super) fn from_sddl(sddl: &str) -> io::Result<Self> {
        let text = wide(sddl.as_ref())?;
        let mut pointer = null_mut();
        // SAFETY: the string is terminated and the output is owned until LocalFree.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut pointer,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(pointer))
    }

    pub(super) fn attributes(&self, inherit: bool) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: i32::from(inherit),
        }
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: conversion allocated this descriptor with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

pub(super) struct Identity {
    sid: PSID,
    pub(super) name: String,
    pub(super) sid_text: String,
}
impl Identity {
    pub(super) fn create() -> io::Result<Self> {
        let mut random = [0u8; 16];
        rand::rngs::OsRng
            .try_fill_bytes(&mut random)
            .map_err(io::Error::other)?;
        let name = format!("akra.validator.{:032x}", u128::from_le_bytes(random));
        let moniker = wide(name.as_ref())?;
        let mut sid = null_mut();
        // SAFETY: a unique name is supplied and the returned SID is freed with FreeSid.
        let result =
            unsafe { DeriveAppContainerSidFromAppContainerName(moniker.as_ptr(), &mut sid) };
        if result < 0 || sid.is_null() {
            return Err(io::Error::other("AppContainer identity is unavailable"));
        }
        let mut identity = Self {
            sid,
            name,
            sid_text: String::new(),
        };
        // SAFETY: the API returned a SID allocation; check before formatting or comparing.
        if unsafe { IsValidSid(sid) } == 0 {
            return Err(io::Error::other("invalid AppContainer identity"));
        }
        identity.sid_text = sid_string(sid)?;
        Ok(identity)
    }

    pub(super) fn sid(&self) -> PSID {
        self.sid
    }

    pub(super) fn workspace_descriptor(&self) -> io::Result<Descriptor> {
        let owner =
            crate::private_fs::windows_current_user_sid_string().map_err(io::Error::other)?;
        // Child rights are read/execute only. Inheritance is protected; no AAP or broad grant.
        Descriptor::from_sddl(&format!(
            "O:{owner}G:SYD:P(A;OICI;FA;;;{owner})(A;OICI;FA;;;SY)(A;OICI;GRGX;;;{})",
            self.sid_text
        ))
    }

    pub(super) fn verify_process(&self, process: HANDLE) -> io::Result<()> {
        let mut token = null_mut();
        // SAFETY: the live suspended process and out-pointer are valid.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY | TOKEN_DUPLICATE, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Handle::new(token)?;
        let container = query(token.raw(), TokenIsAppContainer)?;
        if container.length != size_of::<u32>() || container.first_u32()? != 1 {
            return Err(io::Error::other("process is not an AppContainer"));
        }
        let capabilities = query(token.raw(), TokenCapabilities)?;
        if capabilities.first_u32()? != 0 {
            return Err(io::Error::other("unexpected AppContainer capabilities"));
        }
        let package = query(token.raw(), TokenAppContainerSid)?;
        if package.length < size_of::<PSID>() {
            return Err(io::Error::other("missing AppContainer SID"));
        }
        // SAFETY: the aligned buffer contains at least one pointer.
        let actual = unsafe { *package.words.as_ptr().cast::<PSID>() };
        let start = package.words.as_ptr() as usize;
        let offset = (actual as usize)
            .checked_sub(start)
            .ok_or_else(|| io::Error::other("AppContainer SID is outside token buffer"))?;
        if offset.checked_add(8).is_none_or(|end| end > package.length) {
            return Err(io::Error::other("truncated AppContainer SID"));
        }
        // SAFETY: the fixed SID header has just been checked inside the returned buffer.
        let subauthorities = unsafe { *actual.cast::<u8>().add(1) } as usize;
        if subauthorities > 15
            || offset
                .checked_add(8 + 4 * subauthorities)
                .is_none_or(|end| end > package.length)
        {
            return Err(io::Error::other("truncated AppContainer SID body"));
        }
        // SAFETY: the entire SID is in the checked buffer; expected SID is API-owned.
        if unsafe { IsValidSid(actual) } == 0 || unsafe { EqualSid(actual, self.sid) } == 0 {
            return Err(io::Error::other("unexpected AppContainer SID"));
        }
        self.verify_lpac_access(token.raw())
    }

    fn verify_lpac_access(&self, token: HANDLE) -> io::Result<()> {
        let mut duplicate = null_mut();
        // SAFETY: DuplicateToken does not impersonate this thread; only AccessCheck uses the handle.
        if unsafe { DuplicateToken(token, SecurityImpersonation, &mut duplicate) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let duplicate = Handle::new(duplicate)?;
        // Verify the documented LPAC access semantics instead of treating undocumented
        // TokenInformationClass 46 output as proof. Every API call must itself succeed.
        // Everyone supplies the user half; the second ACE supplies the package half.
        if access_granted(duplicate.raw(), "S-1-15-2-1")?
            || !access_granted(duplicate.raw(), &self.sid_text)?
            || access_granted(duplicate.raw(), "S-1-15-2-999999")?
        {
            return Err(io::Error::other("LPAC access boundary was not established"));
        }
        Ok(())
    }
}
impl Drop for Identity {
    fn drop(&mut self) {
        // SAFETY: this SID came from DeriveAppContainerSidFromAppContainerName.
        unsafe { FreeSid(self.sid) };
    }
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text = null_mut();
    // SAFETY: the caller owns a valid SID and the out-pointer is valid.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = (|| {
        // SID strings have a bounded representation; never scan an unbounded allocation.
        let length = (0..256)
            .find(|index| {
                // SAFETY: the API returned a terminated SID string, whose maximum is below this bound.
                unsafe { *text.add(*index) == 0 }
            })
            .ok_or_else(|| io::Error::other("invalid SID string"))?;
        // SAFETY: the terminated string length was established above.
        String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
            .map_err(io::Error::other)
    })();
    // SAFETY: ConvertSidToStringSidW allocated the string with LocalAlloc.
    unsafe { LocalFree(text.cast()) };
    result
}

struct TokenBuffer {
    words: Vec<usize>,
    length: usize,
}
impl TokenBuffer {
    fn first_u32(&self) -> io::Result<u32> {
        if self.length < size_of::<u32>() {
            return Err(io::Error::other("truncated token information"));
        }
        // SAFETY: usize allocation is aligned for u32 and length was checked.
        Ok(unsafe { *self.words.as_ptr().cast::<u32>() })
    }
}
fn query(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<TokenBuffer> {
    let mut words = vec![0usize; 2048];
    let size = std::mem::size_of_val(words.as_slice());
    let mut length = 0u32;
    // SAFETY: the aligned buffer has size bytes and remains owned through validation.
    if unsafe {
        GetTokenInformation(
            token,
            class,
            words.as_mut_ptr().cast(),
            size as u32,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if length == 0 || length as usize > size {
        return Err(io::Error::other("invalid token information length"));
    }
    Ok(TokenBuffer {
        words,
        length: length as usize,
    })
}

fn access_granted(token: HANDLE, package_sid: &str) -> io::Result<bool> {
    let descriptor =
        Descriptor::from_sddl(&format!("O:SYG:SYD:P(A;;0x1;;;WD)(A;;0x1;;;{package_sid})"))?;
    let mapping = GENERIC_MAPPING {
        GenericRead: 1,
        GenericWrite: 2,
        GenericExecute: 4,
        GenericAll: 7,
    };
    let mut privileges = [0usize; 128];
    let mut length = std::mem::size_of_val(&privileges) as u32;
    let mut granted = 0;
    let mut status = 0;
    // SAFETY: all pointers are aligned, bounded, and live; desired mask has no generic bits.
    if unsafe {
        AccessCheck(
            descriptor.0,
            token,
            1,
            &mapping,
            privileges.as_mut_ptr().cast(),
            &mut length,
            &mut granted,
            &mut status,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if length as usize > std::mem::size_of_val(&privileges)
        || (status != 0 && granted != 1)
        || (status == 0 && granted != 0)
    {
        return Err(io::Error::other("invalid access check result"));
    }
    Ok(status != 0)
}
