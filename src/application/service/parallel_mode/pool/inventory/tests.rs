use super::*;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::application::service::parallel_mode::pool::{
    PoolSlotCleanupIdentity, derive_default_pool_root,
};
use std::path::PathBuf;
use std::sync::Mutex;

struct Runtime {
    guard_error: Option<String>,
    output: Result<ParallelCommandOutput, String>,
    calls: Mutex<Vec<Vec<OsString>>>,
}

impl Runtime {
    fn new(output: Result<ParallelCommandOutput, String>) -> Self {
        Self {
            guard_error: None,
            output,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl ParallelModeRuntimePort for Runtime {
    fn ensure_git_execution_safe(&self, _: &Path) -> Result<(), String> {
        self.guard_error.clone().map_or(Ok(()), Err)
    }
    fn run_git_command(
        &self,
        args: &[OsString],
        stdin: Option<&[u8]>,
    ) -> Result<ParallelCommandOutput, String> {
        assert!(stdin.is_none());
        assert_eq!(
            args.iter().skip(2).cloned().collect::<Vec<_>>(),
            ["worktree", "list", "--porcelain"].map(OsString::from)
        );
        self.calls.lock().unwrap().push(args.to_vec());
        self.output.clone()
    }
    fn canonicalize_path(&self, path: &Path) -> std::io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
    fn path_is_symlink(&self, _: &Path) -> std::io::Result<bool> {
        Ok(false)
    }
    fn detect_git_repo_root(&self, _: &str) -> Option<String> {
        panic!("unexpected repository probe")
    }
    fn command_succeeds(&self, _: &str, _: &[&str]) -> bool {
        panic!("unexpected command")
    }
    fn run_command(&self, _: &str, _: &[&str], _: Option<&str>) -> Option<String> {
        panic!("unexpected command")
    }
    fn run_command_with_stdin(&self, _: &str, _: &[&str], _: &str) -> Option<String> {
        panic!("unexpected command")
    }
    fn find_executable(&self, _: &str) -> Option<PathBuf> {
        panic!("unexpected executable probe")
    }
    fn gh_auth_status(&self, _: Option<&str>) -> bool {
        panic!("unexpected remote probe")
    }
    fn current_timestamp(&self) -> String {
        panic!("unexpected timestamp")
    }
    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        path.to_path_buf()
    }
    fn path_exists(&self, _: &Path) -> bool {
        panic!("unexpected path probe")
    }
    fn ensure_directory_exists(&self, _: &Path) -> std::io::Result<()> {
        panic!("unexpected filesystem mutation")
    }
    fn write_runtime_mirror_atomic(&self, _: &Path, _: &Path, _: &str) -> std::io::Result<()> {
        panic!("unexpected mirror write")
    }
    fn read_runtime_mirror_optional(&self, _: &Path, _: &Path) -> std::io::Result<Option<String>> {
        panic!("unexpected mirror read")
    }
    fn read_runtime_mirror_directory(
        &self,
        _: &Path,
        _: &Path,
    ) -> std::io::Result<Vec<(PathBuf, String)>> {
        panic!("unexpected mirror read")
    }
    fn remove_runtime_mirror_file(&self, _: &Path, _: &Path) -> std::io::Result<()> {
        panic!("unexpected mirror removal")
    }
}

fn output(exit_code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> ParallelCommandOutput {
    ParallelCommandOutput {
        exit_code,
        stdout: stdout.to_vec(),
        stderr: stderr.to_vec(),
    }
}

#[test]
fn inventory_failures_retain_kind_and_block_cleanup_registration() {
    let cases = [
        (
            FailureKind::ConfigurationRejected,
            Runtime {
                guard_error: Some("unsafe config token=do-not-log".into()),
                ..Runtime::new(Ok(output(Some(0), b"worktree /fixture\n", b"")))
            },
        ),
        (
            FailureKind::ExecutionFailed,
            Runtime::new(Err("spawn failed secret=do-not-log".into())),
        ),
        (
            FailureKind::TimedOut,
            Runtime::new(Err(
                "command `parallel runtime git command` timed out after 30s".into(),
            )),
        ),
        (
            FailureKind::NonzeroExit,
            Runtime::new(Ok(output(
                Some(128),
                b"",
                b"fatal: not a git repository: secret=do-not-log",
            ))),
        ),
        (
            FailureKind::NonzeroExit,
            Runtime::new(Ok(output(None, b"", b"signal termination"))),
        ),
        (
            FailureKind::InvalidUtf8,
            Runtime::new(Ok(output(Some(0), &[0xff], b""))),
        ),
        (
            FailureKind::EmptyOutput,
            Runtime::new(Ok(output(Some(0), b" \n", b""))),
        ),
        (
            FailureKind::MalformedOutput,
            Runtime::new(Ok(output(Some(0), b"not porcelain\n", b""))),
        ),
    ];
    let repo_root = std::env::temp_dir().join("inventory-contract-only");
    let repo_text = repo_root.to_string_lossy();
    let pool_root = derive_default_pool_root(&repo_root);
    let slot_path = pool_root.join("slot-1");
    let identity = PoolSlotCleanupIdentity::new(
        &repo_text,
        &repo_root,
        &pool_root,
        "slot-1",
        &slot_path,
        "akra-agent/slot-1/fixture",
    );
    for (kind, runtime) in cases {
        let failure = load(&runtime, &repo_text).unwrap_err();
        assert_eq!(failure.kind, kind);
        let error = identity.validate(&runtime).unwrap_err();
        assert!(error.starts_with("git worktree inventory could not be loaded for cleanup"));
        assert!(error.contains(kind.code()), "{error}");
        assert!(!error.contains("do-not-log"));
        assert!(!format!("{failure:?}").contains("do-not-log"));
        let expected_calls = if kind == FailureKind::ConfigurationRejected {
            0
        } else {
            2
        };
        assert_eq!(runtime.calls.lock().unwrap().len(), expected_calls);
    }
}

#[test]
fn inventory_exit_code_and_bounded_diagnostic_exclude_arbitrary_stderr() {
    let secret = "https://user:secret@example.invalid/repo ghp_do_not_log";
    let failure = decode(output(
        Some(128),
        b"",
        format!("fatal: permission denied: {secret}").as_bytes(),
    ))
    .unwrap_err();
    assert_eq!(failure.exit_code, Some(128));
    assert_eq!(failure.diagnostic, "permission_denied");
    assert!(failure.to_string().contains("exit_code=128"));
    assert!(!format!("{failure:?}").contains(secret));
    let mut overlong = vec![b'x'; 4096];
    overlong.extend_from_slice(b"permission denied secret");
    assert_eq!(safe_diagnostic_category(&overlong), "redacted");
}

#[test]
fn inventory_reads_a_real_isolated_git_repository() {
    let root = std::env::temp_dir().join(format!(
        "akra-inventory-contract-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let initialized = crate::test_utils::git_command()
        .args(["init", "-q"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(initialized.status.success());
    let records = load(
        &GitParallelModeRuntimeAdapter::new(),
        &root.to_string_lossy(),
    )
    .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        std::fs::canonicalize(&records[0].path).unwrap(),
        std::fs::canonicalize(&root).unwrap()
    );
}
