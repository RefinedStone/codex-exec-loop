//! Observe the whole blocking restore operation independently of pipe lifetime.
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::oneshot;

use crate::application::port::inbound::vault_restore_session_port::{
    VaultRestoreReconnectRequest, VaultRestoreSessionFailure, VaultRestoreSessionHold,
    VaultRestoreSessionPort, VaultRestoreSessionPrepared, VaultRestoreSessionRequest,
    VaultRestoreSessionReview,
};
use crate::application::port::outbound::evidence_vault_port::VaultRestoreResponse;

type RestoreOutcome = Result<(), VaultRestoreSessionFailure>;

pub(super) struct RestoreCompletionPort {
    inner: Arc<dyn VaultRestoreSessionPort>,
    completed: Mutex<Option<oneshot::Sender<RestoreOutcome>>>,
}

impl RestoreCompletionPort {
    pub(super) fn observe(
        inner: Arc<dyn VaultRestoreSessionPort>,
    ) -> (Arc<Self>, oneshot::Receiver<RestoreOutcome>) {
        let (sender, receiver) = oneshot::channel();
        (
            Arc::new(Self {
                inner,
                completed: Mutex::new(Some(sender)),
            }),
            receiver,
        )
    }
}

impl VaultRestoreSessionPort for RestoreCompletionPort {
    fn prepare(
        &self,
        request: VaultRestoreSessionRequest,
    ) -> Result<VaultRestoreSessionPrepared, VaultRestoreSessionHold> {
        self.inner.prepare(request)
    }

    fn restore(&self) -> Result<VaultRestoreResponse, VaultRestoreSessionFailure> {
        let result = self.inner.restore();
        if let Some(completed) = self.completed.lock().unwrap().take() {
            let _ = completed.send(result.as_ref().map(|_| ()).map_err(|failure| *failure));
        }
        result
    }

    fn review(&self) -> Result<VaultRestoreSessionReview, VaultRestoreSessionHold> {
        self.inner.review()
    }

    fn reconnect(
        &self,
        request: VaultRestoreReconnectRequest,
    ) -> Result<(), VaultRestoreSessionHold> {
        self.inner.reconnect(request)
    }

    fn cancel(&self) {
        self.inner.cancel();
    }

    fn is_live(&self) -> bool {
        self.inner.is_live()
    }

    fn deadline(&self) -> Instant {
        self.inner.deadline()
    }
}
