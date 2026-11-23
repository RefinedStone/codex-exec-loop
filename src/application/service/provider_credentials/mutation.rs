//! The profile owner serializes connection publication with the final HTTP admission.
//! Network I/O holds an owned count, never this mutex.
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use crate::application::port::outbound::provider_credential_mutation_port::{
    ProviderCredentialMutationPort, ProviderMutationAuthorizationPort,
    ProviderMutationSendGuardPort,
};
use crate::application::port::outbound::provider_credentials_port::{
    ProviderCredentialLedgerPort, ProviderCredentialReadPort,
};
use crate::domain::provider_credentials::*;

use super::ProviderCredentialService;

const MAX_IN_FLIGHT: usize = 256;

pub(super) struct MutationAdmission {
    profile_id: String,
    ledger: Arc<dyn ProviderCredentialLedgerPort>,
    closing: Arc<AtomicBool>,
    state: Mutex<Vec<ConnectionRef>>,
    drained: Condvar,
}

impl MutationAdmission {
    pub(super) fn new(
        profile_id: String,
        ledger: Arc<dyn ProviderCredentialLedgerPort>,
        closing: Arc<AtomicBool>,
    ) -> Self {
        Self {
            profile_id,
            ledger,
            closing,
            state: Mutex::new(Vec::new()),
            drained: Condvar::new(),
        }
    }

    pub(super) fn with_connection_change<T>(
        &self,
        operation: impl FnOnce() -> CredentialResult<T>,
    ) -> CredentialResult<T> {
        let _guard = self
            .state
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        operation()
    }

    pub(super) fn close(&self) -> CredentialResult<()> {
        // Signal first so an owner waiting inside a guarded CAS cannot publish late.
        self.closing.store(true, Ordering::SeqCst);
        let _guard = self
            .state
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        Ok(())
    }

    pub(super) fn drain(
        &self,
        provider: Option<Provider>,
        before_revision: u64,
        deadline: Instant,
    ) -> CredentialResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        while state.iter().any(|reference| {
            provider.is_none_or(|provider| reference.provider == provider)
                && reference.revision < before_revision
        }) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ProviderCredentialError::OutcomeUnknown);
            }
            let (next, result) = self
                .drained
                .wait_timeout(state, remaining)
                .map_err(|_| ProviderCredentialError::Unavailable)?;
            state = next;
            if result.timed_out()
                && state.iter().any(|reference| {
                    provider.is_none_or(|provider| reference.provider == provider)
                        && reference.revision < before_revision
                })
            {
                return Err(ProviderCredentialError::OutcomeUnknown);
            }
        }
        Ok(())
    }

    fn begin(self: &Arc<Self>, expected: &ConnectionRef) -> CredentialResult<MutationSendGuard> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProviderCredentialError::Unavailable)?;
        if self.closing.load(Ordering::SeqCst) {
            return Err(ProviderCredentialError::Cancelled);
        }
        if expected.profile_id != self.profile_id {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let ledger = self.ledger.load(expected.provider)?;
        ledger.validate()?;
        if ledger.profile_id != self.profile_id
            || ledger.provider != expected.provider
            || ledger
                .active
                .as_ref()
                .map(|active| &active.summary.reference)
                != Some(expected)
        {
            return Err(ProviderCredentialError::Conflict);
        }
        if self.closing.load(Ordering::SeqCst) {
            return Err(ProviderCredentialError::Cancelled);
        }
        if state.len() >= MAX_IN_FLIGHT {
            return Err(ProviderCredentialError::Busy);
        }
        state.push(expected.clone());
        Ok(MutationSendGuard {
            admission: self.clone(),
            reference: expected.clone(),
        })
    }
}

struct MutationAuthorization {
    admission: Arc<MutationAdmission>,
    expected: ConnectionRef,
    used: AtomicBool,
}
impl ProviderMutationAuthorizationPort for MutationAuthorization {
    fn begin_send(&self) -> CredentialResult<Box<dyn ProviderMutationSendGuardPort>> {
        if self.used.swap(true, Ordering::SeqCst) {
            return Err(ProviderCredentialError::Conflict);
        }
        Ok(Box::new(self.admission.begin(&self.expected)?))
    }
}

struct MutationSendGuard {
    admission: Arc<MutationAdmission>,
    reference: ConnectionRef,
}
impl ProviderMutationSendGuardPort for MutationSendGuard {}
impl Drop for MutationSendGuard {
    fn drop(&mut self) {
        // A poisoned owner remains closed to new admissions; release still permits shutdown.
        let mut state = self
            .admission
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(index) = state
            .iter()
            .position(|reference| reference == &self.reference)
        {
            state.swap_remove(index);
        }
        self.admission.drained.notify_all();
    }
}

impl ProviderCredentialMutationPort for ProviderCredentialService {
    fn authorize_mutation(
        &self,
        expected: &ConnectionRef,
    ) -> CredentialResult<Arc<dyn ProviderMutationAuthorizationPort>> {
        self.validate_current(expected)?;
        Ok(Arc::new(MutationAuthorization {
            admission: self.mutations.clone(),
            expected: expected.clone(),
            used: AtomicBool::new(false),
        }))
    }
}
