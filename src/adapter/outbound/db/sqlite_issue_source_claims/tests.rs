use super::*;
use crate::domain::issue_work::IssueProjectScope;
use std::sync::{Arc, Barrier};

struct Fixture {
    root: PathBuf,
    first: IssueReference,
    second: IssueReference,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-source-claims-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        for name in ["first", "second"] {
            std::fs::create_dir(root.join(name)).unwrap();
        }
        let issue = |name: &str| IssueReference {
            provider: IssueProvider::Jira,
            server_id: "cloud-fixture".into(),
            repository_id: "10000".into(),
            issue_id: "12345".into(),
            project_scope: IssueProjectScope {
                workspace_dir: std::fs::canonicalize(root.join(name))
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned(),
            },
        };
        Self {
            first: issue("first"),
            second: issue("second"),
            root,
        }
    }
    fn adapter(&self) -> SqliteIssueSourceClaimsAdapter {
        let runtime = self.root.join("runtime");
        SqliteIssueSourceClaimsAdapter::new(move || Ok(runtime.clone()))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn binding(value: &str) -> IssueAutomationBindingId {
    IssueAutomationBindingId::new(value).unwrap()
}

#[test]
fn second_overlapping_binding_cannot_claim_after_restart_or_repeated_discovery() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .adapter()
            .claim(&fixture.first, &binding("first"))
            .unwrap()
    );
    assert!(
        !fixture
            .adapter()
            .claim(&fixture.second, &binding("second"))
            .unwrap()
    );
    assert!(
        fixture
            .adapter()
            .owns(&fixture.first, &binding("first"))
            .unwrap()
    );
    assert!(
        fixture
            .adapter()
            .claim(&fixture.first, &binding("first"))
            .unwrap()
    );
    assert!(
        !fixture
            .adapter()
            .claim(&fixture.first, &binding("changed-binding"))
            .unwrap()
    );
    assert!(
        !fixture
            .adapter()
            .owns(&fixture.second, &binding("second"))
            .unwrap()
    );
    let mut moved = fixture.second.clone();
    moved.repository_id = "20000".into();
    assert!(!fixture.adapter().claim(&moved, &binding("second")).unwrap());
}

#[test]
fn concurrent_overlapping_bindings_have_one_durable_winner() {
    let fixture = Fixture::new();
    // Initialize an unrelated claim so this test isolates competing claim transactions.
    let mut other = fixture.first.clone();
    other.issue_id = "54321".into();
    fixture
        .adapter()
        .claim(&other, &binding("initial"))
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [(&fixture.first, "first"), (&fixture.second, "second")]
        .into_iter()
        .map(|(issue, owner)| {
            let adapter = fixture.adapter();
            let issue = issue.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter.claim(&issue, &binding(owner)).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|owned| **owned).count(), 1);
    assert_eq!(
        fixture
            .adapter()
            .owns(&fixture.first, &binding("first"))
            .unwrap(),
        results[0]
    );
    assert_eq!(
        fixture
            .adapter()
            .owns(&fixture.second, &binding("second"))
            .unwrap(),
        results[1]
    );
}

#[test]
fn absent_read_is_side_effect_free_and_invalid_identity_does_not_create_a_claim() {
    let fixture = Fixture::new();
    assert!(
        !fixture
            .adapter()
            .owns(&fixture.first, &binding("first"))
            .unwrap()
    );
    assert!(!fixture.root.join("runtime").exists());
    let mut invalid = fixture.first.clone();
    invalid.issue_id = "../12345".into();
    assert!(
        fixture
            .adapter()
            .claim(&invalid, &binding("first"))
            .is_err()
    );
    assert!(!fixture.root.join("runtime").exists());
}

#[test]
fn a_different_database_schema_is_not_repaired_or_reassigned() {
    let fixture = Fixture::new();
    fixture
        .adapter()
        .claim(&fixture.first, &binding("first"))
        .unwrap();
    let mut opened = crate::private_sqlite::open_existing(
        &fixture.root.join("runtime"),
        FILES,
        Synchronous::Extra,
    )
    .unwrap()
    .unwrap();
    opened
        .parts()
        .0
        .pragma_update(None, "user_version", 99)
        .unwrap();
    drop(opened);
    assert!(
        fixture
            .adapter()
            .claim(&fixture.second, &binding("second"))
            .is_err()
    );
    assert!(
        fixture
            .adapter()
            .owns(&fixture.first, &binding("first"))
            .is_err()
    );
}
