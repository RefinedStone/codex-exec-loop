/*
 * Vault 키 저장소의 OS 어댑터이다 (#2422, #2658).
 * 읽기는 OS 키만 조회한다. 최초 생성은 비밀값 없는 durable intent로 프로세스 사이를 조정한다.
 * 시험은 합성 backend만 사용하며 실제 OS 키 저장소에는 접근하지 않는다.
 */
use std::sync::{Arc, Mutex};

use rand::{RngCore, rngs::OsRng};
use zeroize::Zeroize;

use crate::application::port::outbound::evidence_vault_port::VaultKey;
use crate::application::port::outbound::vault_key_store_port::{
    VaultKeyStoreError, VaultKeyStorePort,
};

mod initialization;

use initialization::CredentialBackend;

const KEY_STORE_SERVICE: &str = "akra-evidence-vault";
const KEY_STORE_ACCOUNT: &str = "device-key-v1";
const VAULT_KEY_LEN: usize = 32;
const UNAVAILABLE: &str = "the Vault key store request could not be completed";

/// OS 키 저장소의 한 항목을 사용한다. 조립만으로 파일이나 키를 만들지 않는다.
pub struct OsVaultKeyStoreAdapter {
    credential: Arc<dyn CredentialBackend>,
    #[cfg(test)]
    runtime_override: Option<std::path::PathBuf>,
}

impl Default for OsVaultKeyStoreAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl OsVaultKeyStoreAdapter {
    pub fn new() -> Self {
        Self {
            credential: Arc::new(OsCredentialBackend),
            #[cfg(test)]
            runtime_override: None,
        }
    }

    fn runtime(&self) -> Result<std::path::PathBuf, VaultKeyStoreError> {
        #[cfg(test)]
        if let Some(runtime) = &self.runtime_override {
            return Ok(runtime.clone());
        }
        if !cfg!(any(
            target_os = "macos",
            target_os = "windows",
            target_os = "linux"
        )) {
            return Err(VaultKeyStoreError::Unsupported);
        }
        crate::stable_user_data_root::stable_user_data_root()
            .map(|root| root.join("global").join("vault-key").join("runtime"))
            .map_err(|_| unavailable())
    }
}

impl VaultKeyStorePort for OsVaultKeyStoreAdapter {
    fn load_key(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
        // Native 열람 경로는 runtime selector나 intent DB를 부르지 않는다.
        validate_loaded(self.credential.load().map_err(safe_error)?)
    }

    fn get_or_create_key(&self) -> Result<VaultKey, VaultKeyStoreError> {
        initialization::get_or_create(
            &self.runtime()?,
            KEY_STORE_SERVICE,
            KEY_STORE_ACCOUNT,
            self.credential.as_ref(),
        )
    }
}

struct OsCredentialBackend;

impl OsCredentialBackend {
    fn entry(&self) -> Result<keyring::Entry, VaultKeyStoreError> {
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

impl CredentialBackend for OsCredentialBackend {
    fn load(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
        match self.entry()?.get_secret() {
            Ok(key) => Ok(Some(VaultKey::new(key))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(key_store_error(error)),
        }
    }

    fn store(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError> {
        self.entry()?
            .set_secret(key.expose())
            .map_err(key_store_error)
    }
}

fn key_store_error(error: keyring::Error) -> VaultKeyStoreError {
    match error {
        keyring::Error::NoStorageAccess(_) => VaultKeyStoreError::AuthenticationCancelled,
        // 기존 Linux 미지원 분류를 보존한다.
        keyring::Error::PlatformFailure(_) if cfg!(target_os = "linux") => {
            VaultKeyStoreError::Unsupported
        }
        keyring::Error::BadEncoding(mut secret) => {
            secret.zeroize();
            unavailable()
        }
        // OS 오류 문자열도 키 또는 저장한 값을 담을 수 있으므로 고정 문구만 사용한다.
        _ => unavailable(),
    }
}

fn unavailable() -> VaultKeyStoreError {
    VaultKeyStoreError::Unavailable(UNAVAILABLE.to_string())
}

fn safe_error(error: VaultKeyStoreError) -> VaultKeyStoreError {
    match error {
        VaultKeyStoreError::Unavailable(_) => unavailable(),
        other => other,
    }
}

fn validate_loaded(key: Option<VaultKey>) -> Result<Option<VaultKey>, VaultKeyStoreError> {
    if key
        .as_ref()
        .is_some_and(|key| key.expose().len() != VAULT_KEY_LEN)
    {
        return Err(unavailable());
    }
    Ok(key)
}

fn generate_key() -> Result<VaultKey, VaultKeyStoreError> {
    let mut bytes = vec![0; VAULT_KEY_LEN];
    if OsRng.try_fill_bytes(&mut bytes).is_err() {
        bytes.zeroize();
        return Err(unavailable());
    }
    Ok(VaultKey::new(bytes))
}

#[derive(Default)]
struct FakeVaultKeyStoreState {
    key: Option<VaultKey>,
    failure: Option<VaultKeyStoreError>,
    load_count: usize,
    initialization_count: usize,
}

/// 메모리 가짜 키 저장소이다. 조회·최초 생성은 한 mutex 구간에서 수행한다.
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

    pub fn lose_key(&self) {
        self.with_state(|state| state.key = None);
    }

    pub fn fail_with(&self, failure: Option<VaultKeyStoreError>) {
        self.with_state(|state| state.failure = failure);
    }

    pub fn stored_key(&self) -> Option<VaultKey> {
        self.with_state(|state| state.key.clone())
    }

    /// 순수 읽기 호출만 센다. 초기화 호출은 별도의 계약이다.
    pub fn load_count(&self) -> usize {
        self.with_state(|state| state.load_count)
    }

    /// 초기화 계약에 진입한 횟수이다. 기존 키 재사용과 실패도 센다.
    pub fn initialization_count(&self) -> usize {
        self.with_state(|state| state.initialization_count)
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
                None => validate_loaded(state.key.clone()),
            }
        })
    }

    fn get_or_create_key(&self) -> Result<VaultKey, VaultKeyStoreError> {
        self.with_state(|state| {
            state.initialization_count += 1;
            if let Some(failure) = &state.failure {
                return Err(failure.clone());
            }
            if let Some(key) = validate_loaded(state.key.clone())? {
                return Ok(key);
            }
            let key = generate_key()?;
            state.key = Some(key.clone());
            Ok(key)
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    pub(crate) use super::initialization::CredentialBackend;
    use super::*;

    pub(crate) fn get_or_create_with_store(
        runtime: &Path,
        store: &dyn CredentialBackend,
    ) -> Result<VaultKey, VaultKeyStoreError> {
        initialization::get_or_create(runtime, KEY_STORE_SERVICE, KEY_STORE_ACCOUNT, store)
    }

    pub(crate) fn adapter_with_store(
        runtime: PathBuf,
        store: Arc<dyn CredentialBackend>,
    ) -> OsVaultKeyStoreAdapter {
        OsVaultKeyStoreAdapter {
            credential: store,
            runtime_override: Some(runtime),
        }
    }
}
