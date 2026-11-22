//! Immutable encrypted generations and a process-shared CurrentVersion CAS.
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::super::private_runtime_lease::PrivateRuntimeLease;
use super::{
    FilesystemEvidenceVaultStoreAdapter, aead_key, open_record, record_relative_path, scope_bytes,
    seal_record, secure_fs, storage_error,
};
use crate::application::port::outbound::evidence_vault_port::{
    VaultBindingCommitPolicy, VaultBindingCommitRequest, VaultBindingCommitResult,
    VaultBindingFields, VaultBindingReceipt, VaultBindingSnapshot, VaultBindingState,
    VaultBindingToken, VaultBindingTransaction, VaultPlaintext, VaultRecordBinding,
    VaultRecordField, VaultRecordKey, VaultRecordLoadRequest, VaultRecordScope, VaultStoreError,
};

const BINDING_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

fn binding_lock_name(scope: &VaultRecordScope) -> String {
    format!(
        "evidence-vault-binding-{:x}.lock",
        Sha256::digest(scope_bytes(scope))
    )
}

pub(super) fn begin(
    store: &FilesystemEvidenceVaultStoreAdapter,
    request: VaultRecordLoadRequest,
) -> Result<Box<dyn VaultBindingTransaction + '_>, VaultStoreError> {
    if request.scope != VaultRecordScope::current_version(request.scope.issue.clone()) {
        return Err(VaultStoreError::Storage(
            "binding transactions require the canonical current-version scope".to_string(),
        ));
    }
    // A malformed caller key is not a recoverable damaged pointer.
    aead_key(request.key.key.expose())?;
    let lease = PrivateRuntimeLease::acquire(
        store.vault_root.clone(),
        binding_lock_name(&request.scope),
        BINDING_LOCK_TIMEOUT,
    )
    .map_err(storage_error)?;
    let (_, snapshot) = read_snapshot(store, &request.scope, &request.key)?;
    lease.verify_identity().map_err(storage_error)?;
    Ok(Box::new(FilesystemBinding {
        store,
        lease,
        scope: request.scope,
        key: request.key,
        snapshot,
    }))
}

fn read_snapshot(
    store: &FilesystemEvidenceVaultStoreAdapter,
    scope: &VaultRecordScope,
    key: &VaultRecordKey,
) -> Result<(Option<String>, VaultBindingSnapshot), VaultStoreError> {
    let scope_data = scope_bytes(scope);
    // Filesystem safety/read failures must not become permission to recover an unreadable pointer.
    let raw = secure_fs::read_optional_file(&store.vault_root, &record_relative_path(&scope_data))
        .map_err(storage_error)?;
    let token = VaultBindingToken::observed(
        scope.clone(),
        raw.as_ref()
            .map(|value| Sha256::digest(value.as_bytes()).into()),
    );
    let state = match &raw {
        None => VaultBindingState::Missing,
        Some(value) => match open_record(value, key, &scope_data) {
            Ok(version) if !version.expose().is_empty() => VaultBindingState::Readable {
                version: version.expose().to_string(),
            },
            Ok(_) => VaultBindingState::Unreadable {
                error: VaultStoreError::CorruptRecord,
            },
            Err(error) => VaultBindingState::Unreadable { error },
        },
    };
    Ok((raw, VaultBindingSnapshot { token, state }))
}

struct FilesystemBinding<'a> {
    store: &'a FilesystemEvidenceVaultStoreAdapter,
    lease: PrivateRuntimeLease,
    scope: VaultRecordScope,
    key: VaultRecordKey,
    snapshot: VaultBindingSnapshot,
}

impl FilesystemBinding<'_> {
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
        let scope = scope_bytes(&self.field_scope(version, field));
        secure_fs::read_optional_file(&self.store.vault_root, &record_relative_path(&scope))
            .map_err(storage_error)?
            .map(|body| open_record(&body, &self.key, &scope))
            .transpose()
    }

    fn stage_field(
        &self,
        version: &str,
        field: VaultRecordField,
        plaintext: &VaultPlaintext,
    ) -> Result<bool, VaultStoreError> {
        self.lease.verify_identity().map_err(storage_error)?;
        let scope = scope_bytes(&self.field_scope(version, field));
        let sealed = seal_record(&self.key, &scope, plaintext)?;
        let sealed = std::str::from_utf8(&sealed).map_err(storage_error)?;
        // Even a non-cooperating writer must not cause an immutable generation to be overwritten.
        // Staging uncertainty is Storage: CurrentVersion still points at the previous bundle.
        let inserted = secure_fs::compare_and_swap_optional_file(
            &self.store.vault_root,
            &record_relative_path(&scope),
            None,
            Some(sealed),
        )
        .map_err(storage_error)?;
        self.lease.verify_identity().map_err(storage_error)?;
        if !inserted && self.read_field(version, field)?.as_ref() != Some(plaintext) {
            return Ok(false);
        }
        #[cfg(all(test, unix))]
        test_hooks::after_field(field);
        Ok(true)
    }
}

impl VaultBindingTransaction for FilesystemBinding<'_> {
    fn snapshot(&self) -> &VaultBindingSnapshot {
        &self.snapshot
    }

    fn load_version_fields(
        &mut self,
        version: &str,
    ) -> Result<VaultBindingFields, VaultStoreError> {
        self.lease.verify_identity().map_err(storage_error)?;
        let fields = VaultBindingFields {
            title: self.read_field(version, VaultRecordField::Title)?,
            body: self.read_field(version, VaultRecordField::Body)?,
        };
        self.lease.verify_identity().map_err(storage_error)?;
        Ok(fields)
    }

    fn commit(
        mut self: Box<Self>,
        request: VaultBindingCommitRequest,
    ) -> Result<VaultBindingCommitResult, VaultStoreError> {
        self.lease.verify_identity().map_err(storage_error)?;
        if request.expected.scope() != &self.scope {
            return Ok(VaultBindingCommitResult::Conflict);
        }
        let version = request.generation_version(&self.key)?;
        let (observed, current) = read_snapshot(self.store, &self.scope, &self.key)?;
        let fields = self.load_version_fields(&version)?;
        // A committed attempt has a different current token. Check its full immutable identity
        // and payload before rejecting the original expected token as stale.
        if matches!(&current.state, VaultBindingState::Readable { version: value } if value == &version)
        {
            return if fields == request.fields {
                Ok(VaultBindingCommitResult::AlreadyCommitted(
                    VaultBindingReceipt {
                        version,
                        token: current.token,
                    },
                ))
            } else {
                Ok(VaultBindingCommitResult::Conflict)
            };
        }
        if request.expected != current.token {
            return Ok(VaultBindingCommitResult::Conflict);
        }
        let allowed = match request.policy {
            VaultBindingCommitPolicy::ReplaceExpected => {
                !matches!(current.state, VaultBindingState::Unreadable { .. })
            }
            VaultBindingCommitPolicy::InstallIfMissing => {
                matches!(current.state, VaultBindingState::Missing)
            }
            VaultBindingCommitPolicy::RecoverUnreadable => {
                matches!(current.state, VaultBindingState::Unreadable { .. })
            }
        };
        if !allowed {
            return Ok(VaultBindingCommitResult::Conflict);
        }
        // Validate the complete existing generation before adding a missing staging field.
        for (stored, desired) in [
            (&fields.title, &request.fields.title),
            (&fields.body, &request.fields.body),
        ] {
            if stored.is_some() && stored != desired {
                return Ok(VaultBindingCommitResult::Conflict);
            }
        }
        for (field, stored, desired) in [
            (
                VaultRecordField::Title,
                &fields.title,
                &request.fields.title,
            ),
            (VaultRecordField::Body, &fields.body, &request.fields.body),
        ] {
            if let (None, Some(plaintext)) = (stored, desired)
                && !self.stage_field(&version, field, plaintext)?
            {
                return Ok(VaultBindingCommitResult::Conflict);
            }
        }
        if self.load_version_fields(&version)? != request.fields {
            return Ok(VaultBindingCommitResult::Conflict);
        }

        #[cfg(all(test, unix))]
        test_hooks::before_pointer();
        self.lease.verify_identity().map_err(storage_error)?;
        let scope = scope_bytes(&self.scope);
        let sealed = seal_record(&self.key, &scope, &VaultPlaintext::new(version.clone()))?;
        let sealed = std::str::from_utf8(&sealed).map_err(storage_error)?;
        let replaced = secure_fs::compare_and_swap_optional_file(
            &self.store.vault_root,
            &record_relative_path(&scope),
            observed.as_deref(),
            Some(sealed),
        )
        .map_err(|error| {
            if secure_fs::atomic_replacement_completed(&error) {
                VaultStoreError::WriteOutcomeUnknown
            } else {
                storage_error(error)
            }
        })?;
        if !replaced {
            self.lease.verify_identity().map_err(storage_error)?;
            return Ok(VaultBindingCommitResult::Conflict);
        }

        // Pointer replacement has occurred. Any later uncertainty must not promise preservation.
        self.lease
            .verify_identity()
            .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
        let persisted =
            secure_fs::read_optional_file(&self.store.vault_root, &record_relative_path(&scope))
                .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
        if persisted.as_deref() != Some(sealed) {
            return Err(VaultStoreError::WriteOutcomeUnknown);
        }
        self.lease
            .verify_identity()
            .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
        Ok(VaultBindingCommitResult::Committed(VaultBindingReceipt {
            version,
            token: VaultBindingToken::observed(
                self.scope.clone(),
                Some(Sha256::digest(sealed.as_bytes()).into()),
            ),
        }))
    }
}

#[cfg(all(test, unix))]
mod test_hooks {
    use std::cell::RefCell;

    use super::VaultRecordField;

    type FieldHook = Box<dyn FnOnce(VaultRecordField)>;
    thread_local! {
        static AFTER_FIELD: RefCell<Option<FieldHook>> = const { RefCell::new(None) };
        static BEFORE_POINTER: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    }

    pub(super) fn install_after_field(hook: impl FnOnce(VaultRecordField) + 'static) {
        AFTER_FIELD.with(|slot| assert!(slot.borrow_mut().replace(Box::new(hook)).is_none()));
    }

    pub(super) fn after_field(field: VaultRecordField) {
        let hook = AFTER_FIELD.with(|slot| slot.borrow_mut().take());
        if let Some(hook) = hook {
            hook(field);
        }
    }

    pub(super) fn install_before_pointer(hook: impl FnOnce() + 'static) {
        BEFORE_POINTER.with(|slot| assert!(slot.borrow_mut().replace(Box::new(hook)).is_none()));
    }

    pub(super) fn before_pointer() {
        let hook = BEFORE_POINTER.with(|slot| slot.borrow_mut().take());
        if let Some(hook) = hook {
            hook();
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
