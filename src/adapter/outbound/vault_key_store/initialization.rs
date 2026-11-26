//! OS 저장 요청은 호출 프로세스가 죽은 뒤 완료될 수 있다.
//! Pending을 먼저 영속화하고, 미확정 attempt에는 새 candidate를 보내지 않는다.
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::application::port::outbound::evidence_vault_port::VaultKey;
use crate::application::port::outbound::vault_key_store_port::VaultKeyStoreError;

use super::{generate_key, safe_error, unavailable, validate_loaded};

#[cfg(test)]
pub(crate) mod diagnostics;
mod journal;
use journal::{Journal, Phase, State};

const MAX_STATE_RETRIES: usize = 8;
const SCOPE_DOMAIN: &[u8] = b"akra-evidence-vault-key-initialization-scope-v1\0";
const FINGERPRINT_DOMAIN: &[u8] = b"akra-evidence-vault-key-initialization-fingerprint-v1\0";

/// OS 저장 I/O는 adapter 내부에만 있다. 공개 port는 교체 저장을 노출하지 않는다.
pub(crate) trait CredentialBackend: Send + Sync {
    fn load(&self) -> Result<Option<VaultKey>, VaultKeyStoreError>;
    fn store(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError>;
}

pub(super) fn get_or_create(
    runtime: &Path,
    service: &str,
    account: &str,
    credential: &dyn CredentialBackend,
) -> Result<VaultKey, VaultKeyStoreError> {
    let scope = scope_hash(service, account);
    let mut journal = Journal::open(runtime, scope.clone())?;
    for _ in 0..MAX_STATE_RETRIES {
        let observed = journal.snapshot()?;
        // 외부 keyring I/O 중 SQLite writer lock을 보유하지 않는다.
        #[cfg(test)]
        diagnostics::step("credential_load");
        let loaded = credential.load();
        journal.verify_identity()?;
        let loaded = match loaded {
            Ok(key) => validate_loaded(key)?,
            Err(error) => {
                return Err(
                    if observed
                        .as_ref()
                        .is_some_and(|state| state.phase == Phase::Pending)
                    {
                        unavailable()
                    } else {
                        safe_error(error)
                    },
                );
            }
        };
        let transaction = journal.begin()?;
        let current = transaction.snapshot()?;
        if current != observed {
            // None을 조회하는 동안 다른 프로세스가 생성했으면 그 이전 응답을 쓰지 않는다.
            drop(transaction);
            continue;
        }

        if let Some(key) = loaded {
            if let Some(pending) = current
                .as_ref()
                .filter(|state| state.phase == Phase::Pending)
            {
                if fingerprint(&scope, &key) != pending.fingerprint {
                    return Err(unavailable());
                }
                transaction.settle(pending)?;
            }
            transaction.commit()?;
            return Ok(key);
        }

        if current
            .as_ref()
            .is_some_and(|state| state.phase == Phase::Pending)
        {
            return Err(unavailable());
        }
        let candidate = generate_key()?;
        let pending = transaction.pending(current.as_ref(), fingerprint(&scope, &candidate))?;
        // commit 실패에는 OS 요청을 보내지 않는다. 기록이 실제 남았으면 재조회가 보류한다.
        transaction.commit()?;
        return store_once(&mut journal, &scope, pending, candidate, credential);
    }
    Err(unavailable())
}

fn store_once(
    journal: &mut Journal,
    scope: &str,
    pending: State,
    candidate: VaultKey,
    credential: &dyn CredentialBackend,
) -> Result<VaultKey, VaultKeyStoreError> {
    journal.verify_identity()?;
    #[cfg(test)]
    diagnostics::step("credential_store");
    let stored = credential.store(&candidate);
    journal.verify_identity()?;
    // 오류는 원격 미실행의 증명이 아니다. Pending을 지우거나 같은 요청을 재전송하지 않는다.
    stored.map_err(|_| unavailable())?;

    #[cfg(test)]
    diagnostics::step("credential_readback");
    let loaded = credential.load();
    journal.verify_identity()?;
    let key = validate_loaded(loaded.map_err(|_| unavailable())?)?.ok_or_else(unavailable)?;
    if fingerprint(scope, &key) != pending.fingerprint {
        return Err(unavailable());
    }
    let transaction = journal.begin()?;
    match transaction.snapshot()? {
        Some(current)
            if current.generation == pending.generation
                && current.fingerprint == pending.fingerprint =>
        {
            if current.phase == Phase::Pending {
                transaction.settle(&pending)?;
            }
        }
        _ => return Err(unavailable()),
    }
    // 다른 프로세스가 이미 같은 readback으로 Settled로 바꾸었어도 같은 키만 반환한다.
    transaction.commit()?;
    Ok(key)
}

fn scope_hash(service: &str, account: &str) -> String {
    hash_parts(SCOPE_DOMAIN, &[service.as_bytes(), account.as_bytes()])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn fingerprint(scope: &str, key: &VaultKey) -> [u8; 32] {
    hash_parts(FINGERPRINT_DOMAIN, &[scope.as_bytes(), key.expose()])
}

fn hash_parts(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(domain);
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().into()
}

#[cfg(test)]
mod tests;
