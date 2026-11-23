use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::domain::issue_automation::{
    IssueAutomationAutonomy, IssueAutomationFlag, IssueDecisionResumePhase, IssueTransition,
    IssueTriageResult,
};
use crate::domain::issue_work::IssueProvider;

/// Rebuilds the exact pre-v10 selection CHECK and removes later additions.
/// Migration regressions then downgrade the remaining tables to their target version.
pub(super) fn make_pre_admission_schema(connection: &Connection) {
    connection
        .execute_batch(
            "DROP TABLE issue_decision_reminders;
         DROP TABLE issue_start_budget_waits;
         DROP TABLE issue_work_admissions;
         ALTER TABLE issue_autonomy_selection RENAME TO issue_autonomy_selection_v10;",
        )
        .unwrap();
    connection
        .execute_batch(super::autonomy::LEGACY_SCHEMA)
        .unwrap();
    connection.execute_batch(
        "INSERT INTO issue_autonomy_selection (workspace_dir,requested_level,revision,pending_command_id)
         SELECT workspace_dir,requested_level,revision,pending_command_id FROM issue_autonomy_selection_v10;
         DROP TABLE issue_autonomy_selection_v10;"
    ).unwrap();
}

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) workspace: PathBuf,
    pub(super) runtime: PathBuf,
}

impl Fixture {
    pub(super) fn new(label: &str) -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-automation-{label}-{}-{}-{:08x}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            rand::random::<u32>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let workspace = root.join("workspace");
        let runtime = root.join("runtime");
        crate::private_directory::ensure_private_directory(&workspace).unwrap();
        crate::private_directory::ensure_private_directory(&runtime).unwrap();
        Self {
            root,
            workspace,
            runtime,
        }
    }

    pub(super) fn cwd(&self) -> &str {
        self.workspace.to_str().unwrap()
    }

    pub(super) fn adapter(&self) -> SqliteIssueAutomationStoreAdapter {
        let runtime = self.runtime.clone();
        SqliteIssueAutomationStoreAdapter::new(move |_| Ok(runtime.clone()))
    }

    pub(super) fn database(&self) -> PathBuf {
        self.runtime.join(private_store::DATABASE_NAME)
    }

    pub(super) fn binding(&self, id: &str) -> IssueAutomationBinding {
        binding_for(id, &reference(self.cwd(), "I_1"))
    }

    pub(super) fn insert(&self) -> (IssueAutomationBinding, TrackedIssue) {
        let adapter = self.adapter();
        let binding = self.binding("binding-1");
        adapter.insert_binding(self.cwd(), &binding).unwrap();
        let new = discovered(&binding, "I_1", "masked body");
        let issue = adapter.insert_issue(self.cwd(), &new).unwrap().issue;
        (binding, issue)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn reference(workspace_dir: &str, issue_id: &str) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_1".to_string(),
        issue_id: issue_id.to_string(),
        project_scope: IssueProjectScope {
            workspace_dir: workspace_dir.to_string(),
        },
    }
}

pub(super) fn binding_for(id: &str, reference: &IssueReference) -> IssueAutomationBinding {
    IssueAutomationBinding::new(
        IssueAutomationBindingId::new(id).unwrap(),
        reference.provider,
        &reference.server_id,
        &reference.repository_id,
        reference.project_scope.clone(),
    )
    .unwrap()
}

pub(super) fn snapshot(body: &str) -> IssueAutomationSnapshot {
    IssueAutomationSnapshot {
        basis: IssueComparisonBasis {
            // Only a fingerprint enters this ledger. It need not equal the disclosed copy's hash.
            field_hash: IssueFieldHash::of_fields(
                "synthetic provider title",
                "synthetic provider body",
            ),
            provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
        },
        shared_copy: IssueSharedCopy {
            title: "Shared title".to_string(),
            body: body.to_string(),
        },
        policy_version: "policy-v1".to_string(),
    }
}

pub(super) fn discovered(
    binding: &IssueAutomationBinding,
    issue_id: &str,
    body: &str,
) -> TrackedIssue {
    TrackedIssue::discovered(
        IssueReference {
            provider: binding.provider(),
            server_id: binding.server_id().to_string(),
            repository_id: binding.repository_id().to_string(),
            issue_id: issue_id.to_string(),
            project_scope: binding.project_scope().clone(),
        },
        binding,
        snapshot(body),
    )
    .unwrap()
}

pub(super) fn save_changed(
    adapter: &SqliteIssueAutomationStoreAdapter,
    cwd: &str,
    candidate: &TrackedIssue,
) -> TrackedIssue {
    match adapter
        .save_issue(cwd, candidate.revision(), candidate)
        .unwrap()
    {
        IssueAutomationIssueSaveResult::Saved {
            record,
            changed: true,
        } => *record,
        other => panic!("expected a changed issue save, got {other:?}"),
    }
}

pub(super) fn transition_and_save(
    adapter: &SqliteIssueAutomationStoreAdapter,
    cwd: &str,
    issue: &TrackedIssue,
    transition: IssueTransition,
) -> TrackedIssue {
    let mut candidate = issue.clone();
    candidate.transition(transition).unwrap();
    save_changed(adapter, cwd, &candidate)
}

#[test]
fn bindings_are_inserted_once_and_coordinates_cannot_be_rebound() {
    let fixture = Fixture::new("bindings");
    let adapter = fixture.adapter();
    let binding = fixture.binding("binding-1");
    let inserted = adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    assert!(inserted.inserted);
    assert_eq!(inserted.binding.revision, 1);
    assert_eq!(
        inserted.binding.binding.autonomy(),
        IssueAutomationAutonomy::Observe
    );

    let mut differently_configured = binding.clone();
    differently_configured.set_autonomy(IssueAutomationAutonomy::Auto);
    let duplicate = adapter
        .insert_binding(fixture.cwd(), &differently_configured)
        .unwrap();
    assert!(!duplicate.inserted);
    assert_eq!(duplicate.binding, inserted.binding);

    let mut changed_coordinates = binding.to_record();
    changed_coordinates.repository_id = "R_elsewhere".to_string();
    let changed_coordinates = IssueAutomationBinding::hydrate(changed_coordinates).unwrap();
    assert!(
        adapter
            .insert_binding(fixture.cwd(), &changed_coordinates)
            .is_err()
    );
    assert!(
        adapter
            .save_binding(fixture.cwd(), 1, &changed_coordinates)
            .is_err()
    );
    assert_eq!(
        adapter.list_bindings(fixture.cwd()).unwrap(),
        vec![inserted.binding]
    );
}

#[test]
fn binding_cas_supports_a_single_retry_and_rejects_aba() {
    let fixture = Fixture::new("binding-cas");
    let adapter = fixture.adapter();
    let binding = fixture.binding("binding-1");
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    let mut proposed = binding.clone();
    proposed.set_autonomy(IssueAutomationAutonomy::Propose);
    let saved = match adapter.save_binding(fixture.cwd(), 1, &proposed).unwrap() {
        IssueAutomationBindingSaveResult::Saved {
            record,
            changed: true,
        } => *record,
        other => panic!("unexpected binding save {other:?}"),
    };
    assert_eq!(saved.revision, 2);
    assert_eq!(
        adapter.save_binding(fixture.cwd(), 1, &proposed).unwrap(),
        IssueAutomationBindingSaveResult::Saved {
            record: Box::new(saved.clone()),
            changed: false
        },
    );
    assert_eq!(
        adapter.save_binding(fixture.cwd(), 2, &proposed).unwrap(),
        IssueAutomationBindingSaveResult::Saved {
            record: Box::new(saved),
            changed: false
        },
    );
    adapter.save_binding(fixture.cwd(), 2, &binding).unwrap();
    adapter.save_binding(fixture.cwd(), 3, &proposed).unwrap();
    assert_eq!(
        adapter.save_binding(fixture.cwd(), 1, &proposed).unwrap(),
        IssueAutomationBindingSaveResult::Conflict {
            actual_revision: Some(4)
        },
    );
}

#[test]
fn rediscovery_preserves_progress_flags_snapshot_and_the_existing_binding() {
    let fixture = Fixture::new("rediscovery");
    let adapter = fixture.adapter();
    let (binding, initial) = fixture.insert();
    let ready = transition_and_save(
        &adapter,
        fixture.cwd(),
        &initial,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    let queued = transition_and_save(&adapter, fixture.cwd(), &ready, IssueTransition::Queue);
    let running = transition_and_save(&adapter, fixture.cwd(), &queued, IssueTransition::Start);
    let mut marked = running.clone();
    for flag in [
        IssueAutomationFlag::Held,
        IssueAutomationFlag::Stale,
        IssueAutomationFlag::Unknown,
    ] {
        marked.set_flag(flag, true);
    }
    let marked = save_changed(&adapter, fixture.cwd(), &marked);
    let other_binding = fixture.binding("binding-2");
    adapter
        .insert_binding(fixture.cwd(), &other_binding)
        .unwrap();
    let fresh = discovered(
        &other_binding,
        "I_1",
        "new discovery must not replace progress",
    );
    let duplicate = adapter.insert_issue(fixture.cwd(), &fresh).unwrap();
    assert!(!duplicate.inserted);
    assert_eq!(duplicate.issue, marked);
    assert_eq!(duplicate.issue.binding_id(), binding.id());
    assert_eq!(
        adapter.list_issues(fixture.cwd(), None).unwrap(),
        vec![marked.clone()]
    );
    assert_eq!(
        adapter
            .list_issues(fixture.cwd(), Some(binding.id()))
            .unwrap(),
        vec![marked]
    );
    assert!(
        adapter
            .list_issues(fixture.cwd(), Some(other_binding.id()))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_unique_key_contains_every_reference_coordinate_and_uses_binary_ids() {
    let fixture = Fixture::new("full-key");
    let adapter = fixture.adapter();
    let baseline = reference(fixture.cwd(), "I_Key");
    let other_workspace = fixture.root.join("other-workspace");
    crate::private_directory::ensure_private_directory(&other_workspace).unwrap();
    let other_cwd = other_workspace.to_str().unwrap();
    let mut variants = vec![baseline.clone()];
    let mut provider = baseline.clone();
    provider.provider = IssueProvider::Jira;
    variants.push(provider);
    let mut server = baseline.clone();
    server.server_id = "another.example".to_string();
    variants.push(server);
    let mut repository = baseline.clone();
    repository.repository_id = "R_2".to_string();
    variants.push(repository);
    let mut issue = baseline.clone();
    issue.issue_id = "I_other".to_string();
    variants.push(issue);
    let mut case = baseline.clone();
    case.issue_id = "I_key".to_string();
    variants.push(case);
    let mut project = baseline;
    project.project_scope.workspace_dir = other_cwd.to_string();
    variants.push(project);
    for (index, reference) in variants.iter().enumerate() {
        let binding = binding_for(&format!("binding-{index}"), reference);
        let cwd = &reference.project_scope.workspace_dir;
        adapter.insert_binding(cwd, &binding).unwrap();
        let issue =
            TrackedIssue::discovered(reference.clone(), &binding, snapshot("shared")).unwrap();
        let inserted = adapter.insert_issue(cwd, &issue).unwrap();
        assert!(inserted.inserted);
        assert_eq!(
            adapter.get_issue(cwd, reference).unwrap(),
            Some(inserted.issue)
        );
    }
    let database = Connection::open(fixture.database()).unwrap();
    let count: i64 = database
        .query_row("SELECT count(*) FROM tracked_issues", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count as usize, variants.len());
    assert_eq!(adapter.list_issues(other_cwd, None).unwrap().len(), 1);
    assert_eq!(
        adapter.list_issues(fixture.cwd(), None).unwrap().len(),
        variants.len() - 1
    );
}

#[test]
fn scope_mismatch_and_missing_or_mismatched_binding_fail_before_insertion() {
    let fixture = Fixture::new("scope");
    let adapter = fixture.adapter();
    let binding = fixture.binding("binding-1");
    let issue = discovered(&binding, "I_1", "shared");
    assert!(adapter.insert_issue(fixture.cwd(), &issue).is_err());
    let other = fixture.root.join("different-scope");
    crate::private_directory::ensure_private_directory(&other).unwrap();
    let other_cwd = other.to_str().unwrap();
    assert!(adapter.insert_binding(other_cwd, &binding).is_err());
    assert!(adapter.insert_issue(other_cwd, &issue).is_err());
    assert!(adapter.get_issue(other_cwd, issue.reference()).is_err());
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();

    let mut mismatched = binding.to_record();
    mismatched.repository_id = "R_other".to_string();
    let mismatched = IssueAutomationBinding::hydrate(mismatched).unwrap();
    let issue = discovered(&mismatched, "I_1", "shared");
    assert!(adapter.insert_issue(fixture.cwd(), &issue).is_err());
    assert!(adapter.list_issues(fixture.cwd(), None).unwrap().is_empty());
}

#[test]
fn issue_cas_is_idempotent_only_at_the_immediate_next_revision() {
    let fixture = Fixture::new("issue-cas");
    let adapter = fixture.adapter();
    let (_, initial) = fixture.insert();
    let mut candidate = initial.clone();
    candidate
        .transition(IssueTransition::Assess(IssueTriageResult::Ready))
        .unwrap();
    let saved = save_changed(&adapter, fixture.cwd(), &candidate);
    assert_eq!(saved.revision(), 2);
    assert_eq!(
        adapter.save_issue(fixture.cwd(), 1, &candidate).unwrap(),
        IssueAutomationIssueSaveResult::Saved {
            record: Box::new(saved.clone()),
            changed: false
        },
    );
    assert_eq!(
        adapter.save_issue(fixture.cwd(), 2, &saved).unwrap(),
        IssueAutomationIssueSaveResult::Saved {
            record: Box::new(saved.clone()),
            changed: false
        },
    );
    assert!(adapter.save_issue(fixture.cwd(), 2, &candidate).is_err());
    let mut held = saved;
    held.set_flag(IssueAutomationFlag::Held, true);
    let mut held = save_changed(&adapter, fixture.cwd(), &held);
    held.set_flag(IssueAutomationFlag::Held, false);
    let returned = save_changed(&adapter, fixture.cwd(), &held);
    assert!(returned.same_content(&candidate));
    assert_eq!(returned.revision(), 4);
    assert_eq!(
        adapter.save_issue(fixture.cwd(), 1, &candidate).unwrap(),
        IssueAutomationIssueSaveResult::Conflict {
            actual_revision: Some(4)
        },
    );
}

#[test]
fn hydrated_candidates_cannot_skip_lifecycle_transitions_or_reset_running_work() {
    let fixture = Fixture::new("transition");
    let adapter = fixture.adapter();
    let (binding, initial) = fixture.insert();
    let mut forged = initial.to_record();
    forged.state = IssueAutomationState::Done;
    let forged = TrackedIssue::hydrate(forged).unwrap();
    assert!(
        adapter
            .save_issue(fixture.cwd(), initial.revision(), &forged)
            .is_err()
    );
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), initial.reference())
            .unwrap(),
        Some(initial.clone())
    );

    let ready = transition_and_save(
        &adapter,
        fixture.cwd(),
        &initial,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    let queued = transition_and_save(&adapter, fixture.cwd(), &ready, IssueTransition::Queue);
    let running = transition_and_save(&adapter, fixture.cwd(), &queued, IssueTransition::Start);
    let mut reset = running.to_record();
    reset.state = IssueAutomationState::Discovered;
    let reset = TrackedIssue::hydrate(reset).unwrap();
    assert!(
        adapter
            .save_issue(fixture.cwd(), running.revision(), &reset)
            .is_err()
    );
    assert!(adapter.insert_issue(fixture.cwd(), &running).is_err());

    let mut skipped_new = discovered(&binding, "I_new", "shared");
    skipped_new
        .transition(IssueTransition::Assess(IssueTriageResult::Ready))
        .unwrap();
    assert!(adapter.insert_issue(fixture.cwd(), &skipped_new).is_err());
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), running.reference())
            .unwrap(),
        Some(running)
    );
}

#[test]
fn two_adapters_cannot_both_save_different_candidates_at_one_revision() {
    let fixture = Fixture::new("cas-race");
    let (_, initial) = fixture.insert();
    let mut first = initial.clone();
    let mut second = initial;
    first
        .transition(IssueTransition::Assess(IssueTriageResult::Ready))
        .unwrap();
    second
        .transition(IssueTransition::Assess(IssueTriageResult::NeedsInfo))
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [first, second]
        .into_iter()
        .map(|candidate| {
            let adapter = fixture.adapter();
            let cwd = fixture.cwd().to_string();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter
                    .save_issue(&cwd, candidate.revision(), &candidate)
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                IssueAutomationIssueSaveResult::Saved { changed: true, .. }
            ))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                IssueAutomationIssueSaveResult::Conflict {
                    actual_revision: Some(2)
                }
            ))
            .count(),
        1
    );
}

#[test]
fn concurrent_insert_returns_the_winning_row_without_overwriting_it() {
    let fixture = Fixture::new("insert-race");
    let binding = fixture.binding("binding-1");
    fixture
        .adapter()
        .insert_binding(fixture.cwd(), &binding)
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["first shared copy", "second shared copy"]
        .into_iter()
        .map(|body| {
            let adapter = fixture.adapter();
            let cwd = fixture.cwd().to_string();
            let candidate = discovered(&binding, "I_1", body);
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter.insert_issue(&cwd, &candidate).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.inserted).count(), 1);
    assert_eq!(results[0].issue, results[1].issue);
    assert_eq!(results[0].issue.revision(), 1);
}

#[test]
fn reopening_preserves_decision_resume_flags_and_the_provider_basis() {
    let fixture = Fixture::new("reopen");
    let adapter = fixture.adapter();
    let (binding, initial) = fixture.insert();
    let ready = transition_and_save(
        &adapter,
        fixture.cwd(),
        &initial,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    let mut pending = ready;
    pending.transition(IssueTransition::AwaitDecision).unwrap();
    pending.set_flag(IssueAutomationFlag::Held, true);
    pending.set_flag(IssueAutomationFlag::Stale, true);
    pending.set_flag(IssueAutomationFlag::Unknown, true);
    let saved = save_changed(&adapter, fixture.cwd(), &pending);
    let mut binding = binding;
    binding.set_autonomy(IssueAutomationAutonomy::Gated);
    adapter.save_binding(fixture.cwd(), 1, &binding).unwrap();
    drop(adapter);

    let reopened = fixture.adapter();
    let restored = reopened
        .get_issue(fixture.cwd(), saved.reference())
        .unwrap()
        .unwrap();
    assert_eq!(restored, saved);
    assert_eq!(
        restored.decision_resume(),
        Some(IssueDecisionResumePhase::Ready)
    );
    assert_ne!(
        restored.snapshot().basis.field_hash,
        IssueFieldHash::of_fields(
            &restored.snapshot().shared_copy.title,
            &restored.snapshot().shared_copy.body,
        ),
    );
    assert_eq!(
        reopened
            .get_binding(fixture.cwd(), binding.id())
            .unwrap()
            .unwrap()
            .binding
            .autonomy(),
        IssueAutomationAutonomy::Gated,
    );
}

#[test]
fn only_the_shared_copy_and_the_provider_fingerprint_are_persisted() {
    let fixture = Fixture::new("shared-only");
    let adapter = fixture.adapter();
    let binding = fixture.binding("binding-1");
    adapter.insert_binding(fixture.cwd(), &binding).unwrap();
    // Synthetic source values exist only in this test's memory; no raw fixture file is created.
    let source_title = format!("synthetic-local-title-{:032x}", rand::random::<u128>());
    let source_body = format!("synthetic-local-body-{:032x}", rand::random::<u128>());
    let shared_copy = IssueSharedCopy {
        title: "Shared issue".to_string(),
        body: "Removed source details.".to_string(),
    };
    let issue = TrackedIssue::discovered(
        reference(fixture.cwd(), "I_1"),
        &binding,
        IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields(&source_title, &source_body),
                provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
            },
            shared_copy: shared_copy.clone(),
            policy_version: "p1".to_string(),
        },
    )
    .unwrap();
    adapter.insert_issue(fixture.cwd(), &issue).unwrap();
    let loaded = adapter
        .get_issue(fixture.cwd(), issue.reference())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.snapshot().shared_copy, shared_copy);
    for entry in fs::read_dir(&fixture.runtime).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            let bytes = fs::read(entry.path()).unwrap();
            for plaintext in [&source_title, &source_body] {
                assert!(
                    !bytes
                        .windows(plaintext.len())
                        .any(|part| part == plaintext.as_bytes())
                );
            }
        }
    }
}

#[test]
fn unsupported_or_malformed_schema_is_not_repaired_or_silently_replaced() {
    for (label, version) in [
        ("future-schema", SCHEMA_VERSION + 1),
        ("malformed-schema", SCHEMA_VERSION),
    ] {
        let fixture = Fixture::new(label);
        let database = Connection::open(fixture.database()).unwrap();
        database
            .execute_batch(
                "CREATE TABLE keep_me (value TEXT); INSERT INTO keep_me VALUES ('unchanged');",
            )
            .unwrap();
        database
            .pragma_update(None, "application_id", APPLICATION_ID)
            .unwrap();
        database
            .pragma_update(None, "user_version", version)
            .unwrap();
        drop(database);
        assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());
        let database = Connection::open(fixture.database()).unwrap();
        let retained: String = database
            .query_row("SELECT value FROM keep_me", [], |row| row.get(0))
            .unwrap();
        let current: i64 = database
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(retained, "unchanged");
        assert_eq!(current, version);
    }
}

#[test]
fn corrupt_stored_values_fail_hydration_instead_of_becoming_missing_issues() {
    for (label, corruption) in [
        ("invalid-revision", "UPDATE tracked_issues SET revision = 0"),
        (
            "invalid-resume",
            "UPDATE tracked_issues SET state = 'awaiting_decision', decision_resume = NULL",
        ),
        ("invalid-flag", "UPDATE tracked_issues SET held = 2"),
        (
            "invalid-hash",
            "UPDATE tracked_issues SET source_hash_sha256 = 'broken'",
        ),
    ] {
        let fixture = Fixture::new(label);
        let (_, issue) = fixture.insert();
        let database = Connection::open(fixture.database()).unwrap();
        database
            .execute_batch("PRAGMA ignore_check_constraints = ON;")
            .unwrap();
        database.execute_batch(corruption).unwrap();
        drop(database);
        assert!(
            fixture
                .adapter()
                .get_issue(fixture.cwd(), issue.reference())
                .is_err()
        );
        assert!(fixture.adapter().list_issues(fixture.cwd(), None).is_err());
    }
}

#[test]
fn new_ledger_does_not_open_or_modify_the_sibling_planning_database() {
    let fixture = Fixture::new("separate-store");
    let planning = fixture.runtime.join("planning-authority.db");
    let database = Connection::open(&planning).unwrap();
    database
        .execute_batch(
            "CREATE TABLE authority_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO authority_metadata VALUES ('schema_version', '19');
         INSERT INTO authority_metadata VALUES ('fixture_marker', 'unchanged');",
        )
        .unwrap();
    drop(database);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&planning, fs::Permissions::from_mode(0o640)).unwrap();
    }
    let bytes_before = fs::read(&planning).unwrap();
    let (_, issue) = fixture.insert();
    fixture
        .adapter()
        .get_issue(fixture.cwd(), issue.reference())
        .unwrap();
    assert_eq!(fs::read(&planning).unwrap(), bytes_before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&planning).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    let database = Connection::open(planning).unwrap();
    let version: String = database
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, "19");
}

#[test]
fn database_and_journal_hardlinks_are_rejected_without_touching_the_link_target() {
    for suffix in ["", "-journal"] {
        let fixture = Fixture::new(if suffix.is_empty() {
            "hardlink-main"
        } else {
            "hardlink-journal"
        });
        let target = fixture.root.join("outside.db");
        fs::write(&target, b"untouched target").unwrap();
        let mut database = fixture.database().into_os_string();
        database.push(suffix);
        fs::hard_link(&target, PathBuf::from(database)).unwrap();
        assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());
        assert_eq!(fs::read(target).unwrap(), b"untouched target");
    }
}

#[cfg(unix)]
#[test]
fn unix_private_runtime_and_special_file_boundaries_are_enforced() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let fixture = Fixture::new("nonprivate-runtime");
    fs::set_permissions(&fixture.runtime, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());
    assert!(!fixture.database().exists());

    for suffix in ["", "-journal", "-wal", "-shm"] {
        let fixture = Fixture::new("symlink-file");
        let target = fixture.root.join("outside.db");
        fs::write(&target, b"untouched target").unwrap();
        let mut database = fixture.database().into_os_string();
        database.push(suffix);
        symlink(&target, PathBuf::from(database)).unwrap();
        assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());
        assert_eq!(fs::read(target).unwrap(), b"untouched target");
    }

    let fixture = Fixture::new("fifo-file");
    let path = std::ffi::CString::new(fixture.database().as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: the path is a live NUL-terminated synthetic fixture path.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());

    let fixture = Fixture::new("private-mode");
    fixture.insert();
    assert_eq!(
        fs::metadata(fixture.database())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(windows)]
#[test]
fn windows_database_has_a_private_owner_acl() {
    use crate::private_fs::{
        WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
        WINDOWS_READ_CONTROL, validate_windows_path_identity,
        validate_windows_private_owner_and_acl,
    };
    use std::os::windows::fs::OpenOptionsExt;

    let fixture = Fixture::new("windows-private-acl");
    fixture.insert();
    let path = fixture.database();
    let file = fs::OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .unwrap();
    validate_windows_path_identity(&path, &file, false).unwrap();
    validate_windows_private_owner_and_acl(&path, &file).unwrap();
}

#[test]
fn wal_mode_is_rejected_without_migrating_the_existing_database() {
    let fixture = Fixture::new("wal");
    let database = Connection::open(fixture.database()).unwrap();
    database.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE retained (value TEXT); INSERT INTO retained VALUES ('kept');").unwrap();
    assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());
    let mode: String = database
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let retained: String = database
        .query_row("SELECT value FROM retained", [], |row| row.get(0))
        .unwrap();
    assert_eq!(retained, "kept");
}

#[test]
fn saves_require_a_persisted_sqlite_revision_and_cannot_acknowledge_insert_as_a_retry() {
    let fixture = Fixture::new("save-revision-boundary");
    let adapter = fixture.adapter();
    let (binding, stored) = fixture.insert();
    let unsaved = discovered(&binding, "I_1", "masked body");
    assert!(stored.same_content(&unsaved));
    for expected in [0, i64::MAX as u64 + 1, u64::MAX] {
        assert!(
            adapter
                .save_binding(fixture.cwd(), expected, &binding)
                .is_err()
        );
        let candidate = if expected == 0 { &unsaved } else { &stored };
        assert!(
            adapter
                .save_issue(fixture.cwd(), expected, candidate)
                .is_err()
        );
    }
    assert_eq!(
        adapter
            .get_issue(fixture.cwd(), stored.reference())
            .unwrap(),
        Some(stored)
    );
    assert_eq!(
        adapter
            .get_binding(fixture.cwd(), binding.id())
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}
