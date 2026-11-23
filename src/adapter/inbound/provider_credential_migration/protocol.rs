//! Nonsecret headers for a separate credential migration endpoint. Secret bytes have no JSON DTO.
use crate::domain::provider_credentials::*;
use serde::{Deserialize, Serialize};
pub(crate) const VERSION: u8 = 1;

#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "command",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum Request {
    Offer {
        version: u8,
        id: String,
        profile_id: String,
        operation_id: String,
        provider: Provider,
        expected_revision: u64,
        configuration: ConnectionConfiguration,
        snapshot_id: String,
    },
    Operation {
        version: u8,
        id: String,
        profile_id: String,
        operation_id: String,
        provider: Provider,
    },
    Cleanup {
        version: u8,
        id: String,
        profile_id: String,
        operation_id: String,
        provider: Provider,
        snapshot_id: String,
    },
    ValidateLink {
        version: u8,
        id: String,
        reference: ConnectionRef,
        url: String,
    },
}
impl Request {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Offer { id, .. }
            | Self::Operation { id, .. }
            | Self::Cleanup { id, .. }
            | Self::ValidateLink { id, .. } => id,
        }
    }
    pub(crate) fn profile_id(&self) -> &str {
        match self {
            Self::Offer { profile_id, .. }
            | Self::Operation { profile_id, .. }
            | Self::Cleanup { profile_id, .. } => profile_id,
            Self::ValidateLink { reference, .. } => &reference.profile_id,
        }
    }
    pub(crate) fn operation_id(&self) -> Option<&str> {
        match self {
            Self::Offer { operation_id, .. }
            | Self::Operation { operation_id, .. }
            | Self::Cleanup { operation_id, .. } => Some(operation_id),
            Self::ValidateLink { .. } => None,
        }
    }
    pub(crate) fn provider(&self) -> Provider {
        match self {
            Self::Offer { provider, .. }
            | Self::Operation { provider, .. }
            | Self::Cleanup { provider, .. } => *provider,
            Self::ValidateLink { reference, .. } => reference.provider,
        }
    }
    pub(crate) fn validate(&self, profile: &str) -> CredentialResult<()> {
        let version = match self {
            Self::Offer { version, .. }
            | Self::Operation { version, .. }
            | Self::Cleanup { version, .. }
            | Self::ValidateLink { version, .. } => *version,
        };
        if version != VERSION
            || !valid_operation_id(self.id())
            || self.profile_id() != profile
            || !valid_profile_id(profile)
            || self
                .operation_id()
                .is_some_and(|id| !valid_operation_id(id))
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        match self {
            Self::Offer {
                expected_revision,
                configuration,
                snapshot_id,
                ..
            } if !valid_revision(*expected_revision)
                || !valid_snapshot_id(snapshot_id)
                || configuration.normalized(self.provider())? != *configuration =>
            {
                return Err(ProviderCredentialError::InvalidRequest);
            }
            Self::Cleanup { snapshot_id, .. } if !valid_snapshot_id(snapshot_id) => {
                return Err(ProviderCredentialError::InvalidRequest);
            }
            Self::ValidateLink { reference, url, .. }
                if !valid_connection_id(&reference.connection_id)
                    || !valid_revision(reference.revision)
                    || url.is_empty()
                    || url.len() > 4096
                    || url.chars().any(char::is_control) =>
            {
                return Err(ProviderCredentialError::InvalidRequest);
            }
            _ => {}
        }
        Ok(())
    }
    pub(crate) fn offer(&self) -> Option<LegacyImportOffer> {
        match self {
            Self::Offer {
                profile_id,
                operation_id,
                provider,
                expected_revision,
                configuration,
                snapshot_id,
                ..
            } => Some(LegacyImportOffer {
                profile_id: profile_id.clone(),
                operation_id: operation_id.clone(),
                provider: *provider,
                expected_revision: *expected_revision,
                configuration: configuration.clone(),
                snapshot_id: snapshot_id.clone(),
            }),
            _ => None,
        }
    }
    pub(crate) fn operation(&self) -> Option<CredentialOperationRequest> {
        Some(CredentialOperationRequest {
            profile_id: self.profile_id().into(),
            provider: self.provider(),
            operation_id: self.operation_id()?.into(),
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Hello {
    pub version: u8,
    pub profile_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum Response {
    Prepared {
        version: u8,
        id: String,
        profile_id: String,
        operation_id: String,
        expected_revision: u64,
    },
    Committed {
        version: u8,
        id: String,
        profile_id: String,
        operation_id: String,
        receipt: OperationReceipt,
    },
    Operation {
        version: u8,
        id: String,
        profile_id: String,
        operation_id: String,
        receipt: OperationReceipt,
    },
    Held {
        version: u8,
        id: String,
        reason: ProviderCredentialError,
    },
    Unknown {
        version: u8,
        id: String,
        profile_id: String,
        operation_id: String,
    },
    ValidatedLink {
        version: u8,
        id: String,
        url: String,
    },
}
impl Response {
    pub(crate) fn from_receipt(id: &str, receipt: OperationReceipt) -> Self {
        let profile_id = receipt.profile_id.clone();
        let operation_id = receipt.operation_id.clone();
        if receipt.status == CredentialOperationStatus::Committed && receipt.legacy_cleanup_allowed
        {
            Self::Committed {
                version: VERSION,
                id: id.into(),
                profile_id,
                operation_id,
                receipt,
            }
        } else {
            Self::Operation {
                version: VERSION,
                id: id.into(),
                profile_id,
                operation_id,
                receipt,
            }
        }
    }
    pub(crate) fn held(id: &str, reason: ProviderCredentialError) -> Self {
        Self::Held {
            version: VERSION,
            id: id.into(),
            reason,
        }
    }
    pub(crate) fn unknown(request: &Request) -> Self {
        Self::Unknown {
            version: VERSION,
            id: request.id().into(),
            profile_id: request.profile_id().into(),
            operation_id: request.operation_id().unwrap_or_default().into(),
        }
    }
    pub(crate) fn validate(&self, request: &Request) -> CredentialResult<()> {
        let (version, id) = match self {
            Self::Prepared { version, id, .. }
            | Self::Committed { version, id, .. }
            | Self::Operation { version, id, .. }
            | Self::Held { version, id, .. }
            | Self::Unknown { version, id, .. }
            | Self::ValidatedLink { version, id, .. } => (*version, id),
        };
        if version != VERSION || id != request.id() {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        match self {
            Self::Prepared {
                profile_id,
                operation_id,
                expected_revision,
                ..
            } => {
                let Some(offer) = request.offer() else {
                    return Err(ProviderCredentialError::InvalidRequest);
                };
                if profile_id != &offer.profile_id
                    || operation_id != &offer.operation_id
                    || *expected_revision != offer.expected_revision
                {
                    return Err(ProviderCredentialError::InvalidRequest);
                }
            }
            Self::Committed {
                profile_id,
                operation_id,
                receipt,
                ..
            }
            | Self::Operation {
                profile_id,
                operation_id,
                receipt,
                ..
            } => {
                if profile_id != request.profile_id()
                    || Some(operation_id.as_str()) != request.operation_id()
                    || receipt.profile_id != *profile_id
                    || receipt.operation_id != *operation_id
                    || receipt.provider != request.provider()
                {
                    return Err(ProviderCredentialError::InvalidRequest);
                }
                if matches!(self, Self::Committed { .. })
                    && (receipt.status != CredentialOperationStatus::Committed
                        || !receipt.legacy_cleanup_allowed)
                {
                    return Err(ProviderCredentialError::InvalidRequest);
                }
                if let Some(offer) = request.offer()
                    && (receipt.expected_revision != offer.expected_revision
                        || receipt.legacy_snapshot_id.as_deref()
                            != Some(offer.snapshot_id.as_str()))
                {
                    return Err(ProviderCredentialError::InvalidRequest);
                }
            }
            Self::Unknown {
                profile_id,
                operation_id,
                ..
            } if profile_id != request.profile_id()
                || Some(operation_id.as_str()) != request.operation_id() =>
            {
                return Err(ProviderCredentialError::InvalidRequest);
            }
            Self::ValidatedLink { url, .. } if !matches!(request,Request::ValidateLink{url:expected,..} if expected==url) =>
            {
                return Err(ProviderCredentialError::InvalidRequest);
            }
            _ => {}
        }
        Ok(())
    }
}
