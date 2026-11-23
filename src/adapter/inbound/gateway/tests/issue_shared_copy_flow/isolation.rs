use std::fs::{self, File};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CASE_ENV: &str = "AKRA_TEST_ISSUE_SHARED_COPY_CASE";
const ROOT_ENV: &str = "AKRA_TEST_ISSUE_SHARED_COPY_ROOT";
const CHILD_DEADLINE: Duration = Duration::from_secs(90);

/// 실제 정책 어댑터의 사용자 홈과 authority resolver의 test temp를 별도 프로세스로 격리한다.
/// 부모의 환경을 바꾸지 않으며, helper 전용 시험은 추가하지 않는다.
pub(super) fn run_isolated(case: &str, body: impl Future<Output = ()>) {
    if std::env::var(CASE_ENV).ok().as_deref() == Some(case) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("isolated Gateway runtime")
            .block_on(body);
        return;
    }

    let root = TemporaryRoot::new(case);
    let log_path = root.0.join("child-output.log");
    let log = File::create(&log_path).expect("child output file");
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .arg(format!(
            "adapter::inbound::gateway::tests::issue_shared_copy_flow::{case}"
        ))
        .args(["--exact", "--nocapture", "--test-threads=1"])
        .env_remove(CASE_ENV)
        .env_remove(ROOT_ENV)
        .env(CASE_ENV, case)
        .env(ROOT_ENV, &root.0)
        .env("AKRA_HOME", root.0.join("user"))
        .env("TMP", &root.0)
        .env("TEMP", &root.0)
        .env("TMPDIR", &root.0)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().expect("clone child output")))
        .stderr(Stdio::from(log));
    let mut child = ChildGuard(command.spawn().expect("spawn isolated Gateway scenario"));
    let deadline = Instant::now() + CHILD_DEADLINE;
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("poll isolated Gateway scenario") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "Gateway child exceeded {CHILD_DEADLINE:?}:\n{}",
            fs::read_to_string(&log_path).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "isolated Gateway scenario failed ({status}):\n{}",
        fs::read_to_string(&log_path).unwrap_or_default()
    );
}

pub(super) fn root() -> PathBuf {
    std::env::var_os(ROOT_ENV)
        .map(PathBuf::from)
        .expect("Gateway fixture must run inside its isolated child")
}

struct TemporaryRoot(PathBuf);

impl TemporaryRoot {
    fn new(case: &str) -> Self {
        let label: String = case.rsplit("::").next().unwrap().chars().take(40).collect();
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-shared-copy-{label}-{}-{:08x}",
            std::process::id(),
            rand::random::<u32>()
        ));
        crate::private_directory::ensure_private_directory(&path)
            .expect("private isolated scenario root");
        Self(path)
    }
}

impl Drop for TemporaryRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn prepare_directory(path: &Path) {
    crate::private_directory::ensure_private_directory(path).expect("private scenario directory");
}
