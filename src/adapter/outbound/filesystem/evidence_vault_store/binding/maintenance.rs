use super::super::inventory::{CleanupRequest, Deletion, Inventory};
use super::{
    FilesystemBinding, VaultBindingState, VaultBindingToken, VaultBindingTransaction,
    VaultCleanupPolicy, VaultCleanupReport, VaultStoreError, acquire_inventory, storage_error,
};
use crate::application::port::outbound::evidence_vault_port::VaultLifecycleAction;

pub(super) fn apply(
    mut binding: FilesystemBinding<'_>,
    expected: &VaultBindingToken,
    policy: &VaultCleanupPolicy,
) -> Result<VaultCleanupReport, VaultStoreError> {
    binding.lease.verify_identity().map_err(storage_error)?;
    let inventory_lease = acquire_inventory(binding.store, &binding.scope)?;
    binding.inventory = Inventory::read(binding.store, &binding.scope)?;
    if &binding.maintenance_revision()? != expected {
        return Err(VaultStoreError::Storage(
            "the vault revision changed".to_owned(),
        ));
    }
    let live = match &binding.snapshot.state {
        VaultBindingState::Readable { version } => Some(version.as_str()),
        VaultBindingState::Missing | VaultBindingState::Deleted => None,
        VaultBindingState::Unreadable { error } => return Err(error.clone()),
    };
    match policy.action {
        VaultLifecycleAction::DeleteIssue => {
            if binding.inventory.deletion.is_none() {
                binding.inventory.deletion = Some(Deletion {
                    revision: rand::random(),
                    formerly_live: live.map(str::to_owned),
                });
            }
            // A prior rename can be visible without a durable acknowledgement.
            // Make even a resumed tombstone durable before removing any record.
            binding.inventory.persist(binding.store, &binding.scope)?;
            binding
                .lease
                .verify_identity()
                .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
            inventory_lease
                .verify_identity()
                .map_err(|_| VaultStoreError::WriteOutcomeUnknown)?;
        }
        VaultLifecycleAction::RetainRecent if binding.inventory.deletion.is_some() => {
            return Err(VaultStoreError::IssueDeleted);
        }
        VaultLifecycleAction::RetainRecent if live.is_none() => {
            return Err(VaultStoreError::RecordNotFound);
        }
        VaultLifecycleAction::RetainRecent => {}
    }
    binding.inventory.cleanup(
        binding.store,
        CleanupRequest {
            current: &binding.scope,
            key: &binding.key,
            policy,
            live,
            leases: [&binding.lease, &inventory_lease],
        },
    )
}
