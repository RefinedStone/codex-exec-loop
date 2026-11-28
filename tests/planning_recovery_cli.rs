use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use akra_verification_native::git_command;

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("failed to remove recovery fixture: {error}");
        }
    }
}

fn init_checkout(workspace: &Path, branch: &str) {
    let git = git_command::command(["init", "--quiet", "--initial-branch", branch])
        .env("GIT_MASTER", "1")
        .current_dir(workspace)
        .output()
        .expect("run git init");
    assert!(git.status.success(), "{git:?}");
}

#[test]
fn occupied_published_namespace_is_reported_without_initializing_authority() {
    let root = std::env::temp_dir()
        .canonicalize()
        .expect("physical temporary directory")
        .join(format!("akra-planning-recovery-cli-{}", std::process::id()));
    fs::create_dir(&root).expect("unique temporary directory");
    let fixture = Fixture(root);
    let workspace = fixture.0.join("Published Workspace");
    fs::create_dir(&workspace).expect("create fixture checkout");
    init_checkout(&workspace, "original");
    let workspace = workspace.canonicalize().expect("canonical checkout path");
    // Independent reproduction of the v1.3.5 path fixture, including its unsanitized label.
    let hash = workspace
        .to_str()
        .expect("UTF-8 checkout path")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    let profile = fixture.0.join("profile");
    let projects = profile.join("projects");
    let legacy = projects.join(format!("Published Workspace-{:012x}", hash >> 16));
    let runtime = legacy.join("runtime");
    fs::create_dir_all(&runtime).expect("create historical namespace");
    let database = runtime.join("planning-authority.db");
    let sentinel = b"metadata probe must not open this as SQLite";
    fs::write(&database, sentinel).expect("write disposable test bytes");
    fs::rename(workspace.join(".git"), fixture.0.join("original-git"))
        .expect("move original Git metadata away from the checkout");
    init_checkout(&workspace, "replacement");

    let output = Command::new(env!("CARGO_BIN_EXE_akra"))
        .args(["planning-recovery", "diagnose"])
        .current_dir(&workspace)
        .env("AKRA_HOME", &profile)
        .output()
        .expect("run recovery CLI");

    assert!(output.status.success(), "{output:?}");
    let report = String::from_utf8(output.stdout).expect("UTF-8 diagnostics");
    assert_eq!(report.lines().next(), Some("legacy_namespace: occupied"));
    assert_eq!(fs::read(database).expect("legacy bytes"), sentinel);
    assert_eq!(fs::read_dir(projects).expect("project entries").count(), 1);
    assert_eq!(fs::read_dir(runtime).expect("runtime entries").count(), 1);
    assert!(!profile.join("config.toml").exists());
    assert!(!workspace.join(".git").join("akra-repository-id").exists());
}
