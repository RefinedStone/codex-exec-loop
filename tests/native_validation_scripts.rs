use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn make_records_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir();

    for attempt in 0..100 {
        let dir = temp_dir.join(format!(
            "codex-exec-loop-validation-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return dir,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("validation temp dir should be created: {error}"),
        }
    }

    panic!("unique validation temp dir should be allocated");
}

fn write_record(dir: &Path, file_name: &str, body: &str) {
    fs::write(dir.join(file_name), body).expect("validation record should be written");
}

fn write_executable_file(path: &Path, body: &str) {
    fs::write(path, body).expect("script fixture should be written");
    let mut permissions = fs::metadata(path)
        .expect("script fixture metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("script fixture permissions should update");
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_repo_script(script_rel: &str, args: &[&str]) -> std::process::Output {
    Command::new("bash")
        .arg(repo_root().join(script_rel))
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| panic!("{script_rel} should run: {error}"))
}

fn summarize_output(records_dir: &Path, args: &[&str]) -> std::process::Output {
    let mut summary_args = vec![
        "--records-dir".to_string(),
        records_dir.display().to_string(),
    ];
    summary_args.extend(args.iter().map(|arg| (*arg).to_string()));
    let summary_arg_refs = summary_args.iter().map(String::as_str).collect::<Vec<_>>();
    run_repo_script("scripts/summarize_native_validation.sh", &summary_arg_refs)
}

fn summarize(records_dir: &Path, args: &[&str]) -> String {
    let output = summarize_output(records_dir, args);

    assert!(
        output.status.success(),
        "summary script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("summary output should be utf8")
}

#[test]
fn capture_helpers_expose_capture_role_contract() {
    let output = run_repo_script(
        "scripts/capture_native_validation.sh",
        &[
            "--frontend",
            "inline",
            "--check-profile",
            "replay-policy-representative",
            "--capture-role",
            "supplemental-unmatched",
            "--terminal",
            "Test Terminal",
            "--shell",
            "bash",
            "--notes",
            "capture role smoke",
        ],
    );
    assert!(output.status.success(), "capture helper failed");
    let stdout = String::from_utf8(output.stdout).expect("capture helper output should be utf8");
    assert!(stdout.contains("capture_role: supplemental-unmatched"));
    assert!(stdout.contains("check_profile: replay-policy-representative"));

    let ps1 = fs::read_to_string(repo_root().join("scripts/capture_native_validation.ps1"))
        .expect("powershell capture helper should be readable");
    assert!(ps1.contains("ValidateSet(\"counted-row\", \"supplemental-unmatched\")"));
    assert!(ps1.contains("capture_role: $CaptureRole"));
    assert!(ps1.contains("\"replay-policy-representative\""));
}

fn run_release_version_check(tag: &str, manifest_body: &str) -> std::process::Output {
    let dir = make_records_dir();
    let manifest_path = dir.join("Cargo.toml");
    fs::write(&manifest_path, manifest_body).expect("manifest fixture should be written");
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/validate_native_release_version.sh"))
        .arg("--tag")
        .arg(tag)
        .arg("--manifest")
        .arg(&manifest_path)
        .current_dir(repo_root())
        .output()
        .expect("release version validation script should run");
    fs::remove_dir_all(dir).expect("validation temp dir should be removed");
    output
}

fn run_git(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command should run")
}

fn assert_success(output: &std::process::Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn make_cleanup_worktree_fixture() -> (PathBuf, PathBuf, PathBuf) {
    let root = make_records_dir();
    let repo = root.join("repo");
    let feature_worktree = root.join("feature-worktree");
    fs::create_dir(&repo).expect("repo fixture dir should be created");

    assert_success(
        &Command::new("git")
            .arg("init")
            .arg(&repo)
            .output()
            .expect("git init should run"),
        "git init",
    );
    assert_success(
        &run_git(&repo, &["config", "user.email", "akra-test@example.com"]),
        "configure git user.email",
    );
    assert_success(
        &run_git(&repo, &["config", "user.name", "Akra Test"]),
        "configure git user.name",
    );

    fs::write(repo.join("README.md"), "initial\n").expect("initial fixture should be written");
    assert_success(
        &run_git(&repo, &["add", "README.md"]),
        "stage initial fixture",
    );
    assert_success(
        &run_git(&repo, &["commit", "-m", "initial"]),
        "commit initial fixture",
    );
    assert_success(
        &run_git(&repo, &["branch", "-M", "main"]),
        "rename base branch",
    );
    assert_success(
        &Command::new("git")
            .arg("-C")
            .arg(&repo)
            .arg("worktree")
            .arg("add")
            .arg("-b")
            .arg("feature")
            .arg(&feature_worktree)
            .arg("HEAD")
            .output()
            .expect("git worktree add should run"),
        "create feature worktree",
    );

    fs::write(feature_worktree.join("feature.txt"), "feature\n")
        .expect("feature fixture should be written");
    assert_success(
        &run_git(&feature_worktree, &["add", "feature.txt"]),
        "stage feature fixture",
    );
    assert_success(
        &run_git(&feature_worktree, &["commit", "-m", "feature"]),
        "commit feature fixture",
    );

    (root, repo, feature_worktree)
}

fn run_cleanup(repo: &Path, args: &[&str]) -> std::process::Output {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Command::new("bash")
        .arg(repo_root.join("scripts/cleanup_merged_worktrees.sh"))
        .args(args)
        .current_dir(repo)
        .output()
        .expect("cleanup worktree script should run")
}

fn branch_exists(repo: &Path, branch_name: &str) -> bool {
    let ref_name = format!("refs/heads/{branch_name}");
    run_git(
        repo,
        &["show-ref", "--verify", "--quiet", ref_name.as_str()],
    )
    .status
    .success()
}

#[test]
fn release_version_check_accepts_matching_v_tag() {
    let output = run_release_version_check(
        "v1.3.4",
        r#"[package]
name = "codex-exec-loop-native"
version = "1.3.4"
"#,
    );

    assert!(
        output.status.success(),
        "release version check failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("release_version=1.3.4"));
    assert!(stdout.contains("crate_version=1.3.4"));
}

#[test]
fn release_version_check_rejects_mismatched_tag() {
    let output = run_release_version_check(
        "v1.3.4",
        r#"[package]
name = "codex-exec-loop-native"
version = "1.3.3"
"#,
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release tag and Cargo.toml version do not match"));
    assert!(stderr.contains("tag version: 1.3.4"));
    assert!(stderr.contains("Cargo.toml version: 1.3.3"));
}

#[test]
fn release_version_check_rejects_unprefixed_matching_tag() {
    let output = run_release_version_check(
        "1.3.4",
        r#"[package]
name = "codex-exec-loop-native"
version = "1.3.4"
"#,
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release tag must use the v<version> convention"));
    assert!(stderr.contains("1.3.4"));
}

#[test]
fn public_release_and_github_scripts_report_missing_option_values() {
    let cases = [
        (
            "scripts/package_native_release.sh",
            vec!["--target"],
            "package_native_release: missing value for --target",
        ),
        (
            "scripts/verify_native_release.sh",
            vec!["--archive"],
            "verify_native_release: missing value for --archive",
        ),
        (
            "scripts/validate_native_release_version.sh",
            vec!["--tag"],
            "validate_native_release_version: missing value for --tag",
        ),
        (
            "scripts/gh-akra.sh",
            vec!["--github-login"],
            "gh-akra: missing value for --github-login",
        ),
    ];

    for (script, args, expected_error) in cases {
        let output = run_repo_script(script, &args);
        assert!(
            !output.status.success(),
            "{script} should fail for a missing option value"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected_error),
            "{script} should explain the missing option value\nstderr:\n{stderr}"
        );
    }
}

#[test]
fn gh_akra_auth_status_prefers_username_profile_when_user_differs() {
    let windows_users_root = Path::new("/mnt/c/Users");
    if !windows_users_root.is_dir() {
        return;
    }

    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    let home_root = root.join("home");
    let userprofile_root = root.join("userprofile");
    fs::create_dir(&repo).expect("repo fixture dir should be created");
    fs::create_dir(&bin_dir).expect("bin fixture dir should be created");
    fs::create_dir(&home_root).expect("home fixture dir should be created");
    fs::create_dir(&userprofile_root).expect("userprofile fixture dir should be created");

    assert_success(
        &Command::new("git")
            .arg("init")
            .arg(&repo)
            .output()
            .expect("git init should run"),
        "git init",
    );
    assert_success(
        &run_git(&repo, &["config", "credential.helper", ""]),
        "configure empty credential helper",
    );
    assert_success(
        &run_git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/acme/widgets.git",
            ],
        ),
        "configure origin",
    );

    write_executable_file(
        &bin_dir.join("gh"),
        r#"#!/bin/sh
set -eu
exit 1
"#,
    );
    write_executable_file(
        &bin_dir.join("curl"),
        r#"#!/bin/sh
set -eu
config=$(cat)
output_file=$(printf '%s\n' "$config" | sed -n 's/^output = "\(.*\)"$/\1/p')
case "$config" in
  *'Authorization: Bearer akra-token-123'*)
    printf '{"login":"akra"}' > "$output_file"
    ;;
  *)
    printf '{"login":"wrong"}' > "$output_file"
    ;;
esac
printf '200'
"#,
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let wrong_user = format!("aaa-gh-akra-wrong-{nonce}");
    let correct_user = format!("zzz-gh-akra-right-{nonce}");
    let wrong_dir = windows_users_root.join(&wrong_user);
    let correct_dir = windows_users_root.join(&correct_user);
    if let Err(error) = fs::create_dir_all(&wrong_dir) {
        if error.kind() == ErrorKind::PermissionDenied {
            let _ = fs::remove_dir_all(&root);
            return;
        }
        panic!("wrong Windows profile fixture should be created: {error}");
    }
    if let Err(error) = fs::create_dir_all(&correct_dir) {
        let _ = fs::remove_dir_all(&wrong_dir);
        let _ = fs::remove_dir_all(&root);
        if error.kind() == ErrorKind::PermissionDenied {
            return;
        }
        panic!("correct Windows profile fixture should be created: {error}");
    }
    fs::write(
        home_root.join(".git-credentials"),
        "https://home:home-token-123@github.com\n",
    )
    .expect("home credential fixture should be written");
    fs::write(
        userprofile_root.join(".git-credentials"),
        "https://userprofile:userprofile-token-123@github.com\n",
    )
    .expect("userprofile credential fixture should be written");
    fs::write(
        wrong_dir.join(".git-credentials"),
        "https://wrong:wrong-token-123@github.com\n",
    )
    .expect("wrong Windows credential fixture should be written");
    fs::write(
        correct_dir.join(".git-credentials"),
        "https://akra:akra-token-123@github.com\n",
    )
    .expect("correct Windows credential fixture should be written");

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .arg("auth")
        .arg("status")
        .current_dir(&repo)
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("HOME", &home_root)
        .env("USERPROFILE", &userprofile_root)
        .env("AKRA_GITHUB_TOKEN", "")
        .env("GH_TOKEN", "")
        .env("GITHUB_TOKEN", "")
        .env("USER", &wrong_user)
        .env("USERNAME", &correct_user)
        .output()
        .expect("gh-akra auth status should run");

    let _ = fs::remove_file(home_root.join(".git-credentials"));
    let _ = fs::remove_file(userprofile_root.join(".git-credentials"));
    let _ = fs::remove_file(wrong_dir.join(".git-credentials"));
    let _ = fs::remove_file(correct_dir.join(".git-credentials"));
    let _ = fs::remove_dir_all(&wrong_dir);
    let _ = fs::remove_dir_all(&correct_dir);
    let _ = fs::remove_dir_all(&root);

    assert_success(&output, "gh-akra auth status username precedence");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Logged in to github.com as akra"));
}

#[test]
fn gh_akra_auth_status_works_outside_repo_with_local_credentials() {
    let root = make_records_dir();
    let bin_dir = root.join("bin");
    let home_root = root.join("home");
    fs::create_dir(&bin_dir).expect("bin fixture dir should be created");
    fs::create_dir(&home_root).expect("home fixture dir should be created");
    fs::write(
        home_root.join(".git-credentials"),
        "https://akra:outside-home-token-123@github.com\n",
    )
    .expect("home credential fixture should be written");

    write_executable_file(
        &bin_dir.join("gh"),
        r#"#!/bin/sh
set -eu
exit 1
"#,
    );
    write_executable_file(
        &bin_dir.join("curl"),
        r#"#!/bin/sh
set -eu
config=$(cat)
output_file=$(printf '%s\n' "$config" | sed -n 's/^output = "\(.*\)"$/\1/p')
case "$config" in
  *'Authorization: Bearer outside-home-token-123'*)
    printf '{"login":"akra"}' > "$output_file"
    ;;
  *)
    printf '{"login":"wrong"}' > "$output_file"
    ;;
esac
printf '200'
"#,
    );

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .arg("auth")
        .arg("status")
        .current_dir(&root)
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("HOME", &home_root)
        .env("USERPROFILE", "")
        .env("AKRA_GITHUB_TOKEN", "")
        .env("GH_TOKEN", "")
        .env("GITHUB_TOKEN", "")
        .output()
        .expect("gh-akra auth status should run outside a repo");

    let _ = fs::remove_file(home_root.join(".git-credentials"));
    let _ = fs::remove_dir_all(&root);

    assert_success(&output, "gh-akra auth status outside repo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Logged in to github.com as akra"));
}

#[test]
fn gh_akra_auth_status_preserves_repo_scoped_git_credential_fill() {
    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    let home_root = root.join("home");
    fs::create_dir(&repo).expect("repo fixture dir should be created");
    fs::create_dir(&bin_dir).expect("bin fixture dir should be created");
    fs::create_dir(&home_root).expect("home fixture dir should be created");

    assert_success(
        &Command::new("git")
            .arg("init")
            .arg(&repo)
            .output()
            .expect("git init should run"),
        "git init",
    );
    assert_success(
        &run_git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/acme/widgets.git",
            ],
        ),
        "configure origin",
    );
    assert_success(
        &run_git(&repo, &["config", "credential.useHttpPath", "true"]),
        "configure path-sensitive credential path matching",
    );

    assert_success(
        &run_git(
            &repo,
            &[
                "config",
                "credential.helper",
                "!f() { input=$(cat); case \"$input\" in *'path=acme/widgets'*) printf 'username=akra\\npassword=path-token-123\\n' ;; *) printf 'username=akra\\npassword=generic-token-123\\n' ;; esac; }; f",
            ],
        ),
        "configure path-sensitive credential helper",
    );

    write_executable_file(
        &bin_dir.join("gh"),
        r#"#!/bin/sh
set -eu
exit 1
"#,
    );
    write_executable_file(
        &bin_dir.join("curl"),
        r#"#!/bin/sh
set -eu
config=$(cat)
output_file=$(printf '%s\n' "$config" | sed -n 's/^output = "\(.*\)"$/\1/p')
case "$config" in
  *'Authorization: Bearer path-token-123'*)
    printf '{"login":"akra"}' > "$output_file"
    ;;
  *)
    printf '{"login":"wrong"}' > "$output_file"
    ;;
esac
printf '200'
"#,
    );

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .arg("auth")
        .arg("status")
        .current_dir(&repo)
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("HOME", &home_root)
        .env("USERPROFILE", "")
        .env("AKRA_GITHUB_TOKEN", "")
        .env("GH_TOKEN", "")
        .env("GITHUB_TOKEN", "")
        .output()
        .expect("gh-akra auth status should run inside a repo");

    let _ = fs::remove_dir_all(&root);

    assert_success(&output, "gh-akra auth status repo-scoped credential fill");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Logged in to github.com as akra"));
}

#[test]
fn cleanup_explicit_unmerged_branch_is_skipped_by_default() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();

    let output = run_cleanup(&repo, &["--apply", "--base", "main", "--branch", "feature"]);

    assert_success(&output, "cleanup explicit unmerged branch");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[skip] branch not merged into main"));
    assert!(stdout.contains("cleanup complete: removed 0 worktree(s)"));
    assert!(feature_worktree.is_dir());
    assert!(branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn cleanup_force_dirty_does_not_bypass_unmerged_guard() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();
    fs::write(feature_worktree.join("dirty.txt"), "dirty\n")
        .expect("dirty fixture should be written");

    let output = run_cleanup(
        &repo,
        &[
            "--apply",
            "--base",
            "main",
            "--branch",
            "feature",
            "--force-dirty",
        ],
    );

    assert_success(&output, "cleanup explicit dirty unmerged branch");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[skip] branch not merged into main"));
    assert!(feature_worktree.is_dir());
    assert!(branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn cleanup_explicit_rebase_merged_branch_is_removed_by_patch_equivalence() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();
    fs::write(repo.join("base.txt"), "base moved independently\n")
        .expect("base fixture should be written");
    assert_success(&run_git(&repo, &["add", "base.txt"]), "stage base fixture");
    assert_success(
        &run_git(&repo, &["commit", "-m", "base moved"]),
        "commit base fixture",
    );
    assert_success(
        &run_git(&repo, &["cherry-pick", "feature"]),
        "cherry-pick feature patch onto base",
    );
    assert!(
        !run_git(&repo, &["merge-base", "--is-ancestor", "feature", "main"])
            .status
            .success(),
        "fixture should model a rebase-merge style integration, not ancestry"
    );

    let output = run_cleanup(&repo, &["--apply", "--base", "main", "--branch", "feature"]);

    assert_success(&output, "cleanup explicit patch-equivalent branch");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("removing worktree"));
    assert!(stdout.contains("cleanup complete: removed 1 worktree(s)"));
    assert!(!feature_worktree.exists());
    assert!(!branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn cleanup_allow_unmerged_explicit_removes_disposable_branch() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();

    let output = run_cleanup(
        &repo,
        &[
            "--apply",
            "--base",
            "main",
            "--branch",
            "feature",
            "--allow-unmerged-explicit",
        ],
    );

    assert_success(&output, "cleanup explicitly allowed unmerged branch");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[warn] explicit target is not merged into main"));
    assert!(stdout.contains("removing worktree"));
    assert!(stdout.contains("cleanup complete: removed 1 worktree(s)"));
    assert!(!feature_worktree.exists());
    assert!(!branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn cleanup_explicit_missing_target_fails_without_opt_out() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();

    let output = run_cleanup(
        &repo,
        &["--base", "main", "--branch", "definitely-not-a-real-branch"],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("explicit targets not found: branch:definitely-not-a-real-branch"));
    assert!(stderr.contains("--allow-empty-explicit-targets"));
    assert!(feature_worktree.is_dir());
    assert!(branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn cleanup_explicit_missing_target_can_be_ignored() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();

    let output = run_cleanup(
        &repo,
        &[
            "--base",
            "main",
            "--branch",
            "definitely-not-a-real-branch",
            "--allow-empty-explicit-targets",
        ],
    );

    assert_success(&output, "cleanup explicit missing target with opt-out");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("allowed by --allow-empty-explicit-targets"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("dry-run complete: 0 eligible"));

    assert!(feature_worktree.is_dir());
    assert!(branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn cleanup_mixed_explicit_targets_fail_when_any_requested_target_is_missing() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();

    let output = run_cleanup(
        &repo,
        &[
            "--base",
            "main",
            "--branch",
            "feature",
            "--branch",
            "definitely-not-a-real-branch",
        ],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("explicit targets not found: branch:definitely-not-a-real-branch"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[skip] branch not merged into main"));
    assert!(feature_worktree.is_dir());
    assert!(branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn cleanup_apply_mixed_explicit_targets_stays_atomic_on_failure() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();

    let output = run_cleanup(
        &repo,
        &[
            "--apply",
            "--base",
            "main",
            "--branch",
            "feature",
            "--branch",
            "definitely-not-a-real-branch",
            "--allow-unmerged-explicit",
        ],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("explicit targets not found: branch:definitely-not-a-real-branch"));
    assert!(feature_worktree.is_dir());
    assert!(branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn cleanup_apply_dirty_mixed_explicit_targets_stays_atomic_on_failure() {
    let (root, repo, feature_worktree) = make_cleanup_worktree_fixture();
    fs::write(feature_worktree.join("dirty.txt"), "dirty\n")
        .expect("dirty fixture should be written");

    let output = run_cleanup(
        &repo,
        &[
            "--apply",
            "--base",
            "main",
            "--branch",
            "feature",
            "--branch",
            "definitely-not-a-real-branch",
            "--allow-unmerged-explicit",
            "--force-dirty",
        ],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("explicit targets not found: branch:definitely-not-a-real-branch"));
    assert!(feature_worktree.is_dir());
    assert!(branch_exists(&repo, "feature"));

    fs::remove_dir_all(root).expect("cleanup fixture should be removed");
}

#[test]
fn prompt_input_delay_profile_counts_tmux_detached_pty_row() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "prompt-tmux.txt",
        r#"date: 2026-05-09
commit: abc123
os: Ubuntu 24.04.2 LTS / WSL2 6.6.114.1-microsoft-standard-WSL2
terminal: tmux 3.4 detached PTY
shell: bash
frontend: inline
term: tmux-256color
capture_role: counted-row
check_profile: prompt-input-delay-pty
checks:
- launch inline TUI in tmux PTY
result: pass
notes: prompt echo under budget
"#,
    );
    write_record(
        &records_dir,
        "baseline-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
capture_role: counted-row
check_profile: terminal-baseline
checks:
- launch and exit
result: pass
notes: baseline only
"#,
    );

    let output = summarize(&records_dir, &["--check-profile", "prompt-input-delay-pty"]);

    assert!(output.contains("check profile: prompt-input-delay-pty"));
    assert!(output.contains("required pass: 1/5"));
    assert!(output.contains("Linux / tmux detached PTY / bash / inline"));
    assert!(output.contains("prompt-tmux.txt"));
    assert!(!output.contains("baseline-terminal-app.txt"));
    assert!(!output.contains("Unmatched Records"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}

#[test]
fn default_summary_filters_out_prompt_input_delay_records() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "prompt-tmux.txt",
        r#"date: 2026-05-09
commit: abc123
os: Ubuntu 24.04.2 LTS / WSL2 6.6.114.1-microsoft-standard-WSL2
terminal: tmux 3.4 detached PTY
shell: bash
frontend: inline
term: tmux-256color
capture_role: counted-row
check_profile: prompt-input-delay-pty
checks:
- launch inline TUI in tmux PTY
result: pass
notes: prompt echo under budget
"#,
    );
    write_record(
        &records_dir,
        "baseline-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
capture_role: counted-row
check_profile: terminal-baseline
checks:
- launch and exit
result: pass
notes: baseline only
"#,
    );

    let output = summarize(&records_dir, &[]);

    assert!(output.contains("check profile: terminal-baseline"));
    assert!(output.contains("required pass: 1/4"));
    assert!(output.contains("baseline-terminal-app.txt"));
    assert!(!output.contains("prompt-tmux.txt"));
    assert!(!output.contains("Unmatched Records"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}

#[test]
fn supplemental_unmatched_exact_row_does_not_count_toward_terminal_baseline() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "supplemental-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
capture_role: supplemental-unmatched
check_profile: terminal-baseline
checks:
- launch and exit
result: pass
notes: supplemental replay-only evidence
"#,
    );

    let output = summarize(&records_dir, &[]);

    assert!(output.contains("check profile: terminal-baseline"));
    assert!(output.contains("required pass: 0/4"));
    assert!(output.contains("required missing: 4"));
    assert!(output.contains("Unmatched Records"));
    assert!(output.contains("supplemental-terminal-app.txt"));
    assert!(!output.contains("Terminal.app / zsh / inline (supplemental-terminal-app.txt)"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}

#[test]
fn missing_capture_role_does_not_count_toward_terminal_baseline() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "missing-role-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
check_profile: terminal-baseline
checks:
- launch and exit
result: pass
notes: missing capture role
"#,
    );

    let output = summarize(&records_dir, &[]);

    assert!(output.contains("required pass: 0/4"));
    assert!(output.contains("required missing: 4"));
    assert!(output.contains("Unmatched Records"));
    assert!(output.contains("missing-role-terminal-app.txt"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}

#[test]
fn missing_checks_block_does_not_count_toward_terminal_baseline() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "missing-checks-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
capture_role: counted-row
check_profile: terminal-baseline
result: pass
notes: missing checks block
"#,
    );

    let output = summarize(&records_dir, &[]);

    assert!(output.contains("required pass: 0/4"));
    assert!(output.contains("required missing: 4"));
    assert!(output.contains("Unmatched Records"));
    assert!(output.contains("missing-checks-terminal-app.txt"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}

#[test]
fn missing_check_profile_does_not_count_toward_terminal_baseline() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "missing-profile-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
capture_role: counted-row
checks:
- launch and exit
result: pass
notes: missing check profile
"#,
    );

    let output = summarize(&records_dir, &[]);

    assert!(output.contains("required pass: 0/4"));
    assert!(output.contains("required missing: 4"));
    assert!(output.contains("Unmatched Records"));
    assert!(output.contains("missing-profile-terminal-app.txt"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}
#[test]
fn crlf_counted_rows_still_count_toward_terminal_baseline() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "crlf-terminal-app.txt",
        "date: 2026-05-09\r\ncommit: abc123\r\nos: macOS 14.5\r\nterminal: Terminal.app\r\nshell: zsh\r\nfrontend: inline\r\nterm: xterm-256color\r\ncapture_role: counted-row\r\ncheck_profile: terminal-baseline\r\nchecks:\r\n- launch and exit\r\nresult: pass\r\nnotes: crlf baseline row\r\n",
    );

    let output = summarize(&records_dir, &[]);

    assert!(output.contains("required pass: 1/4"));
    assert!(output.contains("crlf-terminal-app.txt"));
    assert!(!output.contains("Unmatched Records"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}

#[test]
fn default_summary_warns_when_required_rows_are_incomplete() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "baseline-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
capture_role: counted-row
check_profile: terminal-baseline
checks:
- launch and exit
result: pass
notes: baseline only
"#,
    );

    let output = summarize(&records_dir, &[]);

    assert!(output.contains("WARNING"));
    assert!(output.contains("--fail-on-incomplete"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}

#[test]
fn fail_on_incomplete_turns_summary_into_a_gate() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "baseline-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
capture_role: counted-row
check_profile: terminal-baseline
checks:
- launch and exit
result: pass
notes: baseline only
"#,
    );

    let output = summarize_output(&records_dir, &["--fail-on-incomplete"]);

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("summary output should be utf8");
    assert!(stdout.contains("required missing: 3"));
    assert!(!stdout.contains("WARNING"));
    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}
