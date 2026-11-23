use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::application::port::inbound::task_delivery_evidence_port::TaskDeliveryEvidenceGatePort;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::configuration::{ConfigurationService, current_process_config_for_workspace};
use crate::domain::planning::delivery_evidence::TaskEvidenceDeliveryMode;

static DELIVERY_POLICY_CACHE: OnceLock<DeliveryPolicyCache> = OnceLock::new();

pub(crate) fn task_delivery_evidence_for_workspace(
    workspace: &str,
    authority: &dyn PlanningAuthorityPort,
) -> Arc<dyn TaskDeliveryEvidenceGatePort> {
    let configuration = resolve_for_workspace(
        workspace,
        authority,
        DELIVERY_POLICY_CACHE.get_or_init(DeliveryPolicyCache::default),
        |canonical| {
            let resolved =
                match current_process_config_for_workspace(canonical).map_err(|error| {
                    format!("delivery evidence configuration is unavailable: {error:#}")
                })? {
                    Some(snapshot) => snapshot,
                    None => ConfigurationService::resolve_read_only(canonical, &[]).map_err(
                        |error| {
                            format!("delivery evidence configuration is unavailable: {error:#}")
                        },
                    )?,
                };
            Ok(resolved.config.parallel.evidence_mode)
        },
    );
    // The graph retains this value (including an error), not a closure that can read a
    // later worker/integration worktree file. A missing setting resolves to Observe;
    // an unreadable or malformed layer stays an unavailable, closed gate.
    crate::composition::task_delivery_evidence::build(
        &configuration.workspace,
        &configuration.repository_identity,
        configuration.mode,
    )
}

struct ResolvedDeliveryConfiguration {
    workspace: String,
    repository_identity: String,
    mode: Result<TaskEvidenceDeliveryMode, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DeliveryPolicyKey {
    RepositoryIdentity(String),
    CanonicalAuthorityRoot(String),
}

#[derive(Default)]
struct DeliveryPolicyCache {
    modes: Mutex<BTreeMap<DeliveryPolicyKey, Result<TaskEvidenceDeliveryMode, String>>>,
}

impl DeliveryPolicyCache {
    fn resolve_once(
        &self,
        key: DeliveryPolicyKey,
        resolve: impl FnOnce() -> Result<TaskEvidenceDeliveryMode, String>,
    ) -> Result<TaskEvidenceDeliveryMode, String> {
        let mut modes = self
            .modes
            .lock()
            .map_err(|_| "delivery evidence policy cache is unavailable".to_owned())?;
        // Rebuilding a service must retain the first result, including an error.
        // Only a new process may read changed project policy for this repository.
        modes.entry(key).or_insert_with(resolve).clone()
    }
}

fn resolve_for_workspace(
    workspace: &str,
    authority: &dyn PlanningAuthorityPort,
    cache: &DeliveryPolicyCache,
    resolve_mode: impl FnOnce(&Path) -> Result<TaskEvidenceDeliveryMode, String>,
) -> ResolvedDeliveryConfiguration {
    let location = match authority.resolve_authority_location(workspace) {
        Ok(location) if Path::new(&location.canonical_repo_root).is_absolute() => location,
        Ok(_) => {
            return ResolvedDeliveryConfiguration {
                workspace: workspace.to_owned(),
                repository_identity: String::new(),
                mode: Err("canonical delivery workspace is unavailable".to_owned()),
            };
        }
        Err(error) => {
            return ResolvedDeliveryConfiguration {
                workspace: workspace.to_owned(),
                repository_identity: String::new(),
                mode: Err(format!(
                    "canonical delivery workspace is unavailable: {error:#}"
                )),
            };
        }
    };
    let canonical = location.canonical_repo_root;
    let repository_identity = location.repository_identity;
    let mode = if repository_identity.trim().is_empty() {
        // The path is sufficient to pin the error, but never substitutes for an
        // authority incarnation when an existing graph authorizes a later effect.
        cache.resolve_once(
            DeliveryPolicyKey::CanonicalAuthorityRoot(canonical.clone()),
            || Err("canonical repository identity is unavailable".to_owned()),
        )
    } else {
        cache.resolve_once(
            DeliveryPolicyKey::RepositoryIdentity(repository_identity.clone()),
            || resolve_mode(Path::new(&canonical)),
        )
    };
    ResolvedDeliveryConfiguration {
        workspace: canonical,
        repository_identity,
        mode,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use std::cell::Cell;

    #[test]
    fn canonical_delivery_policy_is_resolved_once_and_does_not_follow_later_changes() {
        let canonical = std::env::temp_dir().join("akra-evidence-canonical-config");
        let linked = std::env::temp_dir().join("akra-evidence-linked-config");
        let authority = NoopPlanningAuthorityPort::default()
            .with_canonical_repo_root(canonical.to_string_lossy().into_owned());
        let reads = Cell::new(0);
        let current = Cell::new(TaskEvidenceDeliveryMode::Enforce);
        let cache = DeliveryPolicyCache::default();
        let configuration =
            resolve_for_workspace(linked.to_str().unwrap(), &authority, &cache, |path| {
                reads.set(reads.get() + 1);
                assert_eq!(path, canonical.as_path());
                Ok(current.get())
            });
        current.set(TaskEvidenceDeliveryMode::Observe);
        let rebuilt = resolve_for_workspace("another-worker", &authority, &cache, |_| {
            reads.set(reads.get() + 1);
            Ok(current.get())
        });

        assert_eq!(reads.get(), 1);
        assert_eq!(configuration.workspace, canonical.to_string_lossy());
        assert_eq!(
            configuration.repository_identity,
            canonical.to_string_lossy()
        );
        assert_eq!(configuration.mode, Ok(TaskEvidenceDeliveryMode::Enforce));
        assert_eq!(rebuilt.mode, configuration.mode);
    }

    #[test]
    fn malformed_delivery_configuration_remains_unavailable() {
        let canonical = std::env::temp_dir().join("akra-evidence-config-error");
        let authority = NoopPlanningAuthorityPort::default()
            .with_canonical_repo_root(canonical.to_string_lossy().into_owned());
        let configuration = resolve_for_workspace(
            "caller-worktree",
            &authority,
            &DeliveryPolicyCache::default(),
            |_| Err("invalid parallel.evidence_mode".to_owned()),
        );
        assert_eq!(configuration.workspace, canonical.to_string_lossy());
        assert_eq!(
            configuration.mode,
            Err("invalid parallel.evidence_mode".to_owned())
        );
    }

    #[test]
    fn unavailable_canonical_workspace_never_reads_the_callers_configuration() {
        let authority = NoopPlanningAuthorityPort::default()
            .with_canonical_repo_root("not-an-absolute-authority-root");
        let configuration = resolve_for_workspace(
            "caller-worktree",
            &authority,
            &DeliveryPolicyCache::default(),
            |_| {
                panic!("do not fall back to a worker's configuration");
            },
        );
        assert!(configuration.mode.is_err());
    }

    struct ConfigFile(std::path::PathBuf);

    impl ConfigFile {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

            for _ in 0..32 {
                let nonce = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
                let root = std::env::temp_dir().join(format!(
                    "akra-delivery-policy-{}-{nonce}",
                    std::process::id(),
                ));
                match std::fs::create_dir(&root) {
                    Ok(()) => return Self(root),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("could not create delivery policy fixture: {error}"),
                }
            }
            panic!("could not allocate an exclusive delivery policy fixture");
        }

        fn authority(&self) -> NoopPlanningAuthorityPort {
            NoopPlanningAuthorityPort::default()
                .with_canonical_repo_root(self.0.to_string_lossy().into_owned())
        }

        fn write(&self, body: &str) {
            std::fs::write(self.0.join("config.toml"), body).unwrap();
        }

        fn read_mode(root: &Path) -> Result<TaskEvidenceDeliveryMode, String> {
            let body = std::fs::read_to_string(root.join("config.toml"))
                .map_err(|error| error.to_string())?;
            let layer: crate::configuration::AkraConfigLayer =
                toml::from_str(&body).map_err(|error| error.to_string())?;
            Ok(layer
                .parallel
                .and_then(|parallel| parallel.evidence_mode)
                .unwrap_or_default())
        }
    }

    impl Drop for ConfigFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn rebuilt_services_keep_the_first_mode_after_file_change_removal_and_corruption() {
        let fixture = ConfigFile::new();
        fixture.write("[parallel]\nevidence_mode = 'enforce'\n");
        let authority = fixture.authority();
        let cache = DeliveryPolicyCache::default();
        let first = resolve_for_workspace("linked-a", &authority, &cache, ConfigFile::read_mode);
        assert_eq!(first.mode, Ok(TaskEvidenceDeliveryMode::Enforce));

        fixture.write("[parallel]\nevidence_mode = 'observe'\n");
        let changed = resolve_for_workspace("linked-b", &authority, &cache, ConfigFile::read_mode);
        assert_eq!(changed.mode, first.mode);
        std::fs::remove_file(fixture.0.join("config.toml")).unwrap();
        let removed =
            resolve_for_workspace("integration", &authority, &cache, ConfigFile::read_mode);
        assert_eq!(removed.mode, first.mode);
        fixture.write("[parallel]\nevidence_mode = 'disabled'\n");
        let malformed = resolve_for_workspace("worker", &authority, &cache, ConfigFile::read_mode);
        assert_eq!(malformed.mode, first.mode);
    }

    #[test]
    fn a_first_configuration_error_remains_closed_after_file_repair() {
        let fixture = ConfigFile::new();
        fixture.write("[parallel]\nevidence_mode = 'invalid'\n");
        let authority = fixture.authority();
        let cache = DeliveryPolicyCache::default();
        let first = resolve_for_workspace("linked-a", &authority, &cache, ConfigFile::read_mode);
        assert!(first.mode.is_err());

        fixture.write("[parallel]\nevidence_mode = 'enforce'\n");
        let repaired = resolve_for_workspace("linked-b", &authority, &cache, ConfigFile::read_mode);
        assert_eq!(repaired.mode, first.mode);
        std::fs::remove_file(fixture.0.join("config.toml")).unwrap();
        let removed =
            resolve_for_workspace("integration", &authority, &cache, ConfigFile::read_mode);
        assert_eq!(removed.mode, first.mode);
    }

    #[test]
    fn different_canonical_repositories_keep_independent_policies() {
        let first = ConfigFile::new();
        let second = ConfigFile::new();
        first.write("[parallel]\nevidence_mode = 'enforce'\n");
        second.write("[parallel]\nevidence_mode = 'observe'\n");
        let cache = DeliveryPolicyCache::default();
        let first_configuration = resolve_for_workspace(
            "linked-a",
            &first.authority(),
            &cache,
            ConfigFile::read_mode,
        );
        let second_configuration = resolve_for_workspace(
            "linked-b",
            &second.authority(),
            &cache,
            ConfigFile::read_mode,
        );
        assert_ne!(
            first_configuration.workspace,
            second_configuration.workspace
        );
        assert_eq!(
            first_configuration.mode,
            Ok(TaskEvidenceDeliveryMode::Enforce)
        );
        assert_eq!(
            second_configuration.mode,
            Ok(TaskEvidenceDeliveryMode::Observe)
        );
    }
}
