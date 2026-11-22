//! Atomic in-memory Vault transactions for service tests. Filesystem/process tests exercise the
//! production lease, ciphertext CAS, and durability boundary separately.
use std::collections::HashMap;
use std::sync::MutexGuard;

use sha2::{Digest, Sha256};

use crate::application::port::outbound::evidence_vault_port::{
    VaultBindingCommitPolicy, VaultBindingCommitRequest, VaultBindingCommitResult,
    VaultBindingFields, VaultBindingReceipt, VaultBindingSnapshot, VaultBindingState,
    VaultBindingToken, VaultBindingTransaction, VaultPlaintext, VaultRecordBinding,
    VaultRecordField, VaultRecordKey, VaultRecordLoadRequest, VaultRecordSaveRequest,
    VaultRecordScope, VaultStoreError,
};

pub(crate) trait TestBindingRecord: Sized {
    fn open(&self, key: &VaultRecordKey) -> Result<VaultPlaintext, VaultStoreError>;
    fn seal(request: VaultRecordSaveRequest) -> Self;
    fn fingerprint(&self) -> [u8; 32];
}

impl TestBindingRecord for VaultPlaintext {
    fn open(&self, _: &VaultRecordKey) -> Result<VaultPlaintext, VaultStoreError> {
        Ok(self.clone())
    }
    fn seal(request: VaultRecordSaveRequest) -> Self {
        request.plaintext
    }
    fn fingerprint(&self) -> [u8; 32] {
        Sha256::digest(self.expose().as_bytes()).into()
    }
}

impl TestBindingRecord for (VaultRecordKey, VaultPlaintext) {
    fn open(&self, key: &VaultRecordKey) -> Result<VaultPlaintext, VaultStoreError> {
        if self.0.key_id != key.key_id {
            return Err(VaultStoreError::KeyMismatch);
        }
        if self.0.key != key.key {
            return Err(VaultStoreError::IntegrityFailure);
        }
        Ok(self.1.clone())
    }
    fn seal(request: VaultRecordSaveRequest) -> Self {
        (request.key, request.plaintext)
    }
    fn fingerprint(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update((self.0.key_id.len() as u64).to_be_bytes());
        digest.update(self.0.key_id.as_bytes());
        digest.update(self.0.key.expose());
        digest.update(self.1.expose().as_bytes());
        digest.finalize().into()
    }
}

type BeforeWrite<'a> = Box<dyn Fn(&VaultRecordSaveRequest) -> Result<(), VaultStoreError> + 'a>;
type AfterPointer<'a> = Box<dyn Fn() -> Result<(), VaultStoreError> + 'a>;

#[derive(Default)]
pub(crate) struct TestBindingHooks<'a> {
    pub before_write: Option<BeforeWrite<'a>>,
    pub after_pointer: Option<AfterPointer<'a>>,
}

pub(crate) fn begin_test_binding<'a, V: TestBindingRecord + 'a>(
    records: MutexGuard<'a, HashMap<VaultRecordScope, V>>,
    request: VaultRecordLoadRequest,
    hooks: TestBindingHooks<'a>,
) -> Result<Box<dyn VaultBindingTransaction + 'a>, VaultStoreError> {
    if request.scope != VaultRecordScope::current_version(request.scope.issue.clone()) {
        return Err(VaultStoreError::Storage(
            "binding transactions require the canonical current-version scope".to_string(),
        ));
    }
    let current = records.get(&request.scope);
    let token = VaultBindingToken::observed(
        request.scope.clone(),
        current.map(TestBindingRecord::fingerprint),
    );
    let state = match current {
        None => VaultBindingState::Missing,
        Some(value) => match value.open(&request.key) {
            Ok(version) if !version.expose().is_empty() => VaultBindingState::Readable {
                version: version.expose().to_string(),
            },
            Ok(_) => VaultBindingState::Unreadable {
                error: VaultStoreError::CorruptRecord,
            },
            Err(error) => VaultBindingState::Unreadable { error },
        },
    };
    Ok(Box::new(TestBinding {
        records,
        scope: request.scope,
        key: request.key,
        snapshot: VaultBindingSnapshot { token, state },
        hooks,
    }))
}

struct TestBinding<'a, V> {
    records: MutexGuard<'a, HashMap<VaultRecordScope, V>>,
    scope: VaultRecordScope,
    key: VaultRecordKey,
    snapshot: VaultBindingSnapshot,
    hooks: TestBindingHooks<'a>,
}

impl<V: TestBindingRecord> TestBinding<'_, V> {
    fn field_scope(&self, version: &str, field: VaultRecordField) -> VaultRecordScope {
        VaultRecordScope {
            issue: self.scope.issue.clone(),
            field,
            binding: VaultRecordBinding::ConfirmedVersion(version.to_string()),
        }
    }

    fn read_field(
        &self,
        version: &str,
        field: VaultRecordField,
    ) -> Result<Option<VaultPlaintext>, VaultStoreError> {
        self.records
            .get(&self.field_scope(version, field))
            .map(|value| value.open(&self.key))
            .transpose()
    }

    fn write(&mut self, request: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
        if let Some(hook) = &self.hooks.before_write {
            hook(&request)?;
        }
        let scope = request.scope.clone();
        self.records.insert(scope, V::seal(request));
        Ok(())
    }
}

impl<V: TestBindingRecord> VaultBindingTransaction for TestBinding<'_, V> {
    fn snapshot(&self) -> &VaultBindingSnapshot {
        &self.snapshot
    }

    fn load_version_fields(
        &mut self,
        version: &str,
    ) -> Result<VaultBindingFields, VaultStoreError> {
        Ok(VaultBindingFields {
            title: self.read_field(version, VaultRecordField::Title)?,
            body: self.read_field(version, VaultRecordField::Body)?,
        })
    }

    fn commit(
        mut self: Box<Self>,
        request: VaultBindingCommitRequest,
    ) -> Result<VaultBindingCommitResult, VaultStoreError> {
        if request.expected.scope() != &self.scope {
            return Ok(VaultBindingCommitResult::Conflict);
        }
        let version = request.generation_version(&self.key)?;
        let stored_fields = self.load_version_fields(&version)?;
        if matches!(&self.snapshot.state, VaultBindingState::Readable { version: current } if current == &version)
        {
            return if stored_fields == request.fields {
                Ok(VaultBindingCommitResult::AlreadyCommitted(
                    VaultBindingReceipt {
                        version,
                        token: self.snapshot.token.clone(),
                    },
                ))
            } else {
                Ok(VaultBindingCommitResult::Conflict)
            };
        }
        if request.expected != self.snapshot.token {
            return Ok(VaultBindingCommitResult::Conflict);
        }
        let allowed = match request.policy {
            VaultBindingCommitPolicy::ReplaceExpected => {
                !matches!(self.snapshot.state, VaultBindingState::Unreadable { .. })
            }
            VaultBindingCommitPolicy::InstallIfMissing => {
                matches!(self.snapshot.state, VaultBindingState::Missing)
            }
            VaultBindingCommitPolicy::RecoverUnreadable => {
                matches!(self.snapshot.state, VaultBindingState::Unreadable { .. })
            }
        };
        if !allowed {
            return Ok(VaultBindingCommitResult::Conflict);
        }
        for (existing, desired) in [
            (&stored_fields.title, &request.fields.title),
            (&stored_fields.body, &request.fields.body),
        ] {
            if existing.is_some() && existing != desired {
                return Ok(VaultBindingCommitResult::Conflict);
            }
        }
        for (field, existing, desired) in [
            (
                VaultRecordField::Title,
                stored_fields.title,
                request.fields.title,
            ),
            (
                VaultRecordField::Body,
                stored_fields.body,
                request.fields.body,
            ),
        ] {
            match (existing, desired) {
                (Some(existing), Some(desired)) if existing == desired => {}
                (Some(_), _) => return Ok(VaultBindingCommitResult::Conflict),
                (None, Some(plaintext)) => {
                    self.write(VaultRecordSaveRequest {
                        scope: self.field_scope(&version, field),
                        key: self.key.clone(),
                        plaintext,
                    })?;
                }
                (None, None) => {}
            }
        }
        self.write(VaultRecordSaveRequest {
            scope: self.scope.clone(),
            key: self.key.clone(),
            plaintext: VaultPlaintext::new(version.clone()),
        })?;
        let token = VaultBindingToken::observed(
            self.scope.clone(),
            self.records
                .get(&self.scope)
                .map(TestBindingRecord::fingerprint),
        );
        if let Some(hook) = &self.hooks.after_pointer {
            hook().map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
        }
        Ok(VaultBindingCommitResult::Committed(VaultBindingReceipt {
            version,
            token,
        }))
    }
}
