//! Restore uses real issue services and SQLite, with no provider process or model turn.
use super::*;
use crate::adapter::outbound::filesystem::RepositoryKernelLockHeld;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::domain::issue_automation::autonomy::IssueAutonomyLevel;
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::coordinator::IssueWorkBlockReason;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionRequest,
};
use std::path::Path;
use std::sync::atomic::AtomicUsize;

mod fixture;
mod integration;
mod process;
use fixture::*;

#[test]
fn discovery_reads_existing_identity_without_initializing_missing_or_replaced_repositories() {
    let f = Fixture::new();
    let home = SqlitePlanningAuthorityAdapter::authority_home_root().unwrap();
    let location =
        SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(&f.workspace)
            .unwrap()
            .unwrap();
    assert_eq!(location.repository_identity, f.candidate.repository_key);
    assert!(Path::new(&location.runtime_dir).starts_with(home));
    let missing = f.root.join("missing");
    assert!(
        SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(
            &missing.display().to_string()
        )
        .is_err()
    );
    assert!(!missing.exists());

    let plain = f.root.join("plain");
    std::fs::create_dir(&plain).unwrap();
    assert!(
        SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(
            &plain.display().to_string()
        )
        .unwrap()
        .is_none()
    );
    let fresh = f.root.join("fresh");
    std::fs::create_dir(&fresh).unwrap();
    initialize_git(&fresh);
    assert!(
        SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(
            &fresh.display().to_string()
        )
        .is_err()
    );
    assert!(!fresh.join(".git/akra-repository-id").exists());
    assert!(!fresh.join(".git/akra-repository-id.lock").exists());
}

#[test]
fn separate_git_dir_checkout_resolves_as_its_own_primary_checkout() {
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-restart-separate-git-dir-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    crate::private_directory::ensure_private_directory(&root).unwrap();
    let workspace = root.join("repository");
    std::fs::create_dir(&workspace).unwrap();
    let metadata = root.join("git-metadata");
    let metadata_text = git_path(&metadata);
    git(&workspace, &["init", "--separate-git-dir", &metadata_text]);
    git(
        &workspace,
        &["config", "user.email", "restart-fixture@example.invalid"],
    );
    git(&workspace, &["config", "user.name", "Restart Fixture"]);
    std::fs::write(workspace.join("README"), "separate git dir fixture\n").unwrap();
    git(&workspace, &["add", "README"]);
    git(&workspace, &["commit", "-qm", "fixture"]);

    let workspace_text = workspace.display().to_string();
    let prepared =
        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&workspace_text).unwrap();
    let location = SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(
        &workspace_text,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        location.canonical_repo_root,
        workspace.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(location.repository_identity, prepared.repository_identity);

    std::fs::remove_dir_all(&root).unwrap();
    let _ = std::fs::remove_dir_all(&prepared.runtime_dir);
}
#[test]
fn no_client_boot_collects_and_coordinates_once_across_restart_without_model_turns() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    assert_eq!(
        f.registry.list().unwrap(),
        std::slice::from_ref(&f.candidate)
    );
    let first = f.kernels();
    assert_eq!(
        f.restore(&first).unwrap(),
        RepositoryRestartOutcome::Started
    );
    wait_until(|| {
        f.work().is_some_and(|work| {
            work.task_ids == ["restart-task"]
                && work.blocked_reason == Some(IssueWorkBlockReason::ExecutionDisabled)
                && work.permit_source == Some("a".repeat(40))
        })
    });
    let before = f.work().unwrap();
    assert_eq!(
        before.blocked_reason,
        Some(IssueWorkBlockReason::ExecutionDisabled)
    );
    assert!(f.source.discoveries.load(Ordering::SeqCst) > 0);
    assert!(f.source.resyncs.load(Ordering::SeqCst) > 0);
    assert_eq!(f.execution.0.lock().unwrap().submissions, 1);
    let permits = f.execution.0.lock().unwrap().grants.clone();
    assert_eq!(permits.len(), 1);
    assert!(
        !first
            .entry(&f.candidate.repository_key)
            .unwrap()
            .lock()
            .as_ref()
            .unwrap()
            .host
            .observe(Duration::from_secs(5))
            .unwrap()
            .parallel
            .enabled
    );
    first.shutdown();

    // New adapters and services read the persisted registry and ledger. Only the fake
    // external worker is shared, representing work that survives a Gateway restart.
    let mut reopened = f.reopen();
    reopened.execution = f.execution.clone();
    let candidates = reopened.registry.list().unwrap();
    assert_eq!(candidates, std::slice::from_ref(&f.candidate));
    let second = reopened.kernels();
    assert_eq!(
        second
            .restore_registered_with(
                &candidates[0],
                reopened.registry.as_ref(),
                |_, admission| reopened.recover(admission)
            )
            .unwrap(),
        RepositoryRestartOutcome::Started,
    );
    wait_until(|| reopened.source.discoveries.load(Ordering::SeqCst) > 0);
    wait_until(|| {
        reopened
            .work()
            .is_some_and(|work| work.task_ids == before.task_ids)
    });
    let (client, key, root) = second
        .attach_with_workspace_root(&reopened.workspace, false)
        .unwrap();
    assert_eq!(
        client.client_id(),
        1,
        "neither restore nor observation creates a hidden client"
    );
    assert_eq!(key, f.candidate.repository_key);
    assert_eq!(root, f.candidate.workspace_root);
    second.shutdown();
    assert_eq!(reopened.execution.0.lock().unwrap().submissions, 1);
    assert_eq!(reopened.execution.0.lock().unwrap().grants, permits);
    let after = reopened.work().unwrap();
    assert_eq!(after.order, before.order);
    assert_eq!(after.task_ids, before.task_ids);
    assert_eq!(after.approved_basis, before.approved_basis);
    assert_eq!(after.blocked_reason, before.blocked_reason);
    reopened.no_model_turns();
    f.no_model_turns();
}

#[test]
fn gated_without_selection_and_current_policy_downgrade_never_build_a_runtime() {
    let f = Fixture::new();
    f.registry.register(&f.candidate).unwrap();
    let kernels = f.kernels();
    assert_eq!(
        f.restore(&kernels).unwrap(),
        RepositoryRestartOutcome::Ineligible
    );
    assert!(
        f.registry.list().unwrap().is_empty(),
        "remove only an unselected preregistration orphan"
    );
    assert_eq!(f.builds.load(Ordering::SeqCst), 0);
    f.select(IssueAutonomyLevel::Gated);
    let capped = f
        .policy
        .0
        .lock()
        .unwrap()
        .replace("autonomy_cap = \"gated\"", "autonomy_cap = \"propose\"");
    *f.policy.0.lock().unwrap() = capped;
    assert_eq!(
        f.restore(&kernels).unwrap(),
        RepositoryRestartOutcome::Ineligible
    );
    assert_eq!(
        f.registry.list().unwrap(),
        std::slice::from_ref(&f.candidate),
        "the explicit high selection survives a temporary policy cap"
    );
    assert_eq!(f.builds.load(Ordering::SeqCst), 0);
    assert_eq!(f.source.discoveries.load(Ordering::SeqCst), 0);
    kernels.shutdown();
}

#[test]
fn unreadable_policy_and_failed_durable_recovery_do_not_start_any_runtime() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    let kernels = f.kernels();
    *f.policy.0.lock().unwrap() = "not valid = [".into();
    assert_eq!(
        f.restore(&kernels).unwrap(),
        RepositoryRestartOutcome::Ineligible
    );
    let result = kernels.restore_registered_with(&f.candidate, f.registry.as_ref(), |_, _| {
        bail!("synthetic unfinished authority fence")
    });
    assert!(result.is_err());
    assert_eq!(f.builds.load(Ordering::SeqCst), 0);
    assert!(
        production::acquire_repository_kernel_lock(&f.workspace).is_ok(),
        "failed startup releases its OS lock"
    );
    kernels.shutdown();
}

#[test]
fn tui_owned_lock_refuses_restore_before_recovery_or_runtime_assembly() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    let held = production::acquire_repository_kernel_lock(&f.workspace).unwrap();
    let kernels = f.kernels();
    let recovery_calls = AtomicUsize::new(0);
    let error = kernels
        .restore_registered_with(&f.candidate, f.registry.as_ref(), |_, admission| {
            recovery_calls.fetch_add(1, Ordering::SeqCst);
            f.recover(admission)
        })
        .unwrap_err();
    assert!(error.downcast_ref::<RepositoryKernelLockHeld>().is_some());
    assert_eq!(recovery_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.builds.load(Ordering::SeqCst), 0);
    drop(held);
    kernels.shutdown();
}

#[test]
fn replacing_checkout_between_discovery_and_lock_never_initializes_the_new_clone() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    let location =
        SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(&f.workspace)
            .unwrap()
            .unwrap();
    std::fs::rename(&f.workspace, f.root.join("original")).unwrap();
    std::fs::create_dir(&f.workspace).unwrap();
    initialize_git(Path::new(&f.workspace));
    let kernels = f.kernels();
    let recovery_calls = AtomicUsize::new(0);
    assert!(
        kernels
            .restore_registered_location(
                &f.candidate,
                f.candidate.clone(),
                location,
                f.registry.as_ref(),
                |_, admission| {
                    recovery_calls.fetch_add(1, Ordering::SeqCst);
                    f.recover(admission)
                },
            )
            .is_err()
    );
    assert_eq!(recovery_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.builds.load(Ordering::SeqCst), 0);
    assert!(
        !Path::new(&f.workspace)
            .join(".git/akra-repository-id")
            .exists()
    );
    assert!(
        !Path::new(&f.workspace)
            .join(".git/akra-repository-id.lock")
            .exists()
    );
    kernels.shutdown();
}

#[test]
fn strict_recovery_refuses_a_checkout_replaced_at_the_recovery_boundary_without_creating_databases()
{
    let f = Fixture::new();
    // A candidate alone creates no repository ledger or planning database.
    f.registry.register(&f.candidate).unwrap();
    let location =
        SqlitePlanningAuthorityAdapter::resolve_existing_repository_kernel_location(&f.workspace)
            .unwrap()
            .unwrap();
    let runtime = std::path::PathBuf::from(location.runtime_dir);
    let kernels = f.kernels();
    let recovery_calls = AtomicUsize::new(0);
    let result = kernels.restore_registered_with(
        &f.candidate,
        f.registry.as_ref(),
        |workspace, admission| {
            recovery_calls.fetch_add(1, Ordering::SeqCst);
            std::fs::rename(workspace, f.root.join("original")).unwrap();
            crate::private_directory::ensure_private_directory(Path::new(workspace)).unwrap();
            initialize_git(Path::new(workspace));
            production::recover_pending_issue_interventions_for_restart(
                workspace,
                &f.candidate.repository_key,
            )?;
            f.recover(admission)
        },
    );
    assert!(result.is_err());
    assert_eq!(recovery_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.builds.load(Ordering::SeqCst), 0);
    assert!(
        !Path::new(&f.workspace)
            .join(".git/akra-repository-id")
            .exists()
    );
    assert!(
        !Path::new(&f.workspace)
            .join(".git/akra-repository-id.lock")
            .exists()
    );
    assert!(!runtime.join("issue-automation.db").exists());
    assert!(!runtime.join("planning-authority.db").exists());
    kernels.shutdown();
}

#[test]
fn linked_worktree_candidates_share_one_primary_kernel() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    let linked = f.root.join("linked");
    git(
        Path::new(&f.workspace),
        &[
            "worktree",
            "add",
            "-qb",
            "restart-linked",
            &git_path(&linked),
        ],
    );
    let from_linked = production::issue_automation_restart_target(&linked.display().to_string())
        .unwrap()
        .unwrap();
    assert_eq!(from_linked, f.candidate);
    let kernels = f.kernels();
    let candidate = IssueAutomationRestartEntry {
        workspace_root: linked.display().to_string(),
        ..f.candidate.clone()
    };
    assert_eq!(
        kernels
            .restore_registered_with(&candidate, f.registry.as_ref(), |workspace, admission| {
                assert_eq!(workspace, f.workspace);
                f.recover(admission)
            })
            .unwrap(),
        RepositoryRestartOutcome::Started
    );
    assert_eq!(
        f.restore(&kernels).unwrap(),
        RepositoryRestartOutcome::AlreadyRunning
    );
    assert_eq!(kernels.entries().len(), 1);
    assert_eq!(f.builds.load(Ordering::SeqCst), 1);
    kernels.shutdown();
}

#[test]
fn pending_hold_recovers_before_runtime_and_collection_off_stays_off() {
    let f = Fixture::new();
    f.select(IssueAutonomyLevel::Gated);
    let first = f.kernels();
    f.restore(&first).unwrap();
    wait_until(|| {
        f.work()
            .is_some_and(|work| work.task_ids == ["restart-task"])
    });
    first.shutdown();
    let permits = f.execution.0.lock().unwrap().grants.clone();
    let collection = f.store.ensure_collection(&f.workspace).unwrap();
    f.store
        .set_collection_mode(
            &f.workspace,
            Some(collection.epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
    let request = IssueInterventionRequest {
        command_id: "restart-pending-hold".into(),
        issue: f.source.observation.issue.clone(),
        action: IssueInterventionAction::Hold,
    };
    f.store
        .begin_issue_intervention(&f.workspace, &request)
        .unwrap();
    let discoveries = f.source.discoveries.load(Ordering::SeqCst);
    let second = f.kernels();
    second
        .restore_registered_with(&f.candidate, f.registry.as_ref(), |_, admission| {
            let service = f.recover(admission)?;
            assert!(f.execution.0.lock().unwrap().control.held);
            assert!(
                f.store
                    .get_issue_intervention(&f.workspace, &request.command_id)?
                    .unwrap()
                    .receipt
                    .is_some()
            );
            Ok(service)
        })
        .unwrap();
    let host = second.entry(&f.candidate.repository_key).unwrap();
    assert!(
        !host
            .lock()
            .as_ref()
            .unwrap()
            .host
            .observe(Duration::from_secs(5))
            .unwrap()
            .parallel
            .enabled
    );
    second.shutdown();
    assert_eq!(
        f.store
            .get_collection(&f.workspace)
            .unwrap()
            .unwrap()
            .config
            .mode,
        IssueCollectionMode::Off
    );
    assert!(
        f.store
            .get_issue(&f.workspace, &request.issue)
            .unwrap()
            .unwrap()
            .flags()
            .held
    );
    assert_eq!(f.source.discoveries.load(Ordering::SeqCst), discoveries);
    assert_eq!(f.execution.0.lock().unwrap().grants, permits);
    f.no_model_turns();
}

#[test]
fn shutdown_wins_while_restore_waits_for_the_repository_entry() {
    let f = Arc::new(Fixture::new());
    f.select(IssueAutonomyLevel::Gated);
    let kernels = Arc::new(f.kernels());
    let release = super::tests::hold_repository_for_test(&kernels, &f.workspace);
    let restoring = kernels.clone();
    let fixture = f.clone();
    let restore = std::thread::spawn(move || fixture.restore(&restoring));
    let stopping = kernels.clone();
    let shutdown = std::thread::spawn(move || stopping.shutdown());
    wait_until(|| kernels.is_stopping());
    drop(release);
    shutdown.join().unwrap();
    assert!(restore.join().unwrap().is_err());
    assert_eq!(f.builds.load(Ordering::SeqCst), 0);
    assert!(production::acquire_repository_kernel_lock(&f.workspace).is_ok());
}

#[test]
fn concurrent_attach_reuses_the_clientless_kernel_instead_of_building_another() {
    let f = Arc::new(Fixture::new());
    f.select(IssueAutonomyLevel::Gated);
    let kernels = Arc::new(f.kernels());
    let (entered, entry) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let restoring = kernels.clone();
    let fixture = f.clone();
    let restore = std::thread::spawn(move || {
        restoring.restore_registered_with(
            &fixture.candidate,
            fixture.registry.as_ref(),
            |_, admission| {
                entered.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(10)).unwrap();
                fixture.recover(admission)
            },
        )
    });
    entry.recv_timeout(Duration::from_secs(10)).unwrap();
    let attaching = kernels.clone();
    let workspace = f.workspace.clone();
    let attach =
        std::thread::spawn(move || attaching.attach_with_workspace_root(&workspace, false));
    release.send(()).unwrap();
    assert_eq!(
        restore.join().unwrap().unwrap(),
        RepositoryRestartOutcome::Started
    );
    let (client, key, _) = attach.join().unwrap().unwrap();
    assert_eq!(client.client_id(), 1);
    assert_eq!(key, f.candidate.repository_key);
    assert_eq!(f.builds.load(Ordering::SeqCst), 1);
    kernels.shutdown();
    f.no_model_turns();
}

#[test]
fn missing_restart_checkout_is_pruned_from_the_registry() {
    let f = Fixture::new();
    f.registry.register(&f.candidate).unwrap();
    std::fs::remove_dir_all(&f.workspace).unwrap();

    let kernels = f.kernels();
    assert_eq!(
        kernels
            .restore_registered(&f.candidate, f.registry.as_ref())
            .unwrap(),
        RepositoryRestartOutcome::Unsupported
    );
    assert!(f.registry.list().unwrap().is_empty());
    kernels.shutdown();
}

#[test]
fn replaced_restart_checkout_is_pruned_without_transferring_authority() {
    let f = Fixture::new();
    f.registry.register(&f.candidate).unwrap();
    std::fs::remove_dir_all(&f.workspace).unwrap();
    std::fs::create_dir(&f.workspace).unwrap();
    initialize_git(Path::new(&f.workspace));

    let kernels = f.kernels();
    assert_eq!(
        kernels
            .restore_registered(&f.candidate, f.registry.as_ref())
            .unwrap(),
        RepositoryRestartOutcome::Unsupported
    );
    assert!(f.registry.list().unwrap().is_empty());
    assert!(
        !Path::new(&f.workspace)
            .join(".git/akra-repository-id")
            .exists()
    );
    kernels.shutdown();
}

#[test]
fn a_stopped_registry_refuses_restore_before_resolving_the_workspace() {
    let kernels = RepositoryKernelRegistry::default();
    kernels.shutdown();
    let candidate = IssueAutomationRestartEntry {
        repository_key: "never-read".into(),
        workspace_root: "/missing-restart-workspace".into(),
    };
    let error = kernels
        .restore_registered(&candidate, &MemoryRegistry::default())
        .unwrap_err();
    assert!(
        error.to_string().contains("gateway is stopping"),
        "{error:#}"
    );
}
