//! One Gateway owns a profile's credentials. SQLite records intent; unique OS slots hold secrets.
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use rand::RngCore;

use crate::application::port::outbound::provider_credentials_port::*;
use crate::domain::provider_credentials::*;

const CAS_ATTEMPTS: usize = 64;
const CLEANUP_BUDGET: Duration = Duration::from_secs(5);
const ACQUIRE_BUDGET: Duration = Duration::from_secs(20);
const MAX_CLEANUP_CALLS: usize = 128;
type OwnerMap = HashMap<(Provider, String), NativeOperationOwner>;
mod mutation;

pub struct ProviderCredentialService {
    profile_id: String,
    ledger: Arc<dyn ProviderCredentialLedgerPort>,
    key_store: Arc<dyn ProviderKeyStorePort>,
    prompt: Arc<dyn NativePromptPort>,
    verifier: Arc<dyn ProviderAccountVerifierPort>,
    owners: Mutex<OwnerMap>,
    closing: Arc<AtomicBool>,
    mutations: Arc<mutation::MutationAdmission>,
    changed: tokio::sync::broadcast::Sender<ProviderCredentialChanged>,
}

impl ProviderCredentialService {
    pub fn new(
        profile_id: String,
        ledger: Arc<dyn ProviderCredentialLedgerPort>,
        key_store: Arc<dyn ProviderKeyStorePort>,
        prompt: Arc<dyn NativePromptPort>,
        verifier: Arc<dyn ProviderAccountVerifierPort>,
    ) -> CredentialResult<Self> {
        if !valid_profile_id(&profile_id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let (changed, _) = tokio::sync::broadcast::channel(64);
        let closing = Arc::new(AtomicBool::new(false));
        let mutations = Arc::new(mutation::MutationAdmission::new(
            profile_id.clone(),
            ledger.clone(),
            closing.clone(),
        ));
        Ok(Self {
            profile_id,
            ledger,
            key_store,
            prompt,
            verifier,
            owners: Mutex::new(HashMap::new()),
            closing,
            mutations,
            changed,
        })
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ProviderCredentialChanged> {
        self.changed.subscribe()
    }

    /// Fence pending input without key-store I/O, then drain bounded admitted HTTP mutations.
    pub fn shutdown(&self) -> CredentialResult<()> {
        self.mutations.close()?;
        for owner in self
            .owners
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?
            .values()
        {
            owner.cancel();
        }
        for provider in [Provider::Jira, Provider::Linear] {
            self.change(provider, |ledger| {
                for operation in &mut ledger.operations {
                    if operation.receipt.status.is_pending() {
                        operation.receipt.status = CredentialOperationStatus::Cancelled;
                        operation.receipt.error = Some(ProviderCredentialError::Cancelled);
                    }
                }
                recompute_cleanup(ledger);
                Ok(())
            })?;
        }
        self.mutations
            .drain(None, u64::MAX, Instant::now() + ACQUIRE_BUDGET)
    }

    /// Call only after acquiring the Gateway's exclusive profile owner and before serving requests.
    /// Recovery fences the old owner without opening/unlocking the OS store or republishing its work.
    pub fn recover(&self) -> CredentialResult<()> {
        if !self
            .owners
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?
            .is_empty()
        {
            return Err(ProviderCredentialError::Busy);
        }
        for provider in [Provider::Jira, Provider::Linear] {
            self.change(provider, |ledger| {
                for operation in &mut ledger.operations {
                    if operation.receipt.status.is_pending() {
                        let unknown = operation.parts.contains(&SecretPartState::Submitted);
                        operation.receipt.status = if unknown {
                            CredentialOperationStatus::OutcomeUnknown
                        } else {
                            CredentialOperationStatus::Cancelled
                        };
                        operation.receipt.error = Some(if unknown {
                            ProviderCredentialError::OutcomeUnknown
                        } else {
                            ProviderCredentialError::Cancelled
                        });
                    }
                }
                recompute_cleanup(ledger);
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Metadata only: never read, unlock, enumerate, repair, or probe key-store entries.
    pub fn state(&self, provider: Provider) -> CredentialResult<ProviderCredentialState> {
        let ledger = self.load(provider)?;
        Ok(ProviderCredentialState {
            profile_id: self.profile_id.clone(),
            provider,
            revision: ledger.revision,
            status: if ledger.active.is_some() {
                ProviderConnectionStatus::Connected
            } else {
                ProviderConnectionStatus::Disconnected
            },
            active_connection: ledger.active.map(|active| active.summary),
            operation: ledger
                .operations
                .last()
                .map(|operation| operation.receipt.clone()),
            capabilities: ProviderCredentialCapabilities {
                native_input: self.prompt.supported() && self.key_store.supported(),
                legacy_import: self.prompt.supported() && self.key_store.supported(),
                keyring: self.key_store.supported(),
            },
        })
    }

    pub fn operation(
        &self,
        request: CredentialOperationRequest,
    ) -> CredentialResult<OperationReceipt> {
        self.validate_operation_request(&request)?;
        let ledger = self.load(request.provider)?;
        let receipt = find_operation(&ledger, &request.operation_id)?
            .receipt
            .clone();
        if receipt.status == CredentialOperationStatus::Committed {
            let before_revision = receipt
                .connection
                .as_ref()
                .map(|reference| reference.revision)
                .unwrap_or(next_revision(receipt.expected_revision)?);
            self.mutations.drain(
                Some(request.provider),
                before_revision,
                Instant::now() + ACQUIRE_BUDGET,
            )?;
        }
        Ok(receipt)
    }

    pub fn start_connect(
        &self,
        mut request: ConnectRequest,
        owner: NativeOperationOwner,
    ) -> CredentialResult<OperationReceipt> {
        self.validate_connect(&request)?;
        request.configuration = request.configuration.normalized(request.provider)?;
        let (receipt, created) =
            self.begin(&request, CredentialOperationKind::Connect, None, &owner)?;
        if !created {
            return Ok(receipt);
        }
        self.insert_owner(&request, &owner)?;
        let result = (|| {
            self.guard_operation(&request, &owner)?;
            if !self.prompt.supported() || !self.key_store.supported() {
                return Err(ProviderCredentialError::Unsupported);
            }
            let token = self
                .prompt
                .prompt(&request, owner.cancellation(), owner.deadline())?;
            self.claim_verification(&request, &owner, CredentialOperationStatus::AwaitingInput)?;
            self.accept_secret(&request, token, &owner)
        })();
        self.finish_attempt(&request, &owner, result)
    }

    /// This durable fence is independent of the key-store worker, so a hung OS write cannot
    /// prevent cancellation or replacement from invalidating a late publication.
    pub fn cancel(
        &self,
        request: CredentialOperationRequest,
    ) -> CredentialResult<OperationReceipt> {
        self.validate_operation_request(&request)?;
        if let Some(owner) = self
            .owners
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?
            .get(&(request.provider, request.operation_id.clone()))
            .cloned()
        {
            owner.cancel();
        }
        self.change(request.provider, |ledger| {
            let operation = find_operation_mut(ledger, &request.operation_id)?;
            if operation.receipt.status.is_pending() {
                operation.receipt.status = CredentialOperationStatus::Cancelled;
                operation.receipt.error = Some(ProviderCredentialError::Cancelled);
            }
            recompute_cleanup(ledger);
            Ok(find_operation(ledger, &request.operation_id)?
                .receipt
                .clone())
        })
    }

    pub fn disconnect(&self, request: DisconnectRequest) -> CredentialResult<OperationReceipt> {
        self.validate_intent(
            &request.profile_id,
            &request.operation_id,
            request.expected_revision,
        )?;
        let (receipt, superseded) = self.change(request.provider, |ledger| {
            if let Ok(operation) = find_operation(ledger, &request.operation_id) {
                if operation.receipt.kind != CredentialOperationKind::Disconnect
                    || operation.receipt.expected_revision != request.expected_revision
                {
                    return Err(ProviderCredentialError::Conflict);
                }
                return Ok((operation.receipt.clone(), None));
            }
            if ledger.revision != request.expected_revision {
                return Err(ProviderCredentialError::Conflict);
            }
            if ledger.operations.len() >= MAX_OPERATIONS {
                return Err(ProviderCredentialError::CapacityExceeded);
            }
            ledger.revision = next_revision(ledger.revision)?;
            ledger.active = None;
            ledger.tombstone = true;
            let mut superseded = Vec::new();
            for operation in &mut ledger.operations {
                if operation.receipt.status.is_pending() {
                    superseded.push(operation.receipt.operation_id.clone());
                    operation.receipt.status = CredentialOperationStatus::Superseded;
                    operation.receipt.error = Some(ProviderCredentialError::Conflict);
                }
            }
            ledger.operations.push(StoredOperation {
                receipt: OperationReceipt {
                    operation_id: request.operation_id.clone(),
                    profile_id: self.profile_id.clone(),
                    provider: request.provider,
                    expected_revision: request.expected_revision,
                    kind: CredentialOperationKind::Disconnect,
                    status: CredentialOperationStatus::Committed,
                    connection: None,
                    cleanup_pending: false,
                    legacy_snapshot_id: None,
                    legacy_cleanup_allowed: false,
                    error: None,
                },
                configuration: ConnectionConfiguration::default(),
                owner_id: "disconnect".to_owned(),
                candidate: None,
                parts: Vec::new(),
                legacy_cleaned: false,
            });
            recompute_cleanup(ledger);
            Ok((
                find_operation(ledger, &request.operation_id)?
                    .receipt
                    .clone(),
                Some(superseded),
            ))
        })?;
        self.mutations.drain(
            Some(request.provider),
            next_revision(request.expected_revision)?,
            Instant::now() + ACQUIRE_BUDGET,
        )?;
        if superseded.is_none() {
            // An explicit replay retries only the old generation cleanup; it never re-fences
            // current input or increments the tombstone revision.
            let _ = self.cleanup_candidates_through(
                request.provider,
                next_revision(request.expected_revision)?,
            );
            return self
                .operation(CredentialOperationRequest {
                    operation_id: request.operation_id,
                    profile_id: request.profile_id,
                    provider: request.provider,
                })
                .or(Ok(receipt));
        }
        let superseded = superseded.unwrap_or_default();
        // Cancel input/HTTP promptly after the durable fence, before waiting behind any OS call.
        for ((provider, operation), owner) in self
            .owners
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?
            .iter()
        {
            if *provider == request.provider && superseded.contains(operation) {
                owner.cancel();
            }
        }
        // Failure retains a successful tombstone and an explicit cleanupPending receipt.
        let _ = self.cleanup_candidates_through(
            request.provider,
            next_revision(request.expected_revision)?,
        );
        self.operation(CredentialOperationRequest {
            operation_id: request.operation_id,
            profile_id: request.profile_id,
            provider: request.provider,
        })
        .or(Ok(receipt))
    }

    fn load(&self, provider: Provider) -> CredentialResult<CredentialLedger> {
        let ledger = self.ledger.load(provider)?;
        ledger.validate()?;
        if ledger.profile_id != self.profile_id || ledger.provider != provider {
            return Err(ProviderCredentialError::Corrupt);
        }
        Ok(ledger)
    }

    fn change<T>(
        &self,
        provider: Provider,
        update: impl Fn(&mut CredentialLedger) -> CredentialResult<T>,
    ) -> CredentialResult<T> {
        self.change_guarded(provider, update, &|| true)
    }

    fn change_guarded<T>(
        &self,
        provider: Provider,
        update: impl Fn(&mut CredentialLedger) -> CredentialResult<T>,
        publish_allowed: &dyn Fn() -> bool,
    ) -> CredentialResult<T> {
        for _ in 0..CAS_ATTEMPTS {
            let previous = self.load(provider)?;
            let mut next = previous.clone();
            let result = update(&mut next)?;
            if next == previous {
                return Ok(result);
            }
            next.row_version = next_revision(previous.row_version)?;
            next.validate()?;
            let exchange = || {
                self.ledger
                    .compare_exchange_guarded(&previous, &next, publish_allowed)
            };
            let changed = if previous.active != next.active {
                self.mutations.with_connection_change(exchange)?
            } else {
                exchange()?
            };
            if changed {
                let _ = self.changed.send(ProviderCredentialChanged {
                    provider,
                    profile_id: self.profile_id.clone(),
                    revision: next.revision,
                });
                return Ok(result);
            }
            std::thread::yield_now();
        }
        Err(ProviderCredentialError::Busy)
    }

    fn validate_intent(
        &self,
        profile: &str,
        operation: &str,
        revision: u64,
    ) -> CredentialResult<()> {
        if profile != self.profile_id
            || !valid_profile_id(profile)
            || !valid_operation_id(operation)
            || !valid_revision(revision)
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        Ok(())
    }

    fn validate_connect(&self, request: &ConnectRequest) -> CredentialResult<()> {
        self.validate_intent(
            &request.profile_id,
            &request.operation_id,
            request.expected_revision,
        )?;
        request.configuration.normalized(request.provider)?;
        Ok(())
    }

    fn validate_operation_request(
        &self,
        request: &CredentialOperationRequest,
    ) -> CredentialResult<()> {
        self.validate_intent(&request.profile_id, &request.operation_id, 0)
    }

    fn begin(
        &self,
        request: &ConnectRequest,
        kind: CredentialOperationKind,
        snapshot: Option<&str>,
        owner: &NativeOperationOwner,
    ) -> CredentialResult<(OperationReceipt, bool)> {
        if !owner.is_live() {
            return Err(ProviderCredentialError::Cancelled);
        }
        self.change_guarded(
            request.provider,
            |ledger| {
                if let Ok(operation) = find_operation(ledger, &request.operation_id) {
                    if operation.receipt.kind != kind
                        || operation.receipt.expected_revision != request.expected_revision
                        || operation.configuration != request.configuration
                        || operation.receipt.legacy_snapshot_id.as_deref() != snapshot
                    {
                        return Err(ProviderCredentialError::Conflict);
                    }
                    return Ok((operation.receipt.clone(), false));
                }
                if self.closing.load(Ordering::SeqCst) {
                    return Err(ProviderCredentialError::Cancelled);
                }
                if ledger.revision != request.expected_revision {
                    return Err(ProviderCredentialError::Conflict);
                }
                if kind == CredentialOperationKind::LegacyImport
                    && (ledger.revision != 0 || ledger.tombstone || ledger.active.is_some())
                {
                    return Err(ProviderCredentialError::Conflict);
                }
                if ledger
                    .operations
                    .iter()
                    .any(|operation| operation.receipt.status.is_pending())
                {
                    return Err(ProviderCredentialError::Busy);
                }
                ensure_history_capacity(ledger)?;
                let receipt = OperationReceipt {
                    operation_id: request.operation_id.clone(),
                    profile_id: self.profile_id.clone(),
                    provider: request.provider,
                    expected_revision: request.expected_revision,
                    kind,
                    status: CredentialOperationStatus::AwaitingInput,
                    connection: None,
                    cleanup_pending: false,
                    legacy_snapshot_id: snapshot.map(str::to_owned),
                    legacy_cleanup_allowed: false,
                    error: None,
                };
                ledger.operations.push(StoredOperation {
                    receipt: receipt.clone(),
                    configuration: request.configuration.clone(),
                    owner_id: owner.id().to_owned(),
                    candidate: None,
                    parts: Vec::new(),
                    legacy_cleaned: false,
                });
                recompute_cleanup(ledger);
                Ok((receipt, true))
            },
            &|| owner.is_live() && !self.closing.load(Ordering::SeqCst),
        )
    }

    fn insert_owner(
        &self,
        request: &ConnectRequest,
        owner: &NativeOperationOwner,
    ) -> CredentialResult<()> {
        self.owners
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?
            .insert(
                (request.provider, request.operation_id.clone()),
                owner.clone(),
            );
        Ok(())
    }

    fn guard_operation(
        &self,
        request: &ConnectRequest,
        owner: &NativeOperationOwner,
    ) -> CredentialResult<()> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(ProviderCredentialError::Cancelled);
        }
        let ledger = self.load(request.provider)?;
        check_pending(&ledger, request, owner)
    }

    fn claim_verification(
        &self,
        request: &ConnectRequest,
        owner: &NativeOperationOwner,
        phase: CredentialOperationStatus,
    ) -> CredentialResult<bool> {
        self.change(request.provider, |ledger| {
            check_pending(ledger, request, owner)?;
            let operation = find_operation_mut(ledger, &request.operation_id)?;
            if operation.receipt.status != phase {
                return Ok(false);
            }
            operation.receipt.status = CredentialOperationStatus::Verifying;
            Ok(true)
        })
    }

    fn accept_secret(
        &self,
        request: &ConnectRequest,
        token: SecretToken,
        owner: &NativeOperationOwner,
    ) -> CredentialResult<()> {
        self.guard_operation(request, owner)?;
        let metadata =
            self.verifier
                .verify(request, &token, owner.cancellation(), owner.deadline())?;
        metadata.validate(request.provider)?;
        if metadata.configuration != request.configuration {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        self.guard_operation(request, owner)?;
        let candidate = StoredConnection {
            summary: ConnectionSummary {
                reference: ConnectionRef {
                    profile_id: self.profile_id.clone(),
                    provider: request.provider,
                    connection_id: new_connection_id()?,
                    revision: next_revision(request.expected_revision)?,
                },
                metadata,
            },
            operation_id: request.operation_id.clone(),
            part_count: token.expose().len().div_ceil(SECRET_CHUNK_BYTES) as u16,
        };
        let capsules = encode_secret_capsules(&candidate, &token)?;
        self.change(request.provider, |ledger| {
            check_pending(ledger, request, owner)?;
            let operation = find_operation_mut(ledger, &request.operation_id)?;
            operation.receipt.status = CredentialOperationStatus::Storing;
            operation.candidate = Some(candidate.clone());
            operation.parts = vec![SecretPartState::Prepared; usize::from(candidate.part_count)];
            Ok(())
        })?;
        for (index, capsule) in capsules.iter().enumerate() {
            self.change(request.provider, |ledger| {
                check_pending(ledger, request, owner)?;
                let operation = find_operation_mut(ledger, &request.operation_id)?;
                if operation.candidate.as_ref() != Some(&candidate)
                    || operation.parts[index] != SecretPartState::Prepared
                {
                    return Err(ProviderCredentialError::Conflict);
                }
                // Persist before submitting to the OS. A crash can never misclassify a late write
                // as an unsent part, including an error or lost response from the OS API.
                operation.parts[index] = SecretPartState::Submitted;
                Ok(())
            })?;
            if let Err(error) = self.guard_operation(request, owner) {
                self.change(request.provider, |ledger| {
                    let operation = find_operation_mut(ledger, &request.operation_id)?;
                    if operation.candidate.as_ref() == Some(&candidate)
                        && operation.parts[index] == SecretPartState::Submitted
                    {
                        operation.parts[index] = SecretPartState::Prepared;
                    }
                    recompute_cleanup(ledger);
                    Ok(())
                })?;
                return Err(error);
            }
            let part = part_key(&candidate, index as u16);
            self.key_store.write_part_until(
                &part,
                capsule,
                owner.cancellation_flag(),
                owner.deadline(),
            )?;
            let readback = self
                .key_store
                .read_part_until(&part, owner.cancellation_flag(), owner.deadline())?
                .ok_or(ProviderCredentialError::OutcomeUnknown)?;
            if readback.as_slice() != capsule.as_slice() {
                return Err(ProviderCredentialError::Corrupt);
            }
            self.change(request.provider, |ledger| {
                let operation = find_operation_mut(ledger, &request.operation_id)?;
                if operation.candidate.as_ref() != Some(&candidate) {
                    return Err(ProviderCredentialError::Conflict);
                }
                // Completion is recorded even after cancellation; it makes eventual deletion
                // provable without granting the cancelled operation permission to publish.
                if operation.parts[index] == SecretPartState::Submitted {
                    operation.parts[index] = SecretPartState::Verified;
                }
                recompute_cleanup(ledger);
                Ok(())
            })?;
        }
        self.change_guarded(
            request.provider,
            |ledger| {
                check_pending(ledger, request, owner)?;
                let operation = find_operation_mut(ledger, &request.operation_id)?;
                if operation.receipt.status != CredentialOperationStatus::Storing
                    || operation.candidate.as_ref() != Some(&candidate)
                    || !operation
                        .parts
                        .iter()
                        .all(|part| *part == SecretPartState::Verified)
                {
                    return Err(ProviderCredentialError::Conflict);
                }
                operation.receipt.status = CredentialOperationStatus::Committed;
                operation.receipt.connection = Some(candidate.summary.reference.clone());
                operation.receipt.legacy_cleanup_allowed =
                    operation.receipt.kind == CredentialOperationKind::LegacyImport;
                operation.receipt.error = None;
                ledger.revision = candidate.summary.reference.revision;
                ledger.active = Some(candidate.clone());
                recompute_cleanup(ledger);
                Ok(())
            },
            &|| owner.is_live() && !self.closing.load(Ordering::SeqCst),
        )
    }

    fn finish_attempt(
        &self,
        request: &ConnectRequest,
        owner: &NativeOperationOwner,
        result: CredentialResult<()>,
    ) -> CredentialResult<OperationReceipt> {
        if let Err(error) = result {
            self.change(request.provider, |ledger| {
                let operation = find_operation_mut(ledger, &request.operation_id)?;
                if operation.receipt.status.is_pending() {
                    let unknown = operation.parts.contains(&SecretPartState::Submitted);
                    operation.receipt.status =
                        if !owner.is_live() || error == ProviderCredentialError::Cancelled {
                            CredentialOperationStatus::Cancelled
                        } else if unknown {
                            CredentialOperationStatus::OutcomeUnknown
                        } else {
                            CredentialOperationStatus::Failed
                        };
                    operation.receipt.error = Some(error);
                }
                recompute_cleanup(ledger);
                Ok(())
            })?;
        }
        self.owners
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?
            .remove(&(request.provider, request.operation_id.clone()));
        let _ = self.cleanup_candidates(request.provider);
        self.operation(CredentialOperationRequest {
            operation_id: request.operation_id.clone(),
            profile_id: self.profile_id.clone(),
            provider: request.provider,
        })
    }

    fn cleanup_candidates(&self, provider: Provider) -> CredentialResult<()> {
        self.cleanup_candidates_through(provider, MAX_SAFE_REVISION)
    }

    fn cleanup_candidates_through(
        &self,
        provider: Provider,
        through_revision: u64,
    ) -> CredentialResult<()> {
        let deadline = Instant::now() + CLEANUP_BUDGET;
        let mut calls = 0usize;
        let snapshot = self.load(provider)?;
        for operation in &snapshot.operations {
            let Some(candidate) = operation.candidate.as_ref() else {
                continue;
            };
            if operation.receipt.status.is_pending()
                || snapshot.active.as_ref() == Some(candidate)
                || candidate.summary.reference.revision > through_revision
            {
                continue;
            }
            for (index, state) in operation.parts.iter().enumerate() {
                if matches!(state, SecretPartState::Prepared | SecretPartState::Deleted) {
                    continue;
                }
                // Recheck before every OS effect. New connections always use other random slots.
                let latest = self.load(provider)?;
                if latest.active.as_ref() == Some(candidate) {
                    continue;
                }
                let part = part_key(candidate, index as u16);
                cleanup_budget(&self.closing, deadline, &mut calls)?;
                match self
                    .key_store
                    .read_part_until(&part, self.closing.clone(), deadline)
                {
                    Ok(Some(capsule)) => {
                        if !capsule_has_binding(candidate, index as u16, &capsule)? {
                            return Err(ProviderCredentialError::Corrupt);
                        }
                        self.mark_retired_part(candidate, index, SecretPartState::Verified)?;
                        cleanup_budget(&self.closing, deadline, &mut calls)?;
                        self.key_store
                            .delete_part_until(&part, self.closing.clone(), deadline)?;
                        cleanup_budget(&self.closing, deadline, &mut calls)?;
                        if self
                            .key_store
                            .read_part_until(&part, self.closing.clone(), deadline)?
                            .is_none()
                        {
                            self.mark_retired_part(candidate, index, SecretPartState::Deleted)?;
                        } else {
                            return Err(ProviderCredentialError::OutcomeUnknown);
                        }
                    }
                    Ok(None) if *state == SecretPartState::Verified => {
                        self.mark_retired_part(candidate, index, SecretPartState::Deleted)?;
                    }
                    // An unobserved Submitted write may still complete. Absence cannot settle it.
                    Ok(None) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        self.change(provider, |ledger| {
            recompute_cleanup(ledger);
            Ok(())
        })
    }

    fn mark_retired_part(
        &self,
        candidate: &StoredConnection,
        index: usize,
        state: SecretPartState,
    ) -> CredentialResult<()> {
        self.change(candidate.summary.reference.provider, |ledger| {
            if ledger.active.as_ref() == Some(candidate) {
                return Err(ProviderCredentialError::Conflict);
            }
            let operation = find_operation_mut(ledger, &candidate.operation_id)?;
            if operation.candidate.as_ref() != Some(candidate)
                || operation.receipt.status.is_pending()
            {
                return Err(ProviderCredentialError::Conflict);
            }
            if operation.parts[index] != SecretPartState::Deleted {
                operation.parts[index] = state;
            }
            recompute_cleanup(ledger);
            Ok(())
        })
    }
}

impl ProviderCredentialReadPort for ProviderCredentialService {
    fn current_state(&self, provider: Provider) -> CredentialResult<ProviderCredentialState> {
        self.state(provider)
    }

    fn validate_current(&self, expected: &ConnectionRef) -> CredentialResult<()> {
        if self.closing.load(Ordering::SeqCst) {
            return Err(ProviderCredentialError::Cancelled);
        }
        if expected.profile_id != self.profile_id
            || !valid_connection_id(&expected.connection_id)
            || !valid_revision(expected.revision)
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let ledger = self.load(expected.provider)?;
        if ledger
            .active
            .as_ref()
            .map(|connection| &connection.summary.reference)
            != Some(expected)
        {
            return Err(ProviderCredentialError::Conflict);
        }
        Ok(())
    }

    fn acquire(&self, expected: &ConnectionRef) -> CredentialResult<ProviderCredentialLease> {
        self.validate_current(expected)?;
        let ledger = self.load(expected.provider)?;
        let active = ledger.active.ok_or(ProviderCredentialError::NotConnected)?;
        if &active.summary.reference != expected {
            return Err(ProviderCredentialError::Conflict);
        }
        let deadline = Instant::now() + ACQUIRE_BUDGET;
        let mut capsules = Vec::with_capacity(usize::from(active.part_count));
        for index in 0..active.part_count {
            self.validate_current(expected)?;
            if Instant::now() >= deadline {
                return Err(ProviderCredentialError::Cancelled);
            }
            let read = self.key_store.read_part_until(
                &part_key(&active, index),
                self.closing.clone(),
                deadline,
            );
            self.validate_current(expected)?;
            capsules.push(read?.ok_or(ProviderCredentialError::Unavailable)?);
        }
        let token = decode_secret_capsules(&active, &capsules)?;
        self.validate_current(expected)?;
        Ok(ProviderCredentialLease {
            reference: expected.clone(),
            metadata: active.summary.metadata,
            token,
        })
    }
}

impl ProviderCredentialMigrationPort for ProviderCredentialService {
    fn prepare_import(
        &self,
        mut offer: LegacyImportOffer,
        owner: NativeOperationOwner,
    ) -> CredentialResult<OperationReceipt> {
        self.validate_connect(&offer.connect_request())?;
        if !valid_snapshot_id(&offer.snapshot_id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        offer.configuration = offer.configuration.normalized(offer.provider)?;
        let request = offer.connect_request();
        let (receipt, created) = self.begin(
            &request,
            CredentialOperationKind::LegacyImport,
            Some(&offer.snapshot_id),
            &owner,
        )?;
        if !created {
            // A prepared receipt is a ticket for the original private session only. A new
            // helper may inspect terminal receipts, but cannot reuse another owner\'s consent.
            if receipt.status == CredentialOperationStatus::Prepared {
                self.guard_operation(&request, &owner)?;
            }
            return Ok(receipt);
        }
        self.insert_owner(&request, &owner)?;
        let result = (|| {
            self.guard_operation(&request, &owner)?;
            if !self.prompt.supported() || !self.key_store.supported() {
                return Err(ProviderCredentialError::Unsupported);
            }
            if !self
                .prompt
                .confirm_migration(&offer, owner.cancellation(), owner.deadline())?
            {
                return Err(ProviderCredentialError::Cancelled);
            }
            self.change(offer.provider, |ledger| {
                check_pending(ledger, &request, &owner)?;
                find_operation_mut(ledger, &offer.operation_id)?
                    .receipt
                    .status = CredentialOperationStatus::Prepared;
                Ok(())
            })
        })();
        if result.is_err() {
            return self.finish_attempt(&request, &owner, result);
        }
        self.operation(CredentialOperationRequest {
            operation_id: offer.operation_id,
            profile_id: offer.profile_id,
            provider: offer.provider,
        })
    }

    fn import_secret(
        &self,
        mut offer: LegacyImportOffer,
        token: SecretToken,
        owner: NativeOperationOwner,
    ) -> CredentialResult<OperationReceipt> {
        self.validate_connect(&offer.connect_request())?;
        if !valid_snapshot_id(&offer.snapshot_id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        offer.configuration = offer.configuration.normalized(offer.provider)?;
        let request = offer.connect_request();
        let ledger = self.load(offer.provider)?;
        let operation = find_operation(&ledger, &offer.operation_id)?;
        if operation.receipt.kind != CredentialOperationKind::LegacyImport
            || operation.receipt.legacy_snapshot_id.as_deref() != Some(offer.snapshot_id.as_str())
            || operation.receipt.expected_revision != offer.expected_revision
            || operation.configuration != offer.configuration
        {
            return Err(ProviderCredentialError::Conflict);
        }
        if operation.receipt.status == CredentialOperationStatus::Committed {
            return Ok(operation.receipt.clone());
        }
        if operation.receipt.status != CredentialOperationStatus::Prepared
            || operation.owner_id != owner.id()
        {
            return Err(ProviderCredentialError::Conflict);
        }
        if !self.claim_verification(&request, &owner, CredentialOperationStatus::Prepared)? {
            return self.operation(CredentialOperationRequest {
                operation_id: offer.operation_id,
                profile_id: offer.profile_id,
                provider: offer.provider,
            });
        }
        let result = self.accept_secret(&request, token, &owner);
        self.finish_attempt(&request, &owner, result)
    }

    fn migration_operation(
        &self,
        request: CredentialOperationRequest,
    ) -> CredentialResult<OperationReceipt> {
        self.operation(request)
    }

    fn acknowledge_legacy_cleanup(
        &self,
        request: LegacyCleanupRequest,
    ) -> CredentialResult<OperationReceipt> {
        self.validate_intent(&request.profile_id, &request.operation_id, 0)?;
        if !valid_snapshot_id(&request.snapshot_id) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        self.change(request.provider, |ledger| {
            let operation = find_operation_mut(ledger, &request.operation_id)?;
            if operation.receipt.kind != CredentialOperationKind::LegacyImport
                || operation.receipt.status != CredentialOperationStatus::Committed
                || operation.receipt.legacy_snapshot_id.as_deref()
                    != Some(request.snapshot_id.as_str())
            {
                return Err(ProviderCredentialError::Conflict);
            }
            if !operation.legacy_cleaned && !operation.receipt.legacy_cleanup_allowed {
                return Err(ProviderCredentialError::Conflict);
            }
            operation.legacy_cleaned = true;
            operation.receipt.legacy_cleanup_allowed = false;
            recompute_cleanup(ledger);
            Ok(find_operation(ledger, &request.operation_id)?
                .receipt
                .clone())
        })
    }

    fn cancel_owner(&self, owner: &NativeOperationOwner) -> CredentialResult<()> {
        owner.cancel();
        for provider in [Provider::Jira, Provider::Linear] {
            self.change(provider, |ledger| {
                for operation in &mut ledger.operations {
                    if operation.owner_id == owner.id() && operation.receipt.status.is_pending() {
                        operation.receipt.status = CredentialOperationStatus::Cancelled;
                        operation.receipt.error = Some(ProviderCredentialError::Cancelled);
                    }
                }
                recompute_cleanup(ledger);
                Ok(())
            })?;
        }
        self.owners
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?
            .retain(|_, saved| saved.id() != owner.id());
        Ok(())
    }
}

fn find_operation<'a>(
    ledger: &'a CredentialLedger,
    id: &str,
) -> CredentialResult<&'a StoredOperation> {
    ledger
        .operations
        .iter()
        .find(|operation| operation.receipt.operation_id == id)
        .ok_or(ProviderCredentialError::NotFound)
}
fn find_operation_mut<'a>(
    ledger: &'a mut CredentialLedger,
    id: &str,
) -> CredentialResult<&'a mut StoredOperation> {
    ledger
        .operations
        .iter_mut()
        .find(|operation| operation.receipt.operation_id == id)
        .ok_or(ProviderCredentialError::NotFound)
}
fn check_pending(
    ledger: &CredentialLedger,
    request: &ConnectRequest,
    owner: &NativeOperationOwner,
) -> CredentialResult<()> {
    if !owner.is_live() {
        return Err(ProviderCredentialError::Cancelled);
    }
    let operation = find_operation(ledger, &request.operation_id)?;
    if ledger.revision != request.expected_revision
        || !operation.receipt.status.is_pending()
        || operation.owner_id != owner.id()
        || operation.configuration != request.configuration
    {
        return Err(ProviderCredentialError::Conflict);
    }
    Ok(())
}
fn next_revision(value: u64) -> CredentialResult<u64> {
    value
        .checked_add(1)
        .filter(|next| valid_revision(*next))
        .ok_or(ProviderCredentialError::CapacityExceeded)
}
fn ensure_history_capacity(ledger: &CredentialLedger) -> CredentialResult<()> {
    if ledger.operations.len() >= MAX_OPERATIONS - 1 {
        Err(ProviderCredentialError::CapacityExceeded)
    } else {
        Ok(())
    }
}
fn new_connection_id() -> CredentialResult<String> {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| ProviderCredentialError::Unavailable)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn part_key(connection: &StoredConnection, index: u16) -> KeyringPart {
    KeyringPart {
        profile_id: connection.summary.reference.profile_id.clone(),
        provider: connection.summary.reference.provider,
        connection_id: connection.summary.reference.connection_id.clone(),
        index,
    }
}
fn recompute_cleanup(ledger: &mut CredentialLedger) {
    let active_id = ledger
        .active
        .as_ref()
        .map(|connection| connection.summary.reference.connection_id.as_str());
    let mut any_pending = false;
    let mut pending_revisions = Vec::new();
    for operation in &mut ledger.operations {
        operation.receipt.legacy_cleanup_allowed = operation.receipt.kind
            == CredentialOperationKind::LegacyImport
            && operation.receipt.status == CredentialOperationStatus::Committed
            && !operation.legacy_cleaned;
        let retired = operation.candidate.as_ref().is_some_and(|candidate| {
            Some(candidate.summary.reference.connection_id.as_str()) != active_id
        }) && !operation.receipt.status.is_pending();
        if retired {
            for part in &mut operation.parts {
                if *part == SecretPartState::Prepared {
                    *part = SecretPartState::Deleted;
                }
            }
        }
        operation.receipt.cleanup_pending = (retired
            && operation
                .parts
                .iter()
                .any(|part| *part != SecretPartState::Deleted))
            || (operation.receipt.legacy_cleanup_allowed && !operation.legacy_cleaned);
        if retired
            && operation
                .parts
                .iter()
                .any(|part| *part != SecretPartState::Deleted)
            && let Some(candidate) = &operation.candidate
        {
            pending_revisions.push(candidate.summary.reference.revision);
        }
        any_pending |= operation.receipt.cleanup_pending;
    }
    for operation in &mut ledger.operations {
        if operation.receipt.kind == CredentialOperationKind::Disconnect {
            operation.receipt.cleanup_pending = pending_revisions
                .iter()
                .any(|revision| *revision <= operation.receipt.expected_revision.saturating_add(1));
        }
    }
    if let Some(last) = ledger.operations.last_mut() {
        last.receipt.cleanup_pending |= any_pending;
    }
}

fn cleanup_budget(
    closing: &AtomicBool,
    deadline: Instant,
    calls: &mut usize,
) -> CredentialResult<()> {
    if closing.load(Ordering::SeqCst) || Instant::now() >= deadline || *calls >= MAX_CLEANUP_CALLS {
        return Err(ProviderCredentialError::Cancelled);
    }
    *calls += 1;
    Ok(())
}

#[cfg(test)]
mod tests;
