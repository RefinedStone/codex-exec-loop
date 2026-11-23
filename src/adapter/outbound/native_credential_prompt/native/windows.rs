use super::*;
use windows_sys::Win32::Foundation::{ERROR_CANCELLED, ERROR_INSUFFICIENT_BUFFER, GetLastError};
use windows_sys::Win32::Security::Credentials::{
    CREDUI_INFOW, CREDUIWIN_GENERIC, CredUIPromptForWindowsCredentialsW,
    CredUnPackAuthenticationBufferW,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IDOK, MB_DEFBUTTON2, MB_ICONWARNING, MB_OKCANCEL, MessageBoxW,
};
use zeroize::{Zeroize, Zeroizing};
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(super) fn confirm(
    text: &str,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> CredentialResult<bool> {
    let text = wide(text);
    let title = wide("Akra 회사 데이터 정책 확인");
    // SAFETY: NUL-terminated immutable buffers outlive this synchronous child-only dialog.
    let response = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OKCANCEL | MB_DEFBUTTON2 | MB_ICONWARNING,
        )
    };
    super::super::check_live(cancelled, deadline)?;
    Ok(response == IDOK)
}
struct CredentialBuffer {
    pointer: *mut core::ffi::c_void,
    length: usize,
}
impl Drop for CredentialBuffer {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            // SAFETY: CredUI returned this allocation and its exact byte count.
            unsafe {
                std::slice::from_raw_parts_mut(self.pointer.cast::<u8>(), self.length).zeroize();
                CoTaskMemFree(self.pointer);
            }
        }
    }
}
pub(super) fn prompt(
    request: &ConnectRequest,
    text: &str,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> CredentialResult<SecretToken> {
    super::super::check_live(cancelled, deadline)?;
    let text = wide(&format!(
        "{text}\n암호 입력 칸에 API 토큰을 붙여 넣으세요. 사용자 이름은 이 연결의 계정을 확인하는 용도이며 저장하지 않습니다."
    ));
    let title = wide("Akra API 토큰 입력");
    let ui = CREDUI_INFOW {
        cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
        hwndParent: std::ptr::null_mut(),
        pszMessageText: text.as_ptr(),
        pszCaptionText: title.as_ptr(),
        hbmBanner: std::ptr::null_mut(),
    };
    let mut package = 0;
    let mut pointer = std::ptr::null_mut();
    let mut length = 0;
    let mut save = 0;
    // SAFETY: this runs only in the managed prompt child. No save flag or key-store API is used.
    let status = unsafe {
        CredUIPromptForWindowsCredentialsW(
            &ui,
            0,
            &mut package,
            std::ptr::null(),
            0,
            &mut pointer,
            &mut length,
            &mut save,
            CREDUIWIN_GENERIC,
        )
    };
    let buffer = CredentialBuffer {
        pointer,
        length: length as usize,
    };
    if status == ERROR_CANCELLED {
        return Err(ProviderCredentialError::Cancelled);
    }
    if status != 0 {
        return Err(ProviderCredentialError::Unavailable);
    }
    super::super::check_live(cancelled, deadline)?;
    if buffer.pointer.is_null() || buffer.length == 0 || buffer.length > 256 * 1024 {
        return Err(ProviderCredentialError::TooLarge);
    }
    unpack(request.provider, &buffer)
}
fn unpack(provider: Provider, buffer: &CredentialBuffer) -> CredentialResult<SecretToken> {
    let (mut user_len, mut domain_len, mut password_len) = (0u32, 0u32, 0u32);
    // SAFETY: documented sizing call; the CredUI allocation remains live.
    let probed = unsafe {
        CredUnPackAuthenticationBufferW(
            0,
            buffer.pointer,
            buffer.length as u32,
            std::ptr::null_mut(),
            &mut user_len,
            std::ptr::null_mut(),
            &mut domain_len,
            std::ptr::null_mut(),
            &mut password_len,
        )
    };
    if probed != 0 || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err(ProviderCredentialError::Unavailable);
    }
    if user_len > 65537 || domain_len > 65537 || password_len == 0 || password_len > 65537 {
        return Err(ProviderCredentialError::TooLarge);
    }
    let mut user = Zeroizing::new(vec![0u16; user_len.max(1) as usize]);
    let mut domain = Zeroizing::new(vec![0u16; domain_len.max(1) as usize]);
    let mut password = Zeroizing::new(vec![0u16; password_len as usize]);
    // SAFETY: each zeroizing output allocation has at least the probed capacity.
    if unsafe {
        CredUnPackAuthenticationBufferW(
            0,
            buffer.pointer,
            buffer.length as u32,
            user.as_mut_ptr(),
            &mut user_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            password.as_mut_ptr(),
            &mut password_len,
        )
    } == 0
    {
        return Err(ProviderCredentialError::Unavailable);
    }
    let count = password_len as usize;
    if count == 0 || count > password.len() {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    token_from_password_words(provider, &password[..count])
}
fn token_from_password_words(provider: Provider, words: &[u16]) -> CredentialResult<SecretToken> {
    if words.last() != Some(&0) {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    let mut bytes = Zeroizing::new(Vec::new());
    for word in char::decode_utf16(words[..words.len() - 1].iter().copied()) {
        let character = word.map_err(|_| ProviderCredentialError::InvalidRequest)?;
        let mut encoded = Zeroizing::new([0u8; 4]);
        bytes.extend_from_slice(character.encode_utf8(&mut *encoded).as_bytes());
        if bytes.len() > crate::private_ipc::credential_frames::MAX_SECRET_BYTES {
            return Err(ProviderCredentialError::TooLarge);
        }
    }
    SecretToken::from_bytes(provider, std::mem::take(&mut *bytes))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_generic_credential_blob_roundtrips_through_the_actual_unpack_boundary() {
        use windows_sys::Win32::Security::Credentials::{
            CRED_PACK_GENERIC_CREDENTIALS, CredPackAuthenticationBufferW,
        };
        use windows_sys::Win32::System::Com::CoTaskMemAlloc;
        // These APIs only serialize memory. No prompt, credential read, or key-store write is called.
        let username = Zeroizing::new(wide("synthetic-api-account"));
        let password = Zeroizing::new(wide("synthetic-packed-토큰-🧪"));
        let mut length = 0u32;
        // SAFETY: NUL-terminated synthetic buffers are live, and this is the documented sizing call.
        let probe = unsafe {
            CredPackAuthenticationBufferW(
                CRED_PACK_GENERIC_CREDENTIALS,
                username.as_ptr(),
                password.as_ptr(),
                std::ptr::null_mut(),
                &mut length,
            )
        };
        let error = unsafe { GetLastError() };
        assert_eq!(probe, 0);
        assert_eq!(error, ERROR_INSUFFICIENT_BUFFER);
        assert!((1..=256 * 1024).contains(&length));
        // SAFETY: allocate exactly the probed size; CredentialBuffer owns and zeroes/frees it.
        let pointer = unsafe { CoTaskMemAlloc(length as usize) };
        assert!(!pointer.is_null());
        let buffer = CredentialBuffer {
            pointer,
            length: length as usize,
        };
        // SAFETY: the newly allocated output spans length bytes.
        unsafe {
            std::ptr::write_bytes(buffer.pointer.cast::<u8>(), 0, buffer.length);
        }
        // SAFETY: output capacity matches the successful sizing probe and all inputs remain live.
        let packed = unsafe {
            CredPackAuthenticationBufferW(
                CRED_PACK_GENERIC_CREDENTIALS,
                username.as_ptr(),
                password.as_ptr(),
                buffer.pointer.cast::<u8>(),
                &mut length,
            )
        };
        assert_ne!(packed, 0);
        assert_eq!(length as usize, buffer.length);
        let token = unpack(Provider::Jira, &buffer).unwrap();
        assert_eq!(token.as_str(), "synthetic-packed-토큰-🧪");
        assert!(token.expose().len() < crate::private_ipc::credential_frames::MAX_SECRET_BYTES);
    }

    #[test]
    fn synthetic_credential_words_are_bounded_and_never_accept_malformed_utf16() {
        let words = wide("synthetic-CredUI-token");
        assert_eq!(
            token_from_password_words(Provider::Jira, &words)
                .unwrap()
                .as_str(),
            "synthetic-CredUI-token"
        );
        assert!(token_from_password_words(Provider::Jira, &[0xd800, 0]).is_err());
        assert!(token_from_password_words(Provider::Jira, &[65]).is_err());
        assert!(token_from_password_words(Provider::Jira, &[0]).is_err());
        assert!(token_from_password_words(Provider::Linear, &vec![65; 4098]).is_err());
    }
}
