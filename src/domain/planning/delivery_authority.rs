//! Delivery authority is part of the immutable reviewed source, never inferred from a boolean.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanningWorkDeliveryAuthority {
    HumanReview,
    Auto {
        #[serde(deserialize_with = "deserialize_merge_revision")]
        merge_gate_revision: u64,
        #[serde(deserialize_with = "deserialize_repository")]
        source_repository: String,
    },
}

impl PlanningWorkDeliveryAuthority {
    pub fn validate(&self) -> Result<(), String> {
        if let Self::Auto {
            merge_gate_revision,
            source_repository,
        } = self
        {
            if !valid_merge_revision(*merge_gate_revision) {
                return Err(
                    "automatic delivery requires a positive persisted Merge revision".into(),
                );
            }
            if !valid_repository(source_repository) {
                return Err(
                    "automatic delivery requires a canonical owner/repository identity".into(),
                );
            }
        }
        Ok(())
    }
}

fn valid_merge_revision(value: u64) -> bool {
    // Merge gate revisions come from the issue ledger's signed SQLite revision range.
    (1..=i64::MAX as u64).contains(&value)
}

fn valid_repository(value: &str) -> bool {
    // Match the provider's canonical source coordinate without depending on issue lifecycle
    // types. Do not normalize a stored grant to a different spelling during deserialization.
    if value.len() > 4096 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return false;
    }
    let Some((owner, name)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && owner
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && owner
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !name.is_empty()
        && !matches!(name, "." | "..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn deserialize_merge_revision<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u64, D::Error> {
    let value = u64::deserialize(deserializer)?;
    if valid_merge_revision(value) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "invalid automatic delivery Merge revision",
        ))
    }
}

fn deserialize_repository<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if valid_repository(&value) {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "invalid automatic delivery repository identity",
        ))
    }
}

#[cfg(test)]
mod tests;
