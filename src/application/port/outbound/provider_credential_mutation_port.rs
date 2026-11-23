//! A current credential reference is not a send permit. Mutations also need a one-shot
//! admission at the transport's final send boundary, retained until its outcome is known.
use std::sync::Arc;

use crate::application::port::outbound::provider_credentials_port::ProviderCredentialReadPort;
use crate::domain::provider_credentials::{ConnectionRef, CredentialResult};

/// Owned, non-serializable admission. Dropping it releases one in-flight mutation.
pub trait ProviderMutationSendGuardPort: Send {}

pub trait ProviderMutationAuthorizationPort: Send + Sync {
    /// May succeed once. The transport calls this immediately before its sole HTTP send.
    fn begin_send(&self) -> CredentialResult<Box<dyn ProviderMutationSendGuardPort>>;
}

pub trait ProviderCredentialMutationPort: ProviderCredentialReadPort {
    fn authorize_mutation(
        &self,
        expected: &ConnectionRef,
    ) -> CredentialResult<Arc<dyn ProviderMutationAuthorizationPort>>;
}
