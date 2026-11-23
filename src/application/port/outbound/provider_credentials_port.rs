//! Real storage, native input and authenticated HTTP boundaries for provider credentials.
use crate::domain::provider_credentials::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct NativeOperationOwner {
    id: String,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl NativeOperationOwner {
    pub fn new(id: String, deadline: Instant) -> CredentialResult<Self> {
        if !valid_operation_id(&id) || deadline <= Instant::now() {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        Ok(Self {
            id,
            cancelled: Arc::new(AtomicBool::new(false)),
            deadline,
        })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn cancellation(&self) -> &AtomicBool {
        &self.cancelled
    }
    pub fn cancellation_flag(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn is_live(&self) -> bool {
        !self.cancelled.load(Ordering::SeqCst) && Instant::now() < self.deadline
    }
}

pub trait NativePromptPort: Send + Sync {
    fn supported(&self) -> bool {
        true
    }
    fn prompt(
        &self,
        request: &ProviderCredentialPromptRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<SecretToken>;
    fn confirm_migration(
        &self,
        request: &LegacyImportOffer,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<bool>;
}

pub trait ProviderAccountVerifierPort: Send + Sync {
    fn verify(
        &self,
        request: &ConnectRequest,
        token: &SecretToken,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<ConnectionMetadata>;
}

/// All compare-and-swap checks and publication of the whole secret-free document are one SQL transaction.
pub trait ProviderCredentialLedgerPort: Send + Sync {
    fn load(&self, provider: Provider) -> CredentialResult<CredentialLedger>;
    fn compare_exchange(
        &self,
        expected: &CredentialLedger,
        next: &CredentialLedger,
    ) -> CredentialResult<bool> {
        self.compare_exchange_guarded(expected, next, &|| true)
    }
    /// The guard is checked while holding the writer, including immediately before commit.
    fn compare_exchange_guarded(
        &self,
        expected: &CredentialLedger,
        next: &CredentialLedger,
        publish_allowed: &dyn Fn() -> bool,
    ) -> CredentialResult<bool>;
}

/// Internal key names derive only from validated profile/provider/connection IDs and bounded part indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyringPart {
    pub profile_id: String,
    pub provider: Provider,
    pub connection_id: String,
    pub index: u16,
}
pub trait ProviderKeyStorePort: Send + Sync {
    fn supported(&self) -> bool {
        true
    }
    fn read_part(&self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>>;
    fn write_part(&self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()>;
    fn delete_part(&self, part: &KeyringPart) -> CredentialResult<()>;
    fn read_part_until(
        &self,
        part: &KeyringPart,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
    ) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        check_key_store_deadline(&cancelled, deadline)?;
        self.read_part(part)
    }
    fn write_part_until(
        &self,
        part: &KeyringPart,
        capsule: &[u8],
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
    ) -> CredentialResult<()> {
        check_key_store_deadline(&cancelled, deadline)?;
        self.write_part(part, capsule)
    }
    fn delete_part_until(
        &self,
        part: &KeyringPart,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
    ) -> CredentialResult<()> {
        check_key_store_deadline(&cancelled, deadline)?;
        self.delete_part(part)
    }
}
fn check_key_store_deadline(cancelled: &AtomicBool, deadline: Instant) -> CredentialResult<()> {
    if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
        return Err(ProviderCredentialError::Cancelled);
    }
    Ok(())
}

/// Raw OS calls are serialized by the OS adapter's single dedicated worker.
pub trait ProviderKeyStoreBackend: Send + 'static {
    fn read_part(&mut self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>>;
    fn write_part(&mut self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()>;
    fn delete_part(&mut self, part: &KeyringPart) -> CredentialResult<()>;
}

/// Only an authenticated provider HTTP adapter receives this non-serializable, zeroizing lease.
pub struct ProviderCredentialLease {
    pub reference: ConnectionRef,
    pub metadata: ConnectionMetadata,
    pub token: SecretToken,
}
pub trait ProviderCredentialReadPort: Send + Sync {
    fn current_state(&self, provider: Provider) -> CredentialResult<ProviderCredentialState>;
    fn acquire(&self, expected: &ConnectionRef) -> CredentialResult<ProviderCredentialLease>;
    fn validate_current(&self, expected: &ConnectionRef) -> CredentialResult<()>;
}

/// Dedicated native/private transport only. Public Gateway wire must not expose import_secret.
pub trait ProviderCredentialMigrationPort: Send + Sync {
    fn prepare_import(
        &self,
        offer: LegacyImportOffer,
        owner: NativeOperationOwner,
    ) -> CredentialResult<OperationReceipt>;
    fn import_secret(
        &self,
        offer: LegacyImportOffer,
        token: SecretToken,
        owner: NativeOperationOwner,
    ) -> CredentialResult<OperationReceipt>;
    fn migration_operation(
        &self,
        request: CredentialOperationRequest,
    ) -> CredentialResult<OperationReceipt>;
    fn acknowledge_legacy_cleanup(
        &self,
        request: LegacyCleanupRequest,
    ) -> CredentialResult<OperationReceipt>;
    fn cancel_owner(&self, owner: &NativeOperationOwner) -> CredentialResult<()>;
}
