use crate::domain::storage_analysis::hotspots::should_expand;
use crate::domain::storage_analysis::*;
use anyhow::{Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(super) struct StorageIndex {
    pub conn: Connection,
    pub known_total: std::cell::Cell<u64>,
}
#[derive(Debug, Clone)]
pub(super) struct Observation {
    pub parent: i64,
    pub name: String,
    pub path: PathBuf,
    pub directory: bool,
    pub logical: u64,
    pub allocated: Option<u64>,
    pub identity: Option<String>,
    pub issue: Option<StorageIssue>,
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    binding: String,
    offset: u64,
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn padded(v: u64) -> String {
    format!("{v:020}")
}
fn number(v: &str) -> Result<u64> {
    v.parse().map_err(|_| anyhow!("storage_corrupt_index"))
}
fn node_id(id: &str) -> Result<i64> {
    id.strip_prefix('n')
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| anyhow!("storage_not_found"))
}
impl StorageIndex {
    pub fn create(dir: &Path, id: &str, volume: &StorageVolume) -> Result<Self> {
        let conn = Connection::open(dir.join("index.sqlite3"))?;
        conn.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA cache_size=-8192; PRAGMA temp_store=FILE;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE nodes(id INTEGER PRIMARY KEY,parent INTEGER,name TEXT NOT NULL,path TEXT NOT NULL,kind TEXT NOT NULL,
            logical TEXT NOT NULL,known TEXT NOT NULL,unknown INTEGER NOT NULL DEFAULT 0,
            issues TEXT NOT NULL DEFAULT '[]',protection TEXT NOT NULL DEFAULT '[]',classification TEXT NOT NULL DEFAULT '[]',
            object_key TEXT,pending INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX node_parent ON nodes(parent,kind);
        CREATE INDEX pending_dirs ON nodes(pending,id);
        CREATE INDEX object_paths ON nodes(object_key);
        CREATE TABLE objects(key TEXT PRIMARY KEY,owner INTEGER NOT NULL,allocated TEXT,links INTEGER NOT NULL);
        CREATE INDEX object_owner ON objects(owner);
        ")?;
        let snapshot = StorageSnapshot {
            schema_version: 1,
            id: id.into(),
            revision: 1,
            volume_id: volume.id.clone(),
            root_node_id: "n1".into(),
            state: StorageState::Running,
            started_at: now(),
            finished_at: None,
            files_scanned: 0,
            directories_scanned: 0,
            issues: vec![],
        };
        conn.execute(
            "INSERT INTO meta VALUES('snapshot',?1)",
            [serde_json::to_string(&snapshot)?],
        )?;
        conn.execute("INSERT INTO nodes(id,parent,name,path,kind,logical,known,pending) VALUES(1,NULL,?1,?2,'directory',?3,?3,1)",
            params![volume.label,volume.root_path,padded(0)])?;
        Ok(Self {
            conn,
            known_total: std::cell::Cell::new(0),
        })
    }
    pub fn open(dir: &Path) -> Result<Self> {
        super::reject_symlink(dir)?;
        super::reject_symlink(&dir.join("index.sqlite3"))?;
        let conn = Connection::open_with_flags(
            dir.join("index.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        conn.execute_batch("PRAGMA cache_size=-8192; PRAGMA temp_store=FILE;")?;
        Ok(Self {
            conn,
            known_total: std::cell::Cell::new(0),
        })
    }
    pub fn snapshot(&self) -> Result<StorageSnapshot> {
        let json: String =
            self.conn
                .query_row("SELECT value FROM meta WHERE key='snapshot'", [], |r| {
                    r.get(0)
                })?;
        Ok(serde_json::from_str(&json)?)
    }
    pub fn save_snapshot(&self, s: &StorageSnapshot) -> Result<()> {
        self.conn.execute(
            "UPDATE meta SET value=?1 WHERE key='snapshot'",
            [serde_json::to_string(s)?],
        )?;
        Ok(())
    }
    pub fn finish(&mut self, state: StorageState, issue: Option<StorageIssue>) -> Result<()> {
        let mut s = self.snapshot()?;
        s.state = state;
        s.finished_at = Some(now());
        if let Some(i) = issue {
            s.issues.push(i);
        }
        self.save_snapshot(&s)
    }
    pub fn pending(&self, limit: usize) -> Result<Vec<(i64, PathBuf)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,path FROM nodes WHERE pending=1 ORDER BY id LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?)))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn visited(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE nodes SET pending=0 WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn mark_issue(&self, id: i64, issue: StorageIssue) -> Result<()> {
        let raw: String =
            self.conn
                .query_row("SELECT issues FROM nodes WHERE id=?1", [id], |r| r.get(0))?;
        let mut issues: Vec<StorageIssue> = serde_json::from_str(&raw)?;
        if let Some(previous) = issues.iter_mut().find(|i| i.code == issue.code) {
            previous.count += issue.count;
        } else if issues.len() < 32 {
            issues.push(issue);
        }
        self.conn.execute(
            "UPDATE nodes SET issues=?1,unknown=1 WHERE id=?2",
            params![serde_json::to_string(&issues)?, id],
        )?;
        Ok(())
    }
    pub fn record(&self, o: Observation) -> Result<i64> {
        let key = o
            .identity
            .clone()
            .unwrap_or_else(|| format!("unknown:{}", rand::random::<u128>()));
        let unknown =
            o.issue.is_some() || (!o.directory && (o.allocated.is_none() || o.identity.is_none()));
        let issues = o.issue.into_iter().collect::<Vec<_>>();
        self.conn.execute("INSERT INTO nodes(parent,name,path,kind,logical,known,unknown,issues,object_key,pending)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![o.parent,o.name,o.path.to_string_lossy(),
            if o.directory{"directory"}else{"file"},padded(o.logical),padded(o.allocated.unwrap_or(0)),
            unknown,serde_json::to_string(&issues)?,Some(key.clone()),
            i64::from(o.directory && !unknown)])?;
        let id = self.conn.last_insert_rowid();
        if !o.directory {
            let previous: Option<(i64, Option<String>, u64)> = self
                .conn
                .query_row(
                    "SELECT owner,allocated,links FROM objects WHERE key=?1",
                    [&key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let allocated = o.allocated.filter(|_| o.identity.is_some()).map(padded);
            if let Some((owner, old, links)) = previous {
                let lca = self.lca(owner, o.parent)?;
                // Conflicting observations of one identity are explicitly unknown.
                let same = old == allocated;
                self.conn.execute(
                    "UPDATE objects SET owner=?1,links=?2,allocated=?3 WHERE key=?4",
                    params![lca, links + 1, if same { old.clone() } else { None }, key],
                )?;
                if !same {
                    if let Some(old) = old {
                        self.known_total
                            .set(self.known_total.get().saturating_sub(number(&old)?));
                    }
                    self.mark_issue(
                        lca,
                        StorageIssue::one(
                            "metadata_changed",
                            "The same file identity changed during the scan",
                        ),
                    )?;
                }
            } else {
                self.conn.execute(
                    "INSERT INTO objects(key,owner,allocated,links) VALUES(?1,?2,?3,1)",
                    params![key, o.parent, allocated],
                )?;
                if let Some(value) = allocated {
                    self.known_total.set(
                        self.known_total
                            .get()
                            .checked_add(number(&value)?)
                            .ok_or_else(|| anyhow!("storage_size_overflow"))?,
                    );
                }
            }
        }
        Ok(id)
    }
    fn parent(&self, id: i64) -> Result<Option<i64>> {
        self.conn
            .query_row("SELECT parent FROM nodes WHERE id=?1", [id], |r| r.get(0))
            .map_err(Into::into)
    }
    fn lca(&self, a: i64, b: i64) -> Result<i64> {
        let mut ancestors = std::collections::HashSet::new();
        let mut current = Some(a);
        while let Some(id) = current {
            ancestors.insert(id);
            current = self.parent(id)?;
        }
        current = Some(b);
        while let Some(id) = current {
            if ancestors.contains(&id) {
                return Ok(id);
            }
            current = self.parent(id)?;
        }
        bail!("storage_disconnected_tree")
    }
    pub fn aggregate(&self) -> Result<()> {
        // Reverse insertion order is post-order: every child is inserted after its parent.
        let mut dirs = self
            .conn
            .prepare("SELECT id FROM nodes WHERE kind='directory' ORDER BY id DESC")?;
        let ids = dirs.query_map([], |r| r.get::<_, i64>(0))?;
        for id in ids {
            let id = id?;
            let mut known = 0_u64;
            let mut logical = 0_u64;
            let mut unknown: bool = self.conn.query_row(
                "SELECT unknown OR pending FROM nodes WHERE id=?1",
                [id],
                |r| r.get(0),
            )?;
            let mut children = self
                .conn
                .prepare("SELECT kind,logical,known,unknown FROM nodes WHERE parent=?1")?;
            for row in children.query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })? {
                let (kind, l, k, u) = row?;
                logical = logical
                    .checked_add(number(&l)?)
                    .ok_or_else(|| anyhow!("storage_size_overflow"))?;
                if kind == "directory" {
                    known = known
                        .checked_add(number(&k)?)
                        .ok_or_else(|| anyhow!("storage_size_overflow"))?;
                }
                unknown |= u;
            }
            for shared in [false, true] {
                let mut own = 0_u64;
                let mut count = 0_u64;
                let mut missing = false;
                let mut stmt = self
                    .conn
                    .prepare("SELECT allocated FROM objects WHERE owner=?1 AND (links>1)=?2")?;
                for value in
                    stmt.query_map(params![id, shared], |r| r.get::<_, Option<String>>(0))?
                {
                    count += 1;
                    match value? {
                        Some(v) => {
                            own = own
                                .checked_add(number(&v)?)
                                .ok_or_else(|| anyhow!("storage_size_overflow"))?
                        }
                        None => missing = true,
                    }
                }
                if count > 0 {
                    let kind = if shared {
                        "shared_files"
                    } else {
                        "direct_files"
                    };
                    self.conn.execute("INSERT INTO nodes(parent,name,path,kind,logical,known,unknown) SELECT ?1,?2,path,?3,?4,?5,?6 FROM nodes WHERE id=?1",
                        params![id,if shared{"공유 파일"}else{"직속 파일"},kind,padded(0),padded(own),missing])?;
                }
                known = known
                    .checked_add(own)
                    .ok_or_else(|| anyhow!("storage_size_overflow"))?;
                unknown |= missing;
            }
            self.conn.execute(
                "UPDATE nodes SET logical=?1,known=?2,unknown=?3 WHERE id=?4",
                params![padded(logical), padded(known), unknown, id],
            )?;
        }
        Ok(())
    }
    pub fn is_descendant(&self, node: &str, root: &str) -> Result<bool> {
        let (Ok(mut id), Ok(root)) = (node_id(node), node_id(root)) else {
            return Ok(false);
        };
        for _ in 0..4096 {
            if id == root {
                return Ok(self
                    .conn
                    .query_row("SELECT 1 FROM nodes WHERE id=?1", [id], |r| {
                        r.get::<_, u8>(0)
                    })
                    .optional()?
                    .is_some());
            }
            match self.parent(id) {
                Ok(Some(parent)) => id = parent,
                _ => return Ok(false),
            }
        }
        Ok(false)
    }
    fn authorize(&self, node: &str, root: Option<&str>) -> Result<i64> {
        let id = node_id(node)?;
        if let Some(root) = root {
            if !self.is_descendant(node, root)? {
                bail!("storage_not_found");
            }
        }
        if self
            .conn
            .query_row("SELECT 1 FROM nodes WHERE id=?1", [id], |r| {
                r.get::<_, u8>(0)
            })
            .optional()?
            .is_none()
        {
            bail!("storage_not_found");
        }
        Ok(id)
    }
    pub fn get_node(&self, node: &str, root: Option<&str>) -> Result<StorageNode> {
        let id = self.authorize(node, root)?;
        let s = self.snapshot()?;
        if s.state == StorageState::Running {
            bail!("storage_snapshot_interrupted");
        }
        let (parent,name,kind,logical,known,unknown,issues,protection,classification,key):(Option<i64>,String,String,String,String,bool,String,String,String,Option<String>)=
            self.conn.query_row("SELECT parent,name,kind,logical,known,unknown,issues,protection,classification,object_key FROM nodes WHERE id=?1",[id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?)))?;
        let is_root = parent.is_none() || root == Some(node);
        let denominator: Option<u64> = if is_root {
            None
        } else {
            parent
                .map(|p| {
                    self.conn
                        .query_row("SELECT known FROM nodes WHERE id=?1", [p], |r| {
                            r.get::<_, String>(0)
                        })
                })
                .transpose()?
                .map(|n| number(&n))
                .transpose()?
        };
        let bytes = number(&known)?;
        let logical = number(&logical)?;
        let allocation_owner_id = if let Some(key) = key {
            self.conn
                .query_row("SELECT owner FROM objects WHERE key=?1", [key], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?
                .map(|id| format!("n{id}"))
        } else {
            None
        };
        let mut node = StorageNode {
            snapshot_id: s.id,
            revision: s.revision,
            id: node.into(),
            parent_id: if is_root {
                None
            } else {
                parent.map(|p| format!("n{p}"))
            },
            name,
            kind: match kind.as_str() {
                "directory" => StorageNodeKind::Directory,
                "file" => StorageNodeKind::File,
                "shared_files" => StorageNodeKind::SharedFiles,
                _ => StorageNodeKind::DirectFiles,
            },
            allocated_bytes: if unknown {
                None
            } else {
                Some(bytes.to_string())
            },
            known_allocated_bytes: bytes.to_string(),
            logical_bytes: logical.to_string(),
            ratio: StorageRatio::new(
                Some(bytes),
                denominator,
                is_root,
                unknown || !matches!(s.state, StorageState::Complete),
            ),
            state: if unknown {
                StorageState::Partial
            } else {
                StorageState::Complete
            },
            issues: serde_json::from_str(&issues)?,
            protection: serde_json::from_str(&protection)?,
            classification: serde_json::from_str(&classification)?,
            allocation_owner_id,
        };
        if let (Some(owner), Some(root)) = (&node.allocation_owner_id, root) {
            if !self.is_descendant(owner, root)? {
                node.allocation_owner_id = None;
                node.ratio = StorageRatio::new(None, None, false, true);
            }
        }
        Ok(node)
    }
    pub fn identity(&self, node: &str) -> Result<Option<String>> {
        let id = self.authorize(node, None)?;
        let (identity, parent, kind): (Option<String>, Option<i64>, String) = self.conn.query_row(
            "SELECT object_key,parent,kind FROM nodes WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if matches!(kind.as_str(), "direct_files" | "shared_files") {
            if let Some(parent) = parent {
                return self.identity(&format!("n{parent}"));
            }
        }
        Ok(identity.filter(|id| !id.starts_with("unknown:")))
    }
    pub fn path(&self, node: &str) -> Result<PathBuf> {
        let id = self.authorize(node, None)?;
        Ok(PathBuf::from(self.conn.query_row(
            "SELECT path FROM nodes WHERE id=?1",
            [id],
            |r| r.get::<_, String>(0),
        )?))
    }
    pub fn ancestors(&self, node: &str, root: Option<&str>) -> Result<Vec<StorageNode>> {
        let mut id = self.authorize(node, root)?;
        let mut result = Vec::new();
        for _ in 0..4096 {
            let current = format!("n{id}");
            result.push(self.get_node(&current, root)?);
            if root == Some(current.as_str()) {
                break;
            }
            match self.parent(id)? {
                Some(p) => id = p,
                None => break,
            }
        }
        result.reverse();
        Ok(result)
    }
    pub fn page(&self, q: &StorageQuery, files: bool) -> Result<StoragePage<StorageNode>> {
        q.validate().map_err(|e| anyhow!(e))?;
        let parent = self.authorize(&q.node_id, q.scope_root_id.as_deref())?;
        let s = self.snapshot()?;
        let binding = binding(q, &s, if files { "files" } else { "children" }, None)?;
        let offset = read_cursor(q.cursor.as_deref(), &binding)?;
        let tree = if files && q.recursive {
            "WITH RECURSIVE tree(id) AS (SELECT ?1 UNION ALL SELECT n.id FROM nodes n JOIN tree t ON n.parent=t.id WHERE n.kind='directory') "
        } else {
            ""
        };
        let range = if files && q.recursive {
            "parent IN (SELECT id FROM tree)"
        } else {
            "parent=?1"
        };
        let kind = if files { "kind='file'" } else { "kind!='file'" };
        let filter = "(?2 IS NULL OR instr(lower(name),lower(?2))>0) AND (?3 IS NULL OR lower(name) LIKE ?3 ESCAPE '!')";
        let extension = q.extension_filter.as_ref().map(|v| {
            format!(
                "%.{}",
                v.trim_start_matches('.')
                    .replace('!', "!!")
                    .replace('%', "!%")
                    .replace('_', "!_")
            )
        });
        let order = match q.sort {
            StorageSort::Name => "name COLLATE BINARY ASC,id ASC",
            StorageSort::AllocatedDesc => "known DESC,id ASC",
        };
        let sql = format!(
            "{tree}SELECT id FROM nodes WHERE {range} AND {kind} AND {filter} ORDER BY {order} LIMIT ?4 OFFSET ?5"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![
                parent,
                q.name_filter,
                extension,
                (q.limit + 1) as i64,
                offset
            ],
            |r| r.get::<_, i64>(0),
        )?;
        let mut ids = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let more = ids.len() > q.limit;
        ids.truncate(q.limit);
        let total: u64 = self.conn.query_row(
            &format!("{tree}SELECT count(*) FROM nodes WHERE {range} AND {kind} AND {filter}"),
            params![parent, q.name_filter, extension],
            |r| r.get(0),
        )?;
        let items = ids
            .into_iter()
            .map(|id| self.get_node(&format!("n{id}"), q.scope_root_id.as_deref()))
            .collect::<Result<Vec<_>>>()?;
        Ok(StoragePage {
            snapshot_id: s.id,
            revision: s.revision,
            items,
            total_count: Some(total),
            next_cursor: if more {
                Some(write_cursor(&binding, offset + q.limit as u64)?)
            } else {
                None
            },
        })
    }
    pub fn hotspots(
        &self,
        q: &StorageQuery,
        t: &HotspotThresholds,
    ) -> Result<StoragePage<StorageHotspot>> {
        q.validate().map_err(|e| anyhow!(e))?;
        t.values().map_err(|e| anyhow!(e))?;
        let root = self.authorize(&q.node_id, q.scope_root_id.as_deref())?;
        let s = self.snapshot()?;
        let bind = binding(q, &s, "hotspots", Some(t))?;
        let offset = read_cursor(q.cursor.as_deref(), &bind)?;
        // Temporary work queues live in SQLite's file-backed temp store, not a renderer-sized vector.
        self.conn.execute_batch("DROP TABLE IF EXISTS temp.frontier; DROP TABLE IF EXISTS temp.hotspots;
            CREATE TEMP TABLE frontier(id INTEGER PRIMARY KEY);
            CREATE TEMP TABLE hotspots(id INTEGER PRIMARY KEY,node INTEGER,role TEXT,bytes TEXT,reason TEXT);")?;
        self.conn
            .execute("INSERT INTO frontier VALUES(?1)", [root])?;
        loop {
            let next: Option<i64> = self
                .conn
                .query_row("SELECT id FROM frontier ORDER BY id LIMIT 1", [], |r| {
                    r.get(0)
                })
                .optional()?;
            let Some(id) = next else {
                break;
            };
            self.conn
                .execute("DELETE FROM frontier WHERE id=?1", [id])?;
            let node = self.get_node(&format!("n{id}"), q.scope_root_id.as_deref())?;
            let parent = node.known_bytes();
            let mut expanded = 0_u64;
            let mut count = 0_u64;
            let mut stmt = self.conn.prepare(
                "SELECT id,known FROM nodes WHERE parent=?1 AND kind='directory' ORDER BY id",
            )?;
            for row in
                stmt.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            {
                let (child, bytes) = row?;
                let bytes = number(&bytes)?;
                if id == 1 || should_expand(bytes, parent, t).map_err(|e| anyhow!(e))? {
                    expanded = expanded
                        .checked_add(bytes)
                        .ok_or_else(|| anyhow!("storage_size_overflow"))?;
                    count += 1;
                    self.conn
                        .execute("INSERT INTO frontier VALUES(?1)", [child])?;
                }
            }
            if count == 0 {
                self.conn.execute("INSERT INTO hotspots(node,role,bytes,reason) VALUES(?1,'folder',?2,'no_child_above_threshold')",params![id,padded(parent)])?;
            } else if parent > expanded {
                self.conn.execute("INSERT INTO hotspots(node,role,bytes,reason) VALUES(?1,'remainder',?2,'direct_shared_and_unexpanded_children')",params![id,padded(parent-expanded)])?;
            }
        }
        let total: u64 = self
            .conn
            .query_row("SELECT count(*) FROM hotspots", [], |r| r.get(0))?;
        let mut stmt=self.conn.prepare("SELECT node,role,bytes,reason FROM hotspots ORDER BY bytes DESC,node ASC,role ASC LIMIT ?1 OFFSET ?2")?;
        let mut items = Vec::new();
        for row in stmt.query_map(params![(q.limit + 1) as u64, offset], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            let (id, role, bytes, reason) = row?;
            let node = self.get_node(&format!("n{id}"), q.scope_root_id.as_deref())?;
            items.push(StorageHotspot {
                provisional: node.state != StorageState::Complete,
                node,
                role,
                reason,
                included_bytes: number(&bytes)?.to_string(),
            });
        }
        let more = items.len() > q.limit;
        items.truncate(q.limit);
        Ok(StoragePage {
            snapshot_id: s.id,
            revision: s.revision,
            items,
            total_count: Some(total),
            next_cursor: if more {
                Some(write_cursor(&bind, offset + q.limit as u64)?)
            } else {
                None
            },
        })
    }
}
fn binding(
    q: &StorageQuery,
    s: &StorageSnapshot,
    kind: &str,
    t: Option<&HotspotThresholds>,
) -> Result<String> {
    let mut q = q.clone();
    q.cursor = None;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(q, s.revision, kind, t))?)
    ))
}
fn read_cursor(cursor: Option<&str>, binding: &str) -> Result<u64> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    if cursor.len() > 1024 {
        bail!("storage_invalid_cursor");
    }
    let cursor: Cursor = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|_| anyhow!("storage_invalid_cursor"))?,
    )
    .map_err(|_| anyhow!("storage_invalid_cursor"))?;
    if cursor.binding != binding || cursor.offset > i64::MAX as u64 {
        bail!("storage_invalid_cursor");
    }
    Ok(cursor.offset)
}
fn write_cursor(binding: &str, offset: u64) -> Result<String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&Cursor {
        binding: binding.into(),
        offset,
    })?))
}
