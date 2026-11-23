//! Observes only already-running kernels. No attach, launch, session, lease, or Git mutation.
use crate::adapter::outbound::storage_analysis::StorageRuntimeRoots;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::core::app::ConversationSnapshot;
use crate::domain::storage_analysis::StorageEvidence;

use super::{RepositoryKernelRegistry, TryLockError};

impl RepositoryKernelRegistry {
    pub(crate) fn storage_known_roots(&self, limit: usize) -> StorageRuntimeRoots {
        let limit = limit.min(256);
        let mut roots = StorageRuntimeRoots {
            paths: Vec::new(),
            complete: true,
        };
        let repositories = match self.repositories.try_lock() {
            Ok(entries) => entries,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return StorageRuntimeRoots::default(),
        };
        roots.complete = repositories.len() <= limit;
        let entries = repositories
            .values()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        drop(repositories);
        let started = Instant::now();
        for entry in entries {
            if started.elapsed() >= Duration::from_secs(1) {
                roots.complete = false;
                break;
            }
            let kernel = match entry.kernel.try_lock() {
                Ok(kernel) => kernel,
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    roots.complete = false;
                    continue;
                }
            };
            let Some(kernel) = kernel.as_ref() else {
                continue;
            };
            record_known_root(&mut roots, kernel.workspace_root(), limit);
            let view = match kernel.host.observe(Duration::from_millis(200)) {
                Ok(view) => view,
                Err(_) => {
                    roots.complete = false;
                    continue;
                }
            };
            if let ConversationSnapshot::Ready(ready) = &view.snapshot.conversation {
                record_known_root(&mut roots, &ready.workspace_directory, limit);
            }
            if let Some(supervisor) = &view.parallel.supervisor {
                for detail in &supervisor.detail.lane_sessions {
                    record_known_root(&mut roots, &detail.worktree_path, limit);
                }
            }
        }
        roots
    }

    pub(crate) fn storage_runtime_evidence(&self, worktree: &Path) -> Vec<StorageEvidence> {
        let started = Instant::now();
        let mut reasons = Vec::new();
        let mut matched = false;
        for entry in self.entries() {
            if started.elapsed() >= Duration::from_secs(1) {
                reasons.push(evidence(
                    "runtime_unknown",
                    "조회 시간 한도로 일부 호스트 상태를 확인하지 못했습니다.",
                ));
                break;
            }
            let kernel = match entry.kernel.try_lock() {
                Ok(kernel) => kernel,
                Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    reasons.push(evidence(
                        "runtime_unknown",
                        "호스트 전환 중 상태를 확정할 수 없습니다.",
                    ));
                    continue;
                }
            };
            let Some(kernel) = kernel.as_ref() else {
                continue;
            };
            let primary = same_tree(Path::new(kernel.workspace_root()), worktree);
            let view = match kernel.host.observe(Duration::from_millis(200)) {
                Ok(view) => view,
                Err(_) => {
                    if primary {
                        reasons.push(evidence(
                            "runtime_unknown",
                            "기존 호스트가 조회에 응답하지 않았습니다.",
                        ));
                    }
                    continue;
                }
            };
            matched |= primary;
            if let ConversationSnapshot::Ready(ready) = &view.snapshot.conversation
                && same_tree(Path::new(&ready.workspace_directory), worktree)
            {
                matched = true;
                if view.snapshot.conversation_runtime.has_active_turn()
                    || view.snapshot.conversation_runtime.approval.is_some()
                    || view
                        .snapshot
                        .conversation_runtime
                        .auto_follow
                        .has_live_activity()
                    || view.snapshot.conversation_runtime.post_turn.is_in_flight()
                {
                    reasons.push(evidence(
                        "active_session",
                        "이 worktree에서 실행·승인·후속 처리가 진행 중입니다.",
                    ));
                }
            }
            if let Some(supervisor) = &view.parallel.supervisor {
                for detail in &supervisor.detail.lane_sessions {
                    if same_tree(Path::new(&detail.worktree_path), worktree) {
                        matched = true;
                        reasons.push(StorageEvidence {
                            code: "akra_runtime_slot".into(),
                            detail: format!(
                                "Akra 슬롯 상태: {}. 이 관찰만으로 해제 여부를 판정하지 않습니다.",
                                detail.state_label
                            ),
                            observed_at: detail.updated_at.clone(),
                        });
                    }
                }
            }
            if primary
                && view.thread_activity.iter().any(|thread| {
                    thread.running || thread.approval.is_some() || thread.auto_follow_active
                })
            {
                // A background thread's exact cwd may not be exposed by the current projection.
                // Preserve the conservative evidence instead of calling this checkout inactive.
                reasons.push(evidence(
                    "repository_active_session",
                    "같은 저장소의 대화가 실행 중이며 worktree 소유 범위는 추가 확인이 필요합니다.",
                ));
            }
        }
        if matched && reasons.is_empty() {
            reasons.push(evidence(
                "runtime_observed",
                "현존 호스트 상태를 조회했습니다. 삭제 가능 판정이 아닙니다.",
            ));
        }
        reasons
    }
}

fn record_known_root(roots: &mut StorageRuntimeRoots, path: &str, limit: usize) {
    if path.len() > 32768 || path.is_empty() {
        roots.complete = false;
        return;
    }
    let path = PathBuf::from(path);
    if roots.paths.contains(&path) {
        return;
    }
    if roots.paths.len() >= limit {
        roots.complete = false;
        return;
    }
    roots.paths.push(path);
}
fn evidence(code: &str, detail: &str) -> StorageEvidence {
    StorageEvidence {
        code: code.into(),
        detail: detail.into(),
        observed_at: chrono::Utc::now().to_rfc3339(),
    }
}
fn same_tree(candidate: &Path, root: &Path) -> bool {
    let (Ok(candidate), Ok(root)) = (
        std::fs::canonicalize(candidate),
        std::fs::canonicalize(root),
    ) else {
        return false;
    };
    #[cfg(windows)]
    {
        let candidate = candidate.to_string_lossy().to_lowercase();
        let root = root
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase();
        candidate == root
            || candidate
                .strip_prefix(&root)
                .is_some_and(|tail| tail.starts_with(['\\', '/']))
    }
    #[cfg(not(windows))]
    {
        candidate.starts_with(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observing_storage_does_not_create_kernel_entries() {
        let registry = RepositoryKernelRegistry::default();
        assert!(
            registry
                .storage_runtime_evidence(Path::new(env!("CARGO_MANIFEST_DIR")))
                .is_empty()
        );
        assert!(registry.entries().is_empty());
        let roots = registry.storage_known_roots(16);
        assert!(roots.paths.is_empty() && roots.complete);
        assert!(registry.entries().is_empty());
    }
    #[test]
    fn storage_known_root_inventory_is_bounded_and_reports_omissions() {
        let mut roots = StorageRuntimeRoots {
            paths: Vec::new(),
            complete: true,
        };
        record_known_root(&mut roots, "fixture-one", 2);
        record_known_root(&mut roots, "fixture-one", 2);
        record_known_root(&mut roots, "fixture-two", 2);
        assert_eq!(roots.paths.len(), 2);
        assert!(roots.complete);
        record_known_root(&mut roots, "fixture-three", 2);
        assert_eq!(roots.paths.len(), 2);
        assert!(!roots.complete);
    }
}
