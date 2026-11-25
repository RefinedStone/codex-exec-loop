use super::*;
use crate::adapter::outbound::filesystem::issue_creation_journal::IssueCreationJournalAdapter;
use crate::application::port::outbound::issue_creation_journal_port::IssueCreationJournalPort;
use std::fs;
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-vault-workspace-{}-{}-한글 space",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let fixture = Self(root);
        fixture.git(&["init", "--quiet"]);
        fixture
    }

    fn git(&self, arguments: &[&str]) {
        let output = crate::test_utils::git_command()
            .current_dir(&self.0)
            .args([
                "-c",
                "core.hooksPath=",
                "-c",
                "user.name=Akra Test",
                "-c",
                "user.email=akra@example.invalid",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture Git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn files(&self) -> Vec<PathBuf> {
        fn walk(root: &Path, directory: &Path, paths: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                paths.push(entry.path().strip_prefix(root).unwrap().to_path_buf());
                if entry.file_type().unwrap().is_dir() {
                    walk(root, &entry.path(), paths);
                }
            }
        }
        let mut files = Vec::new();
        walk(&self.0, &self.0, &mut files);
        files.sort();
        files
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn vault_workspace_read_keeps_the_creation_scope_without_creating_authority_files() {
    let fixture = Fixture::new();
    let before = fixture.files();
    let plain = fixture.0.to_str().unwrap();
    let expected = fs::canonicalize(&fixture.0)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let journal = IssueCreationJournalAdapter::new(|_| {
        panic!("canonicalization must not open the creation runtime");
    });
    assert_eq!(journal.canonical_workspace(plain).unwrap(), expected);
    assert_eq!(canonical_workspace(plain).unwrap(), expected);
    assert_eq!(canonical_workspace(&expected).unwrap(), expected);
    assert_eq!(fixture.files(), before);
    assert!(!fixture.0.join(".git/akra-repository-id").exists());
    assert!(!fixture.0.join(".git/akra-repository-id.lock").exists());
    assert!(!fixture.0.join(".akra").exists());
    assert!(!fixture.0.join(".akra").exists());
    #[cfg(windows)]
    {
        assert!(!plain.starts_with(r"\\?\"));
        assert!(expected.starts_with(r"\\?\"));
        assert_ne!(plain, expected);
        assert!(expected.contains("한글 space"));
    }
}

#[test]
fn vault_workspace_rejects_linked_and_child_paths_without_creating_a_marker() {
    let fixture = Fixture::new();
    fixture.git(&["commit", "--quiet", "--allow-empty", "-m", "fixture"]);
    let linked = fixture.0.join("linked");
    fixture.git(&[
        "worktree",
        "add",
        "--quiet",
        "-b",
        "fixture-linked",
        linked.to_str().unwrap(),
    ]);
    let child = fixture.0.join("nested");
    fs::create_dir(&child).unwrap();
    let before = fixture.files();
    let primary = fs::canonicalize(&fixture.0).unwrap();
    assert_eq!(
        SqlitePlanningAuthorityAdapter::resolve_primary_checkout_root_for_read(
            linked.to_str().unwrap(),
        )
        .unwrap()
        .unwrap(),
        primary,
    );
    assert!(canonical_workspace(linked.to_str().unwrap()).is_err());
    assert!(canonical_workspace(child.to_str().unwrap()).is_err());
    assert_eq!(
        canonical_workspace(primary.to_str().unwrap()).unwrap(),
        primary.to_str().unwrap()
    );
    assert_eq!(fixture.files(), before);
    assert!(!fixture.0.join(".git/akra-repository-id").exists());
}

#[test]
fn vault_workspace_missing_path_remains_missing() {
    let fixture = Fixture::new();
    let missing = fixture.0.join("missing");
    assert!(canonical_workspace(missing.to_str().unwrap()).is_err());
    assert!(!missing.exists());
    assert!(!fixture.0.join(".git/akra-repository-id").exists());
}
