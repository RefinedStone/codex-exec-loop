//! Prospective ownership under the canonical issue binding lease.
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    FilesystemEvidenceVaultStoreAdapter, open_record, push_part, record_relative_path, scope_bytes,
};
use super::{storage_error, vault_fs};
use crate::application::port::outbound::evidence_vault_port::{
    VaultBindingToken, VaultCleanupPolicy, VaultCleanupReport, VaultLifecycleAction,
    VaultRecordBinding, VaultRecordField, VaultRecordKey, VaultRecordScope, VaultStoreError,
};

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inventory {
    records: Vec<Entry>,
    pub(super) deletion: Option<Deletion>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Deletion {
    pub(super) revision: [u8; 32],
    pub(super) formerly_live: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    kind: Kind,
    title: bool,
    retired_at: Option<u64>,
    retained: bool,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
enum Kind {
    Current,
    Generation(String),
    Publish(String),
}

impl Kind {
    fn from_binding(binding: &VaultRecordBinding) -> Self {
        match binding {
            VaultRecordBinding::CurrentVersion => Self::Current,
            VaultRecordBinding::ConfirmedVersion(version) => Self::Generation(version.clone()),
            VaultRecordBinding::PublishIntent(intent) => Self::Publish(intent.clone()),
        }
    }

    fn binding(&self) -> VaultRecordBinding {
        match self {
            Self::Current => VaultRecordBinding::CurrentVersion,
            Self::Generation(version) => VaultRecordBinding::ConfirmedVersion(version.clone()),
            Self::Publish(intent) => VaultRecordBinding::PublishIntent(intent.clone()),
        }
    }
}

const RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

impl Inventory {
    pub(super) fn read(
        store: &FilesystemEvidenceVaultStoreAdapter,
        current: &VaultRecordScope,
    ) -> Result<Self, VaultStoreError> {
        vault_fs::read_optional_file(&store.vault_root, &Self::path(current))
            .map_err(storage_error)?
            .map(|body| serde_json::from_str(&body).map_err(storage_error))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    pub(super) fn register(
        &mut self,
        store: &FilesystemEvidenceVaultStoreAdapter,
        current: &VaultRecordScope,
        record: &VaultRecordScope,
    ) -> Result<(), VaultStoreError> {
        if self.deletion.is_some() {
            return Err(VaultStoreError::IssueDeleted);
        }
        let path = record_relative_path(&scope_bytes(record));
        let kind = Kind::from_binding(&record.binding);
        let title = record.field == VaultRecordField::Title;
        // Never claim an existing unindexed file, even if a retry can read it.
        if !self
            .records
            .iter()
            .any(|entry| entry.kind == kind && entry.title == title)
            && vault_fs::read_optional_file(&store.vault_root, &path)
                .map_err(storage_error)?
                .is_none()
        {
            self.records.push(Entry {
                kind,
                title,
                retired_at: None,
                retained: false,
            });
            self.persist(store, current)?;
        }
        Ok(())
    }

    pub(super) fn mark_generation(
        &mut self,
        version: &str,
        retain: bool,
    ) -> Result<bool, VaultStoreError> {
        let now = unix_seconds()?;
        let mut changed = false;
        for entry in &mut self.records {
            if matches!(&entry.kind, Kind::Generation(value) if value == version) {
                if retain {
                    changed |= !entry.retained;
                    entry.retained = true;
                } else if entry.retired_at.is_none() && !entry.retained {
                    entry.retired_at = Some(now);
                    changed = true;
                }
            }
        }
        Ok(changed)
    }

    pub(super) fn cleanup(
        &self,
        store: &FilesystemEvidenceVaultStoreAdapter,
        request: CleanupRequest<'_>,
    ) -> Result<VaultCleanupReport, VaultStoreError> {
        let now = unix_seconds()?;
        let mut report = VaultCleanupReport {
            tombstoned: self.deletion.is_some(),
            legacy_unverified: true,
            ..VaultCleanupReport::default()
        };
        for (index, entry) in self.records.iter().enumerate() {
            if request
                .leases
                .iter()
                .any(|lease| lease.verify_identity().is_err())
            {
                report.cleanup_pending += self.records.len() - index;
                break;
            }
            let scope = VaultRecordScope {
                issue: request.current.issue.clone(),
                field: if entry.title {
                    VaultRecordField::Title
                } else {
                    VaultRecordField::Body
                },
                binding: entry.kind.binding(),
            };
            let data = scope_bytes(&scope);
            let path = record_relative_path(&data);
            let observed = match vault_fs::read_optional_file(&store.vault_root, &path) {
                Ok(Some(value)) => value,
                Ok(None) => continue,
                Err(_) => {
                    report.cleanup_pending += 1;
                    continue;
                }
            };
            let eligible = !entry.retained
                && match request.policy.action {
                    VaultLifecycleAction::DeleteIssue => match &entry.kind {
                        Kind::Current => true,
                        Kind::Publish(intent) => request.policy.disposable_intents.contains(intent),
                        Kind::Generation(version) => {
                            entry.retired_at.is_some()
                                || self
                                    .deletion
                                    .as_ref()
                                    .and_then(|deleted| deleted.formerly_live.as_ref())
                                    == Some(version)
                        }
                    },
                    VaultLifecycleAction::RetainRecent => {
                        matches!(&entry.kind, Kind::Generation(value) if Some(value.as_str()) != request.live)
                            && entry.retired_at.is_some_and(|retired| {
                                now.saturating_sub(retired) >= RETENTION_SECS
                            })
                    }
                };
            if !eligible || open_record(&observed, request.key, &data).is_err() {
                report.retained += 1;
                continue;
            }
            match vault_fs::remove_observed_file(&store.vault_root, &path, &observed) {
                Ok(true) => report.removed += 1,
                Ok(false) | Err(_) => report.cleanup_pending += 1,
            }
        }
        Ok(report)
    }

    pub(super) fn revision(
        &self,
        binding: &VaultBindingToken,
    ) -> Result<VaultBindingToken, VaultStoreError> {
        let mut digest = Sha256::new();
        if let Some(ciphertext) = binding.ciphertext_sha256() {
            digest.update(ciphertext);
        }
        digest.update(serde_json::to_vec(self).map_err(storage_error)?);
        Ok(VaultBindingToken::observed(
            binding.scope().clone(),
            Some(digest.finalize().into()),
        ))
    }

    pub(super) fn persist(
        &self,
        store: &FilesystemEvidenceVaultStoreAdapter,
        current: &VaultRecordScope,
    ) -> Result<(), VaultStoreError> {
        let body = serde_json::to_vec(self).map_err(storage_error)?;
        vault_fs::write_file_atomic(&store.vault_root, &Self::path(current), &body).map_err(
            |error| {
                if vault_fs::write_outcome_unknown(&error) {
                    VaultStoreError::WriteOutcomeUnknown
                } else {
                    storage_error(error)
                }
            },
        )
    }

    fn path(current: &VaultRecordScope) -> PathBuf {
        let mut scope = scope_bytes(current);
        push_part(&mut scope, "prospective-inventory-v1");
        record_relative_path(&scope)
    }
}

pub(super) struct CleanupRequest<'a> {
    pub(super) current: &'a VaultRecordScope,
    pub(super) key: &'a VaultRecordKey,
    pub(super) policy: &'a VaultCleanupPolicy,
    pub(super) live: Option<&'a str>,
    pub(super) leases: [&'a vault_fs::VaultBindingLease; 2],
}

fn unix_seconds() -> Result<u64, VaultStoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(storage_error)
}

#[cfg(all(test, any(unix, windows)))]
mod tests;
