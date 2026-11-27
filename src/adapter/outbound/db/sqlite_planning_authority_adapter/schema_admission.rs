//! Schema admission preserves identity checks before transactional migration.
use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension};

use super::{
    AUTHORITY_STORE_MODE, AUTHORITY_STORE_SCHEMA_VERSION, read_metadata_string_connection,
    table_exists,
};
use crate::domain::planning::PlanningAuthorityLocation;

// v5 is the genuine published v1.3.5 format. Its tables migrate through the
// existing atomic schema transaction. Unverified v6 remains unsupported.
const PUBLISHED_LEGACY_AUTHORITY_SCHEMA_VERSION: i64 = 5;
const MINIMUM_MIGRATABLE_AUTHORITY_STORE_SCHEMA_VERSION: i64 = 7;

pub(super) fn validate_authority_store_schema(
    connection: &Connection,
    location: &PlanningAuthorityLocation,
) -> Result<()> {
    let metadata_exists = table_exists(connection, "authority_metadata")?;
    if !metadata_exists {
        let foreign_object_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to inspect unmarked authority-store objects")?;
        if foreign_object_count != 0 {
            return Err(anyhow!(
                "unmarked non-empty SQLite database cannot be adopted as an authority-store"
            ));
        }
        return Ok(());
    }

    let schema_version = connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("failed to read authority-store schema version")?
        .ok_or_else(|| anyhow!("authority-store schema version is missing"))?;
    let parsed_schema_version = schema_version
        .parse::<i64>()
        .map_err(|_| anyhow!("unsupported authority-store schema version: {schema_version}"))?;
    if !(MINIMUM_MIGRATABLE_AUTHORITY_STORE_SCHEMA_VERSION..=AUTHORITY_STORE_SCHEMA_VERSION)
        .contains(&parsed_schema_version)
        && parsed_schema_version != PUBLISHED_LEGACY_AUTHORITY_SCHEMA_VERSION
    {
        return Err(anyhow!(
            "unsupported authority-store schema version: {schema_version}"
        ));
    }

    let mode = read_metadata_string_connection(connection, "mode")?
        .ok_or_else(|| anyhow!("authority-store mode marker is missing"))?;
    if mode != AUTHORITY_STORE_MODE {
        return Err(anyhow!("unsupported authority-store mode: {mode}"));
    }
    if let Some(repository_identity) =
        read_metadata_string_connection(connection, "repository_identity")?
    {
        if repository_identity != location.repository_identity {
            return Err(anyhow!(
                "authority-store is bound to a different repository identity"
            ));
        }
    } else {
        let canonical_repo_root =
            read_metadata_string_connection(connection, "canonical_repo_root")?.ok_or_else(
                || anyhow!("authority-store canonical repository binding is missing"),
            )?;
        if canonical_repo_root != location.canonical_repo_root {
            return Err(anyhow!(
                "legacy authority-store is bound to a different canonical repository"
            ));
        }
    }

    Ok(())
}
