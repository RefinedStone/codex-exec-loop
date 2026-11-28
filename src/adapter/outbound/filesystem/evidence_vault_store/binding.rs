//! Immutable encrypted generations and a process-shared CurrentVersion CAS.
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::super::vault_fs::VaultBindingLease;
use super::inventory::Inventory;
use super::{
    FilesystemEvidenceVaultStoreAdapter, aead_key, open_record, record_relative_path, scope_bytes,
    seal_record, storage_error, vault_fs,
};
use crate::application::port::outbound::evidence_vault_port::{
    VaultBindingCommitPolicy, VaultBindingCommitRequest, VaultBindingCommitResult,
    VaultBindingFields, VaultBindingReceipt, VaultBindingSnapshot, VaultBindingState,
    VaultBindingToken, VaultBindingTransaction, VaultCleanupPolicy, VaultCleanupReport,
    VaultPlaintext, VaultRecordBinding, VaultRecordField, VaultRecordKey, VaultRecordLoadRequest,
    VaultRecordScope, VaultStoreError,
};

const BINDING_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

mod maintenance;

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
    let lease = acquire(store, &request.scope)?;
    let inventory = Inventory::read(store, &request.scope)?;
    let (_, snapshot) = read_snapshot(store, &request.scope, &request.key, &inventory)?;
    lease.verify_identity().map_err(storage_error)?;
    Ok(Box::new(FilesystemBinding {
        store,
        lease,
        scope: request.scope,
        key: request.key,
        snapshot,
        inventory,
    }))
}

pub(super) fn acquire(
    store: &FilesystemEvidenceVaultStoreAdapter,
    scope: &VaultRecordScope,
) -> Result<VaultBindingLease, VaultStoreError> {
    VaultBindingLease::acquire(
        store.vault_root.clone(),
        binding_lock_name(scope),
        BINDING_LOCK_TIMEOUT,
    )
    .map_err(storage_error)
}

fn read_snapshot(
    store: &FilesystemEvidenceVaultStoreAdapter,
    scope: &VaultRecordScope,
    key: &VaultRecordKey,
    inventory: &Inventory,
) -> Result<(Option<String>, VaultBindingSnapshot), VaultStoreError> {
    if let Some(deletion) = &inventory.deletion {
        return Ok((
            None,
            VaultBindingSnapshot {
                token: VaultBindingToken::observed(scope.clone(), Some(deletion.revision)),
                state: VaultBindingState::Deleted,
            },
        ));
    }
    let scope_data = scope_bytes(scope);
    // Filesystem safety/read failures must not become permission to recover an unreadable pointer.
    let raw = vault_fs::read_optional_file(&store.vault_root, &record_relative_path(&scope_data))
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

pub(super) fn acquire_inventory(
    store: &FilesystemEvidenceVaultStoreAdapter,
    scope: &VaultRecordScope,
) -> Result<VaultBindingLease, VaultStoreError> {
    let mut bytes = scope_bytes(scope);
    super::push_part(&mut bytes, "prospective-inventory-lease");
    let name = format!("evidence-vault-binding-{:x}.lock", Sha256::digest(bytes));
    VaultBindingLease::acquire(store.vault_root.clone(), name, BINDING_LOCK_TIMEOUT)
        .map_err(storage_error)
}

struct FilesystemBinding<'a> {
    store: &'a FilesystemEvidenceVaultStoreAdapter,
    lease: VaultBindingLease,
    scope: VaultRecordScope,
    key: VaultRecordKey,
    snapshot: VaultBindingSnapshot,
    inventory: Inventory,
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
        vault_fs::read_optional_file(&self.store.vault_root, &record_relative_path(&scope))
            .map_err(storage_error)?
            .map(|body| open_record(&body, &self.key, &scope))
            .transpose()
    }

    fn stage_field(
        &mut self,
        version: &str,
        field: VaultRecordField,
        plaintext: &VaultPlaintext,
    ) -> Result<bool, VaultStoreError> {
        self.lease.verify_identity().map_err(storage_error)?;
        let field_scope = self.field_scope(version, field);
        self.inventory
            .register(self.store, &self.scope, &field_scope)
            .map_err(storage_error)?;
        let scope = scope_bytes(&field_scope);
        let sealed = seal_record(&self.key, &scope, plaintext)?;
        let sealed = std::str::from_utf8(&sealed).map_err(storage_error)?;
        // Even a non-cooperating writer must not cause an immutable generation to be overwritten.
        // Staging uncertainty is Storage: CurrentVersion still points at the previous bundle.
        let inserted = vault_fs::compare_and_swap_file(
            &self.store.vault_root,
            &record_relative_path(&scope),
            None,
            sealed,
        )
        .map_err(storage_error)?;
        self.lease.verify_identity().map_err(storage_error)?;
        if !inserted && self.read_field(version, field)?.as_ref() != Some(plaintext) {
            return Ok(false);
        }
        #[cfg(all(test, any(unix, windows)))]
        test_hooks::after_field(field);
        Ok(true)
    }
}

impl VaultBindingTransaction for FilesystemBinding<'_> {
    fn snapshot(&self) -> &VaultBindingSnapshot {
        &self.snapshot
    }

    fn retain_reconnect_source(&mut self, version: &str) -> Result<(), VaultStoreError> {
        self.lease.verify_identity().map_err(storage_error)?;
        let inventory_lease = acquire_inventory(self.store, &self.scope)?;
        self.inventory = Inventory::read(self.store, &self.scope)?;
        if self.inventory.mark_generation(version, true)? {
            self.inventory.persist(self.store, &self.scope)?;
        }
        inventory_lease.verify_identity().map_err(storage_error)?;
        self.lease.verify_identity().map_err(storage_error)
    }

    fn maintenance_revision(&self) -> Result<VaultBindingToken, VaultStoreError> {
        Inventory::read(self.store, &self.scope)?.revision(&self.snapshot.token)
    }

    fn maintain(
        self: Box<Self>,
        expected: &VaultBindingToken,
        policy: &VaultCleanupPolicy,
    ) -> Result<VaultCleanupReport, VaultStoreError> {
        maintenance::apply(*self, expected, policy)
    }

    fn load_version_fields(
        &mut self,
        version: &str,
    ) -> Result<VaultBindingFields, VaultStoreError> {
        self.lease.verify_identity().map_err(storage_error)?;
        if matches!(self.snapshot.state, VaultBindingState::Deleted) {
            return Err(VaultStoreError::IssueDeleted);
        }
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
        if matches!(self.snapshot.state, VaultBindingState::Deleted) {
            return Err(VaultStoreError::IssueDeleted);
        }
        let _inventory_lease = acquire_inventory(self.store, &self.scope)?;
        self.inventory = Inventory::read(self.store, &self.scope)?;
        if request.expected.scope() != &self.scope {
            return Ok(VaultBindingCommitResult::Conflict);
        }
        let version = request.generation_version(&self.key)?;
        let (observed, current) =
            read_snapshot(self.store, &self.scope, &self.key, &self.inventory)?;
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

        self.lease.verify_identity().map_err(storage_error)?;
        self.inventory
            .register(self.store, &self.scope, &self.scope)
            .map_err(storage_error)?;
        #[cfg(all(test, any(unix, windows)))]
        test_hooks::before_pointer();
        let scope = scope_bytes(&self.scope);
        let sealed = seal_record(&self.key, &scope, &VaultPlaintext::new(version.clone()))?;
        let sealed = std::str::from_utf8(&sealed).map_err(storage_error)?;
        let replaced = vault_fs::compare_and_swap_file(
            &self.store.vault_root,
            &record_relative_path(&scope),
            observed.as_deref(),
            sealed,
        )
        .map_err(|error| {
            if vault_fs::write_outcome_unknown(&error) {
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
            vault_fs::read_optional_file(&self.store.vault_root, &record_relative_path(&scope))
                .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
        if persisted.as_deref() != Some(sealed) {
            return Err(VaultStoreError::WriteOutcomeUnknown);
        }
        self.lease
            .verify_identity()
            .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
        if let VaultBindingState::Readable { version: previous } = &current.state
            && self
                .inventory
                .mark_generation(previous, false)
                .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?
        {
            // A crash before this receipt only leaves an extra record retained.
            self.inventory
                .persist(self.store, &self.scope)
                .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
        }
        Ok(VaultBindingCommitResult::Committed(VaultBindingReceipt {
            version,
            token: VaultBindingToken::observed(
                self.scope.clone(),
                Some(Sha256::digest(sealed.as_bytes()).into()),
            ),
        }))
    }
}

#[cfg(all(test, any(unix, windows)))]
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

#[cfg(all(test, any(unix, windows)))]
mod tests;
