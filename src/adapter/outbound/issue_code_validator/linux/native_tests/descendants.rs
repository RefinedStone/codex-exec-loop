//! Observe only descendants of the calling test thread, never signal a guessed PID.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

#[derive(Clone)]
struct Identity {
    pid: u32,
    parent: u32,
    birth: String,
    state: String,
    session: u32,
    validator: bool,
    detached_validator_child: bool,
}

fn process_stat(pid: u32) -> io::Result<Option<Identity>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid process identity");
    let (_, fields) = stat.rsplit_once(") ").ok_or_else(invalid)?;
    let fields = fields.split_whitespace().collect::<Vec<_>>();
    Ok(Some(Identity {
        pid,
        parent: fields
            .get(1)
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?,
        birth: fields.get(19).ok_or_else(invalid)?.to_string(),
        state: fields.first().ok_or_else(invalid)?.to_string(),
        session: fields
            .get(3)
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?,
        validator: false,
        detached_validator_child: false,
    }))
}

fn identity(pid: u32) -> Option<Identity> {
    let mut entry = process_stat(pid).ok()??;
    let command = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    entry.validator = command
        .split(|value| *value == 0)
        .any(|value| value == b"/validator");
    Some(entry)
}

fn children(path: &str) -> Vec<u32> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok())
        .collect()
}

pub(super) struct Observed(BTreeMap<u32, Identity>);

impl Observed {
    pub(super) fn assert_detached_validator_was_seen_and_stopped(self) {
        assert!(
            self.0.values().filter(|entry| entry.validator).count() >= 2,
            "both the real validator and its forked child must have been observed"
        );
        assert!(
            self.0.values().any(|entry| entry.detached_validator_child),
            "a child of an observed validator must have detached with setsid"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let running = self
                .0
                .values()
                .filter(|entry| {
                    process_stat(entry.pid)
                        .expect("cannot verify owned descendant exit")
                        .is_some_and(|current| current.birth == entry.birth && current.state != "Z")
                })
                .count();
            if running == 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned sandbox descendants are still running"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

pub(super) struct Observer {
    stopped: Arc<AtomicBool>,
    thread: JoinHandle<Observed>,
}

impl Observer {
    pub(super) fn start() -> Self {
        // SAFETY: gettid has no preconditions; the observer is read-only.
        let tid = unsafe { libc::gettid() };
        let root = format!("/proc/self/task/{tid}/children");
        let baseline = children(&root).into_iter().collect::<BTreeSet<_>>();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let thread = thread::spawn(move || {
            let mut owned = BTreeMap::<u32, Identity>::new();
            while !stop.load(Ordering::SeqCst) {
                let mut pending = children(&root)
                    .into_iter()
                    .filter(|pid| !baseline.contains(pid))
                    .collect::<Vec<_>>();
                pending.extend(owned.values().filter_map(|entry| {
                    identity(entry.pid)
                        .filter(|current| current.birth == entry.birth)
                        .map(|v| v.pid)
                }));
                let mut visited = BTreeSet::new();
                while let Some(pid) = pending.pop() {
                    if !visited.insert(pid) {
                        continue;
                    }
                    if let Some(mut entry) = identity(pid) {
                        pending.extend(children(&format!("/proc/{pid}/task/{pid}/children")));
                        if let Some(previous) =
                            owned.get(&pid).filter(|old| old.birth == entry.birth)
                        {
                            entry.validator |= previous.validator;
                            entry.detached_validator_child |= previous.detached_validator_child;
                        }
                        entry.detached_validator_child |= entry.validator
                            && entry.session == pid
                            && owned
                                .get(&entry.parent)
                                .is_some_and(|parent| parent.validator);
                        owned.insert(pid, entry);
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
            Observed(owned)
        });
        Self { stopped, thread }
    }

    pub(super) fn finish(self) -> Observed {
        self.stopped.store(true, Ordering::SeqCst);
        self.thread.join().expect("native descendant observer")
    }
}
