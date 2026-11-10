//! 작업 컨텍스트 장부의 SQLite 구현이다 (plan 22). 사용자당 파일 하나(`$AKRA_HOME/work-context/ledger.sqlite`)이다.
//!
//! 표는 서비스 종류와 범위 종류를 글자로만 저장한다. 새 외부 서비스나 새 범위(파일 경로, 작업)를 더해도 표를
//! 바꾸지 않는다.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};

use crate::application::port::outbound::work_context_store_port::{
    WorkContextLink, WorkContextStorePort,
};
use crate::domain::work_context::ExternalResourceRef;

const DIRECTORY: &str = "work-context";
const FILE: &str = "ledger.sqlite";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS resources (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    key TEXT NOT NULL,
    url TEXT NOT NULL,
    title_hint TEXT,
    UNIQUE (kind, key)
);
CREATE TABLE IF NOT EXISTS links (
    scope_kind TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    resource_id INTEGER NOT NULL REFERENCES resources (id),
    source TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    PRIMARY KEY (scope_kind, scope_id, resource_id)
);
CREATE TABLE IF NOT EXISTS scan_markers (
    scope_kind TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    marker TEXT NOT NULL,
    PRIMARY KEY (scope_kind, scope_id)
);
";

pub(crate) struct SqliteWorkContextStore {
    path: PathBuf,
    // 첫 요청에 연다. Gateway 가 장부를 쓰지 않으면 파일을 만들지 않는다.
    connection: Mutex<Option<Connection>>,
}

impl SqliteWorkContextStore {
    pub(crate) fn from_environment() -> Result<Self> {
        Ok(Self::at(
            crate::configuration::akra_home_dir()?
                .join(DIRECTORY)
                .join(FILE),
        ))
    }

    fn at(path: PathBuf) -> Self {
        Self {
            path,
            connection: Mutex::new(None),
        }
    }

    fn with_connection<T>(&self, work: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let mut guard = self
            .connection
            .lock()
            .map_err(|_| anyhow!("the work context ledger lock is poisoned"))?;
        if guard.is_none() {
            *guard = Some(open(&self.path)?);
        }
        let Some(connection) = guard.as_mut() else {
            return Err(anyhow!("the work context ledger did not open"));
        };
        work(connection)
    }
}

fn open(path: &Path) -> Result<Connection> {
    if let Some(directory) = path.parent() {
        crate::private_directory::ensure_private_directory(directory)?;
    }
    let connection =
        Connection::open(path).with_context(|| format!("failed to open `{}`", path.display()))?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection
        .execute_batch(SCHEMA)
        .context("failed to prepare the work context ledger")?;
    Ok(connection)
}

impl WorkContextStorePort for SqliteWorkContextStore {
    fn link_resources(
        &self,
        scope_kind: &str,
        scope_id: &str,
        source: &str,
        resources: &[ExternalResourceRef],
    ) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        self.with_connection(|connection| {
            let transaction = connection.transaction()?;
            for resource in resources {
                transaction.execute(
                    "INSERT INTO resources (kind, key, url, title_hint) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (kind, key) DO UPDATE SET
                         title_hint = COALESCE(resources.title_hint, excluded.title_hint)",
                    params![resource.kind, resource.key, resource.url, resource.title_hint],
                )?;
                let resource_id: i64 = transaction.query_row(
                    "SELECT id FROM resources WHERE kind = ?1 AND key = ?2",
                    params![resource.kind, resource.key],
                    |row| row.get(0),
                )?;
                transaction.execute(
                    "INSERT OR IGNORE INTO links (scope_kind, scope_id, resource_id, source, first_seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![scope_kind, scope_id, resource_id, source, now],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
    }

    fn links(&self, scope_kind: &str, scope_id: &str) -> Result<Vec<WorkContextLink>> {
        self.with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT r.kind, r.key, r.url, r.title_hint, l.source, l.first_seen_at
                 FROM links l JOIN resources r ON r.id = l.resource_id
                 WHERE l.scope_kind = ?1 AND l.scope_id = ?2
                 ORDER BY l.rowid",
            )?;
            let rows = statement.query_map(params![scope_kind, scope_id], |row| {
                Ok(WorkContextLink {
                    resource: ExternalResourceRef {
                        kind: row.get(0)?,
                        key: row.get(1)?,
                        url: row.get(2)?,
                        title_hint: row.get(3)?,
                    },
                    source: row.get(4)?,
                    first_seen_at: row.get(5)?,
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .context("failed to read work context links")
        })
    }

    fn scan_marker(&self, scope_kind: &str, scope_id: &str) -> Result<Option<String>> {
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT marker FROM scan_markers WHERE scope_kind = ?1 AND scope_id = ?2",
                    params![scope_kind, scope_id],
                    |row| row.get(0),
                )
                .optional()
                .context("failed to read the work context scan marker")
        })
    }

    fn set_scan_marker(&self, scope_kind: &str, scope_id: &str, marker: &str) -> Result<()> {
        self.with_connection(|connection| {
            connection.execute(
                "INSERT INTO scan_markers (scope_kind, scope_id, marker) VALUES (?1, ?2, ?3)
                 ON CONFLICT (scope_kind, scope_id) DO UPDATE SET marker = excluded.marker",
                params![scope_kind, scope_id, marker],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(label: &str) -> (SqliteWorkContextStore, PathBuf) {
        let directory = crate::test_utils::platform_safe_temp_dir()
            .join(format!("akra-work-context-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        (SqliteWorkContextStore::at(directory.join(FILE)), directory)
    }

    fn resource(kind: &str, key: &str, title: Option<&str>) -> ExternalResourceRef {
        ExternalResourceRef {
            kind: kind.to_string(),
            key: key.to_string(),
            url: format!("https://example.test/{key}"),
            title_hint: title.map(str::to_string),
        }
    }

    #[test]
    fn links_keep_first_seen_order_source_and_scope() {
        let (store, directory) = store("links");
        store
            .link_resources(
                "thread",
                "t1",
                "user",
                &[resource("jira", "a/K-1", Some("K-1"))],
            )
            .unwrap();
        store
            .link_resources(
                "thread",
                "t1",
                "agent",
                &[
                    resource("jira", "a/K-1", Some("other")),
                    resource("notion", "abc", None),
                ],
            )
            .unwrap();
        store
            .link_resources(
                "thread",
                "t2",
                "tool",
                &[resource("notion", "abc", Some("Spec"))],
            )
            .unwrap();

        let first = store.links("thread", "t1").unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].resource.key, "a/K-1");
        assert_eq!(first[0].source, "user");
        assert_eq!(first[0].resource.title_hint.as_deref(), Some("K-1"));
        assert_eq!(first[1].resource.kind, "notion");
        // 같은 문서는 한 줄이다. 나중에 알게 된 제목은 빈 제목만 채운다.
        let second = store.links("thread", "t2").unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].resource.title_hint.as_deref(), Some("Spec"));
        assert!(store.links("path", "t1").unwrap().is_empty());
        drop(store);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn scan_markers_are_replaced_per_scope() {
        let (store, directory) = store("markers");
        assert_eq!(store.scan_marker("thread", "t1").unwrap(), None);
        store.set_scan_marker("thread", "t1", "item-1").unwrap();
        store.set_scan_marker("thread", "t1", "item-2").unwrap();
        assert_eq!(
            store.scan_marker("thread", "t1").unwrap().as_deref(),
            Some("item-2")
        );
        assert_eq!(store.scan_marker("thread", "t2").unwrap(), None);
        drop(store);
        let _ = std::fs::remove_dir_all(directory);
    }
}
