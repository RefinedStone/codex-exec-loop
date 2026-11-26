use super::index::StorageIndex;
use crate::domain::storage_analysis::*;
use anyhow::{Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

const MAX_PRIORITIES: usize = 64;
pub(super) const CHILDREN_SQL: &str = "SELECT id,parent,name,pending FROM nodes INDEXED BY node_parent WHERE parent=?1 AND kind='directory' AND id>?2 ORDER BY id LIMIT ?3";

#[derive(Default)]
pub(super) struct ScanSchedulingState {
    nodes: VecDeque<i64>,
    priority_streak: usize,
    #[cfg(test)]
    pub measurement: SchedulingMeasurement,
}

#[cfg(test)]
#[derive(Clone, Copy, Default)]
pub(super) struct SchedulingMeasurement {
    pub dispatches: u64,
    pub dispatches_after_cancel: u64,
    pub last_dispatch: Option<std::time::Instant>,
    pub cancel_observed: Option<std::time::Instant>,
}

#[cfg(test)]
pub(super) fn record_dispatch(state: &Mutex<ScanSchedulingState>, cancelled: bool) {
    let mut state = state.lock().expect("scheduling measurement");
    state.measurement.dispatches += 1;
    state.measurement.dispatches_after_cancel += u64::from(cancelled);
    state.measurement.last_dispatch = Some(std::time::Instant::now());
}

#[cfg(test)]
pub(super) fn record_cancel(state: &Mutex<ScanSchedulingState>) {
    state
        .lock()
        .expect("scheduling measurement")
        .measurement
        .cancel_observed
        .get_or_insert_with(std::time::Instant::now);
}
impl ScanSchedulingState {
    pub fn enqueue(&mut self, id: i64) -> Result<(bool, usize)> {
        if self.nodes.contains(&id) {
            return Ok((false, self.nodes.len()));
        }
        if self.nodes.len() >= MAX_PRIORITIES {
            bail!("storage_priority_queue_full");
        }
        self.nodes.push_back(id);
        Ok((true, self.nodes.len()))
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    fn choose(&mut self, db: &StorageIndex) -> Result<Option<(i64, PathBuf)>> {
        if self.priority_streak < 3 {
            while let Some(id) = self.nodes.pop_front() {
                let next = db
                    .conn
                    .query_row(
                        "SELECT id,path FROM nodes WHERE id=?1 AND pending=1",
                        [id],
                        |r| Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?))),
                    )
                    .optional()?;
                if next.is_some() {
                    self.priority_streak += 1;
                    return Ok(next);
                }
            }
        }
        let next = db
            .conn
            .query_row(
                "SELECT id,path FROM nodes WHERE pending=1 ORDER BY id LIMIT 1",
                [],
                |r| Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?))),
            )
            .optional()?;
        self.priority_streak = 0;
        Ok(next)
    }
}
pub(super) fn select_batch(
    db: &StorageIndex,
    state: &Mutex<ScanSchedulingState>,
    limit: usize,
) -> Result<Vec<(i64, PathBuf)>> {
    let mut state = state
        .lock()
        .map_err(|_| anyhow!("storage_priority_unavailable"))?;
    let mut result = Vec::with_capacity(limit);
    while result.len() < limit {
        let Some(next) = state.choose(db)? else {
            break;
        };
        db.conn.execute(
            "UPDATE nodes SET pending=2 WHERE id=?1 AND pending=1",
            [next.0],
        )?;
        result.push(next);
    }
    Ok(result)
}
fn id(value: &str) -> Result<i64> {
    value
        .strip_prefix('n')
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| anyhow!("storage_not_found"))
}
fn open(dir: &Path) -> Result<Connection> {
    super::reject_symlink(dir)?;
    super::reject_symlink(&dir.join("index.sqlite3"))?;
    let db = Connection::open_with_flags(
        dir.join("index.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.busy_timeout(Duration::from_millis(50))?;
    db.execute_batch("PRAGMA query_only=ON; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-512;")?;
    Ok(db)
}
#[derive(Serialize, Deserialize)]
struct DiscoveryCursor {
    job_id: String,
    parent: i64,
    after: i64,
}
pub(super) fn children(dir: &Path, q: &StorageLiveQuery) -> Result<StorageLivePage> {
    if !(1..=100).contains(&q.limit) {
        bail!("storage_invalid_limit");
    }
    let parent = id(&q.node_id)?;
    let after = if let Some(raw) = &q.cursor {
        if raw.len() > 1024 {
            bail!("storage_invalid_cursor");
        }
        let c: DiscoveryCursor = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(raw)
                .map_err(|_| anyhow!("storage_invalid_cursor"))?,
        )
        .map_err(|_| anyhow!("storage_invalid_cursor"))?;
        if c.job_id != q.job_id || c.parent != parent || c.after < 0 {
            bail!("storage_invalid_cursor");
        }
        c.after
    } else {
        0
    };
    let db = open(dir)?;
    // A short read transaction gives this one page a consistent view without a disk-backed sort.
    db.execute_batch("BEGIN")?;
    let pending: Option<i64> = db
        .query_row(
            "SELECT pending FROM nodes WHERE id=?1 AND kind='directory'",
            [parent],
            |r| r.get(0),
        )
        .optional()?;
    let pending = pending.ok_or_else(|| anyhow!("storage_not_found"))?;
    let mut stmt = db.prepare(CHILDREN_SQL)?;
    let mut items = Vec::with_capacity(q.limit);
    let mut last = after;
    let mut more = false;
    for row in stmt.query_map(params![parent, after, (q.limit + 1) as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })? {
        let (node, parent, name, pending) = row?;
        if items.len() == q.limit {
            more = true;
            break;
        }
        last = node;
        items.push(StorageLiveNode {
            id: format!("n{node}"),
            parent_id: parent.map(|p| format!("n{p}")),
            name,
            enumeration_complete: pending == 0,
        });
    }
    let complete = pending == 0;
    let total = if complete {
        Some(db.query_row("SELECT count(*) FROM nodes INDEXED BY node_parent WHERE parent=?1 AND kind='directory'",[parent],|r|r.get(0))?)
    } else {
        None
    };
    let cursor = if more || !complete {
        Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(&DiscoveryCursor {
            job_id: q.job_id.clone(),
            parent,
            after: last,
        })?))
    } else {
        None
    };
    Ok(StorageLivePage {
        job_id: q.job_id.clone(),
        revision: 0,
        root_node_id: "n1".into(),
        node_id: q.node_id.clone(),
        items,
        total_count: total,
        next_cursor: cursor,
        enumeration_complete: complete,
    })
}
pub(super) fn pending(dir: &Path, node: &str) -> Result<Option<i64>> {
    let db = open(dir)?;
    let node = id(node)?;
    let pending: Option<i64> = db
        .query_row(
            "SELECT pending FROM nodes WHERE id=?1 AND kind='directory'",
            [node],
            |r| r.get(0),
        )
        .optional()?;
    match pending {
        Some(1) => Ok(Some(node)),
        Some(_) => Ok(None),
        None => bail!("storage_not_found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn priority_mailbox_is_bounded_and_retries_are_idempotent() {
        let mut state = ScanSchedulingState::default();
        for id in 1..=64 {
            assert!(state.enqueue(id).unwrap().0);
        }
        assert!(!state.enqueue(1).unwrap().0);
        assert_eq!(
            state.enqueue(65).unwrap_err().to_string(),
            "storage_priority_queue_full"
        );
        assert_eq!(state.len(), 64);
    }
}
