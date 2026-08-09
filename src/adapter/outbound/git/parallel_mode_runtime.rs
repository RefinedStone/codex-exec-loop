use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use chrono::Utc;
use rand::RngCore;

#[cfg(unix)]
use crate::adapter::outbound::filesystem::secure_fs;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::application::port::outbound::parallel_mode_runtime_port::{
    ParallelCommandOutput, ParallelPinnedDirectory, ParallelPoolMutationPermit,
    ParallelWorkerCommitOutcome, ParallelWorkerCommitRequest,
};
use crate::git_subprocess;
use crate::subprocess;

use super::{parallel_normalization_fs, parallel_pool_lock, parallel_worker_commit};

/*
 * GitParallelModeRuntimeAdapter는 parallel mode application service가 요청하는 낮은 수준의
 * runtime primitive를 실제 OS/Git/filesystem 호출로 연결하는 outbound adapter다.
 * pool, readiness, distributor, lease lifecycle은 모두 ParallelModeRuntimePort만 의존하므로,
 * 이 파일에 side effect를 모아 두면 service 계층에는 "언제 실행할지" 정책만 남는다.
 */
#[derive(Debug, Clone, Default)]
pub struct GitParallelModeRuntimeAdapter;

impl GitParallelModeRuntimeAdapter {
    pub fn new() -> Self {
        /*
         * adapter 자체는 설정을 들고 있지 않다. 매 호출마다 현재 process environment와 filesystem을
         * 관찰하므로, service가 Arc<dyn ParallelModeRuntimePort>로 공유해도 내부 상태 동기화가 필요 없다.
         */
        Self
    }

    fn find_executable_in_path(
        &self,
        program: &str,
        path: &std::ffi::OsStr,
        cwd: &Path,
    ) -> Option<PathBuf> {
        if program == "codex" {
            return crate::trusted_executable::resolve_codex_command_from_path(path, cwd)
                .ok()
                .map(|command| command.source_executable);
        }
        if matches!(program, "git" | "gh") {
            return crate::trusted_executable::resolve_native_from_path(program, path, cwd).ok();
        }
        which::which_in(program, Some(path), cwd).ok()
    }
}

fn path_chain_is_link_free(path: &Path) -> bool {
    path.ancestors()
        .take_while(|ancestor| !ancestor.as_os_str().is_empty())
        .all(|ancestor| {
            std::fs::symlink_metadata(ancestor)
                .is_ok_and(|metadata| !metadata_is_link_or_reparse(&metadata))
        })
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    use std::os::windows::fs::MetadataExt;

    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

impl ParallelModeRuntimePort for GitParallelModeRuntimeAdapter {
    fn acquire_pool_mutation_permit(
        &self,
        canonical_repo_root: &Path,
        pool_root: &Path,
        timeout: Duration,
        retry_delay: Duration,
    ) -> Result<Box<dyn ParallelPoolMutationPermit>, String> {
        parallel_pool_lock::acquire_pool_mutation_permit(
            canonical_repo_root,
            pool_root,
            timeout,
            retry_delay,
        )
    }

    fn try_acquire_pool_mutation_permit(
        &self,
        pool_root: &Path,
    ) -> Result<Option<Box<dyn ParallelPoolMutationPermit>>, String> {
        parallel_pool_lock::try_acquire_pool_mutation_permit(pool_root)
    }

    fn environment_variable(&self, name: &str) -> Result<Option<String>, String> {
        match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid Unicode")),
        }
    }

    fn current_process_id(&self) -> u32 {
        std::process::id()
    }

    fn required_process_start_identity(&self, process_id: u32) -> Result<String, String> {
        crate::process_liveness::required_process_start_identity(process_id)
            .map_err(|error| error.to_string())
    }

    fn ensure_git_execution_safe(&self, path: &Path) -> Result<(), String> {
        crate::git_execution_guard::ensure_host_git_execution_config_safe(path)
            .map_err(|error| error.to_string())
    }

    fn run_git_command(
        &self,
        args: &[OsString],
        stdin: Option<&[u8]>,
    ) -> Result<ParallelCommandOutput, String> {
        let mut command = git_subprocess::command(args.iter());
        let output = match stdin {
            Some(input) => subprocess::command_output_with_input(
                &mut command,
                "parallel runtime git command",
                input,
            ),
            None => subprocess::command_output(&mut command, "parallel runtime git command"),
        }
        .map_err(|error| error.to_string())?;
        Ok(ParallelCommandOutput {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn canonicalize_path(&self, path: &Path) -> std::io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn read_directory_paths(&self, path: &Path) -> std::io::Result<Vec<PathBuf>> {
        std::fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect()
    }

    fn path_is_symlink(&self, path: &Path) -> std::io::Result<bool> {
        std::fs::symlink_metadata(path).map(|metadata| metadata_is_link_or_reparse(&metadata))
    }

    fn path_exists_checked(&self, path: &Path) -> std::io::Result<bool> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn paths_match_securely(&self, left: &Path, right: &Path) -> bool {
        if !left.is_absolute()
            || !right.is_absolute()
            || !path_chain_is_link_free(left)
            || !path_chain_is_link_free(right)
        {
            return false;
        }
        match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
    }

    fn create_private_staging_directory(
        &self,
        path: &Path,
    ) -> Result<Box<dyn ParallelPinnedDirectory>, String> {
        parallel_normalization_fs::create_private_staging_directory(path)
    }

    fn atomic_rename_noreplace(&self, source: &Path, destination: &Path) -> std::io::Result<()> {
        parallel_normalization_fs::atomic_rename_noreplace(source, destination)
    }

    fn read_bounded_unshared_regular_file(&self, path: &Path, max_bytes: usize) -> Option<Vec<u8>> {
        parallel_normalization_fs::read_bounded_unshared_regular_file(path, max_bytes)
    }

    fn fill_secure_random(&self, bytes: &mut [u8]) -> Result<(), String> {
        rand::rngs::OsRng
            .try_fill_bytes(bytes)
            .map_err(|error| error.to_string())
    }

    fn detect_git_repo_root(&self, workspace_dir: &str) -> Option<String> {
        /*
         * parallel pool은 사용자가 repo 하위 디렉터리나 slot worktree 안에서 실행해도 canonical repo 기준을
         * 찾아야 한다. 여기서는 git의 `rev-parse --show-toplevel` 결과만 돌려주고, linked worktree 보정 같은
         * 더 높은 정책은 application service 쪽 helper가 담당한다.
         */
        self.run_command(
            "git",
            &["-C", workspace_dir, "rev-parse", "--show-toplevel"],
            None,
        )
        .filter(|value| !value.is_empty())
    }

    fn command_succeeds(&self, program: &str, args: &[&str]) -> bool {
        /*
         * readiness probe는 stdout이 필요 없고 성공/실패만 중요하다.
         * stdout/stderr를 버려 TUI가 background capability check 중 터미널에 noise를 흘리지 않게 한다.
         */
        if crate::git_execution_guard::ensure_git_command_execution_config_safe(program, args, None)
            .is_err()
        {
            return false;
        }
        let mut command = git_subprocess::command_for_program(program, args.iter().copied());
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        subprocess::command_output(&mut command, &format!("{program} {}", args.join(" ")))
            .is_ok_and(|output| output.status.success())
    }

    fn run_command(
        &self,
        program: &str,
        args: &[&str],
        current_dir: Option<&str>,
    ) -> Option<String> {
        /*
         * service 계층은 git/gh command 결과를 대부분 "사용 가능한 문자열인가"로 소비한다.
         * spawn 실패, non-zero exit, invalid utf8, empty stdout을 모두 None으로 축약해 caller가
         * capability degraded/blocker 같은 domain 상태로 바꾸기 쉽게 한다.
         */
        crate::git_execution_guard::ensure_git_command_execution_config_safe(
            program,
            args,
            current_dir,
        )
        .ok()?;
        let mut command = git_subprocess::command_for_program(program, args.iter().copied());
        if let Some(current_dir) = current_dir {
            command.current_dir(current_dir);
        }
        command.stdin(Stdio::null());
        command.stderr(Stdio::null());

        let output =
            subprocess::command_output(&mut command, &format!("{program} {}", args.join(" ")))
                .ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let trimmed = stdout.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    fn run_command_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin_body: &str,
    ) -> Option<String> {
        /*
         * GitHub fallback script처럼 민감한 입력을 argv에 남기면 안 되는 경로가 이 primitive를 쓴다.
         * stdin을 명시적으로 닫은 뒤 wait해야 child process가 EOF를 보고 종료한다.
         */
        let mut command = git_subprocess::command_for_program(program, args.iter().copied());
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = subprocess::spawn(&mut command).ok()?;

        let mut stdin = child.take_stdin()?;
        stdin.write_all(stdin_body.as_bytes()).ok()?;
        drop(stdin);

        let output =
            subprocess::wait_with_output(child, &format!("{program} {}", args.join(" "))).ok()?;
        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8(output.stdout).ok()?;
        let trimmed = stdout.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    fn find_executable(&self, program: &str) -> Option<PathBuf> {
        /*
         * readiness는 binary가 있는지와 어디에서 발견됐는지를 operator-facing detail로 보여 줄 수 있다.
         * which crate에 PATH 탐색 규칙을 맡겨 platform별 executable lookup 차이를 adapter 안에 가둔다.
         */
        let cwd = std::env::current_dir().ok()?;
        let path = std::env::var_os("PATH")?;
        self.find_executable_in_path(program, &path, &cwd)
    }

    fn gh_auth_status(&self, repo_root: Option<&str>) -> bool {
        /*
         * GitHub delivery/readiness는 `gh auth status`가 현재 repo context에서 성공하는지만 필요하다.
         * repo_root가 있으면 그 디렉터리에서 실행해 gh가 올바른 host/account configuration을 고르게 한다.
         */
        let Some(cwd) = repo_root
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
        else {
            return false;
        };
        let Ok(executable) =
            crate::trusted_executable::resolve_native_from_current_path("gh", &cwd)
        else {
            return false;
        };
        let mut command = Command::new(executable);
        if crate::trusted_executable::configure_credential_command_environment(
            &mut command,
            &cwd,
            true,
        )
        .is_err()
        {
            return false;
        }
        for name in ["GH_TOKEN", "GITHUB_TOKEN"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.args(["auth", "status"]);
        if let Some(repo_root) = repo_root {
            command.current_dir(repo_root);
        }
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::null());
        command.env("GIT_TERMINAL_PROMPT", "0");
        subprocess::command_output(&mut command, "gh auth status")
            .is_ok_and(|output| output.status.success())
    }

    fn prepare_parallel_worker_commit(
        &self,
        request: ParallelWorkerCommitRequest<'_>,
    ) -> Result<ParallelWorkerCommitOutcome, String> {
        parallel_worker_commit::prepare_parallel_worker_commit(request)
    }

    fn current_timestamp(&self) -> String {
        /*
         * lease audit, distributor records, session detail events는 runtime time source를 통해 timestamp를 받는다.
         * production은 UTC RFC3339를 쓰고, tests는 fake runtime으로 deterministic value를 제공한다.
         */
        Utc::now().to_rfc3339()
    }

    fn canonicalize_best_effort(&self, path: &Path) -> PathBuf {
        /*
         * pool/lease 비교는 symlink와 relative path 차이에 민감하지만, cleanup 중에는 path가 아직 없을 수도 있다.
         * canonicalize가 실패해도 원래 path를 반환해 caller가 "없는 경로" 같은 상태를 계속 판정할 수 있게 한다.
         */
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn path_exists(&self, path: &Path) -> bool {
        /*
         * slot worktree, lease file, session detail directory가 실제로 남아 있는지 확인하는 가장 작은 guard다.
         * 존재 여부만 필요한 caller에게 metadata error의 세부를 노출하지 않는다.
         */
        path.exists()
    }

    fn ensure_directory_exists(&self, path: &Path) -> std::io::Result<()> {
        /*
         * pool root나 session detail directory 생성은 idempotent해야 한다.
         * directory가 이미 있으면 성공으로 보고, 같은 path에 파일이 있으면 caller가 mirror
         * namespace 충돌을 사용자-facing 오류로 바꿀 수 있도록 io error를 보존한다.
         */
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => return Ok(()),
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "path exists and is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        std::fs::create_dir_all(path)
    }

    fn prepare_runtime_mirror_root(&self, path: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            self.ensure_directory_exists(path)
        }
        #[cfg(windows)]
        {
            // Windows native runtime projections are authority-backed and the mirror methods
            // below intentionally perform no filesystem I/O. Creating the pool namespace here
            // with generic ACL inheritance would preempt the hardened pool-lock creator.
            let _ = path;
            Ok(())
        }
    }

    fn write_runtime_mirror_atomic(
        &self,
        pool_root: &Path,
        relative: &Path,
        body: &str,
    ) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            secure_fs::write_file_atomic(pool_root, relative, body.as_bytes())
                .map_err(secure_mirror_io_error)
        }
        #[cfg(windows)]
        {
            let _ = (pool_root, relative, body);
            Ok(())
        }
    }

    fn read_runtime_mirror_optional(
        &self,
        pool_root: &Path,
        relative: &Path,
    ) -> std::io::Result<Option<String>> {
        #[cfg(unix)]
        {
            secure_fs::read_optional_file(pool_root, relative).map_err(secure_mirror_io_error)
        }
        #[cfg(windows)]
        {
            let _ = (pool_root, relative);
            Ok(None)
        }
    }

    fn read_runtime_mirror_directory(
        &self,
        pool_root: &Path,
        relative: &Path,
    ) -> std::io::Result<Vec<(PathBuf, String)>> {
        #[cfg(unix)]
        {
            secure_fs::read_tree(pool_root, relative)
                .map(|records| {
                    records
                        .into_iter()
                        .map(|(path, body)| (PathBuf::from(path), body))
                        .collect()
                })
                .map_err(secure_mirror_io_error)
        }
        #[cfg(windows)]
        {
            let _ = (pool_root, relative);
            Ok(Vec::new())
        }
    }

    fn remove_runtime_mirror_file(&self, pool_root: &Path, relative: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            secure_fs::remove_optional_file(pool_root, relative).map_err(secure_mirror_io_error)
        }
        #[cfg(windows)]
        {
            let _ = (pool_root, relative);
            Ok(())
        }
    }

    fn remove_runtime_mirror_file_if_matches(
        &self,
        pool_root: &Path,
        relative: &Path,
        expected_body: &str,
    ) -> std::io::Result<bool> {
        #[cfg(unix)]
        {
            let observed = secure_fs::read_optional_file(pool_root, relative)
                .map_err(secure_mirror_io_error)?;
            match observed.as_deref() {
                None => secure_fs::compare_and_swap_optional_file(pool_root, relative, None, None)
                    .map_err(secure_mirror_io_error),
                Some(body) if body == expected_body => secure_fs::compare_and_swap_optional_file(
                    pool_root,
                    relative,
                    Some(expected_body),
                    None,
                )
                .map_err(secure_mirror_io_error),
                Some(_) => Ok(false),
            }
        }
        #[cfg(windows)]
        {
            let _ = (pool_root, relative, expected_body);
            Ok(true)
        }
    }

    fn replace_runtime_mirror_file_if_matches(
        &self,
        pool_root: &Path,
        relative: &Path,
        expected_body: &str,
        replacement_body: &str,
    ) -> std::io::Result<bool> {
        #[cfg(unix)]
        {
            secure_fs::compare_and_swap_optional_file(
                pool_root,
                relative,
                Some(expected_body),
                Some(replacement_body),
            )
            .map_err(secure_mirror_io_error)
        }
        #[cfg(windows)]
        {
            let _ = (pool_root, relative, expected_body, replacement_body);
            Ok(true)
        }
    }

    fn compare_and_swap_runtime_mirror_file(
        &self,
        pool_root: &Path,
        relative: &Path,
        expected_body: Option<&str>,
        replacement_body: Option<&str>,
    ) -> std::io::Result<bool> {
        #[cfg(unix)]
        {
            secure_fs::compare_and_swap_optional_file(
                pool_root,
                relative,
                expected_body,
                replacement_body,
            )
            .map_err(secure_mirror_io_error)
        }
        #[cfg(windows)]
        {
            let _ = (pool_root, relative, expected_body, replacement_body);
            Ok(true)
        }
    }
}

#[cfg(unix)]
fn secure_mirror_io_error(error: anyhow::Error) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

#[cfg(all(test, unix))]
mod trusted_executable_tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::GitParallelModeRuntimeAdapter;

    #[test]
    fn codex_readiness_accepts_trusted_npm_launcher_and_rejects_repository_path_entries() {
        let cwd = std::env::current_dir().expect("test cwd should resolve");
        let install = safe_fixture_root("parallel-codex-npm");
        let node_install = safe_fixture_root("parallel-node");
        let launcher = install.join("codex.js");
        write_executable(&launcher, "#!/usr/bin/env node\nprocess.exit(0);\n");
        symlink(&launcher, install.join("codex")).expect("npm launcher symlink should create");
        write_native_executable(&node_install.join("node"));
        let trusted_path = std::env::join_paths([install.clone(), node_install.clone()])
            .expect("trusted fixture PATH should join");
        let runtime = GitParallelModeRuntimeAdapter::new();

        assert_eq!(
            runtime.find_executable_in_path("codex", &trusted_path, &cwd),
            Some(fs::canonicalize(&launcher).expect("launcher should canonicalize"))
        );

        let hostile_bin = cwd
            .join("target")
            .join(format!("akra-hostile-parallel-codex-{}", unique_suffix()));
        fs::create_dir_all(&hostile_bin).expect("hostile repository bin should create");
        let marker = hostile_bin.join("executed");
        let hostile_script = format!("#!/bin/sh\nset -eu\n: > '{}'\nexit 0\n", marker.display());
        write_executable(&hostile_bin.join("codex"), &hostile_script);
        write_executable(&hostile_bin.join("node"), &hostile_script);
        let hostile_path = std::env::join_paths(
            std::iter::once(hostile_bin.clone())
                .chain(std::iter::once(install.clone()))
                .chain(std::iter::once(node_install.clone())),
        )
        .expect("hostile fixture PATH should join");
        assert_eq!(
            runtime.find_executable_in_path("codex", &hostile_path, &cwd),
            None
        );
        assert!(!marker.exists(), "repository Codex or Node.js was executed");

        let _ = fs::remove_dir_all(&hostile_bin);
        let _ = fs::remove_dir_all(&install);
        let _ = fs::remove_dir_all(&node_install);
    }

    fn safe_fixture_root(label: &str) -> PathBuf {
        let root = PathBuf::from(std::env::var_os("HOME").expect("HOME should be available"))
            .join(".cache")
            .join(format!("akra-{label}-{}", unique_suffix()));
        fs::create_dir_all(&root).expect("safe fixture root should create");
        let mut permissions = fs::metadata(&root)
            .expect("fixture metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&root, permissions).expect("fixture directory should be safe");
        root
    }

    fn write_executable(path: &Path, contents: &str) {
        fs::write(path, contents).expect("executable fixture should write");
        let mut permissions = fs::metadata(path)
            .expect("fixture metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("fixture should be executable");
    }

    fn write_native_executable(path: &Path) {
        crate::trusted_executable::copy_native_executable_fixture(path)
            .expect("native executable fixture should copy");
    }

    fn unique_suffix() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow Unix epoch")
            .as_nanos()
    }
}

#[cfg(all(test, windows))]
mod windows_runtime_mirror_tests {
    use std::path::Path;

    use super::{GitParallelModeRuntimeAdapter, ParallelModeRuntimePort};

    #[test]
    fn windows_parallel_runtime_uses_sqlite_authority_without_filesystem_mirrors() {
        let runtime = GitParallelModeRuntimeAdapter::new();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock should follow the Unix epoch")
            .as_nanos();
        let unavailable_root = std::env::temp_dir().join(format!(
            "akra-disabled-windows-mirror-{}-{nonce}",
            std::process::id()
        ));
        let relative = Path::new(".leases/slot-1.json");

        runtime
            .write_runtime_mirror_atomic(&unavailable_root, relative, "authority projection")
            .expect("disabled Windows mirror writes should be non-fatal");
        assert_eq!(
            runtime
                .read_runtime_mirror_optional(&unavailable_root, relative)
                .expect("disabled Windows mirror reads should be cache misses"),
            None
        );
        assert!(
            runtime
                .read_runtime_mirror_directory(&unavailable_root, Path::new(".leases"))
                .expect("disabled Windows mirror lists should be empty")
                .is_empty()
        );
        runtime
            .remove_runtime_mirror_file(&unavailable_root, relative)
            .expect("disabled Windows mirror removal should be idempotent");
        assert!(
            runtime
                .remove_runtime_mirror_file_if_matches(
                    &unavailable_root,
                    relative,
                    "authority projection",
                )
                .expect("disabled Windows mirror CAS removal should be satisfied")
        );
        assert!(!unavailable_root.exists());
    }
}

#[cfg(all(test, unix))]
mod secure_runtime_mirror_tests {
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{GitParallelModeRuntimeAdapter, ParallelModeRuntimePort, secure_fs};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    const MIRROR_CASES: [(&str, &str); 3] = [
        (".leases/slot-1.json", ".leases/slot-1.tmp"),
        (
            ".distributor-queue/queue-1.json",
            ".distributor-queue/queue-1.json.tmp",
        ),
        (
            ".agent-sessions/session-1.json",
            ".agent-sessions/session-1.json.tmp",
        ),
    ];

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "akra-secure-runtime-mirror-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("secure mirror test root should create");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("secure mirror test root should be private");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_private_file(path: &Path, body: &str) {
        fs::write(path, body).expect("sentinel should write");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .expect("sentinel should be private");
    }

    fn create_private_parent(root: &Path, relative: &Path) {
        let parent = root.join(relative.parent().expect("mirror path should have parent"));
        fs::create_dir_all(&parent).expect("mirror parent should create");
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))
            .expect("mirror parent should be private");
    }

    #[test]
    fn runtime_mirror_write_ignores_predictable_symlink_and_hardlink_temps() {
        let root = TestDirectory::new("legacy-temp-links");
        let sentinel_root = TestDirectory::new("legacy-temp-sentinel");
        let sentinel = sentinel_root.path().join("sentinel");
        write_private_file(&sentinel, "sentinel");
        let runtime = GitParallelModeRuntimeAdapter::new();

        for (relative, legacy_temp) in MIRROR_CASES {
            let relative = Path::new(relative);
            let legacy_temp = root.path().join(legacy_temp);
            create_private_parent(root.path(), relative);

            symlink(&sentinel, &legacy_temp).expect("legacy temporary symlink should create");
            runtime
                .write_runtime_mirror_atomic(root.path(), relative, "first")
                .expect("an unrelated predictable temporary symlink must not affect the write");
            assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
            assert!(
                fs::symlink_metadata(&legacy_temp)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_file(&legacy_temp).unwrap();

            fs::hard_link(&sentinel, &legacy_temp)
                .expect("legacy temporary hardlink should create");
            runtime
                .write_runtime_mirror_atomic(root.path(), relative, "second")
                .expect("an unrelated predictable temporary hardlink must not affect the write");
            assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
            assert_eq!(fs::metadata(&sentinel).unwrap().nlink(), 2);
            fs::remove_file(&legacy_temp).unwrap();
            assert_eq!(
                runtime
                    .read_runtime_mirror_optional(root.path(), relative)
                    .unwrap()
                    .as_deref(),
                Some("second")
            );
        }
    }

    #[test]
    fn runtime_mirror_operations_reject_leaf_symlinks_and_hardlinks() {
        let root = TestDirectory::new("leaf-links");
        let sentinel_root = TestDirectory::new("leaf-link-sentinel");
        let sentinel = sentinel_root.path().join("sentinel");
        write_private_file(&sentinel, "sentinel");
        let runtime = GitParallelModeRuntimeAdapter::new();

        for (relative, _) in MIRROR_CASES {
            let relative = Path::new(relative);
            let leaf = root.path().join(relative);
            create_private_parent(root.path(), relative);

            symlink(&sentinel, &leaf).expect("mirror leaf symlink should create");
            assert!(
                runtime
                    .write_runtime_mirror_atomic(root.path(), relative, "overwrite")
                    .is_err()
            );
            assert!(
                runtime
                    .read_runtime_mirror_optional(root.path(), relative)
                    .is_err()
            );
            assert!(
                runtime
                    .remove_runtime_mirror_file(root.path(), relative)
                    .is_err()
            );
            assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
            fs::remove_file(&leaf).unwrap();

            fs::hard_link(&sentinel, &leaf).expect("mirror leaf hardlink should create");
            assert!(
                runtime
                    .write_runtime_mirror_atomic(root.path(), relative, "overwrite")
                    .is_err()
            );
            assert!(
                runtime
                    .read_runtime_mirror_optional(root.path(), relative)
                    .is_err()
            );
            assert!(
                runtime
                    .remove_runtime_mirror_file(root.path(), relative)
                    .is_err()
            );
            assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
            fs::remove_file(&leaf).unwrap();
        }
    }

    #[test]
    fn runtime_mirror_operations_reject_metadata_directory_symlinks() {
        let root = TestDirectory::new("metadata-links");
        let outside = TestDirectory::new("metadata-link-target");
        let sentinel = outside.path().join("sentinel");
        write_private_file(&sentinel, "sentinel");
        let runtime = GitParallelModeRuntimeAdapter::new();

        for (relative, _) in MIRROR_CASES {
            let relative = Path::new(relative);
            let metadata_name = relative.components().next().unwrap().as_os_str();
            let metadata_path = root.path().join(metadata_name);
            symlink(outside.path(), &metadata_path)
                .expect("metadata directory symlink should create");

            assert!(
                runtime
                    .write_runtime_mirror_atomic(root.path(), relative, "overwrite")
                    .is_err()
            );
            assert!(
                runtime
                    .read_runtime_mirror_optional(root.path(), relative)
                    .is_err()
            );
            assert!(
                runtime
                    .read_runtime_mirror_directory(root.path(), Path::new(metadata_name))
                    .is_err()
            );
            assert!(
                runtime
                    .remove_runtime_mirror_file(root.path(), relative)
                    .is_err()
            );
            assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
            assert!(!outside.path().join(relative.file_name().unwrap()).exists());
            fs::remove_file(&metadata_path).unwrap();
        }
    }

    #[test]
    fn runtime_mirror_operations_reject_a_symlinked_pool_root() {
        let root_parent = TestDirectory::new("pool-root-link-parent");
        let outside = TestDirectory::new("pool-root-link-target");
        let sentinel = outside.path().join("sentinel");
        write_private_file(&sentinel, "sentinel");
        let pool_root = root_parent.path().join("akra-pool");
        symlink(outside.path(), &pool_root).expect("pool root symlink should create");
        let relative = Path::new(".leases/slot-1.json");
        let runtime = GitParallelModeRuntimeAdapter::new();

        assert!(
            runtime
                .write_runtime_mirror_atomic(&pool_root, relative, "overwrite")
                .is_err()
        );
        assert!(
            runtime
                .read_runtime_mirror_optional(&pool_root, relative)
                .is_err()
        );
        assert!(
            runtime
                .read_runtime_mirror_directory(&pool_root, Path::new(".leases"))
                .is_err()
        );
        assert!(
            runtime
                .remove_runtime_mirror_file(&pool_root, relative)
                .is_err()
        );
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
        assert!(!outside.path().join(".leases/slot-1.json").exists());
    }

    #[test]
    fn runtime_mirror_write_rejects_ancestor_replacement_race() {
        let root = TestDirectory::new("ancestor-race");
        let outside = TestDirectory::new("ancestor-race-target");
        let sentinel = outside.path().join("sentinel");
        write_private_file(&sentinel, "sentinel");
        let relative = Path::new(".agent-sessions/session-race.json");
        create_private_parent(root.path(), relative);
        let metadata = root.path().join(".agent-sessions");
        let displaced = root.path().join(".agent-sessions-displaced");
        let replacement_target = outside.0.clone();
        let metadata_for_hook = metadata.clone();
        let displaced_for_hook = displaced.clone();
        secure_fs::install_before_atomic_replace_hook(move || {
            fs::rename(&metadata_for_hook, &displaced_for_hook)
                .expect("pinned metadata directory should move");
            symlink(&replacement_target, &metadata_for_hook)
                .expect("attacker replacement symlink should create");
        });

        let runtime = GitParallelModeRuntimeAdapter::new();
        let error = runtime
            .write_runtime_mirror_atomic(root.path(), relative, "candidate")
            .expect_err("an ancestor replacement must fail the final reachability check");
        assert!(error.to_string().contains("ancestor"));
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
        assert!(!outside.path().join("session-race.json").exists());
    }

    #[test]
    fn runtime_mirror_lifecycle_is_private_atomic_and_bounded_to_the_pool() {
        let root = TestDirectory::new("lifecycle");
        let runtime = GitParallelModeRuntimeAdapter::new();
        let relative = Path::new(".distributor-queue/queue.json");

        runtime
            .write_runtime_mirror_atomic(root.path(), relative, "record")
            .expect("private mirror should write");
        let installed = root.path().join(relative);
        let metadata = fs::metadata(&installed).unwrap();
        assert_eq!(metadata.mode() & 0o777, 0o600);
        assert_eq!(metadata.nlink(), 1);
        let directory = fs::metadata(root.path().join(".distributor-queue")).unwrap();
        assert_eq!(directory.mode() & 0o077, 0);

        assert_eq!(
            runtime
                .read_runtime_mirror_directory(root.path(), Path::new(".distributor-queue"))
                .unwrap(),
            vec![(PathBuf::from("queue.json"), "record".to_string())]
        );
        runtime
            .remove_runtime_mirror_file(root.path(), relative)
            .expect("private mirror should remove");
        assert_eq!(
            runtime
                .read_runtime_mirror_optional(root.path(), relative)
                .unwrap(),
            None
        );
    }

    #[test]
    fn runtime_mirror_compare_and_delete_preserves_replacement_body() {
        let root = TestDirectory::new("compare-delete");
        let runtime = GitParallelModeRuntimeAdapter::new();
        let relative = Path::new(".leases/slot-1.json");

        runtime
            .write_runtime_mirror_atomic(root.path(), relative, "generation-one")
            .expect("first mirror generation should write");
        runtime
            .write_runtime_mirror_atomic(root.path(), relative, "generation-two")
            .expect("replacement mirror generation should write");
        assert!(
            !runtime
                .remove_runtime_mirror_file_if_matches(root.path(), relative, "generation-one",)
                .expect("stale mirror compare-and-delete should be rejected")
        );
        assert_eq!(
            runtime
                .read_runtime_mirror_optional(root.path(), relative)
                .expect("replacement mirror should remain")
                .as_deref(),
            Some("generation-two")
        );
        assert!(
            runtime
                .remove_runtime_mirror_file_if_matches(root.path(), relative, "generation-two",)
                .expect("exact mirror compare-and-delete should succeed")
        );
        assert_eq!(
            runtime
                .read_runtime_mirror_optional(root.path(), relative)
                .expect("removed mirror should read as missing"),
            None
        );
        assert!(
            runtime
                .remove_runtime_mirror_file_if_matches(root.path(), relative, "generation-two")
                .expect("missing mirror compare-and-delete should be idempotent")
        );
    }
}
