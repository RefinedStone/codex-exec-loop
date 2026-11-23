//! Gateway boot reads only discovery candidates; each repository independently proves authority.
use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::{
    IssueAutomationRestartEntry, IssueAutomationRestartRegistryPort,
};
use crate::composition::repository_kernel::{RepositoryKernelRegistry, RepositoryRestartOutcome};

pub(super) fn start(
    kernels: Arc<RepositoryKernelRegistry>,
    registry: Arc<dyn IssueAutomationRestartRegistryPort>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        if kernels.is_stopping() {
            return;
        }
        if let Err(error) = scan(
            registry.as_ref(),
            || kernels.is_stopping(),
            |candidate| kernels.restore_registered(candidate, registry.as_ref()),
        ) {
            // A broken discovery registry must not take away manual Gateway access.
            tracing::warn!(error = %format!("{error:#}"), "issue automation restart discovery failed");
        }
    })
}

fn scan(
    registry: &dyn IssueAutomationRestartRegistryPort,
    should_stop: impl Fn() -> bool,
    restore: impl Fn(&IssueAutomationRestartEntry) -> Result<RepositoryRestartOutcome>,
) -> Result<()> {
    if should_stop() {
        return Ok(());
    }
    // list returns a detached snapshot. No registry lock is held while a repository starts.
    for candidate in registry.list()? {
        if should_stop() {
            break;
        }
        match restore(&candidate) {
            Ok(outcome) => tracing::info!(
                repository = %candidate.repository_key, ?outcome,
                "issue automation restart candidate checked"
            ),
            Err(error) => tracing::warn!(
                repository = %candidate.repository_key, error = %format!("{error:#}"),
                "issue automation restart candidate skipped"
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use std::sync::Mutex;

    struct Registry {
        entries: Vec<IssueAutomationRestartEntry>,
        broken: bool,
    }
    impl IssueAutomationRestartRegistryPort for Registry {
        fn register(&self, _: &IssueAutomationRestartEntry) -> Result<()> {
            panic!("boot does not register");
        }
        fn unregister(&self, _: &str) -> Result<()> {
            panic!("the fake restoration owns cleanup");
        }
        fn list(&self) -> Result<Vec<IssueAutomationRestartEntry>> {
            if self.broken {
                bail!("unreadable test registry");
            }
            Ok(self.entries.clone())
        }
    }

    #[test]
    fn startup_without_a_client_continues_after_one_repository_fails() {
        let registry = Registry {
            entries: ["locked", "ready"]
                .into_iter()
                .map(|key| IssueAutomationRestartEntry {
                    repository_key: key.into(),
                    workspace_root: "/synthetic".into(),
                })
                .collect(),
            broken: false,
        };
        let checked = Mutex::new(Vec::new());
        scan(
            &registry,
            || false,
            |entry| {
                checked.lock().unwrap().push(entry.repository_key.clone());
                if entry.repository_key == "locked" {
                    bail!("another owner holds the repository");
                }
                Ok(RepositoryRestartOutcome::Started)
            },
        )
        .unwrap();
        assert_eq!(*checked.lock().unwrap(), ["locked", "ready"]);
    }

    #[test]
    fn shutdown_stops_the_scan_before_resolving_another_candidate() {
        let kernels = Arc::new(RepositoryKernelRegistry::default());
        let registry = Registry {
            entries: ["first", "must-not-resolve"]
                .into_iter()
                .map(|key| IssueAutomationRestartEntry {
                    repository_key: key.into(),
                    workspace_root: "/synthetic".into(),
                })
                .collect(),
            broken: false,
        };
        let visited = Arc::new(Mutex::new(Vec::new()));
        let (entered, entry) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let scanning = kernels.clone();
        let calls = visited.clone();
        let worker = std::thread::spawn(move || {
            scan(
                &registry,
                || scanning.is_stopping(),
                |candidate| {
                    calls.lock().unwrap().push(candidate.repository_key.clone());
                    entered.send(()).unwrap();
                    released
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                    Ok(RepositoryRestartOutcome::Started)
                },
            )
        });
        entry
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        kernels.shutdown();
        release.send(()).unwrap();
        worker.join().unwrap().unwrap();
        assert_eq!(*visited.lock().unwrap(), ["first"]);
    }

    #[tokio::test]
    async fn broken_startup_discovery_finishes_without_stopping_manual_registry_access() {
        struct ManualWorkspace {
            root: std::path::PathBuf,
            runtime: std::path::PathBuf,
        }
        impl Drop for ManualWorkspace {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.root);
                let _ = std::fs::remove_dir_all(&self.runtime);
            }
        }
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-manual-after-broken-restart-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let workspace = root.display().to_string();
        let location = crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&workspace).unwrap();
        let _workspace = ManualWorkspace {
            root,
            runtime: std::path::PathBuf::from(location.runtime_dir),
        };
        let kernels = Arc::new(RepositoryKernelRegistry::default());
        start(
            kernels.clone(),
            Arc::new(Registry {
                entries: Vec::new(),
                broken: true,
            }),
        )
        .await
        .unwrap();
        assert!(
            !kernels.is_stopping(),
            "discovery does not shut down manual Gateway access"
        );
        // The existing kernel test composition injects an offline source/origin. Merely
        // attaching and querying never dispatches startup checks or a provider/model turn.
        let (client, key, primary) = kernels
            .attach_with_workspace_root(&workspace, false)
            .unwrap();
        assert_eq!(client.client_id(), 1);
        assert_eq!(key, location.repository_identity);
        assert_eq!(primary, location.canonical_repo_root);
        assert!(
            client.query().is_some(),
            "manual attachment starts a responsive kernel after discovery fails"
        );
        drop(client);
        kernels.shutdown();
    }
}
