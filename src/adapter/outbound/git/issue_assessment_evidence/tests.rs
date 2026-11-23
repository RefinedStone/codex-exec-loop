use super::*;
use std::fs;
use std::io::Write;
use std::process::Stdio;

struct Fixture {
    root: PathBuf,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-assessment-evidence-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        command(&root, &["init", "-q", "-b", "main"]);
        command(&root, &["config", "user.name", "Assessment Fixture"]);
        command(&root, &["config", "user.email", "fixture@example.invalid"]);
        command(&root, &["config", "core.autocrlf", "false"]);
        fs::write(root.join("README.md"), "A bounded public description.\n").unwrap();
        command(&root, &["add", "."]);
        command(&root, &["commit", "-q", "-m", "fixture base"]);
        let base = command(&root, &["rev-parse", "HEAD"]);
        Self { root, base }
    }

    fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn commit(&self) -> String {
        command(&self.root, &["add", "-A"]);
        self.commit_index()
    }

    fn commit_index(&self) -> String {
        command(&self.root, &["commit", "-q", "-m", "fixture evidence"]);
        command(&self.root, &["rev-parse", "HEAD"])
    }

    fn blob(&self, bytes: &[u8]) -> String {
        let mut child = crate::test_utils::git_command()
            .arg("-C")
            .arg(&self.root)
            .args(["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn index_entry(&self, mode: &str, oid: &str, path: &str) {
        command(
            &self.root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{mode},{oid},{path}"),
            ],
        );
    }

    fn read(
        &self,
        commit: &str,
        paths: &[&str],
    ) -> Result<IssueAssessmentRepositoryEvidence, Error> {
        self.read_with_policy(commit, paths, &IssueAutomationPolicy::builtin())
    }

    fn read_with_policy(
        &self,
        commit: &str,
        paths: &[&str],
        policy: &IssueAutomationPolicy,
    ) -> Result<IssueAssessmentRepositoryEvidence, Error> {
        let selected_paths = paths
            .iter()
            .map(|path| (*path).to_owned())
            .collect::<Vec<_>>();
        GitIssueAssessmentEvidenceAdapter::new().read(IssueAssessmentEvidenceRequest {
            workspace_directory: self.root.to_str().unwrap(),
            commit_sha: commit,
            selected_paths: &selected_paths,
            policy,
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn command(root: &Path, args: &[&str]) -> String {
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git fixture {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn exact_commit_evidence_is_sorted_and_proves_absence_without_reading_dirty_files() {
    let fixture = Fixture::new();
    fixture.write("src/lib.rs", "pub fn value() -> u8 { 1 }\n");
    let source = fixture.commit();
    fixture.write(
        "src/lib.rs",
        "uncommitted content must never enter model input\n",
    );
    fixture.write(
        "docs/new.md",
        "untracked content must never enter model input\n",
    );
    let first = fixture
        .read(&source, &["src/lib.rs", "docs/new.md", "README.md"])
        .unwrap();
    let reordered = fixture
        .read(&source, &["README.md", "src/lib.rs", "docs/new.md"])
        .unwrap();
    assert_eq!(first, reordered);
    assert_eq!(first.commit_sha, source);
    assert_eq!(first.selection_version, ISSUE_ASSESSMENT_EVIDENCE_VERSION);
    assert_eq!(first.snapshot_sha256.len(), 64);
    assert_eq!(first.entries.len(), 3);
    assert_eq!(
        first.entries[1],
        IssueAssessmentEvidenceEntry::Absent {
            path: "docs/new.md".into()
        }
    );
    assert!(
        matches!(&first.entries[2], IssueAssessmentEvidenceEntry::File { path, text, blob_sha }
        if path == "src/lib.rs" && text == "pub fn value() -> u8 { 1 }\n" && blob_sha.len() == source.len())
    );
    let serialized = serde_json::to_string(&first).unwrap();
    assert!(!serialized.contains(fixture.root.to_str().unwrap()));
    assert!(!serialized.contains("uncommitted"));
    assert!(!serialized.contains("untracked"));
}

#[test]
fn a_new_head_invalidates_previous_evidence_and_changes_its_snapshot_identity() {
    let fixture = Fixture::new();
    let first = fixture.read(&fixture.base, &["README.md"]).unwrap();
    fixture.write("README.md", "Updated public description.\n");
    let source = fixture.commit();
    assert_eq!(
        fixture.read(&fixture.base, &["README.md"]),
        Err(Error::SourceChanged)
    );
    assert_eq!(
        GitIssueAssessmentEvidenceAdapter::new()
            .current_commit_sha(fixture.root.to_str().unwrap())
            .unwrap(),
        source
    );
    let updated = fixture.read(&source, &["README.md"]).unwrap();
    assert_ne!(first.snapshot_sha256, updated.snapshot_sha256);
}

#[test]
fn built_in_and_repository_protection_reject_the_whole_selection() {
    let fixture = Fixture::new();
    let mut policy = IssueAutomationPolicy::builtin();
    policy.protection.paths.push("docs/internal/**".into());
    for path in [
        ".github/workflows/check.yml",
        "scripts/release/build.sh",
        ".akra/issue-automation.toml",
        "AGENTS.md",
        "src/AGENTS.md",
        "src/.env",
        "src/.env.production",
        "src/credentials.json",
        "docs/private.pem",
        "docs/private.key",
        "src/agents.md",
        "docs/internal/plan.md",
    ] {
        assert_eq!(
            fixture.read_with_policy(&fixture.base, &["README.md", path], &policy),
            Err(Error::ProtectedPath),
            "{path}"
        );
    }
}

#[test]
fn desktop_source_test_and_script_paths_are_in_the_static_selection() {
    let paths = [
        "desktop/src/issueAutomation/view.ts",
        "desktop/test/issueAutomation.test.ts",
        "desktop/scripts/check-types.ts",
    ]
    .map(str::to_owned);
    assert_eq!(
        permitted_paths(&paths, &IssueAutomationPolicy::builtin()).unwrap(),
        [
            "desktop/scripts/check-types.ts",
            "desktop/src/issueAutomation/view.ts",
            "desktop/test/issueAutomation.test.ts"
        ]
    );
}

#[test]
fn desktop_static_roots_do_not_override_builtin_or_repository_protection() {
    let mut policy = IssueAutomationPolicy::builtin();
    policy
        .protection
        .paths
        .push("desktop/src/internal/**".into());
    for path in [
        "desktop/src/AGENTS.md",
        "desktop/src/agents.md",
        "desktop/test/.env",
        "desktop/scripts/credentials.json",
        "desktop/src/private.key",
        "desktop/src/internal/model.ts",
    ] {
        assert_eq!(
            permitted_paths(&["README.md".into(), path.into()], &policy),
            Err(Error::ProtectedPath),
            "{path}"
        );
    }
}

#[test]
fn desktop_selection_does_not_expand_to_other_roots_or_similar_prefixes() {
    let policy = IssueAutomationPolicy::builtin();
    for path in [
        "desktop/src",
        "desktop/src-extra/file.ts",
        "desktop/tests/file.ts",
        "desktop/assets/icon.svg",
        "desktop/package.json",
    ] {
        assert_eq!(
            permitted_paths(&[path.into()], &policy),
            Err(Error::NoEligiblePaths),
            "{path}"
        );
    }
}

#[test]
fn ambiguous_paths_and_paths_outside_the_static_selection_are_rejected() {
    let policy = IssueAutomationPolicy::builtin();
    for path in [
        "",
        "/etc/passwd",
        "../src/lib.rs",
        "src/../lib.rs",
        "src//lib.rs",
        "src/./lib.rs",
        "src\\lib.rs",
        "src/*",
        "src/:value",
        "docs/a b.md",
        "docs/control\n.md",
    ] {
        assert_eq!(
            permitted_paths(&[path.to_owned()], &policy),
            Err(Error::UnsupportedEntry),
            "{path:?}"
        );
    }
    for path in [
        "other/file.rs",
        "docs/.private/file.md",
        "src/.hidden",
        "assets/model.txt",
    ] {
        assert_eq!(
            permitted_paths(&[path.to_owned()], &policy),
            Err(Error::NoEligiblePaths),
            "{path}"
        );
    }
    assert_eq!(permitted_paths(&[], &policy), Err(Error::NoEligiblePaths));
    assert_eq!(
        permitted_paths(&["README.md".into(), "README.md".into()], &policy),
        Err(Error::Incomplete)
    );
    assert_eq!(
        permitted_paths(
            &(0..17)
                .map(|index| format!("src/file{index}.rs"))
                .collect::<Vec<_>>(),
            &policy
        ),
        Err(Error::TooLarge)
    );
    assert_eq!(
        permitted_paths(&[format!("src/{}file.rs", "nested/".repeat(32))], &policy),
        Err(Error::TooLarge)
    );
}

#[test]
fn symlinks_and_gitlinks_are_not_files_and_never_imply_a_missing_child() {
    let fixture = Fixture::new();
    let link = fixture.blob(b"../../outside");
    fixture.index_entry("120000", &link, "docs/link");
    fixture.index_entry("160000", &fixture.base, "src/submodule");
    let source = fixture.commit_index();
    for path in [
        "docs/link",
        "docs/link/secret.txt",
        "src/submodule",
        "src/submodule/lib.rs",
    ] {
        assert_eq!(
            fixture.read(&source, &["README.md", path]),
            Err(Error::UnsupportedEntry),
            "{path}"
        );
    }
}

#[test]
fn a_regular_file_ancestor_does_not_count_as_proven_absence() {
    let fixture = Fixture::new();
    fixture.write("docs/not-a-directory", "a regular file\n");
    let source = fixture.commit();
    assert_eq!(
        fixture.read(&source, &["docs/not-a-directory/child.md"]),
        Err(Error::UnsupportedEntry)
    );
}

#[test]
fn binary_invalid_utf8_and_secret_spans_fail_the_entire_snapshot() {
    let fixture = Fixture::new();
    fixture.write("docs/binary.txt", b"text\0binary");
    fixture.write("docs/non-utf8.txt", [0xff, 0xfe]);
    fixture.write("docs/secret.txt", "token=fixture-not-a-real-secret\n");
    fixture.write("docs/control.txt", b"text\x1bterminal-control");
    let source = fixture.commit();
    for path in [
        "docs/binary.txt",
        "docs/non-utf8.txt",
        "docs/secret.txt",
        "docs/control.txt",
    ] {
        assert_eq!(
            fixture.read(&source, &["README.md", path]),
            Err(Error::InvalidText),
            "{path}"
        );
    }
}

#[test]
fn per_file_and_total_bounds_never_return_a_partial_success() {
    let fixture = Fixture::new();
    for index in 0..5 {
        fixture.write(
            &format!("docs/bound-{index}.txt"),
            vec![b'x'; MAX_ISSUE_ASSESSMENT_EVIDENCE_FILE_BYTES],
        );
    }
    fixture.write(
        "docs/oversized.txt",
        vec![b'x'; MAX_ISSUE_ASSESSMENT_EVIDENCE_FILE_BYTES + 1],
    );
    let source = fixture.commit();
    let paths = (0..5)
        .map(|index| format!("docs/bound-{index}.txt"))
        .collect::<Vec<_>>();
    let paths = paths.iter().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(fixture.read(&source, &paths[..4]).unwrap().entries.len(), 4);
    assert_eq!(fixture.read(&source, &paths), Err(Error::TooLarge));
    assert_eq!(
        fixture.read(&source, &["README.md", "docs/oversized.txt"]),
        Err(Error::TooLarge)
    );
}

#[test]
fn empty_and_executable_regular_files_remain_valid_text_evidence() {
    let fixture = Fixture::new();
    let empty = fixture.blob(b"");
    let script = fixture.blob(b"#!/bin/sh\nprintf 'public fixture\\n'\n");
    fixture.index_entry("100644", &empty, "docs/empty.md");
    fixture.index_entry("100755", &script, "src/tool.sh");
    let source = fixture.commit_index();
    let evidence = fixture
        .read(&source, &["docs/empty.md", "src/tool.sh"])
        .unwrap();
    assert!(
        matches!(&evidence.entries[0], IssueAssessmentEvidenceEntry::File { text, .. } if text.is_empty())
    );
    assert!(
        matches!(&evidence.entries[1], IssueAssessmentEvidenceEntry::File { text, .. } if text.starts_with("#!/bin/sh"))
    );
}

#[test]
fn missing_or_ambiguous_commits_and_nested_workspace_roots_are_unavailable() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.read("HEAD", &["README.md"]),
        Err(Error::UnsupportedEntry)
    );
    assert_eq!(
        fixture.read(&"e".repeat(40), &["README.md"]),
        Err(Error::Unavailable)
    );
    fixture.write("src/lib.rs", "public fixture");
    let adapter = GitIssueAssessmentEvidenceAdapter::new();
    assert_eq!(
        adapter.current_commit_sha(fixture.root.join("src").to_str().unwrap()),
        Err(Error::Unavailable)
    );
}

#[test]
fn execution_bearing_repository_configuration_is_rejected_before_any_evidence_read() {
    let fixture = Fixture::new();
    command(
        &fixture.root,
        &[
            "config",
            "filter.fixture.clean",
            "fixture-command-must-not-run",
        ],
    );
    assert_eq!(
        fixture.read(&fixture.base, &["README.md"]),
        Err(Error::Unavailable)
    );
}

#[test]
fn hardened_fsmonitor_is_disabled_and_does_not_block_exact_commit_evidence() {
    let fixture = Fixture::new();
    let expected = fixture.read(&fixture.base, &["README.md"]).unwrap();
    command(
        &fixture.root,
        &["config", "core.fsmonitor", "fixture-command-must-not-run"],
    );
    // Host Git overrides this repository setting; the guard intentionally permits it.
    let effective = git(&fixture.root, &["config", "--get", "core.fsmonitor"], 128).unwrap();
    assert_eq!(std::str::from_utf8(&effective).unwrap().trim(), "false");
    assert_eq!(
        fixture.read(&fixture.base, &["README.md"]).unwrap(),
        expected
    );
}

#[test]
fn incomplete_or_mismatched_tree_rows_are_errors_not_absence() {
    let oid = "a".repeat(40);
    for row in [
        format!("100644 blob {oid}\tfile.rs"),
        format!("100644 blob {oid}\tother.rs\0"),
        format!("100644 blob {oid}\tfile.rs\0garbage\0"),
        format!("100644  blob {oid}\tfile.rs\0"),
        "100644 blob short\tfile.rs\0".into(),
    ] {
        assert!(matches!(
            tree_entry(row.as_bytes(), "file.rs", 40),
            Err(Error::Incomplete)
        ));
    }
    assert!(tree_entry(&[], "file.rs", 40).unwrap().is_none());
    assert_eq!(
        tree_entry(
            format!("100644 blob {oid}\tfile.rs\0").as_bytes(),
            "file.rs",
            40
        )
        .unwrap()
        .unwrap()
        .oid,
        oid
    );
}
