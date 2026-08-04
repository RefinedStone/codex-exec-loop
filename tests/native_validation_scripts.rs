#![cfg(unix)]

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

fn link_path_command(bin_dir: &Path, command: &str) {
    let source = which::which(command)
        .unwrap_or_else(|error| panic!("{command} should be available for script tests: {error}"));
    symlink(source, bin_dir.join(command))
        .unwrap_or_else(|error| panic!("{command} fixture link should be created: {error}"));
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

fn workflow_job_block<'a>(source: &'a str, job_name: &str) -> &'a str {
    let marker = format!("  {job_name}:\n");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("workflow job should exist: {job_name}"));
    let content_start = start + marker.len();
    let tail = &source[content_start..];
    let end = tail
        .match_indices('\n')
        .find_map(|(newline_offset, _)| {
            let line_start = newline_offset + 1;
            let line_end = tail[line_start..]
                .find('\n')
                .map(|offset| line_start + offset)
                .unwrap_or(tail.len());
            let line = &tail[line_start..line_end];
            (line.starts_with("  ") && !line.starts_with("    ") && line.ends_with(':'))
                .then_some(newline_offset)
        })
        .unwrap_or(tail.len());
    &source[start..content_start + end]
}

fn assert_workflow_actions_are_commit_pinned(workflow: &str) {
    for action in workflow.lines().filter_map(|line| {
        line.trim()
            .strip_prefix("uses: ")
            .filter(|value| !value.starts_with("./"))
    }) {
        let (_, revision_and_comment) = action
            .rsplit_once('@')
            .unwrap_or_else(|| panic!("workflow action has no revision: {action}"));
        let revision = revision_and_comment
            .split_whitespace()
            .next()
            .expect("workflow action revision should not be empty");
        assert_eq!(
            revision.len(),
            40,
            "workflow action is not SHA-pinned: {action}"
        );
        assert!(
            revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "workflow action revision is not hexadecimal: {action}"
        );
    }
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
            "fullscreen",
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

#[test]
fn release_version_check_reports_missing_tomllib_as_a_prerequisite() {
    let root = make_records_dir();
    let bin = root.join("bin");
    fs::create_dir(&bin).expect("fake binary directory should create");
    let python = bin.join("python3");
    write_executable_file(
        &python,
        "#!/bin/sh\nprintf 'ModuleNotFoundError: tomllib\\n' >&2\nexit 1\n",
    );
    let manifest = root.join("Cargo.toml");
    fs::write(
        &manifest,
        "[package]\nname = \"codex-exec-loop-native\"\nversion = \"1.2.3\"\n",
    )
    .expect("release manifest fixture should write");

    let inherited_path = std::env::var_os("PATH").expect("test PATH should be available");
    let fixture_path = std::env::join_paths(
        std::iter::once(bin.clone()).chain(std::env::split_paths(&inherited_path)),
    )
    .expect("fixture PATH should join");
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/validate_native_release_version.sh"))
        .args(["--tag", "v1.2.3", "--manifest"])
        .arg(&manifest)
        .env("PATH", fixture_path)
        .current_dir(repo_root())
        .output()
        .expect("release prerequisite validation should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.trim(),
        "validate_native_release_version: python3 with tomllib support is required"
    );
    assert!(!stderr.contains("ModuleNotFoundError"));
    fs::remove_dir_all(root).expect("release prerequisite fixture should remove");
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

#[derive(Debug)]
struct TarEntry {
    name: String,
    mode: u64,
    uid: u64,
    gid: u64,
    mtime: u64,
    entry_type: u8,
}

fn tar_text(field: &[u8]) -> String {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    String::from_utf8(field[..end].to_vec()).expect("tar header text should be utf8")
}

fn tar_octal(field: &[u8]) -> u64 {
    let text = tar_text(field).trim().to_string();
    u64::from_str_radix(&text, 8).unwrap_or_else(|error| {
        panic!("tar header field should contain octal digits ({text:?}): {error}")
    })
}

fn read_tar_entries(archive_path: &Path) -> Vec<TarEntry> {
    let archive = fs::read(archive_path).expect("release archive should be readable");
    assert_eq!(&archive[..2], &[0x1f, 0x8b], "archive should remain gzip");
    assert_eq!(
        archive[3] & 0x08,
        0,
        "gzip header must not contain a source filename"
    );
    assert_eq!(
        &archive[4..8],
        &[0, 0, 0, 0],
        "gzip header timestamp must be normalized"
    );

    let output = Command::new("gzip")
        .arg("-dc")
        .arg(archive_path)
        .output()
        .expect("gzip should decompress the release archive");
    assert_success(&output, "decompress deterministic release archive");

    let tar = output.stdout;
    let mut entries = Vec::new();
    let mut offset = 0_usize;
    while offset + 512 <= tar.len() {
        let header = &tar[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        assert_eq!(
            &header[257..262],
            b"ustar",
            "archive format should be ustar"
        );

        let name = tar_text(&header[..100]);
        let prefix = tar_text(&header[345..500]);
        let name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let size = tar_octal(&header[124..136]) as usize;
        entries.push(TarEntry {
            name,
            mode: tar_octal(&header[100..108]),
            uid: tar_octal(&header[108..116]),
            gid: tar_octal(&header[116..124]),
            mtime: tar_octal(&header[136..148]),
            entry_type: header[156],
        });
        offset += 512 + size.div_ceil(512) * 512;
    }
    entries
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
        r#"[workspace.package]
version = "9.9.9"

[package]
name = "codex-exec-loop-native"
version = "1.3.4"

[dependencies]
fixture = { version = "8.8.8" }
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
    assert!(stdout.contains("npm_dist_tag=latest"));
}

#[test]
fn release_version_check_rejects_non_string_package_version() {
    let output = run_release_version_check(
        "v1.3.4",
        r#"[package]
name = "codex-exec-loop-native"
version.workspace = true
"#,
    );

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("[package].version must be a string"));
}

#[test]
fn release_version_check_requires_tagged_commit_on_allowed_branch() {
    let root = make_records_dir();
    let repo = root.join("repo");
    fs::create_dir(&repo).expect("release ancestry repo should be created");
    assert_success(
        &run_git(&repo, &["init"]),
        "initialize release ancestry repo",
    );
    assert_success(
        &run_git(&repo, &["config", "user.email", "release-test@example.com"]),
        "configure release test email",
    );
    assert_success(
        &run_git(&repo, &["config", "user.name", "Release Test"]),
        "configure release test name",
    );
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"codex-exec-loop-native\"\nversion = \"1.2.3\"\n",
    )
    .expect("release ancestry manifest should be written");
    assert_success(
        &run_git(&repo, &["add", "Cargo.toml"]),
        "stage release manifest",
    );
    assert_success(
        &run_git(&repo, &["commit", "-m", "trusted release"]),
        "commit trusted release",
    );
    assert_success(
        &run_git(&repo, &["branch", "-M", "prerelease"]),
        "name trusted branch",
    );
    assert_success(&run_git(&repo, &["tag", "v1.2.3"]), "tag trusted release");
    let trusted_sha = String::from_utf8(run_git(&repo, &["rev-parse", "HEAD"]).stdout)
        .expect("trusted sha should be utf8")
        .trim()
        .to_string();

    let run = |release_commit: &str| {
        Command::new("bash")
            .arg(repo_root().join("scripts/validate_native_release_version.sh"))
            .args(["--tag", "v1.2.3", "--release-commit", release_commit])
            .args(["--allowed-ref", "refs/heads/prerelease", "--repository"])
            .arg(&repo)
            .arg("--manifest")
            .arg(repo.join("Cargo.toml"))
            .current_dir(&repo)
            .output()
            .expect("release ancestry validation should run")
    };
    assert_success(&run(&trusted_sha), "trusted release ancestry");

    assert_success(
        &run_git(&repo, &["checkout", "-b", "off-branch"]),
        "create off branch",
    );
    fs::write(repo.join("off-branch.txt"), "unreviewed\n")
        .expect("off-branch fixture should be written");
    assert_success(
        &run_git(&repo, &["add", "off-branch.txt"]),
        "stage off-branch fixture",
    );
    assert_success(
        &run_git(&repo, &["commit", "-m", "off branch release"]),
        "commit off branch",
    );
    assert_success(
        &run_git(&repo, &["tag", "-f", "v1.2.3"]),
        "move release tag off branch",
    );
    let off_branch_sha = String::from_utf8(run_git(&repo, &["rev-parse", "HEAD"]).stdout)
        .expect("off-branch sha should be utf8")
        .trim()
        .to_string();
    let rejected = run(&off_branch_sha);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("release commit is not contained in refs/heads/prerelease")
    );

    fs::remove_dir_all(root).expect("release ancestry fixture should be removed");
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
    assert!(stderr.contains("release tag must use stable vMAJOR.MINOR.PATCH"));
    assert!(stderr.contains("1.3.4"));
}

#[test]
fn release_version_check_rejects_prerelease_and_build_metadata_tags() {
    for version in ["1.3.4-rc.1", "1.3.4+build.7"] {
        let tag = format!("v{version}");
        let manifest = format!(
            r#"[package]
name = "codex-exec-loop-native"
version = "{version}"
"#
        );
        let output = run_release_version_check(&tag, &manifest);

        assert!(!output.status.success(), "{tag} must fail closed");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("official releases require a stable vMAJOR.MINOR.PATCH tag"));
        assert!(stderr.contains("Prerelease and build-metadata tags are not published"));
    }
}

#[test]
fn npm_release_auth_check_fails_closed_and_verifies_configured_token() {
    let root = make_records_dir();
    let bin_dir = root.join("bin");
    let call_log = root.join("npm-call.log");
    let poison_cwd = root.join("poison-cwd");
    let poison_home = root.join("poison-home");
    let poison_user_config = root.join("poison-user.npmrc");
    let runtime_tmp = root.join("runtime-tmp");
    fs::create_dir(&bin_dir).expect("fake npm bin dir should be created");
    fs::create_dir(&poison_cwd).expect("poison cwd should be created");
    fs::create_dir(&poison_home).expect("poison home should be created");
    fs::create_dir(&runtime_tmp).expect("auth runtime temp root should be created");
    let poisoned_config = r#"@refinedstone:registry=https://registry.attacker.invalid/
//registry.attacker.invalid/:_authToken=${NODE_AUTH_TOKEN}
proxy=https://proxy.attacker.invalid/
cafile=/tmp/attacker-ca.pem
strict-ssl=false
"#;
    fs::write(poison_cwd.join(".npmrc"), poisoned_config)
        .expect("poison project npmrc should be written");
    fs::write(poison_home.join(".npmrc"), poisoned_config)
        .expect("poison home npmrc should be written");
    fs::write(&poison_user_config, poisoned_config).expect("poison user npmrc should be written");
    write_executable_file(
        &bin_dir.join("npm"),
        r#"#!/bin/bash
set -euo pipefail
test "$1" = "whoami"
test "$2" = "--registry"
test "$3" = "https://registry.npmjs.org"
test "$PWD" != "${AKRA_NPM_POISON_CWD}"
test "${NODE_AUTH_TOKEN}" = "fixture-token-must-not-be-printed"
test "${NPM_CONFIG_REGISTRY}" = "https://registry.npmjs.org"
test "${NPM_CONFIG_UPDATE_NOTIFIER}" = "false"
test "${NPM_CONFIG_CACHE}" = "$PWD/cache"
test "${NPM_CONFIG_USERCONFIG}" = "$PWD/user.npmrc"
test "${NPM_CONFIG_GLOBALCONFIG}" = "$PWD/global.npmrc"
test -z "${npm_config_proxy+x}"
test -z "${NpM_CoNfIg_CaFiLe+x}"
test -z "${NPM_CONFIG_STRICT_SSL+x}"
test -f "$PWD/.npmrc"
cmp -s "$PWD/.npmrc" "$PWD/user.npmrc"
test ! -s "$PWD/global.npmrc"
grep -Fqx 'registry=https://registry.npmjs.org/' "$PWD/.npmrc"
grep -Fqx '@refinedstone:registry=https://registry.npmjs.org/' "$PWD/.npmrc"
grep -Fqx '//registry.npmjs.org/:_authToken=${NODE_AUTH_TOKEN}' "$PWD/.npmrc"
if grep -Fq "${NODE_AUTH_TOKEN}" "$PWD/.npmrc" "$PWD/user.npmrc" "$PWD/global.npmrc"; then
  exit 71
fi
{
  printf 'args=%s\n' "$*"
  printf 'cwd=%s\n' "$PWD"
  printf 'registry=%s\n' "${NPM_CONFIG_REGISTRY}"
  printf 'cache=%s\n' "${NPM_CONFIG_CACHE}"
  printf 'home=%s\n' "${HOME}"
  printf '%s\n' 'project-config-begin'
  cat "$PWD/.npmrc"
  printf '%s\n' 'project-config-end'
} > "${AKRA_NPM_CALL_LOG}"
printf 'refinedstone\n'
"#,
    );
    let script = repo_root().join("scripts/validate_npm_release_auth.sh");
    let path = format!("{}:/usr/bin:/bin", bin_dir.display());

    let missing = Command::new("bash")
        .arg(&script)
        .env("PATH", &path)
        .env("AKRA_NPM_CALL_LOG", &call_log)
        .env_remove("NODE_AUTH_TOKEN")
        .output()
        .expect("npm release auth check should run without a token");
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("NPM_TOKEN is required for an official tag release"));
    assert!(
        !call_log.exists(),
        "missing-token preflight must not invoke npm"
    );

    let configured = Command::new("bash")
        .arg(&script)
        .current_dir(&poison_cwd)
        .env("PATH", &path)
        .env("AKRA_NPM_CALL_LOG", &call_log)
        .env("AKRA_NPM_POISON_CWD", &poison_cwd)
        .env("HOME", &poison_home)
        .env("TMPDIR", &runtime_tmp)
        .env("NODE_AUTH_TOKEN", "fixture-token-must-not-be-printed")
        .env("NPM_CONFIG_REGISTRY", "https://malicious-registry.invalid")
        .env("NPM_CONFIG_USERCONFIG", &poison_user_config)
        .env("NPM_CONFIG_STRICT_SSL", "false")
        .env("npm_config_proxy", "https://proxy.attacker.invalid")
        .env("NpM_CoNfIg_CaFiLe", "/tmp/attacker-ca.pem")
        .output()
        .expect("npm release auth check should run with a token");
    assert_success(&configured, "configured npm release auth check");
    let stdout = String::from_utf8_lossy(&configured.stdout);
    let configured_stderr = String::from_utf8_lossy(&configured.stderr);
    assert!(stdout.contains("npm release authentication verified"));
    assert!(!stdout.contains("fixture-token-must-not-be-printed"));
    assert!(!configured_stderr.contains("fixture-token-must-not-be-printed"));
    let call_record =
        fs::read_to_string(&call_log).expect("fake npm invocation should be recorded");
    assert!(call_record.contains("args=whoami --registry https://registry.npmjs.org\n"));
    assert!(call_record.contains("registry=https://registry.npmjs.org\n"));
    assert!(call_record.contains("@refinedstone:registry=https://registry.npmjs.org/"));
    assert!(call_record.contains("//registry.npmjs.org/:_authToken=${NODE_AUTH_TOKEN}"));
    assert!(call_record.contains(&format!("home={}\n", poison_home.display())));
    assert!(!call_record.contains("fixture-token-must-not-be-printed"));
    assert!(!call_record.contains("attacker.invalid"));
    assert!(!call_record.contains("attacker-ca.pem"));
    let runtime_path = call_record
        .lines()
        .find_map(|line| line.strip_prefix("cwd="))
        .map(PathBuf::from)
        .expect("fake npm should record its isolated cwd");
    assert!(runtime_path.starts_with(&runtime_tmp));
    assert!(
        !runtime_path.exists(),
        "auth probe runtime including npm logs and configuration must be removed"
    );

    fs::remove_dir_all(root).expect("npm auth fixture should be removed");
}

#[test]
fn release_workflow_gates_assets_on_authenticated_npm_publication() {
    let workflow =
        fs::read_to_string(repo_root().join(".github/workflows/release-native-assets.yml"))
            .expect("release workflow should be readable");
    let validate = workflow_job_block(&workflow, "validate-release");
    let build = workflow_job_block(&workflow, "build-native-assets");
    let publish_npm = workflow_job_block(&workflow, "publish-npm");
    let publish_release = workflow_job_block(&workflow, "publish-release");

    assert!(workflow.contains("      - \"v*\""));
    assert!(workflow.contains("\nconcurrency:"));
    assert!(workflow.contains("group: release-native-assets-${{ github.ref }}"));
    assert!(workflow.contains("cancel-in-progress: false"));
    assert!(validate.contains("fetch-depth: 0"));
    assert!(validate.contains("refs/heads/prerelease:refs/remotes/origin/prerelease"));
    assert!(validate.contains("--release-commit \"${GITHUB_SHA}\""));
    assert!(validate.contains("--allowed-ref refs/remotes/origin/prerelease"));
    assert!(!validate.contains("NODE_AUTH_TOKEN"));
    assert!(build.contains("      - validate-release"));
    assert!(publish_npm.contains("      - build-native-assets"));
    assert!(!publish_npm.contains("      - publish-release"));
    assert!(!publish_npm.contains("configured=false"));
    assert!(!publish_npm.contains("skipping npm publish"));
    assert!(publish_npm.contains("      id-token: write"));
    assert!(publish_npm.contains("    environment: npm-release"));
    assert!(publish_npm.contains("bash scripts/validate_npm_release_auth.sh"));
    assert!(build.contains("source_date_epoch=\"$(git show -s --format=%ct HEAD)\""));
    assert!(build.contains("export SOURCE_DATE_EPOCH=\"${source_date_epoch}\""));
    assert!(build.contains(
        "uses: dtolnay/rust-toolchain@eac0f66a48bc4b70a10b9acd4c4e930f835d95ff # 1.95.0"
    ));
    assert!(!workflow.contains("dtolnay/rust-toolchain@stable"));
    assert_eq!(
        workflow
            .matches("NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}")
            .count(),
        3
    );
    assert!(!workflow.contains("runs-on: ubuntu-latest\n    env:\n      NODE_AUTH_TOKEN"));
    assert!(publish_npm.contains("node npm/scripts/publish-package.mjs"));
    assert!(publish_npm.contains("--expected-version \"${expected_version}\""));
    assert!(publish_npm.contains("--expected-version \"${release_version}\""));
    assert_eq!(
        publish_npm
            .matches("env -u GH_TOKEN node npm/scripts/publish-package.mjs")
            .count(),
        2,
        "npm must not inherit the GitHub API credential"
    );
    assert_eq!(
        publish_npm
            .matches("env -u NODE_AUTH_TOKEN bash .github/scripts/verify-tag-target.sh")
            .count(),
        4,
        "GitHub tag checks must not inherit the npm publish credential"
    );
    assert!(publish_npm.contains("node npm/scripts/verify-native-release-assets.mjs"));
    assert!(publish_npm.contains(".github/scripts/verify-tag-target.sh"));
    assert!(publish_npm.contains("--expected-commit \"${GITHUB_SHA}\""));
    assert!(publish_npm.contains("--tag platform"));
    assert!(publish_npm.contains("--tag latest"));
    assert!(!publish_npm.contains("NPM_CONFIG_REGISTRY"));
    assert!(publish_release.contains("      - build-native-assets"));
    assert!(publish_release.contains("      - publish-npm"));
    assert!(publish_release.contains("      contents: write"));
    assert!(publish_release.contains(".github/scripts/publish-release-assets.sh"));
    assert!(publish_release.contains("--expected-commit \"${GITHUB_SHA}\""));
    assert!(publish_release.contains("node npm/scripts/verify-native-release-assets.mjs"));
    assert_eq!(
        workflow.matches("uses: actions/checkout@").count(),
        workflow.matches("persist-credentials: false").count(),
        "every release checkout must avoid persisting the job token into git credentials"
    );
    assert_workflow_actions_are_commit_pinned(&workflow);
    assert!(!workflow.contains("--clobber"));
    assert!(
        fs::read_to_string(repo_root().join(".github/scripts/publish-release-assets.sh"))
            .expect("release publisher should be readable")
            .contains("--verify-tag")
    );
}

#[test]
fn github_tag_target_verification_peels_annotated_tags_and_fails_closed() {
    let root = make_records_dir();
    let bin_dir = root.join("bin");
    fs::create_dir(&bin_dir).expect("fake gh bin dir should be created");
    let commit_sha = "a".repeat(40);
    let first_tag_sha = "b".repeat(40);
    let second_tag_sha = "c".repeat(40);
    write_executable_file(
        &bin_dir.join("gh"),
        r#"#!/bin/bash
set -euo pipefail
test "$1" = "api"
endpoint="${*: -1}"
case "${FAKE_TAG_MODE}:${endpoint}" in
  missing:*) exit 1 ;;
  lightweight:*/git/ref/tags/v1.2.3)
    printf '{"object":{"type":"commit","sha":"%s"}}\n' "${FAKE_TAG_COMMIT}"
    ;;
  annotated:*/git/ref/tags/v1.2.3|nested:*/git/ref/tags/v1.2.3)
    printf '{"object":{"type":"tag","sha":"%s"}}\n' "${FAKE_TAG_OBJECT_ONE}"
    ;;
  annotated:*/git/tags/*)
    printf '{"object":{"type":"commit","sha":"%s"}}\n' "${FAKE_TAG_COMMIT}"
    ;;
  nested:*/git/tags/${FAKE_TAG_OBJECT_ONE})
    printf '{"object":{"type":"tag","sha":"%s"}}\n' "${FAKE_TAG_OBJECT_TWO}"
    ;;
  nested:*/git/tags/${FAKE_TAG_OBJECT_TWO})
    printf '{"object":{"type":"commit","sha":"%s"}}\n' "${FAKE_TAG_COMMIT}"
    ;;
  cycle:*/git/ref/tags/v1.2.3|cycle:*/git/tags/*)
    printf '{"object":{"type":"tag","sha":"%s"}}\n' "${FAKE_TAG_OBJECT_ONE}"
    ;;
  *) exit 2 ;;
esac
"#,
    );
    let script = repo_root().join(".github/scripts/verify-tag-target.sh");
    let path = format!("{}:/usr/bin:/bin", bin_dir.display());
    let run = |mode: &str, expected_commit: &str| {
        Command::new("bash")
            .arg(&script)
            .args([
                "--repo",
                "refinedstone/akra",
                "--tag",
                "v1.2.3",
                "--expected-commit",
                expected_commit,
            ])
            .env("PATH", &path)
            .env("FAKE_TAG_MODE", mode)
            .env("FAKE_TAG_COMMIT", &commit_sha)
            .env("FAKE_TAG_OBJECT_ONE", &first_tag_sha)
            .env("FAKE_TAG_OBJECT_TWO", &second_tag_sha)
            .output()
            .expect("tag target verifier should run")
    };

    for mode in ["lightweight", "annotated", "nested"] {
        assert_success(&run(mode, &commit_sha), mode);
    }
    for (mode, expected_commit, expected_error) in [
        (
            "lightweight",
            "d".repeat(40),
            "tag target does not match the frozen workflow commit",
        ),
        (
            "missing",
            commit_sha.clone(),
            "failed to resolve the GitHub tag object",
        ),
        (
            "cycle",
            commit_sha.clone(),
            "annotated tag object cycle detected",
        ),
    ] {
        let output = run(mode, &expected_commit);
        assert!(!output.status.success(), "{mode} should fail closed");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(expected_error),
            "unexpected {mode} error: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fs::remove_dir_all(root).expect("tag target fixture should be removed");
}

#[test]
fn rust_toolchain_is_pinned_consistently_for_ci_and_releases() {
    let root = repo_root();
    let toolchain = fs::read_to_string(root.join("rust-toolchain.toml"))
        .expect("Rust toolchain policy should be readable");
    let native_checks = fs::read_to_string(root.join(".github/workflows/native-pr-checks.yml"))
        .expect("native checks workflow should be readable");
    let windows_portable = fs::read_to_string(root.join("scripts/check_windows_portable.sh"))
        .expect("Windows portable validation script should be readable");
    let release = fs::read_to_string(root.join(".github/workflows/release-native-assets.yml"))
        .expect("release workflow should be readable");

    assert!(toolchain.contains("channel = \"1.95.0\""));
    assert!(toolchain.contains("components = [\"clippy\", \"rustfmt\"]"));
    assert!(
        native_checks
            .contains("dtolnay/rust-toolchain@eac0f66a48bc4b70a10b9acd4c4e930f835d95ff # 1.95.0")
    );
    assert!(
        release
            .contains("dtolnay/rust-toolchain@eac0f66a48bc4b70a10b9acd4c4e930f835d95ff # 1.95.0")
    );
    for workflow in [&native_checks, &release] {
        assert!(!workflow.contains("dtolnay/rust-toolchain@stable"));
        assert_workflow_actions_are_commit_pinned(workflow);
    }
    assert_eq!(
        native_checks.matches("uses: actions/checkout@").count(),
        native_checks.matches("persist-credentials: false").count(),
        "every native-check checkout must avoid persisting the job token"
    );
    assert!(native_checks.contains("bash scripts/check_windows_portable.sh"));
    assert!(
        native_checks
            .contains("github.event_name == 'workflow_dispatch' && github.run_id || github.ref")
    );
    assert!(
        windows_portable
            .contains("actual_windows_environment_filter_accepts_mixed_case_allowlisted_keys")
    );
}

#[test]
fn github_release_assets_are_never_overwritten_with_different_bytes() {
    let root = make_records_dir();
    let bin_dir = root.join("bin");
    let local_dir = root.join("local");
    let remote_dir = root.join("remote");
    let call_log = root.join("gh.log");
    let release_title = root.join("release-title");
    let release_draft = root.join("release-draft");
    let download_count = root.join("download-count");
    let tag_check_count = root.join("tag-check-count");
    let tag_commit = "a".repeat(40);
    let moved_tag_commit = "b".repeat(40);
    fs::create_dir(&bin_dir).expect("fake gh bin dir should be created");
    fs::create_dir(&local_dir).expect("local asset dir should be created");
    fs::create_dir(&remote_dir).expect("remote asset dir should be created");
    fs::write(local_dir.join("akra.tar.gz"), "same archive")
        .expect("local archive should be written");
    fs::write(local_dir.join("akra.tar.gz.sha256"), "same checksum")
        .expect("local checksum should be written");
    fs::write(remote_dir.join("akra.tar.gz"), "same archive")
        .expect("remote archive should be written");
    fs::write(remote_dir.join("akra.tar.gz.sha256"), "same checksum")
        .expect("remote checksum should be written");
    fs::write(&release_title, "akra v1.2.3").expect("release title fixture should be written");
    fs::write(&release_draft, "false").expect("release draft fixture should be written");
    write_executable_file(
        &bin_dir.join("gh"),
        r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$*" >> "${FAKE_GH_LOG}"
if [[ "$1" == "api" ]]; then
  count=0
  if [[ -f "${FAKE_GH_TAG_CHECK_COUNT}" ]]; then
    count="$(cat "${FAKE_GH_TAG_CHECK_COUNT}")"
  fi
  count=$((count + 1))
  printf '%s' "${count}" > "${FAKE_GH_TAG_CHECK_COUNT}"
  resolved="${FAKE_GH_TAG_COMMIT}"
  if [[ "${FAKE_GH_TAG_SWAP_AFTER:-0}" -gt 0 && "${count}" -gt "${FAKE_GH_TAG_SWAP_AFTER}" ]]; then
    resolved="${FAKE_GH_MOVED_TAG_COMMIT}"
  fi
  printf '{"object":{"type":"commit","sha":"%s"}}\n' "${resolved}"
  exit 0
fi
test "$1" = "release"
command="$2"
shift 2
case "${command}" in
  view)
    case " $* " in
      *" --json tagName,name,isDraft,isPrerelease,assets "*)
        python3 - "${FAKE_GH_REMOTE}" "${FAKE_GH_RELEASE_TITLE}" "${FAKE_GH_RELEASE_DRAFT}" <<'PY'
import json
import pathlib
import sys

remote = pathlib.Path(sys.argv[1])
title = pathlib.Path(sys.argv[2]).read_text(encoding="utf-8")
is_draft = pathlib.Path(sys.argv[3]).read_text(encoding="utf-8") == "true"
assets = [{"name": path.name} for path in sorted(remote.iterdir()) if path.is_file()]
print(json.dumps({
    "tagName": "v1.2.3",
    "name": title,
    "isDraft": is_draft,
    "isPrerelease": False,
    "assets": assets,
}))
PY
        ;;
      *" --json url "*) printf '%s\n' 'https://github.example/release' ;;
      *) exit 0 ;;
    esac
    ;;
  download)
    pattern=""
    destination=""
    while (($# > 0)); do
      case "$1" in
        --pattern) pattern="$2"; shift 2 ;;
        --dir) destination="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    if [[ -n "${pattern}" ]]; then
      cp "${FAKE_GH_REMOTE}/${pattern}" "${destination}/${pattern}"
    else
      for remote_asset in "${FAKE_GH_REMOTE}"/*; do
        cp "${remote_asset}" "${destination}/$(basename "${remote_asset}")"
      done
    fi
    count=0
    if [[ -f "${FAKE_GH_DOWNLOAD_COUNT}" ]]; then
      count="$(cat "${FAKE_GH_DOWNLOAD_COUNT}")"
    fi
    count=$((count + 1))
    printf '%s' "${count}" > "${FAKE_GH_DOWNLOAD_COUNT}"
    if [[ "${FAKE_GH_REPLACE_AFTER_INITIAL_COMPARE:-0}" == "1" && "${count}" == "2" ]]; then
      printf '%s' 'replacement race' > "${FAKE_GH_REMOTE}/akra.tar.gz"
    fi
    ;;
  upload)
    shift
    while (($# > 0)); do
      case "$1" in
        --repo) shift 2 ;;
        *) cp "$1" "${FAKE_GH_REMOTE}/$(basename "$1")"; shift ;;
      esac
    done
    ;;
  create) exit 0 ;;
  *) exit 2 ;;
esac
"#,
    );

    let script = repo_root().join(".github/scripts/publish-release-assets.sh");
    let path = format!("{}:/usr/bin:/bin", bin_dir.display());
    let run = |replacement_race: bool, tag_swap_after: u64| {
        fs::write(&tag_check_count, "0").expect("tag check count should reset");
        Command::new("bash")
            .arg(&script)
            .args([
                "--tag",
                "v1.2.3",
                "--repo",
                "refinedstone/akra",
                "--expected-commit",
                tag_commit.as_str(),
                "--title",
                "akra v1.2.3",
                "--notes",
                "fixture",
                "--asset-dir",
            ])
            .arg(&local_dir)
            .env("PATH", &path)
            .env("FAKE_GH_LOG", &call_log)
            .env("FAKE_GH_REMOTE", &remote_dir)
            .env("FAKE_GH_RELEASE_TITLE", &release_title)
            .env("FAKE_GH_RELEASE_DRAFT", &release_draft)
            .env("FAKE_GH_DOWNLOAD_COUNT", &download_count)
            .env("FAKE_GH_TAG_CHECK_COUNT", &tag_check_count)
            .env("FAKE_GH_TAG_COMMIT", &tag_commit)
            .env("FAKE_GH_MOVED_TAG_COMMIT", &moved_tag_commit)
            .env("FAKE_GH_TAG_SWAP_AFTER", tag_swap_after.to_string())
            .env(
                "FAKE_GH_REPLACE_AFTER_INITIAL_COMPARE",
                if replacement_race { "1" } else { "0" },
            )
            .output()
            .expect("release asset publisher should run")
    };

    let matching = run(false, 0);
    assert_success(&matching, "matching existing GitHub assets");
    let matching_calls = fs::read_to_string(&call_log).expect("gh calls should be recorded");
    assert!(matching_calls.contains("release download"));
    assert!(!matching_calls.contains("release upload"));
    assert!(!matching_calls.contains("--clobber"));

    fs::write(remote_dir.join("akra.tar.gz"), "different archive")
        .expect("remote mismatch should be written");
    fs::write(&call_log, "").expect("gh call log should reset");
    let mismatched = run(false, 0);
    assert!(!mismatched.status.success());
    let stderr = String::from_utf8_lossy(&mismatched.stderr);
    assert!(stderr.contains("existing asset differs from local build: akra.tar.gz"));
    let mismatch_calls = fs::read_to_string(&call_log).expect("gh calls should be recorded");
    assert!(!mismatch_calls.contains("release upload"));
    assert!(!mismatch_calls.contains("--clobber"));

    fs::write(remote_dir.join("akra.tar.gz"), "same archive")
        .expect("matching remote archive should be restored");
    fs::remove_file(remote_dir.join("akra.tar.gz.sha256"))
        .expect("remote checksum should be removed");
    fs::write(&call_log, "").expect("gh call log should reset");
    let missing = run(false, 0);
    assert_success(&missing, "missing GitHub release asset upload");
    let missing_calls = fs::read_to_string(&call_log).expect("gh calls should be recorded");
    assert!(missing_calls.contains("release upload"));
    assert!(missing_calls.contains("akra.tar.gz.sha256"));
    assert!(!missing_calls.contains("--clobber"));

    fs::write(remote_dir.join("akra.tar.gz"), "same archive")
        .expect("matching remote archive should be restored before race");
    fs::write(remote_dir.join("akra.tar.gz.sha256"), "same checksum")
        .expect("matching remote checksum should be restored before race");
    fs::write(&download_count, "0").expect("download count should reset");
    fs::write(&call_log, "").expect("gh call log should reset");
    let replacement_race = run(true, 0);
    assert!(!replacement_race.status.success());
    assert!(
        String::from_utf8_lossy(&replacement_race.stderr)
            .contains("final release asset differs from local build: akra.tar.gz")
    );
    assert!(
        !fs::read_to_string(&call_log)
            .expect("gh race calls should be recorded")
            .contains("release upload")
    );
    fs::write(remote_dir.join("akra.tar.gz"), "same archive")
        .expect("remote archive should be restored after race");

    fs::write(remote_dir.join("unexpected-debug.zip"), "untrusted")
        .expect("unexpected remote asset should be written");
    fs::write(&call_log, "").expect("gh call log should reset");
    let unexpected = run(false, 0);
    assert!(!unexpected.status.success());
    assert!(
        String::from_utf8_lossy(&unexpected.stderr)
            .contains("existing release contains unexpected assets: unexpected-debug.zip")
    );
    let unexpected_calls = fs::read_to_string(&call_log).expect("gh calls should be recorded");
    assert!(!unexpected_calls.contains("release upload"));
    fs::remove_file(remote_dir.join("unexpected-debug.zip"))
        .expect("unexpected remote asset should be removed");

    fs::write(&release_title, "wrong release title")
        .expect("mismatched release title should be written");
    let metadata_mismatch = run(false, 0);
    assert!(!metadata_mismatch.status.success());
    assert!(
        String::from_utf8_lossy(&metadata_mismatch.stderr)
            .contains("existing release title metadata does not match")
    );

    fs::write(&release_title, "akra v1.2.3").expect("release title should be restored");
    fs::write(&release_draft, "true").expect("draft release state should be written");
    let draft_release = run(false, 0);
    assert!(!draft_release.status.success());
    assert!(
        String::from_utf8_lossy(&draft_release.stderr)
            .contains("existing release must be published and stable")
    );

    fs::write(&release_draft, "false").expect("release state should be restored");
    let moved_tag = run(false, 1);
    assert!(!moved_tag.status.success());
    assert!(
        String::from_utf8_lossy(&moved_tag.stderr)
            .contains("tag target does not match the frozen workflow commit")
    );
    assert!(
        !fs::read_to_string(&call_log)
            .expect("tag drift calls should be recorded")
            .contains("release upload"),
        "tag drift must stop before a new release mutation"
    );

    fs::remove_dir_all(root).expect("GitHub asset fixture should be removed");
}

#[test]
fn native_release_archives_are_reproducible_for_unix_and_windows_targets() {
    const SOURCE_DATE_EPOCH: u64 = 1_700_000_000;
    const VERSION: &str = "9.8.7";
    const TARGETS: [(&str, &str); 2] = [
        ("x86_64-unknown-linux-gnu", "codex-exec-loop-native"),
        ("x86_64-pc-windows-msvc", "codex-exec-loop-native.exe"),
    ];

    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    let home_dir = root.join("home");
    for directory in [
        repo.join("scripts"),
        repo.join("docs/plan"),
        repo.join("assets/app-server/skills/fixture"),
        repo.join("assets/admin/fonts"),
        repo.join("examples"),
        repo.join(".codex-exec-loop/followups"),
        bin_dir.clone(),
        home_dir.clone(),
    ] {
        fs::create_dir_all(directory).expect("release fixture directory should be created");
    }

    fs::copy(
        repo_root().join("scripts/package_native_release.sh"),
        repo.join("scripts/package_native_release.sh"),
    )
    .expect("packaging script should be copied into the fixture");
    let mut package_script_permissions =
        fs::metadata(repo.join("scripts/package_native_release.sh"))
            .expect("packaging script metadata should be readable")
            .permissions();
    package_script_permissions.set_mode(0o755);
    fs::set_permissions(
        repo.join("scripts/package_native_release.sh"),
        package_script_permissions,
    )
    .expect("packaging script should be executable");

    fs::write(
        repo.join("Cargo.toml"),
        format!(
            "[package]\nname = \"codex-exec-loop-native\"\nversion = \"{VERSION}\"\nedition = \"2024\"\n"
        ),
    )
    .expect("fixture manifest should be written");
    fs::write(repo.join("README.md"), "fixture readme\n")
        .expect("fixture readme should be written");
    fs::write(
        repo.join("docs/plan/13-native-packaging-and-operator-runbook.md"),
        "fixture operator runbook\n",
    )
    .expect("fixture runbook should be written");
    fs::write(repo.join("scripts/gh-akra.sh"), "#!/bin/sh\nexit 0\n")
        .expect("fixture runtime script should be written");
    fs::write(
        repo.join("assets/app-server/skills/fixture/SKILL.md"),
        "fixture skill\n",
    )
    .expect("fixture skill should be written");
    fs::write(
        repo.join("assets/admin/fonts/LICENSE-Galmuri.txt"),
        "fixture SIL Open Font License\n",
    )
    .expect("fixture font license should be written");
    fs::write(repo.join("examples/README.md"), "fixture example\n")
        .expect("fixture example should be written");
    fs::write(
        repo.join(".codex-exec-loop/followups/10-review-queue.md"),
        "fixture followup\n",
    )
    .expect("fixture followup should be written");

    assert_success(
        &Command::new("git")
            .arg("init")
            .arg(&repo)
            .output()
            .expect("git init should run"),
        "initialize release fixture repository",
    );
    assert_success(
        &run_git(
            &repo,
            &[
                "add",
                "Cargo.toml",
                "README.md",
                "docs",
                "scripts/gh-akra.sh",
                "assets",
                "examples",
                ".codex-exec-loop",
            ],
        ),
        "stage release fixture inputs",
    );

    write_executable_file(&bin_dir.join("cargo"), "#!/bin/sh\nexit 0\n");
    write_executable_file(
        &bin_dir.join("rustc"),
        "#!/bin/sh\nprintf 'rustc fixture\\nhost: x86_64-unknown-linux-gnu\\n'\n",
    );
    for (target, binary_name) in TARGETS {
        let binary_dir = repo.join("target").join(target).join("release");
        fs::create_dir_all(&binary_dir).expect("fixture target directory should be created");
        fs::write(
            binary_dir.join(binary_name),
            format!("fixture binary for {target}\n"),
        )
        .expect("fixture binary should be written");
    }

    let system_path = std::env::var("PATH").expect("test PATH should be configured");
    let fixture_path = format!("{}:{system_path}", bin_dir.display());
    let package_output = |target: &str, out_dir: &Path| {
        Command::new("bash")
            .arg(repo.join("scripts/package_native_release.sh"))
            .arg("--target")
            .arg(target)
            .arg("--out-dir")
            .arg(out_dir)
            .env("HOME", &home_dir)
            .env("PATH", &fixture_path)
            .env("SOURCE_DATE_EPOCH", SOURCE_DATE_EPOCH.to_string())
            .current_dir(&repo)
            .output()
            .expect("fixture packaging script should run")
    };
    let run_package = |target: &str, out_dir: &Path| {
        let output = package_output(target, out_dir);
        assert_success(
            &output,
            &format!("package deterministic archive for {target}"),
        );
    };

    struct FirstRun {
        target: &'static str,
        binary_name: &'static str,
        out_dir: PathBuf,
        package_name: String,
        archive: Vec<u8>,
        archive_checksum: Vec<u8>,
        bundle_checksum: Vec<u8>,
    }

    let mut first_runs = Vec::new();
    for (target, binary_name) in TARGETS {
        let out_dir = root.join(format!("out-{target}"));
        run_package(target, &out_dir);
        let package_name = format!("codex-exec-loop-native-{VERSION}-{target}");
        let archive_path = out_dir.join(format!("{package_name}.tar.gz"));
        first_runs.push(FirstRun {
            target,
            binary_name,
            archive: fs::read(&archive_path).expect("first release archive should be readable"),
            archive_checksum: fs::read(format!("{}.sha256", archive_path.display()))
                .expect("first archive checksum should be readable"),
            bundle_checksum: fs::read(out_dir.join(&package_name).join("SHA256SUMS.txt"))
                .expect("first bundle checksum should be readable"),
            out_dir,
            package_name,
        });
    }

    thread::sleep(Duration::from_millis(1_200));
    let mut readme_permissions = fs::metadata(repo.join("README.md"))
        .expect("fixture readme metadata should be readable")
        .permissions();
    readme_permissions.set_mode(0o600);
    fs::set_permissions(repo.join("README.md"), readme_permissions)
        .expect("fixture source mode should be changed between runs");

    for first in first_runs {
        run_package(first.target, &first.out_dir);
        let archive_path = first.out_dir.join(format!("{}.tar.gz", first.package_name));
        let bundle_dir = first.out_dir.join(&first.package_name);
        let expected_launcher = if first.target == "x86_64-pc-windows-msvc" {
            "akra.cmd"
        } else {
            "akra"
        };
        assert!(
            bundle_dir.join(expected_launcher).is_file(),
            "the target-specific launcher must be present for {}",
            first.target
        );
        let version_metadata = fs::read_to_string(bundle_dir.join("VERSION.txt"))
            .expect("bundle version metadata should be readable");
        assert!(
            version_metadata.contains(&format!("launcher={expected_launcher}\n")),
            "launcher metadata must name the target-specific launcher for {}",
            first.target
        );
        assert_eq!(
            fs::read(&archive_path).expect("second release archive should be readable"),
            first.archive,
            "same inputs must produce an identical archive for {}",
            first.target
        );
        assert_eq!(
            fs::read(format!("{}.sha256", archive_path.display()))
                .expect("second archive checksum should be readable"),
            first.archive_checksum,
            "archive checksum file must be stable for {}",
            first.target
        );
        assert_eq!(
            fs::read(bundle_dir.join("SHA256SUMS.txt"))
                .expect("second bundle checksum should be readable"),
            first.bundle_checksum,
            "bundle checksum file must be stable for {}",
            first.target
        );

        let entries = read_tar_entries(&archive_path);
        let names = entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>();
        let mut sorted_names = names.clone();
        sorted_names.sort();
        assert_eq!(names, sorted_names, "archive members must be sorted");
        let mut unique_names = names.clone();
        unique_names.dedup();
        assert_eq!(
            names.len(),
            unique_names.len(),
            "archive members must be unique"
        );

        let binary_path = format!("{}/{}", first.package_name, first.binary_name);
        let runtime_script_path = format!("{}/scripts/gh-akra.sh", first.package_name);
        let font_license_path =
            format!("{}/THIRD_PARTY_NOTICES/Galmuri-OFL.txt", first.package_name);
        let launcher_path = format!("{}/akra", first.package_name);
        assert!(
            names.iter().any(|name| name == &font_license_path),
            "embedded admin font license must ship in every native archive"
        );
        for entry in entries {
            assert_eq!(
                entry.uid, 0,
                "archive uid must be normalized: {}",
                entry.name
            );
            assert_eq!(
                entry.gid, 0,
                "archive gid must be normalized: {}",
                entry.name
            );
            assert_eq!(
                entry.mtime, SOURCE_DATE_EPOCH,
                "archive mtime must be normalized: {}",
                entry.name
            );
            let normalized_name = entry.name.trim_end_matches('/');
            let expected_mode = if entry.entry_type == b'5'
                || normalized_name == binary_path
                || normalized_name == runtime_script_path
                || (first.target != "x86_64-pc-windows-msvc" && normalized_name == launcher_path)
            {
                0o755
            } else {
                0o644
            };
            assert_eq!(
                entry.mode & 0o7777,
                expected_mode,
                "archive mode must be normalized: {}",
                entry.name
            );
        }

        let verification = Command::new("bash")
            .arg(repo_root().join("scripts/verify_native_release.sh"))
            .arg("--archive")
            .arg(&archive_path)
            .arg("--bundle-dir")
            .arg(&bundle_dir)
            .arg("--version")
            .arg(VERSION)
            .arg("--target")
            .arg(first.target)
            .arg("--profile")
            .arg("release")
            .output()
            .expect("release verification script should run");
        assert_success(
            &verification,
            &format!("verify deterministic archive for {}", first.target),
        );

        if first.target == "x86_64-unknown-linux-gnu" {
            let manifest_path = bundle_dir.join("SHA256SUMS.txt");
            let metadata_path = bundle_dir.join("VERSION.txt");
            let original_manifest = fs::read_to_string(&manifest_path)
                .expect("bundle checksum manifest should be readable");
            let original_metadata = fs::read_to_string(&metadata_path)
                .expect("bundle release metadata should be readable");
            let verify_bundle = || {
                Command::new("bash")
                    .arg(repo_root().join("scripts/verify_native_release.sh"))
                    .arg("--bundle-dir")
                    .arg(&bundle_dir)
                    .arg("--version")
                    .arg(VERSION)
                    .arg("--target")
                    .arg(first.target)
                    .arg("--profile")
                    .arg("release")
                    .output()
                    .expect("tampered bundle verification should run")
            };
            let assert_rejected = |label: &str, expected_error: &str| {
                let output = verify_bundle();
                assert!(
                    !output.status.success(),
                    "{label} must fail native bundle verification"
                );
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    stderr.contains(expected_error),
                    "{label} should report {expected_error:?}\nstderr:\n{stderr}"
                );
            };

            fs::write(
                &metadata_path,
                original_metadata.replace("version=9.8.7\n", "version=9.8.6\n"),
            )
            .expect("wrong version metadata should be written");
            assert_rejected("wrong VERSION.txt version", "version mismatch");
            fs::write(
                &metadata_path,
                original_metadata.replace(
                    "target=x86_64-unknown-linux-gnu\n",
                    "target=aarch64-apple-darwin\n",
                ),
            )
            .expect("platform-swapped metadata should be written");
            assert_rejected("platform-swapped VERSION.txt", "target mismatch");
            fs::write(&metadata_path, &original_metadata)
                .expect("release metadata should be restored");

            fs::write(&manifest_path, "").expect("empty checksum manifest should be written");
            assert_rejected("empty checksum manifest", "must be nonempty");

            let mut manifest_lines = original_manifest.lines();
            manifest_lines.next();
            let incomplete_manifest = manifest_lines.collect::<Vec<_>>().join("\n") + "\n";
            fs::write(&manifest_path, incomplete_manifest)
                .expect("incomplete checksum manifest should be written");
            assert_rejected(
                "incomplete checksum manifest",
                "cover every regular bundle file",
            );

            let first_manifest_line = original_manifest
                .lines()
                .next()
                .expect("checksum manifest should contain an entry");
            fs::write(
                &manifest_path,
                format!("{original_manifest}{first_manifest_line}\n"),
            )
            .expect("duplicate checksum manifest should be written");
            assert_rejected("duplicate checksum manifest", "repeats a path");

            fs::write(
                &manifest_path,
                format!("{original_manifest}{}  ../escape\n", "0".repeat(64)),
            )
            .expect("traversal checksum manifest should be written");
            assert_rejected("traversal checksum manifest", "unsafe path");

            let replacement = if original_manifest.starts_with('0') {
                "1"
            } else {
                "0"
            };
            let digest_mismatch = format!("{replacement}{}", &original_manifest[1..]);
            fs::write(&manifest_path, digest_mismatch)
                .expect("mismatched checksum manifest should be written");
            assert_rejected("mismatched checksum digest", "checksum mismatch");
            fs::write(&manifest_path, &original_manifest)
                .expect("checksum manifest should be restored");
        }
    }

    let package_name = format!("codex-exec-loop-native-{VERSION}-x86_64-unknown-linux-gnu");
    let unsafe_relative_path = "examples/line\n--checkpoint-action=exec=sh";
    fs::write(repo.join(unsafe_relative_path), "must not be archived\n")
        .expect("unsafe path fixture should be written");
    assert_success(
        &run_git(&repo, &["add", "--", unsafe_relative_path]),
        "stage unsafe archive member fixture",
    );
    let unsafe_out = root.join("out-unsafe-member");
    let unsafe_output = package_output("x86_64-unknown-linux-gnu", &unsafe_out);
    assert!(!unsafe_output.status.success());
    assert!(String::from_utf8_lossy(&unsafe_output.stderr).contains("unsupported bundle path"));
    assert!(
        !unsafe_out.join(format!("{package_name}.tar.gz")).exists(),
        "unsafe member names must fail before archive creation"
    );
    assert_success(
        &run_git(&repo, &["rm", "-f", "--", unsafe_relative_path]),
        "remove unsafe archive member fixture",
    );

    let victim_path = root.join("outside-secret.txt");
    let symlink_relative_path = "assets/app-server/skills/fixture/linked-secret.txt";
    fs::write(&victim_path, "outside victim contents\n")
        .expect("outside symlink victim should be written");
    symlink(&victim_path, repo.join(symlink_relative_path))
        .expect("tracked symlink fixture should be created");
    assert_success(
        &run_git(&repo, &["add", "--", symlink_relative_path]),
        "stage tracked symlink fixture",
    );
    let symlink_out = root.join("out-symlink-member");
    let symlink_output = package_output("x86_64-unknown-linux-gnu", &symlink_out);
    assert!(!symlink_output.status.success());
    assert!(
        String::from_utf8_lossy(&symlink_output.stderr)
            .contains("input must be a non-symlink regular file")
    );
    assert!(
        !symlink_out
            .join(&package_name)
            .join(symlink_relative_path)
            .exists(),
        "tracked symlink contents must never be copied into the bundle"
    );
    assert!(
        !symlink_out.join(format!("{package_name}.tar.gz")).exists(),
        "tracked symlinks must fail before archive creation"
    );

    fs::remove_dir_all(root).expect("release reproducibility fixture should be removed");
}

#[test]
fn bundled_operator_prompts_use_documented_guardrails_and_db_authority() {
    let root = repo_root();
    let examples_readme = fs::read_to_string(root.join("examples/README.md"))
        .expect("example documentation should be readable");
    for file_name in ["reviewable_task.en.txt", "reviewable_task.ko.txt"] {
        let prompt = fs::read_to_string(root.join("examples").join(file_name))
            .unwrap_or_else(|error| panic!("{file_name} should be readable: {error}"));
        assert!(examples_readme.contains(file_name));
        assert!(prompt.contains("# akra-example: reviewable-task"));
        assert!(prompt.contains("repository-instructions-required"));
        assert!(prompt.contains("remote-write-requires-verified-identity"));
        assert!(prompt.contains("AGENTS.md"));
    }

    let followup = fs::read_to_string(root.join(".codex-exec-loop/followups/10-review-queue.md"))
        .expect("queue review followup should be readable");
    assert!(followup.contains("[accepted-db-task-authority]"));
    assert!(followup.contains("[db-queue-projection]"));
    assert!(followup.contains("공식 Akra planning mutation"));
    assert!(!followup.contains("plan_priority_queue.md"));

    let package_script = fs::read_to_string(root.join("scripts/package_native_release.sh"))
        .expect("native packaging script should be readable");
    assert!(package_script.contains("copy_tracked_paths examples .codex-exec-loop/followups"));
}

#[test]
fn gemini_guidance_delegates_to_the_authoritative_delivery_contract() {
    let root = repo_root();
    let guidance = fs::read_to_string(root.join(".gemini/styleguide.md"))
        .expect("Gemini repository guidance should be readable");
    for contract in [
        "`AGENTS.md` is the authoritative instruction entrypoint",
        "origin/prerelease",
        "PR-to-`prerelease`",
        "rebase-merge",
        "worktree-cleanup",
        "Rust edition 2024",
        "pinned Rust 1.95.0 toolchain used by CI",
    ] {
        assert!(
            guidance.contains(contract),
            "Gemini guidance should retain repository contract {contract}"
        );
    }
    assert!(!guidance.contains("러스트 최신 문법"));

    let readme = fs::read_to_string(root.join("README.md")).expect("README should be readable");
    assert!(readme.contains("[.gemini/styleguide.md](.gemini/styleguide.md)"));
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
            ".github/scripts/publish-release-assets.sh",
            vec!["--tag"],
            "publish-release-assets: missing value for --tag",
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
fn gh_akra_auth_status_uses_explicit_token_without_scanning_username_profiles() {
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
        .env("AKRA_GITHUB_TOKEN", "akra-token-123")
        .env("GH_TOKEN", "")
        .env("GITHUB_TOKEN", "")
        .env_remove("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN")
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
fn gh_akra_auth_status_rejects_removed_legacy_file_scan_outside_repo() {
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
        .env("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", "1")
        .output()
        .expect("gh-akra auth status should run outside a repo");

    let _ = fs::remove_file(home_root.join(".git-credentials"));
    let _ = fs::remove_dir_all(&root);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN"));
    assert!(stderr.contains("no longer supported"));
    assert!(!stderr.contains("outside-home-token-123"));
}

#[test]
fn gh_akra_auth_status_does_not_scan_direct_credential_files_by_default() {
    let root = make_records_dir();
    let bin_dir = root.join("bin");
    let home_root = root.join("sensitive-home-path");
    fs::create_dir(&bin_dir).expect("bin fixture dir should be created");
    fs::create_dir(&home_root).expect("home fixture dir should be created");
    fs::write(
        home_root.join(".git-credentials"),
        "https://akra:direct-file-token-must-not-leak@github.com\n",
    )
    .expect("home git credential fixture should be written");
    write_executable_file(
        &bin_dir.join("gh"),
        r#"#!/bin/sh
set -eu
exit 1
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
        .env_remove("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN")
        .output()
        .expect("gh-akra auth status should run outside a repo");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("repository credential helpers"));
    assert!(stderr.contains("direct credential-file scanning"));
    assert!(!stderr.contains("direct-file-token-must-not-leak"));
    assert!(!stderr.contains("sensitive-home-path"));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn gh_akra_rejects_removed_legacy_scan_value_without_echoing_secrets() {
    let root = make_records_dir();
    let invalid_value = "invalid-secret-value-must-not-leak";
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .arg("auth")
        .arg("status")
        .current_dir(&root)
        .env("AKRA_GITHUB_TOKEN", "token-must-not-leak")
        .env("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN", invalid_value)
        .output()
        .expect("gh-akra should reject invalid legacy scan configuration");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN"));
    assert!(stderr.contains("no longer supported"));
    assert!(!stderr.contains(invalid_value));
    assert!(!stderr.contains("token-must-not-leak"));
    assert!(!stderr.contains(root.to_string_lossy().as_ref()));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn gh_akra_rejects_unsupported_remote_without_echoing_its_url() {
    let root = make_records_dir();
    let repo = root.join("repo");
    fs::create_dir(&repo).expect("repo fixture dir should be created");
    assert_success(
        &Command::new("git")
            .arg("init")
            .arg(&repo)
            .output()
            .expect("git init should run"),
        "git init",
    );
    let sensitive_remote =
        "https://remote-user:remote-token-must-not-leak@example.invalid/sensitive/path.git";
    assert_success(
        &run_git(&repo, &["remote", "add", "origin", sensitive_remote]),
        "configure unsupported remote",
    );

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .arg("auth")
        .arg("status")
        .current_dir(&repo)
        .env_remove("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN")
        .output()
        .expect("gh-akra should reject unsupported remote");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not use a supported GitHub URL"));
    assert!(!stderr.contains("remote-token-must-not-leak"));
    assert!(!stderr.contains("sensitive/path"));
    assert!(!stderr.contains(sensitive_remote));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn gh_akra_rejects_https_remote_with_embedded_identity_without_echoing_it() {
    let root = make_records_dir();
    let repo = root.join("repo");
    fs::create_dir(&repo).expect("repo fixture dir should be created");
    assert_success(
        &Command::new("git")
            .arg("init")
            .arg(&repo)
            .output()
            .expect("git init should run"),
        "git init",
    );
    let sensitive_remote =
        "https://embedded-user:embedded-token-must-not-leak@github.com/acme/widgets.git";
    assert_success(
        &run_git(&repo, &["remote", "add", "origin", sensitive_remote]),
        "configure embedded-identity remote",
    );

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .args(["auth", "write-status"])
        .current_dir(&repo)
        .env("AKRA_GITHUB_LOGIN", "embedded-user")
        .env("AKRA_GITHUB_TOKEN", "api-token-must-not-leak")
        .output()
        .expect("gh-akra should reject embedded HTTPS identity");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("must not embed a username or credential"));
    assert!(!stderr.contains("embedded-user"));
    assert!(!stderr.contains("embedded-token-must-not-leak"));
    assert!(!stderr.contains("api-token-must-not-leak"));
    assert!(!stderr.contains(sensitive_remote));
    fs::remove_dir_all(root).expect("embedded identity fixture should be removed");
}

#[test]
fn gh_akra_auth_status_never_executes_repo_scoped_git_credential_helper() {
    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    let home_root = root.join("home");
    fs::create_dir(&repo).expect("repo fixture dir should be created");
    fs::create_dir(&bin_dir).expect("bin fixture dir should be created");
    fs::create_dir(&home_root).expect("home fixture dir should be created");
    let helper_marker = root.join("repo-credential-helper-ran");

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
        &run_git(
            &repo,
            &[
                "config",
                "credential.helper",
                &format!(
                    "!f() {{ : > '{}'; printf 'username=akra\\npassword=path-token-123\\n'; }}; f",
                    helper_marker.display()
                ),
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

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("repository credential helpers"));
    assert!(!stderr.contains("path-token-123"));
    assert!(!helper_marker.exists(), "repository credential helper ran");
}

#[test]
fn gh_akra_git_boundary_ignores_routing_config_helper_and_trace_injection() {
    let root = make_records_dir();
    let repo = root.join("repo");
    let poison = root.join("poison");
    let bin_dir = root.join("bin");
    let home_root = root.join("home");
    for directory in [&repo, &poison, &bin_dir, &home_root] {
        fs::create_dir(directory).expect("git boundary fixture directory should be created");
    }
    for repository in [&repo, &poison] {
        assert_success(
            &Command::new("git")
                .arg("init")
                .arg(repository)
                .output()
                .expect("git init should run"),
            "git init",
        );
    }
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
        "configure protected origin",
    );
    assert_success(
        &run_git(
            &poison,
            &[
                "remote",
                "add",
                "origin",
                "https://attacker.invalid/poison.git",
            ],
        ),
        "configure poisoned origin",
    );
    link_path_command(&bin_dir, "git");

    let helper_marker = root.join("credential-helper-marker");
    let trace_marker = root.join("git-trace-marker");
    let injected_helper = format!(
        "!f() {{ touch '{}'; cat >/dev/null; }}; f",
        helper_marker.display()
    );
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .args(["auth", "status"])
        .current_dir(&repo)
        .env("PATH", format!("{}:/bin", bin_dir.display()))
        .env("HOME", &home_root)
        .env("USERPROFILE", "")
        .env("AKRA_GITHUB_TOKEN", "")
        .env("GH_TOKEN", "")
        .env("GITHUB_TOKEN", "")
        .env_remove("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN")
        .env("GIT_DIR", poison.join(".git"))
        .env("GIT_WORK_TREE", &poison)
        .env("GIT_INDEX_FILE", poison.join(".git/poison-index"))
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "credential.helper")
        .env("GIT_CONFIG_VALUE_0", injected_helper)
        .env("GIT_TRACE", &trace_marker)
        .output()
        .expect("gh-akra should reject missing credentials without trusting Git env");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("gh auth status requires a GitHub token"));
    assert!(!stderr.contains("unsupported GitHub URL"));
    assert!(
        !helper_marker.exists(),
        "injected credential helper must not run"
    );
    assert!(
        !trace_marker.exists(),
        "inherited Git trace target must not be written"
    );
    fs::remove_dir_all(root).expect("git boundary fixture should be removed");
}

#[test]
fn gh_akra_safe_git_stdin_mode_cannot_be_enabled_by_parent_environment() {
    let source = fs::read_to_string(repo_root().join("scripts/gh-akra.sh"))
        .expect("gh-akra source should be readable");
    assert!(!source.contains("AKRA_SAFE_GIT_PIPE_STDIN"));
    assert!(source.contains("safe_git_impl \"$@\""));
    assert!(!source.contains("safe_git_with_stdin"));
    assert!(!source.contains("credential fill"));
    assert!(source.contains("command git \"${hardening_args[@]}\" \"$@\" </dev/null"));
    assert!(
        !source.contains("show-error\\nlocation\\n"),
        "custom bearer headers must never cross a GitHub API redirect"
    );
    assert!(!source.contains("proto-redir"));
}

#[test]
fn gh_akra_api_response_tempfiles_are_bounded_validated_and_cleaned() {
    let root = make_records_dir();
    let bin_dir = root.join("bin");
    let temp_dir = root.join("tmp");
    fs::create_dir(&bin_dir).expect("response boundary bin should be created");
    fs::create_dir(&temp_dir).expect("response boundary tmp should be created");
    write_executable_file(&bin_dir.join("gh"), "#!/bin/sh\nset -eu\nexit 1\n");
    let run = || {
        Command::new("bash")
            .arg(repo_root().join("scripts/gh-akra.sh"))
            .args(["auth", "status"])
            .current_dir(&root)
            .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
            .env("TMPDIR", &temp_dir)
            .env("AKRA_GITHUB_TOKEN", "bounded-token-must-not-leak")
            .env_remove("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN")
            .output()
            .expect("gh-akra response boundary fixture should run")
    };

    write_executable_file(
        &bin_dir.join("curl"),
        r#"#!/bin/sh
set -eu
config=$(cat)
case "$config" in
  *'max-filesize = "8388608"'*) ;;
  *) exit 65 ;;
esac
output_file=$(printf '%s\n' "$config" | sed -n 's/^output = "\(.*\)"$/\1/p')
python3 - "$output_file" <<'PY'
import sys
with open(sys.argv[1], "wb") as stream:
    stream.write(b"x" * 8388609)
PY
printf '200'
"#,
    );
    let oversized = run();
    assert!(!oversized.status.success());
    let stderr = String::from_utf8_lossy(&oversized.stderr);
    assert!(stderr.contains("metadata bounds"));
    assert!(!stderr.contains("bounded-token-must-not-leak"));
    assert_eq!(
        fs::read_dir(&temp_dir)
            .expect("temp dir should be readable")
            .count(),
        0,
        "oversized response tempfile should be removed"
    );

    write_executable_file(
        &bin_dir.join("curl"),
        r#"#!/bin/sh
set -eu
config=$(cat)
output_file=$(printf '%s\n' "$config" | sed -n 's/^output = "\(.*\)"$/\1/p')
rm -f -- "$output_file"
ln -s /etc/passwd "$output_file"
printf '200'
"#,
    );
    let abnormal = run();
    assert!(!abnormal.status.success());
    let stderr = String::from_utf8_lossy(&abnormal.stderr);
    assert!(stderr.contains("not a regular file"));
    assert!(!stderr.contains("bounded-token-must-not-leak"));
    assert_eq!(
        fs::read_dir(&temp_dir)
            .expect("temp dir should be readable")
            .count(),
        0,
        "abnormal response tempfile should be removed"
    );
    fs::remove_dir_all(root).expect("response boundary fixture should be removed");
}

#[test]
fn gh_akra_write_status_supports_system_bash_and_requires_pinned_api_identity() {
    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    fs::create_dir(&repo).expect("write identity repo should be created");
    fs::create_dir(&bin_dir).expect("write identity bin should be created");
    let helper_marker = root.join("write-credential-helper-ran");
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
        "configure write identity remote",
    );
    assert_success(
        &run_git(&repo, &["config", "akra.githubLogin", "akra"]),
        "pin write identity",
    );
    assert_success(
        &run_git(
            &repo,
            &[
                "config",
                "credential.helper",
                &format!(
                    "!f() {{ : > '{}'; printf 'username=akra\\npassword=git-token\\n'; }}; f",
                    helper_marker.display()
                ),
            ],
        ),
        "configure write credential helper",
    );
    write_executable_file(
        &bin_dir.join("curl"),
        r#"#!/bin/sh
set -eu
config=$(cat)
output_file=$(printf '%s\n' "$config" | sed -n 's/^output = "\(.*\)"$/\1/p')
case "$config" in
  *'Authorization: Bearer api-token'*|*'Authorization: Bearer git-token'*)
    printf '{"login":"akra"}' > "$output_file"
    ;;
  *)
    printf '{"login":"other"}' > "$output_file"
    ;;
esac
printf '200'
"#,
    );

    let run = |api_token: &str| {
        Command::new("/bin/bash")
            .arg(repo_root().join("scripts/gh-akra.sh"))
            .args(["auth", "write-status"])
            .current_dir(&repo)
            .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
            .env("AKRA_GITHUB_TOKEN", api_token)
            .env("GH_TOKEN", "")
            .env("GITHUB_TOKEN", "")
            .env_remove("AKRA_GITHUB_LOGIN")
            .output()
            .expect("gh-akra write status should run")
    };

    assert_success(&run("api-token"), "matching GitHub write identities");
    assert!(!helper_marker.exists(), "source credential helper ran");

    assert_success(
        &run_git(&repo, &["config", "--unset", "akra.githubLogin"]),
        "remove write identity pin",
    );
    let missing_pin = run("api-token");
    assert!(!missing_pin.status.success());
    assert!(
        String::from_utf8_lossy(&missing_pin.stderr)
            .contains("GitHub writes require AKRA_GITHUB_LOGIN")
    );

    assert_success(
        &run_git(&repo, &["config", "akra.githubLogin", "akra"]),
        "restore write identity pin",
    );
    let wrong_api = run("wrong-api-token");
    assert!(!wrong_api.status.success());
    assert!(String::from_utf8_lossy(&wrong_api.stderr).contains("API token returned other"));

    assert_success(
        &run_git(
            &repo,
            &[
                "config",
                "credential.helper",
                &format!(
                    "!f() {{ : > '{}'; printf 'username=other\\npassword=git-token\\n'; }}; f",
                    helper_marker.display()
                ),
            ],
        ),
        "configure mismatched write credential username",
    );
    let wrong_git_username = run("api-token");
    assert_success(
        &wrong_git_username,
        "source credential helper must not affect API identity proof",
    );
    assert!(!helper_marker.exists(), "source credential helper ran");

    fs::remove_dir_all(root).expect("write identity fixture should be removed");
}

#[test]
fn gh_akra_does_not_fall_back_to_any_source_git_credential_helper() {
    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    fs::create_dir(&repo).expect("repo fixture dir should be created");
    fs::create_dir(&bin_dir).expect("bin fixture dir should be created");
    let helper_marker = root.join("host-wide-helper-ran");
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
        &run_git(
            &repo,
            &[
                "config",
                "credential.helper",
                &format!(
                    "!f() {{ : > '{}'; printf 'username=akra\\npassword=host-wide-token-must-not-leak\\n'; }}; f",
                    helper_marker.display()
                ),
            ],
        ),
        "configure host-wide-only credential helper",
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
printf '{"login":"akra"}' > "$output_file"
printf '200'
"#,
    );

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .arg("auth")
        .arg("status")
        .current_dir(&repo)
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("AKRA_GITHUB_TOKEN", "")
        .env("GH_TOKEN", "")
        .env("GITHUB_TOKEN", "")
        .env_remove("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN")
        .output()
        .expect("gh-akra auth status should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("repository credential helpers"));
    assert!(!stderr.contains("host-wide-token-must-not-leak"));
    assert!(!helper_marker.exists(), "source credential helper ran");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn gh_akra_review_reply_uses_scoped_endpoint_and_rejects_invalid_ids() {
    let source = fs::read_to_string(repo_root().join("scripts/gh-akra.sh"))
        .expect("gh-akra source should be readable");
    let api_endpoint =
        "\"/repos/${repo_full_name}/pulls/${pr_number}/comments/${comment_id}/replies\"";
    assert_eq!(source.matches(api_endpoint).count(), 1);
    assert!(!source.contains("pulls/comments/${comment_id}/replies"));
    assert!(!source.contains("reply_review_comment_with_gh"));
    assert!(!source.contains("-f \"body=${body}\""));

    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    let curl_log = root.join("curl.log");
    let body_file = root.join("reply.md");
    fs::create_dir(&repo).expect("review reply repo should be created");
    fs::create_dir(&bin_dir).expect("review reply bin should be created");
    fs::write(&body_file, "review \"quoted\" \\ path\nnext line\n")
        .expect("review reply body should be written");

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
        &run_git(&repo, &["config", "akra.githubLogin", "akra"]),
        "configure GitHub write identity",
    );

    for command in ["cat", "git", "mktemp", "python3", "rm", "sed"] {
        link_path_command(&bin_dir, command);
    }
    assert!(
        !bin_dir.join("gh").exists(),
        "the review reply fixture must use the direct API fallback"
    );
    write_executable_file(
        &bin_dir.join("curl"),
        r#"#!/bin/sh
set -eu
config=$(cat)
output_file=$(printf '%s\n' "$config" | sed -n 's/^output = "\(.*\)"$/\1/p')
url=$(printf '%s\n' "$config" | sed -n 's/^url = "\(.*\)"$/\1/p')
printf '%s\n' "$url" >> "${AKRA_GITHUB_CURL_LOG}"
case "$url" in
  'https://api.github.com/user')
    printf '{"login":"akra"}' > "$output_file"
    printf '200'
    ;;
  'https://api.github.com/repos/acme/widgets/pulls/42/comments/9001/replies')
    printf '%s' "$config" | python3 -c '
import json
import sys

values = {}
for line in sys.stdin.read().splitlines():
    if " = " in line:
        key, value = line.split(" = ", 1)
        values[key] = value
if json.loads(values["request"]) != "POST":
    raise SystemExit("review reply request method must be POST")
payload = json.loads(json.loads(values["data"]))
if payload != {"body": "review \"quoted\" \\ path\nnext line"}:
    raise SystemExit("review reply request body did not match the body file")
'
    printf '{}' > "$output_file"
    printf '201'
    ;;
  *)
    printf 'unexpected GitHub API URL: %s\n' "$url" >&2
    exit 64
    ;;
esac
"#,
    );

    let run_reply = |pr_number: &str, comment_id: &str| {
        Command::new("/bin/bash")
            .arg(repo_root().join("scripts/gh-akra.sh"))
            .args([
                "review-reply",
                "--pr",
                pr_number,
                "--comment-id",
                comment_id,
            ])
            .arg("--body-file")
            .arg(&body_file)
            .current_dir(&repo)
            .env("PATH", bin_dir.display().to_string())
            .env("PYTHONOPTIMIZE", "2")
            .env("AKRA_GITHUB_CURL_LOG", &curl_log)
            .env("AKRA_GITHUB_TOKEN", "fixture-token-must-not-leak")
            .env("GH_TOKEN", "")
            .env("GITHUB_TOKEN", "")
            .env_remove("AKRA_GITHUB_LOGIN")
            .output()
            .expect("gh-akra review reply should run")
    };

    let output = run_reply("42", "9001");

    assert_success(&output, "gh-akra review reply API fallback");
    let curl_calls = fs::read_to_string(&curl_log).expect("review reply URLs should be recorded");
    assert!(curl_calls.contains("https://api.github.com/user\n"));
    assert!(
        curl_calls
            .contains("https://api.github.com/repos/acme/widgets/pulls/42/comments/9001/replies\n")
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-token-must-not-leak"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture-token-must-not-leak"));

    for option in ["--pr", "--comment-id"] {
        for invalid_id in [
            "0", "042", "+42", "-42", " 42", "42 ", "../42", "42/7", "42?x=1",
        ] {
            let _ = fs::remove_file(&curl_log);
            let invalid_output = if option == "--pr" {
                run_reply(invalid_id, "9001")
            } else {
                run_reply("42", invalid_id)
            };
            assert!(
                !invalid_output.status.success(),
                "{option} value {invalid_id:?} must be rejected"
            );
            let stderr = String::from_utf8_lossy(&invalid_output.stderr);
            assert!(
                stderr.contains(&format!(
                    "gh-akra: review-reply {option} must be a positive decimal integer"
                )),
                "unexpected validation error for {option} value {invalid_id:?}: {stderr}"
            );
            assert!(
                !curl_log.exists(),
                "invalid {option} value {invalid_id:?} must fail before curl"
            );
        }
    }

    let run_mixed_body_options = |body_first: bool| {
        let mut command = Command::new("/bin/bash");
        command.arg(repo_root().join("scripts/gh-akra.sh")).args([
            "review-reply",
            "--pr",
            "42",
            "--comment-id",
            "9001",
        ]);
        if body_first {
            command.args(["--body", "argv-secret-must-not-leak", "--body-file"]);
            command.arg(&body_file);
        } else {
            command
                .arg("--body-file")
                .arg(&body_file)
                .args(["--body", "argv-secret-must-not-leak"]);
        }
        command
            .current_dir(&repo)
            .env("PATH", bin_dir.display().to_string())
            .env("AKRA_GITHUB_CURL_LOG", &curl_log)
            .env("AKRA_GITHUB_TOKEN", "fixture-token-must-not-leak")
            .env("GH_TOKEN", "")
            .env("GITHUB_TOKEN", "")
            .env_remove("AKRA_GITHUB_LOGIN")
            .output()
            .expect("mixed review reply body options should run")
    };
    for body_first in [true, false] {
        let _ = fs::remove_file(&curl_log);
        let mixed_output = run_mixed_body_options(body_first);
        assert!(!mixed_output.status.success());
        let stderr = String::from_utf8_lossy(&mixed_output.stderr);
        assert!(stderr.contains(
            "gh-akra: review-reply accepts only --body-file for privacy-safe invocation"
        ));
        assert!(!stderr.contains("argv-secret-must-not-leak"));
        assert!(
            !String::from_utf8_lossy(&mixed_output.stdout).contains("argv-secret-must-not-leak")
        );
        assert!(
            !curl_log.exists(),
            "mixed body options must fail before curl"
        );
    }

    let _ = fs::remove_file(&curl_log);
    let equals_body_output = Command::new("/bin/bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .args(["review-reply", "--pr", "42", "--comment-id", "9001"])
        .arg("--body-file")
        .arg(&body_file)
        .arg("--body=argv-secret-must-not-leak")
        .current_dir(&repo)
        .env("PATH", bin_dir.display().to_string())
        .env("AKRA_GITHUB_CURL_LOG", &curl_log)
        .env("AKRA_GITHUB_TOKEN", "fixture-token-must-not-leak")
        .env("GH_TOKEN", "")
        .env("GITHUB_TOKEN", "")
        .env_remove("AKRA_GITHUB_LOGIN")
        .output()
        .expect("equals review reply body option should run");
    assert!(!equals_body_output.status.success());
    let stderr = String::from_utf8_lossy(&equals_body_output.stderr);
    assert!(
        stderr
            .contains("gh-akra: review-reply accepts only --body-file for privacy-safe invocation")
    );
    assert!(!stderr.contains("argv-secret-must-not-leak"));
    assert!(
        !String::from_utf8_lossy(&equals_body_output.stdout).contains("argv-secret-must-not-leak")
    );
    assert!(
        !curl_log.exists(),
        "equals body option must fail before curl"
    );

    fs::remove_dir_all(root).expect("review reply fixture should be removed");
}

#[test]
fn gh_akra_api_fallback_enriches_required_pull_request_gate_fields_without_gh() {
    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    let curl_log = root.join("curl.log");
    fs::create_dir(&repo).expect("repo fixture dir should be created");
    fs::create_dir(&bin_dir).expect("bin fixture dir should be created");

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

    for command in ["cat", "git", "mktemp", "python3", "rm"] {
        link_path_command(&bin_dir, command);
    }
    assert!(
        !bin_dir.join("gh").exists(),
        "the fallback fixture must not expose a gh executable"
    );
    write_executable_file(
        &bin_dir.join("curl"),
        r#"#!/bin/bash
set -euo pipefail
config="$(cat)"
output_file=""
url=""
while IFS= read -r line; do
  case "$line" in
    'output = "'*)
      output_file="${line#output = \"}"
      output_file="${output_file%\"}"
      ;;
    'url = "'*)
      url="${line#url = \"}"
      url="${url%\"}"
      ;;
  esac
done <<< "$config"

printf '%s\n' "$url" >> "${AKRA_GITHUB_CURL_LOG}"
case "$url" in
  'https://api.github.com/repos/acme/widgets/pulls?state=open&base=prerelease&head=acme%3Afeature%2Fready')
    printf '%s' '[{"node_id":"PR_node_42","number":42,"html_url":"https://github.com/acme/widgets/pull/42","title":"Ready change","state":"open","merged_at":null,"base":{"ref":"prerelease"},"head":{"ref":"feature/ready"},"draft":false}]' > "$output_file"
    ;;
  'https://api.github.com/repos/acme/widgets/pulls?state=open&base=prerelease&head=acme%3Afeature%2Fmissing')
    printf '%s' '[]' > "$output_file"
    ;;
  'https://api.github.com/repos/acme/widgets/pulls/42')
    printf '%s' '{"node_id":"PR_node_42","number":42,"html_url":"https://github.com/acme/widgets/pull/42","title":"Ready change","state":"open","merged_at":null,"base":{"ref":"prerelease"},"head":{"ref":"feature/ready"},"draft":false}' > "$output_file"
    ;;
  'https://api.github.com/graphql')
    case "$config" in
      *PR_node_42*) ;;
      *)
        printf '%s\n' 'GraphQL request omitted the pull request node id' >&2
        exit 65
        ;;
    esac
    printf '%s' '{"data":{"nodes":[{"number":42,"headRefOid":"deadbeef42","reviewDecision":"APPROVED","mergeStateStatus":"CLEAN","commits":{"nodes":[{"commit":{"statusCheckRollup":{"contexts":{"nodes":[{"__typename":"CheckRun","conclusion":"SUCCESS"},{"__typename":"StatusContext","state":"SUCCESS"}],"pageInfo":{"hasNextPage":false}}}}}]},"reviews":{"nodes":[{"state":"APPROVED","commit":{"oid":"deadbeef42"}}],"pageInfo":{"hasNextPage":false}}}]}}' > "$output_file"
    ;;
  *)
    printf 'unexpected GitHub API URL: %s\n' "$url" >&2
    exit 64
    ;;
esac
printf '%s' '200'
"#,
    );

    let fields = "number,url,state,baseRefName,headRefName,headRefOid,isDraft,reviewDecision,mergeStateStatus,statusCheckRollup,approvedReviewCommitOids";
    let path = bin_dir.display().to_string();
    let run_fallback = |args: &[&str]| {
        Command::new("/bin/bash")
            .arg(repo_root().join("scripts/gh-akra.sh"))
            .args(args)
            .current_dir(&repo)
            .env("PATH", &path)
            .env("AKRA_GITHUB_CURL_LOG", &curl_log)
            .env("AKRA_GITHUB_TOKEN", "fixture-token-must-not-leak")
            .env("GH_TOKEN", "")
            .env("GITHUB_TOKEN", "")
            .env_remove("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN")
            .output()
            .expect("gh-akra API fallback should run")
    };

    let list_output = run_fallback(&[
        "pr",
        "list",
        "--state",
        "open",
        "--base",
        "prerelease",
        "--head",
        "feature/ready",
        "--json",
        fields,
    ]);
    assert_success(&list_output, "gh-akra PR list API fallback");
    let list: serde_json::Value =
        serde_json::from_slice(&list_output.stdout).expect("PR list JSON should parse");
    let listed = &list[0];
    assert_eq!(listed["headRefOid"], "deadbeef42");
    assert_eq!(listed["reviewDecision"], "APPROVED");
    assert_eq!(listed["mergeStateStatus"], "CLEAN");
    assert_eq!(listed["statusCheckRollup"][0]["conclusion"], "SUCCESS");
    assert_eq!(listed["statusCheckRollup"][1]["state"], "SUCCESS");
    assert_eq!(listed["approvedReviewCommitOids"][0], "deadbeef42");

    let missing_output = run_fallback(&[
        "pr",
        "list",
        "--state",
        "open",
        "--base",
        "prerelease",
        "--head",
        "feature/missing",
        "--json",
        fields,
    ]);
    assert_success(&missing_output, "gh-akra missing PR API fallback");
    let missing: serde_json::Value =
        serde_json::from_slice(&missing_output.stdout).expect("missing PR JSON should parse");
    assert_eq!(missing, serde_json::json!([]));

    let view_output = run_fallback(&["pr", "view", "42", "--json", fields]);
    assert_success(&view_output, "gh-akra PR view API fallback");
    let viewed: serde_json::Value =
        serde_json::from_slice(&view_output.stdout).expect("PR view JSON should parse");
    assert_eq!(viewed["headRefOid"], "deadbeef42");
    assert_eq!(viewed["reviewDecision"], "APPROVED");
    assert_eq!(viewed["mergeStateStatus"], "CLEAN");
    assert_eq!(
        viewed["statusCheckRollup"].as_array().map(Vec::len),
        Some(2)
    );

    let curl_calls = fs::read_to_string(&curl_log).expect("fake curl calls should be recorded");
    assert!(curl_calls.contains("/pulls?state=open&base=prerelease"));
    assert!(curl_calls.contains("/pulls/42"));
    assert_eq!(
        curl_calls.matches("https://api.github.com/graphql").count(),
        2
    );
    for output in [&list_output, &view_output] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains("fixture-token-must-not-leak"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture-token-must-not-leak"));
    }

    fs::remove_dir_all(root).expect("GitHub API fallback fixture should be removed");
}

#[test]
fn gh_akra_api_fallback_ignores_poisoned_home_curl_config() {
    let root = make_records_dir();
    let repo = root.join("repo");
    let bin_dir = root.join("bin");
    let home = root.join("home");
    let trace_path = root.join("poison-trace.log");
    let output_path = root.join("poison-output.log");
    let proxy_contact_path = root.join("poison-proxy-contact.log");
    fs::create_dir(&repo).expect("repo fixture dir should be created");
    fs::create_dir(&bin_dir).expect("bin fixture dir should be created");
    fs::create_dir(&home).expect("home fixture dir should be created");
    fs::write(
        home.join(".curlrc"),
        format!(
            "trace = \"{}\"\noutput = \"{}\"\nproxy = \"http://127.0.0.1:9\"\n",
            trace_path.display(),
            output_path.display(),
        ),
    )
    .expect("poisoned curlrc should be written");

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
    for command in ["cat", "git", "mktemp", "python3", "rm", "sed"] {
        link_path_command(&bin_dir, command);
    }
    write_executable_file(
        &bin_dir.join("gh"),
        r#"#!/bin/sh
set -eu
exit 1
"#,
    );
    write_executable_file(
        &bin_dir.join("curl"),
        &format!(
            r#"#!/bin/sh
set -eu
if [ "${{1-}}" != "-q" ] && [ -f "$HOME/.curlrc" ]; then
  cat > "{trace_path}"
  printf 'poisoned output\n' > "{output_path}"
  printf 'poisoned proxy contacted\n' > "{proxy_contact_path}"
  exit 65
fi
config=$(cat)
output_file=$(printf '%s\n' "$config" | sed -n 's/^output = "\(.*\)"$/\1/p')
printf '{{"login":"akra"}}' > "$output_file"
printf '200'
"#,
            trace_path = trace_path.display(),
            output_path = output_path.display(),
            proxy_contact_path = proxy_contact_path.display(),
        ),
    );

    let output = Command::new("bash")
        .arg(repo_root().join("scripts/gh-akra.sh"))
        .arg("auth")
        .arg("status")
        .current_dir(&repo)
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("HOME", &home)
        .env("AKRA_GITHUB_TOKEN", "bearer-token-must-not-leak")
        .env("GH_TOKEN", "")
        .env("GITHUB_TOKEN", "")
        .env_remove("AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN")
        .output()
        .expect("gh-akra auth status should run");

    assert_success(&output, "gh-akra poisoned curlrc isolation");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Logged in to github.com as akra"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("bearer-token-must-not-leak"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("bearer-token-must-not-leak"));
    for leak_path in [&trace_path, &output_path, &proxy_contact_path] {
        assert!(
            !leak_path.exists(),
            "curl user configuration must not create {}",
            leak_path.display()
        );
    }
    fs::remove_dir_all(root).expect("poisoned curlrc fixture should be removed");
}

#[test]
fn admin_graphic_visual_script_ignores_poisoned_home_curl_config() {
    let root = make_records_dir();
    let bin_dir = root.join("bin");
    let home = root.join("home");
    let trace_path = root.join("poison-trace.log");
    let output_path = root.join("poison-output.log");
    let proxy_contact_path = root.join("poison-proxy-contact.log");
    fs::create_dir(&bin_dir).expect("visual curl fixture bin dir should be created");
    fs::create_dir(&home).expect("visual curl fixture home should be created");
    fs::write(
        home.join(".curlrc"),
        format!(
            "trace = \"{}\"\noutput = \"{}\"\nproxy = \"http://127.0.0.1:9\"\n",
            trace_path.display(),
            output_path.display(),
        ),
    )
    .expect("poisoned curlrc should be written");
    write_executable_file(
        &bin_dir.join("curl"),
        &format!(
            r#"#!/bin/sh
set -eu
if [ "${{1-}}" != "-q" ] && [ -f "$HOME/.curlrc" ]; then
  printf 'poisoned trace\n' > "{trace_path}"
  printf 'poisoned output\n' > "{output_path}"
  printf 'poisoned proxy contacted\n' > "{proxy_contact_path}"
  exit 65
fi
printf 'curl fixture\n'
"#,
            trace_path = trace_path.display(),
            output_path = output_path.display(),
            proxy_contact_path = proxy_contact_path.display(),
        ),
    );

    let script_path = repo_root().join("scripts/check_admin_graphic_visual.sh");
    let script =
        fs::read_to_string(&script_path).expect("admin graphic visual script should be readable");
    assert!(script.contains("command curl -q \"$@\""));
    assert!(script.contains("--curl-config-probe"));
    assert!(script.contains("randomBytes(32).toString(\"hex\")"));
    assert!(!script.to_ascii_lowercase().contains("firefox"));
    for line in script.lines() {
        let trimmed = line.trim_start();
        assert!(
            !trimmed.starts_with("curl "),
            "visual validation must route every curl call through curl_no_config: {line}"
        );
        if trimmed.starts_with("command curl ") {
            assert!(
                matches!(
                    trimmed,
                    "command curl -q \"$@\""
                        | "command curl -q --resolve \"${admin_host}:${port}:127.0.0.1\" \"$@\""
                ),
                "visual validation curl wrapper must disable config before fixed admin host resolution: {line}"
            );
        }
    }

    let output = Command::new("bash")
        .arg(script_path)
        .arg("--curl-config-probe")
        .current_dir(repo_root())
        .env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()))
        .env("HOME", &home)
        .output()
        .expect("admin graphic curl isolation probe should run");

    assert_success(&output, "admin graphic poisoned curlrc isolation");
    for leak_path in [&trace_path, &output_path, &proxy_contact_path] {
        assert!(
            !leak_path.exists(),
            "visual curl isolation must not create {}",
            leak_path.display()
        );
    }
    fs::remove_dir_all(root).expect("admin graphic curl fixture should be removed");
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
frontend: fullscreen
term: tmux-256color
capture_role: counted-row
check_profile: prompt-input-delay-pty
checks:
- launch fullscreen TUI in tmux PTY
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
frontend: fullscreen
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
    assert!(output.contains("Linux / tmux detached PTY / bash / fullscreen"));
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
frontend: fullscreen
term: tmux-256color
capture_role: counted-row
check_profile: prompt-input-delay-pty
checks:
- launch fullscreen TUI in tmux PTY
result: pass
notes: prompt echo under budget
"#,
    );
    write_record(
        &records_dir,
        "baseline-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: Ubuntu 24.04.2 LTS
terminal: Linux terminal
shell: bash
frontend: fullscreen
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
os: Ubuntu 24.04.2 LTS
terminal: Linux terminal
shell: bash
frontend: fullscreen
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
    assert!(
        !output.contains(
            "Linux / direct terminal / bash / fullscreen (supplemental-terminal-app.txt)"
        )
    );

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
os: Ubuntu 24.04.2 LTS
terminal: Linux terminal
shell: bash
frontend: fullscreen
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
os: Ubuntu 24.04.2 LTS
terminal: Linux terminal
shell: bash
frontend: fullscreen
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
os: Ubuntu 24.04.2 LTS
terminal: Linux terminal
shell: bash
frontend: fullscreen
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
        "date: 2026-05-09\r\ncommit: abc123\r\nos: Ubuntu 24.04.2 LTS\r\nterminal: Linux terminal\r\nshell: bash\r\nfrontend: fullscreen\r\nterm: xterm-256color\r\ncapture_role: counted-row\r\ncheck_profile: terminal-baseline\r\nchecks:\r\n- launch and exit\r\nresult: pass\r\nnotes: crlf baseline row\r\n",
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
os: Ubuntu 24.04.2 LTS
terminal: Linux terminal
shell: bash
frontend: fullscreen
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
os: Ubuntu 24.04.2 LTS
terminal: Linux terminal
shell: bash
frontend: fullscreen
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
