//! Nonsecret request and result header used only by the owned native prompt child.
use crate::domain::provider_credentials::{
    ConnectRequest, LegacyImportOffer, Provider, ProviderCredentialError,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum PromptRequest {
    Token {
        version: u8,
        request: ConnectRequest,
    },
    MigrationConsent {
        version: u8,
        offer: LegacyImportOffer,
    },
}
impl PromptRequest {
    pub(crate) fn version(&self) -> u8 {
        match self {
            Self::Token { version, .. } | Self::MigrationConsent { version, .. } => *version,
        }
    }
    pub(crate) fn provider(&self) -> Provider {
        match self {
            Self::Token { request, .. } => request.provider,
            Self::MigrationConsent { offer, .. } => offer.provider,
        }
    }
    pub(crate) fn operation_id(&self) -> &str {
        match self {
            Self::Token { request, .. } => &request.operation_id,
            Self::MigrationConsent { offer, .. } => &offer.operation_id,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum PromptResponse {
    Submitted {
        version: u8,
        operation_id: String,
    },
    Confirmed {
        version: u8,
        operation_id: String,
    },
    Rejected {
        version: u8,
        operation_id: String,
        error: ProviderCredentialError,
    },
}
