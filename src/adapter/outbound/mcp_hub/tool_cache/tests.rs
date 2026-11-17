use std::sync::atomic::{AtomicU64, Ordering};

use super::*;
use crate::domain::mcp_hub::McpHubToolInfo;
use serde_json::json;

struct CacheHome(PathBuf);

impl CacheHome {
    fn new() -> Self {
        static NEXT_HOME: AtomicU64 = AtomicU64::new(0);
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-tool-cache-{}-{}",
            std::process::id(),
            NEXT_HOME.fetch_add(1, Ordering::Relaxed)
        ));
        Self(path)
    }

    fn cache(&self) -> ToolCatalogCache {
        ToolCatalogCache::new(&self.0)
    }
}

impl Drop for CacheHome {
    fn drop(&mut self) {
        if self.0.exists() {
            std::fs::remove_dir_all(&self.0).expect("cache home should be removed");
        }
    }
}

fn key(cache: &ToolCatalogCache) -> CatalogKey {
    CatalogKey {
        server: "docs".into(),
        fingerprint: "config-a".into(),
        generation: cache.generation().unwrap(),
    }
}

fn catalog() -> McpHubToolCatalog {
    McpHubToolCatalog {
        name: "docs".into(),
        tools: vec![McpHubToolInfo {
            name: "search".into(),
            description: "Search documents".into(),
            input_schema: json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        }],
    }
}

#[test]
fn persists_catalog_across_cache_instances() {
    let home = CacheHome::new();
    let cache = home.cache();
    let key = key(&cache);
    let catalog = catalog();
    assert!(cache.lookup(&key, 1_000).unwrap().is_none());
    assert!(cache.store(&key, &catalog, 1_000).unwrap());
    drop(cache);

    let reopened = home.cache();
    assert!(reopened.lookup(&key, 1_001).unwrap() == Some(catalog));
}

#[test]
fn caches_empty_tool_lists() {
    let home = CacheHome::new();
    let cache = home.cache();
    let key = key(&cache);
    let catalog = McpHubToolCatalog {
        name: key.server.clone(),
        tools: vec![],
    };
    assert!(cache.store(&key, &catalog, 1_000).unwrap());
    assert!(cache.lookup(&key, 1_000).unwrap() == Some(catalog));
}

#[test]
fn requires_matching_server_fingerprint_and_generation() {
    let home = CacheHome::new();
    let cache = home.cache();
    let mut key = key(&cache);
    assert!(cache.store(&key, &catalog(), 1_000).unwrap());
    key.fingerprint = "config-b".into();
    assert!(cache.lookup(&key, 1_001).unwrap().is_none());
    key.fingerprint = "config-a".into();
    key.server = "other".into();
    assert!(cache.lookup(&key, 1_001).unwrap().is_none());
    key.server = "docs".into();
    key.generation += 1;
    assert!(cache.lookup(&key, 1_001).unwrap().is_none());
}

#[test]
fn rejects_future_and_expired_entries_at_exact_boundary() {
    let home = CacheHome::new();
    let cache = home.cache();
    let key = key(&cache);
    assert!(cache.store(&key, &catalog(), 1_000).unwrap());

    for now in [999, 1_600, 1_601, i64::MIN, i64::MAX] {
        assert!(cache.lookup(&key, now).unwrap().is_none());
    }
    for now in [1_000, 1_599] {
        assert!(cache.lookup(&key, now).unwrap() == Some(catalog()));
    }
}

#[test]
fn malformed_or_incomplete_json_is_a_miss() {
    let home = CacheHome::new();
    let cache = home.cache();
    let key = key(&cache);
    assert!(cache.store(&key, &catalog(), 1_000).unwrap());
    let connection = cache.open().unwrap();
    for malformed in ["{", "null", "{}", r#"{"name":"other","tools":[]}"#] {
        connection
            .execute("UPDATE tool_catalogs SET catalog = ?1", [malformed])
            .unwrap();
        assert!(cache.lookup(&key, 1_001).unwrap().is_none());
    }
}

#[test]
fn invalidation_prevents_an_old_discovery_from_restoring_entries() {
    let home = CacheHome::new();
    let discovering = home.cache();
    let invalidating = home.cache();
    let old_key = key(&discovering);
    assert!(discovering.store(&old_key, &catalog(), 1_000).unwrap());

    invalidating.invalidate().unwrap();
    let new_key = key(&invalidating);
    assert_eq!(new_key.generation, old_key.generation + 1);
    assert!(discovering.lookup(&old_key, 1_001).unwrap().is_none());
    assert!(!discovering.store(&old_key, &catalog(), 1_001).unwrap());
    assert!(invalidating.lookup(&new_key, 1_001).unwrap().is_none());
    assert!(invalidating.store(&new_key, &catalog(), 1_002).unwrap());
    assert!(discovering.lookup(&new_key, 1_002).unwrap() == Some(catalog()));

    discovering.invalidate().unwrap();
    assert_eq!(invalidating.generation().unwrap(), new_key.generation + 1);
    let connection = invalidating.open().unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM tool_catalogs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn invalidation_rolls_back_generation_when_deletion_fails() {
    let home = CacheHome::new();
    let cache = home.cache();
    let key = key(&cache);
    assert!(cache.store(&key, &catalog(), 1_000).unwrap());
    let connection = cache.open().unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_delete BEFORE DELETE ON tool_catalogs
             BEGIN SELECT RAISE(ABORT, 'test delete failure'); END;",
        )
        .unwrap();

    assert!(cache.invalidate().is_err());
    assert_eq!(cache.generation().unwrap(), key.generation);
    assert!(cache.lookup(&key, 1_001).unwrap() == Some(catalog()));
}

#[test]
fn database_and_directory_errors_are_not_cache_misses() {
    let home = CacheHome::new();
    let cache = home.cache();
    let key = key(&cache);
    std::fs::write(home.0.join(CACHE_FILE), b"not a SQLite database").unwrap();
    assert!(cache.generation().is_err());
    assert!(cache.lookup(&key, 1_000).is_err());
    assert!(cache.store(&key, &catalog(), 1_000).is_err());
    assert!(cache.invalidate().is_err());

    let blocked_home = home.0.join("file");
    std::fs::write(&blocked_home, b"not a directory").unwrap();
    assert!(ToolCatalogCache::new(&blocked_home).generation().is_err());
}

#[test]
fn schema_errors_are_not_cache_misses() {
    let home = CacheHome::new();
    let cache = home.cache();
    let key = key(&cache);
    let connection = cache.open().unwrap();
    connection
        .execute_batch(
            "DROP TABLE tool_catalogs;
             CREATE TABLE tool_catalogs (server TEXT PRIMARY KEY);",
        )
        .unwrap();
    assert!(cache.lookup(&key, 1_000).is_err());
    assert!(cache.store(&key, &catalog(), 1_000).is_err());
}
