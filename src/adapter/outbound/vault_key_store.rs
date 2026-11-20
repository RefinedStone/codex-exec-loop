/*
 * Vault 키 저장소 계약의 구현이다 (#2422).
 *
 * `OsVaultKeyStoreAdapter` 는 `keyring` crate 로 OS 키 저장소를 쓴다: macOS Keychain, Windows Credential Manager,
 * Linux Secret Service. 다른 플랫폼과 Secret Service 가 없는 Linux 는 `Unsupported` 이다.
 * `FakeVaultKeyStoreAdapter` 는 메모리 가짜 구현이다. 시험은 가짜만 쓴다. 실제 OS 키 저장소를 건드리지 않는다.
 */
use std::sync::Mutex;

use zeroize::Zeroize;

use crate::application::port::outbound::evidence_vault_port::VaultKey;
use crate::application::port::outbound::vault_key_store_port::{
    VaultKeyStoreError, VaultKeyStorePort,
};

const KEY_STORE_SERVICE: &str = "akra-evidence-vault";
const KEY_STORE_ACCOUNT: &str = "device-key-v1";

/// OS 키 저장소에 Vault 키 한 개를 둔다.
#[derive(Default)]
pub struct OsVaultKeyStoreAdapter;

impl OsVaultKeyStoreAdapter {
    pub fn new() -> Self {
        Self
    }

    fn entry(&self) -> Result<keyring::Entry, VaultKeyStoreError> {
        // 다른 플랫폼에서 `keyring` 은 메모리 가짜 저장소를 쓴다. 키가 조용히 사라지므로 미지원으로 한다.
        if !cfg!(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "linux"
        )) {
            return Err(VaultKeyStoreError::Unsupported);
        }
        keyring::Entry::new(KEY_STORE_SERVICE, KEY_STORE_ACCOUNT).map_err(key_store_error)
    }
}

impl VaultKeyStorePort for OsVaultKeyStoreAdapter {
    fn load_key(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
        match self.entry()?.get_secret() {
            Ok(key) => Ok(Some(VaultKey::new(key))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(key_store_error(error)),
        }
    }

    fn store_key(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError> {
        self.entry()?
            .set_secret(key.expose())
            .map_err(key_store_error)
    }
}

fn key_store_error(error: keyring::Error) -> VaultKeyStoreError {
    match error {
        keyring::Error::NoStorageAccess(_) => VaultKeyStoreError::AuthenticationCancelled,
        // shortcut: Linux 의 플랫폼 실패는 모두 Secret Service 없음으로 본다, 원인을 나눠 보여줄 때 세분한다.
        keyring::Error::PlatformFailure(_) if cfg!(target_os = "linux") => {
            VaultKeyStoreError::Unsupported
        }
        keyring::Error::PlatformFailure(platform_error) => {
            VaultKeyStoreError::Unavailable(platform_error.to_string())
        }
        keyring::Error::BadEncoding(mut secret) => {
            secret.zeroize();
            VaultKeyStoreError::Unavailable("the stored key cannot be read".to_string())
        }
        // 나머지 오류의 글에 저장한 값이 들어갈 수 있으므로 고정 문구만 쓴다.
        _ => VaultKeyStoreError::Unavailable("the key store rejected the request".to_string()),
    }
}

#[derive(Default)]
struct FakeVaultKeyStoreState {
    key: Option<VaultKey>,
    failure: Option<VaultKeyStoreError>,
    load_count: usize,
}

/// 메모리 가짜 키 저장소이다. 키 상실과 저장소 오류를 흉내 낸다.
#[derive(Default)]
pub struct FakeVaultKeyStoreAdapter {
    state: Mutex<FakeVaultKeyStoreState>,
}

impl FakeVaultKeyStoreAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_key(key: VaultKey) -> Self {
        let store = Self::default();
        store.with_state(|state| state.key = Some(key));
        store
    }

    /// 키 상실을 흉내 낸다.
    pub fn lose_key(&self) {
        self.with_state(|state| state.key = None);
    }

    /// 다음 요청부터 이 오류를 돌려준다. `None` 이면 다시 동작한다.
    pub fn fail_with(&self, failure: Option<VaultKeyStoreError>) {
        self.with_state(|state| state.failure = failure);
    }

    pub fn stored_key(&self) -> Option<VaultKey> {
        self.with_state(|state| state.key.clone())
    }

    /// `load_key` 를 부른 횟수이다.
    pub fn load_count(&self) -> usize {
        self.with_state(|state| state.load_count)
    }

    fn with_state<T>(&self, action: impl FnOnce(&mut FakeVaultKeyStoreState) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        action(&mut state)
    }
}

impl VaultKeyStorePort for FakeVaultKeyStoreAdapter {
    fn load_key(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
        self.with_state(|state| {
            state.load_count += 1;
            match &state.failure {
                Some(failure) => Err(failure.clone()),
                None => Ok(state.key.clone()),
            }
        })
    }

    fn store_key(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError> {
        self.with_state(|state| match &state.failure {
            Some(failure) => Err(failure.clone()),
            None => {
                state.key = Some(key.clone());
                Ok(())
            }
        })
    }
}
