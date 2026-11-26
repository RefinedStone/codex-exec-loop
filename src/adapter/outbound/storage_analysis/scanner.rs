use super::{
    index::{Observation, StorageIndex},
    native, worktree,
};
use crate::domain::storage_analysis::*;
use anyhow::{Result, anyhow};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, sync_channel},
    },
    time::{Duration, Instant},
};

enum Event {
    Entry(Observation),
    Issue(i64, StorageIssue),
    Done(i64),
}

pub(super) struct ScanControl<'a> {
    pub root_volume_binding: native::RootVolumeBinding,
    pub cancelled: &'a AtomicBool,
    pub progress: &'a (dyn Fn(ScanProgress) + Sync),
    pub runtime_probe: Option<&'a super::StorageRuntimeProbe>,
    pub scheduling: &'a std::sync::Mutex<super::live::ScanSchedulingState>,
}
pub(super) fn scan(
    db: &mut StorageIndex,
    root: &Path,
    index_root: &Path,
    request: &ScanRequest,
    control: ScanControl<'_>,
) -> Result<()> {
    let ScanControl {
        root_volume_binding,
        cancelled,
        progress,
        runtime_probe,
        scheduling,
    } = control;
    let root = fs::canonicalize(root)?;
    let index_root = fs::canonicalize(index_root)?;
    let root_meta = native::metadata(&root, &root, None)?;
    if root_meta.issue.is_some() || !root_meta.directory {
        return Err(anyhow!("storage_unverified_root"));
    }
    native::verify_root_volume(&root_meta, &request.volume_id, root_volume_binding)?;
    db.conn.execute(
        "UPDATE nodes SET object_key=?1 WHERE id=1",
        [root_meta.identity.clone()],
    )?;
    let volume = root_meta.volume.clone();
    let mut snapshot = db.snapshot()?;
    let abort = AtomicBool::new(false);
    let mut last = Instant::now();
    let mut first_file_reported = false;
    progress(ScanProgress {
        volume_id: request.volume_id.clone(),
        snapshot_id: snapshot.id.clone(),
        files_scanned: 0,
        directories_scanned: 0,
        known_allocated_bytes: "0".into(),
        state: StorageState::Running,
    });
    loop {
        if cancelled.load(Ordering::Acquire) {
            #[cfg(test)]
            super::live::record_cancel(scheduling);
            break;
        }
        db.conn.execute_batch("BEGIN IMMEDIATE")?;
        let batch = super::live::select_batch(db, scheduling, request.options.concurrency)?;
        if batch.is_empty() {
            db.conn.execute_batch("COMMIT")?;
            break;
        }
        let outcome = std::thread::scope(|scope| -> Result<()> {
            let (tx, rx) = sync_channel::<Event>(128);
            for (id, path) in batch {
                if cancelled.load(Ordering::Acquire) {
                    #[cfg(test)]
                    super::live::record_cancel(scheduling);
                    break;
                }
                let tx = tx.clone();
                let root = &root;
                let excluded = &index_root;
                let volume = &volume;
                let abort = &abort;
                scope.spawn(move || {
                    enumerate(
                        id,
                        path,
                        EnumerationContext {
                            root,
                            excluded,
                            volume,
                            cancelled,
                            abort,
                        },
                        tx,
                    )
                });
                #[cfg(test)]
                super::live::record_dispatch(scheduling, cancelled.load(Ordering::Acquire));
            }
            drop(tx);
            let mut failure = None;
            let mut since_commit = 0_u32;
            for event in rx {
                if failure.is_some() {
                    continue;
                }
                let write = match event {
                    Event::Entry(o) => {
                        if !o.directory {
                            snapshot.files_scanned += 1;
                        }
                        db.record(o).map(|_| ())
                    }
                    Event::Issue(id, issue) => db.mark_issue(id, issue),
                    Event::Done(id) => {
                        snapshot.directories_scanned += 1;
                        db.visited(id)
                    }
                };
                if let Err(error) = write {
                    abort.store(true, Ordering::Release);
                    failure = Some(error);
                }
                since_commit += 1;
                if failure.is_none() && since_commit >= 128 {
                    let commit = db.save_snapshot(&snapshot).and_then(|_| {
                        db.conn
                            .execute_batch("COMMIT; BEGIN IMMEDIATE")
                            .map_err(Into::into)
                    });
                    if let Err(error) = commit {
                        abort.store(true, Ordering::Release);
                        failure = Some(error);
                    }
                    since_commit = 0;
                }
                if (!first_file_reported && snapshot.files_scanned > 0)
                    || last.elapsed() >= Duration::from_millis(200)
                {
                    first_file_reported |= snapshot.files_scanned > 0;
                    progress(ScanProgress {
                        volume_id: request.volume_id.clone(),
                        snapshot_id: snapshot.id.clone(),
                        files_scanned: snapshot.files_scanned,
                        directories_scanned: snapshot.directories_scanned,
                        known_allocated_bytes: db.known_total.get().to_string(),
                        state: if cancelled.load(Ordering::Acquire) {
                            StorageState::Cancelled
                        } else {
                            StorageState::Running
                        },
                    });
                    last = Instant::now();
                }
            }
            if let Some(error) = failure {
                Err(error)
            } else {
                Ok(())
            }
        });
        if let Err(error) = outcome {
            let _ = db.conn.execute_batch("ROLLBACK");
            return Err(error);
        }
        db.save_snapshot(&snapshot)?;
        db.conn.execute_batch("COMMIT")?;
    }
    db.aggregate()?;
    if !cancelled.load(Ordering::Acquire) {
        worktree::annotate(db, cancelled, runtime_probe)?;
    }
    let partial: bool = db
        .conn
        .query_row("SELECT unknown FROM nodes WHERE id=1", [], |r| r.get(0))?;
    snapshot.state = if cancelled.load(Ordering::Acquire) {
        StorageState::Cancelled
    } else if partial {
        StorageState::Partial
    } else {
        StorageState::Complete
    };
    snapshot.finished_at = Some(chrono::Utc::now().to_rfc3339());
    if partial {
        snapshot.issues.push(StorageIssue::one(
            "partial_observation",
            "Some paths or allocations could not be verified; inspect node reasons",
        ));
    }
    db.save_snapshot(&snapshot)?;
    progress(ScanProgress {
        volume_id: request.volume_id.clone(),
        snapshot_id: snapshot.id.clone(),
        files_scanned: snapshot.files_scanned,
        directories_scanned: snapshot.directories_scanned,
        known_allocated_bytes: db.known_total.get().to_string(),
        state: snapshot.state,
    });
    Ok(())
}
fn stopped(cancelled: &AtomicBool, abort: &AtomicBool) -> bool {
    cancelled.load(Ordering::Acquire) || abort.load(Ordering::Acquire)
}
struct EnumerationContext<'a> {
    root: &'a Path,
    excluded: &'a Path,
    volume: &'a str,
    cancelled: &'a AtomicBool,
    abort: &'a AtomicBool,
}
fn enumerate(id: i64, path: PathBuf, context: EnumerationContext<'_>, tx: SyncSender<Event>) {
    let EnumerationContext {
        root,
        excluded,
        volume,
        cancelled,
        abort,
    } = context;
    if stopped(cancelled, abort) {
        return;
    }
    let handle = match native::metadata(&path, root, Some(volume)) {
        Ok(meta) if meta.directory && meta.issue.is_none() => meta,
        Ok(meta) => {
            let _ = tx.send(Event::Issue(
                id,
                meta.issue
                    .unwrap_or_else(|| StorageIssue::one("path_changed", "Directory was replaced")),
            ));
            let _ = tx.send(Event::Done(id));
            return;
        }
        Err(error) => {
            let _ = tx.send(Event::Issue(id, native::classify_error(&error)));
            let _ = tx.send(Event::Done(id));
            return;
        }
    };
    let entries = match fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(error) => {
            let _ = tx.send(Event::Issue(id, native::classify_error(&error.into())));
            let _ = tx.send(Event::Done(id));
            return;
        }
    };
    for entry in entries {
        if stopped(cancelled, abort) {
            return;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(error) => {
                let _ = tx.send(Event::Issue(id, native::classify_error(&error.into())));
                continue;
            }
        };
        let child = entry.path();
        // Exclude the whole private index directory including journals, temporary files and other jobs.
        if child == excluded || child.starts_with(excluded) {
            continue;
        }
        let meta = match native::metadata(&child, root, Some(volume)) {
            Ok(meta) => meta,
            Err(error) => {
                let _ = tx.send(Event::Issue(id, native::classify_error(&error)));
                continue;
            }
        };
        let observation = Observation {
            parent: id,
            name: entry.file_name().to_string_lossy().into_owned(),
            path: child,
            directory: meta.directory,
            logical: if meta.directory { 0 } else { meta.logical },
            allocated: meta.allocated,
            identity: meta.identity,
            issue: meta.issue,
        };
        if tx.send(Event::Entry(observation)).is_err() {
            return;
        }
    }
    drop(handle);
    let _ = tx.send(Event::Done(id));
}
