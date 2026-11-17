use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::domain::mcp_hub::McpHubToolCatalog;

const CACHE_FILE: &str = "tool-catalog.sqlite";
const TTL_SECONDS: i64 = 600;
const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS catalog_generation (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        generation INTEGER NOT NULL CHECK (typeof(generation) = 'integer' AND generation >= 0)
    );
    INSERT OR IGNORE INTO catalog_generation (id, generation) VALUES (1, 0);
    CREATE TABLE IF NOT EXISTS tool_catalogs (
        server TEXT PRIMARY KEY,
        fingerprint TEXT NOT NULL,
        generation INTEGER NOT NULL,
        fetched_at INTEGER NOT NULL,
        catalog TEXT NOT NULL
    );
";

pub(super) struct CatalogKey {
    pub(super) server: String,
    pub(super) fingerprint: String,
    pub(super) generation: i64,
}

pub(super) struct ToolCatalogCache {
    home: PathBuf,
}

impl ToolCatalogCache {
    pub(super) fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
        }
    }

    pub(super) fn generation(&self) -> Result<i64> {
        let connection = self.open()?;
        read_generation(&connection)
    }

    pub(super) fn lookup(&self, key: &CatalogKey, now: i64) -> Result<Option<McpHubToolCatalog>> {
        let connection = self.open()?;
        let entry: Option<(i64, String)> = connection
            .query_row(
                "SELECT fetched_at, catalog FROM tool_catalogs
                 WHERE server = ?1 AND fingerprint = ?2 AND generation = ?3
                   AND generation = (SELECT generation FROM catalog_generation WHERE id = 1)",
                params![key.server, key.fingerprint, key.generation],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .context("failed to read the MCP tool catalog cache")?;
        let Some((fetched_at, json)) = entry else {
            return Ok(None);
        };
        if !now
            .checked_sub(fetched_at)
            .is_some_and(|age| (0..TTL_SECONDS).contains(&age))
        {
            return Ok(None);
        }
        Ok(serde_json::from_str::<McpHubToolCatalog>(&json)
            .ok()
            .filter(|catalog| catalog.name == key.server))
    }

    pub(super) fn store(
        &self,
        key: &CatalogKey,
        catalog: &McpHubToolCatalog,
        now: i64,
    ) -> Result<bool> {
        let json =
            serde_json::to_string(catalog).context("failed to encode the MCP tool catalog")?;
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The key must be captured before discovery. Invalidation wins over an old discovery.
        if read_generation(&transaction)? != key.generation {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO tool_catalogs (server, fingerprint, generation, fetched_at, catalog)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (server) DO UPDATE SET
                 fingerprint = excluded.fingerprint,
                 generation = excluded.generation,
                 fetched_at = excluded.fetched_at,
                 catalog = excluded.catalog",
            params![key.server, key.fingerprint, key.generation, now, json],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub(super) fn invalidate(&self) -> Result<()> {
        let mut connection = self.open()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let generation = read_generation(&transaction)?
            .checked_add(1)
            .context("the MCP tool catalog cache generation is exhausted")?;
        transaction.execute(
            "UPDATE catalog_generation SET generation = ?1 WHERE id = 1",
            [generation],
        )?;
        transaction.execute("DELETE FROM tool_catalogs", [])?;
        transaction.commit()?;
        Ok(())
    }

    fn open(&self) -> Result<Connection> {
        crate::private_directory::ensure_private_directory(&self.home)?;
        let path = self.home.join(CACHE_FILE);
        let connection = Connection::open(&path)
            .with_context(|| format!("failed to open `{}`", path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection
            .execute_batch(SCHEMA)
            .context("failed to prepare the MCP tool catalog cache")?;
        Ok(connection)
    }
}

fn read_generation(connection: &Connection) -> Result<i64> {
    connection
        .query_row(
            "SELECT generation FROM catalog_generation WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .context("failed to read the MCP tool catalog cache generation")
}

#[cfg(test)]
mod tests;
