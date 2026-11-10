//! 저장소당 커널 하나이다 (plan 18 K3-4). 커널 단위는 planning 기준 데이터와 같은 저장소이다. 한 저장소의
//! worktree 들은 큐와 병렬 상태를 함께 쓰므로 한 커널을 쓴다. 병렬 슬롯 worktree(`akra-agent/slot-*`)는 이
//! 커널이 관리하는 작업 공간이고, 커널을 만들지 않는다. 그 안에서 붙으면 거절한다.
//!
//! 조립 순서: 저장소 잠금을 잡고, 런타임을 조립하고, 병렬 보기를 정하고, 커널 호스트를 시작한다.
//!
//! 저장소별 커널 목록(`RepositoryKernelRegistry`, K3-5)은 저장소마다 커널 하나를 처음 붙을 때 만든다. 커널의 작업
//! 공간은 어느 worktree 에서 붙어도 저장소의 대표 checkout 이다. 클라이언트가 모두 떠나도 커널은 남는다. 자동
//! 후속과 병렬 배분은 클라이언트 없이도 돌아야 하기 때문이다. Gateway 가 끝날 때 `shutdown` 이 커널마다 병렬
//! 작업 스레드를 멈추고, 호스트 스레드를 끝내고, 잠금을 푼다. 접속이 목록을 아직 쥐고 있어도 같다.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result, bail};

use crate::adapter::outbound::app_server::GatewayUpstreamSocket;

use crate::adapter::outbound::filesystem::RepositoryKernelLock;
use crate::composition::kernel_host::{KernelCallError, KernelClientHandle, KernelHost};
use crate::composition::native_client_runtime::NativeClientRuntime;
use crate::composition::production;
use crate::core::native_client_port::{NativeClientEvent, ParallelViewState};

/// 화면 없는 커널의 supervisor 화면 상태이다. supervisor 새로 고침은 화면에 보이기 위한 주기적 점검이고,
/// TUI 도 병렬 화면이 닫혀 있으면 `false` 를 보낸다. 배분 깨우기와 정리 재시도는 이 값과 관계없이 돈다.
const HEADLESS_SUPERVISOR_VIEW_ACTIVE: bool = false;

pub(crate) struct RepositoryKernel {
    // 필드는 선언 순서대로 버려진다. 호스트 스레드가 먼저 끝나고, 그 뒤에 잠금이 풀린다.
    host: KernelHost,
    lock: RepositoryKernelLock,
    workspace_root: String,
}

impl RepositoryKernel {
    /// 저장소 신원이다. 같은 저장소의 worktree 들은 같은 값을 갖는다.
    pub(crate) fn repository_key(&self) -> &str {
        self.lock.repository_key()
    }

    /// 커널의 작업 공간이다. 병렬 tick 은 이 폴더에서 돈다.
    pub(crate) fn workspace_root(&self) -> &str {
        &self.workspace_root
    }

    pub(crate) fn attach(&self) -> Result<KernelClientHandle, KernelCallError> {
        self.host.attach()
    }
}

/// 저장소 하나의 커널 자리이다. 커널을 만들거나 멈추는 동안 이 자리만 잠근다. 다른 저장소의 붙기는 기다리지 않는다.
#[derive(Default)]
struct RepositoryKernelEntry {
    kernel: Mutex<Option<RepositoryKernel>>,
}

impl RepositoryKernelEntry {
    fn lock(&self) -> MutexGuard<'_, Option<RepositoryKernel>> {
        self.kernel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 커널이 있으면 멈추고 잠금을 푼다. 버리는 쪽이 호스트 스레드가 끝날 때까지 기다린다.
    fn stop_kernel(&self) {
        let stopped = self.lock().take();
        drop(stopped);
    }
}

/// 저장소별 커널 목록이다. Gateway 가 비동기 접속 사이에서 함께 쓴다.
#[derive(Default)]
pub(crate) struct RepositoryKernelRegistry {
    // 목록 잠금은 자리를 찾거나 넣을 때만 잡는다.
    repositories: Mutex<BTreeMap<String, Arc<RepositoryKernelEntry>>>,
    stopped: AtomicBool,
    /// 커널이 붙을 app-server 이다. 없으면 커널마다 자기 app-server 를 띄운다 (시험).
    app_server: Option<GatewayUpstreamSocket>,
}

impl RepositoryKernelRegistry {
    /// 커널들이 Gateway upstream app-server 를 Desktop 과 함께 쓴다 (K3-5, Q-K3-6).
    pub(crate) fn sharing_app_server(app_server: GatewayUpstreamSocket) -> Self {
        Self {
            app_server: Some(app_server),
            ..Self::default()
        }
    }

    /// `workspace_dir` 가 속한 저장소의 커널에 클라이언트 하나를 붙이고, 커널의 저장소 신원과 함께 돌려준다. 그
    /// 저장소의 커널이 없으면 만든다. 같은 저장소의 다른 worktree 는 같은 커널에 붙는다. 다른 프로세스가 그
    /// 저장소의 커널을 돌리면 `RepositoryKernelLockHeld` 오류이다.
    pub(crate) fn attach(&self, workspace_dir: &str) -> Result<(KernelClientHandle, String)> {
        let target = production::repository_kernel_target(workspace_dir)?;
        let entry = self.entry(&target.repository_key)?;
        let mut kernel = entry.lock();
        // 멈춤은 자리마다 잠근 뒤 커널을 버린다. 그래서 자리 잠금 안에서 확인하면 멈춘 뒤 새 커널을 만들지 않는다.
        self.ensure_running()?;
        if let Some(running) = kernel.as_ref() {
            match running.attach() {
                Ok(client) => return Ok((client, target.repository_key)),
                Err(KernelCallError::TimedOut) => {
                    bail!("the repository kernel did not answer the attach request in time")
                }
                Err(KernelCallError::Stopped) => {}
            }
        }
        // 호스트 스레드가 끝난 커널은 먼저 버려 잠금을 푼 뒤 다시 만든다.
        *kernel = None;
        let built = build_kernel_for_workspace(&target.workspace_root, self.app_server.clone())?;
        let client = built
            .attach()
            .context("the new kernel stopped before a client attached")?;
        let repository_key = built.repository_key().to_string();
        tracing::info!(
            workspace = built.workspace_root(),
            "repository kernel started"
        );
        *kernel = Some(built);
        Ok((client, repository_key))
    }

    /// 모든 커널을 멈춘다 (Gateway 종료). 커널마다 병렬 작업 스레드를 멈추고, 호스트 스레드가 끝날 때까지 기다리고,
    /// 잠금을 푼다. 그 뒤의 붙기는 거절한다. 접속이 목록이나 클라이언트 핸들을 아직 쥐고 있어도 커널은 멈춘다.
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        for entry in self.entries() {
            entry.stop_kernel();
        }
    }

    fn entry(&self, repository_key: &str) -> Result<Arc<RepositoryKernelEntry>> {
        self.ensure_running()?;
        let mut repositories = self
            .repositories
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(repositories
            .entry(repository_key.to_string())
            .or_default()
            .clone())
    }

    fn entries(&self) -> Vec<Arc<RepositoryKernelEntry>> {
        self.repositories
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn ensure_running(&self) -> Result<()> {
        if self.stopped.load(Ordering::SeqCst) {
            bail!("the gateway is stopping; repository kernels no longer start")
        }
        Ok(())
    }
}

/// `workspace_dir` 가 속한 저장소의 커널을 조립하고 시작한다. 다른 프로세스가 그 저장소의 커널을 돌리면
/// 아무것도 시작하지 않고 `RepositoryKernelLockHeld` 오류를 돌려준다.
pub(crate) fn build_kernel_for_workspace(
    workspace_dir: &str,
    app_server: Option<GatewayUpstreamSocket>,
) -> Result<RepositoryKernel> {
    // 잠금을 먼저 잡는다. 런타임 조립이 PR 검증 스케줄러 스레드를 시작하기 때문이다.
    let lock = production::acquire_repository_kernel_lock(workspace_dir)?;
    let runtime = production::build_headless_kernel_runtime(workspace_dir, app_server);
    start_repository_kernel(lock, runtime, workspace_dir)
}

fn start_repository_kernel(
    lock: RepositoryKernelLock,
    mut runtime: NativeClientRuntime,
    workspace_dir: &str,
) -> Result<RepositoryKernel> {
    show_workspace_to_parallel_control(&mut runtime, workspace_dir);
    let host = KernelHost::start(runtime).context("failed to start the kernel host thread")?;
    Ok(RepositoryKernel {
        host,
        lock,
        workspace_root: workspace_dir.to_string(),
    })
}

/// 병렬 tick 은 병렬 보기의 작업 공간에서 돈다. 화면이 알리지 않아도 tick 이 돌도록 조립 때 정한다.
fn show_workspace_to_parallel_control(runtime: &mut NativeClientRuntime, workspace_dir: &str) {
    let _ =
        runtime.dispatch_client_event(NativeClientEvent::ParallelViewChanged(ParallelViewState {
            workspace_directory: workspace_dir.to_string(),
            supervisor_view_active: HEADLESS_SUPERVISOR_VIEW_ACTIVE,
        }));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;

    use crate::adapter::outbound::filesystem::RepositoryKernelLockHeld;
    use crate::composition::native_client_runtime::tests::recording_runtime_for_test;

    /// 그 저장소의 커널 자리를 잠근다. 돌려준 송신자를 버리면 풀린다. 커널 만들기가 오래 걸리는 경우를 흉내 낸다.
    pub(crate) fn hold_repository_for_test(
        registry: &std::sync::Arc<RepositoryKernelRegistry>,
        workspace_dir: &str,
    ) -> std::sync::mpsc::Sender<()> {
        let key = production::repository_kernel_target(workspace_dir)
            .expect("repository target")
            .repository_key;
        let entry = registry.entry(&key).expect("registry is running");
        let (release, released) = std::sync::mpsc::channel::<()>();
        let (held, holding) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _kernel = entry.lock();
            held.send(()).unwrap();
            let _ = released.recv();
        });
        holding.recv().unwrap();
        release
    }

    /// 시험이 끝나면 지우는 임시 폴더이다.
    struct TempFolder {
        path: String,
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn temp_folder(label: &str) -> TempFolder {
        static NEXT_FOLDER: AtomicU64 = AtomicU64::new(0);
        let folder: PathBuf = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-repository-kernel-{label}-{}-{}",
            std::process::id(),
            NEXT_FOLDER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("temp folder should be created");
        TempFolder {
            path: folder.display().to_string(),
        }
    }

    #[test]
    fn assembly_shows_the_folder_to_parallel_control_without_a_screen() {
        let (mut runtime, _turns) = recording_runtime_for_test();
        assert_eq!(runtime.parallel_view_for_test(), None);

        show_workspace_to_parallel_control(&mut runtime, "/repo/worktree");

        assert_eq!(
            runtime.parallel_view_for_test(),
            Some(&ParallelViewState {
                workspace_directory: "/repo/worktree".to_string(),
                supervisor_view_active: false,
            }),
            "the parallel tick has a workspace before any client reports one"
        );
    }

    #[test]
    fn the_kernel_keeps_its_repository_lock_until_it_stops() {
        let folder = temp_folder("lifetime");
        let lock = production::acquire_repository_kernel_lock(&folder.path).expect("first lock");
        let (runtime, _turns) = recording_runtime_for_test();
        let kernel =
            start_repository_kernel(lock, runtime, &folder.path).expect("kernel should start");
        let client = kernel
            .attach()
            .expect("a client attaches to the headless kernel");
        assert!(
            client.query().is_some(),
            "the kernel thread answers queries"
        );

        let refused = production::acquire_repository_kernel_lock(&folder.path)
            .expect_err("a second kernel for the repository is refused");
        assert!(
            refused.downcast_ref::<RepositoryKernelLockHeld>().is_some(),
            "{refused:#}"
        );

        let repository_key = kernel.repository_key().to_string();
        drop(client);
        drop(kernel);
        let next = production::acquire_repository_kernel_lock(&folder.path)
            .expect("the lock is free after the kernel stops");
        assert_eq!(next.repository_key(), repository_key);
    }

    #[test]
    fn a_second_kernel_for_the_repository_is_refused_before_it_builds_anything() {
        let folder = temp_folder("refused");
        let _held = production::acquire_repository_kernel_lock(&folder.path).expect("first lock");

        let Err(error) = build_kernel_for_workspace(&folder.path, None) else {
            panic!("a second kernel for the repository must not start");
        };

        assert!(
            error.downcast_ref::<RepositoryKernelLockHeld>().is_some(),
            "{error:#}"
        );
    }

    #[test]
    fn the_registry_keeps_one_kernel_per_repository_after_clients_leave() {
        let folder = temp_folder("registry");
        let registry = RepositoryKernelRegistry::default();

        let (first, kernel) = registry
            .attach(&folder.path)
            .expect("first client attaches");
        let (second, same_kernel) = registry
            .attach(&folder.path)
            .expect("a second client shares the kernel instead of taking the lock again");
        assert_ne!(first.client_id(), second.client_id());
        assert_eq!(kernel, same_kernel);
        assert_eq!(
            kernel,
            production::repository_kernel_target(&folder.path)
                .expect("repository target")
                .repository_key
        );
        drop((first, second));

        let (again, _) = registry
            .attach(&folder.path)
            .expect("the kernel outlives its clients");
        assert!(again.query().is_some());
        assert!(
            production::acquire_repository_kernel_lock(&folder.path).is_err(),
            "the kept kernel still holds the repository lock"
        );

        drop(again);
        drop(registry);
        assert!(
            production::acquire_repository_kernel_lock(&folder.path).is_ok(),
            "dropping the registry stops the kernel and releases the lock"
        );
    }

    /// 시험용 git 저장소이다. 대표 checkout 은 `<folder>/repo` 이고, pool 은 그 옆에 생긴다.
    fn git_repository(folder: &TempFolder) -> PathBuf {
        let repo = PathBuf::from(&folder.path).join("repo");
        std::fs::create_dir_all(&repo).expect("repo folder should be created");
        for args in [
            &["init", "-q"][..],
            &[
                "-c",
                "user.name=akra",
                "-c",
                "user.email=akra@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        ] {
            run_git(&repo, args);
        }
        std::fs::canonicalize(&repo).expect("repo path should canonicalize")
    }

    fn run_git(directory: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(directory)
            .args(args)
            .output()
            .expect("git should run");
        assert!(output.status.success(), "git {args:?}: {output:?}");
    }

    fn add_worktree(repo: &std::path::Path, path: &std::path::Path, branch: &str) -> String {
        let path_text = path.display().to_string();
        run_git(repo, &["worktree", "add", "-q", "-b", branch, &path_text]);
        path_text
    }

    #[test]
    fn shutdown_stops_kernels_that_sessions_still_share() {
        // K3-5 review: a session task kept an `Arc` of the registry, so dropping the gateway's copy did not
        // stop the kernels before the upstream app-server stopped.
        let folder = temp_folder("shutdown");
        let registry = std::sync::Arc::new(RepositoryKernelRegistry::default());
        let session_copy = registry.clone();
        let (client, _) = session_copy
            .attach(&folder.path)
            .expect("a client attaches");

        registry.shutdown();

        assert!(
            production::acquire_repository_kernel_lock(&folder.path).is_ok(),
            "shutdown releases the lock while a session still holds the registry and a client"
        );
        assert!(client.query().is_none(), "the kernel thread stopped");
        assert!(
            session_copy.attach(&folder.path).is_err(),
            "no kernel starts after shutdown"
        );
    }

    #[test]
    fn a_kernel_attached_from_a_linked_worktree_works_in_the_primary_checkout() {
        let folder = temp_folder("linked-worktree");
        let repo = git_repository(&folder);
        let linked = add_worktree(
            &repo,
            &PathBuf::from(&folder.path).join("feature"),
            "feature",
        );
        let registry = RepositoryKernelRegistry::default();

        let (_from_linked, kernel) = registry.attach(&linked).expect("linked worktree attaches");
        let (_from_primary, same_kernel) = registry
            .attach(&repo.display().to_string())
            .expect("the primary checkout shares the kernel");

        assert_eq!(kernel, same_kernel);
        let entry = registry.entry(&kernel).expect("registry is running");
        assert_eq!(
            entry
                .lock()
                .as_ref()
                .map(|kernel| kernel.workspace_root().to_string()),
            Some(repo.display().to_string()),
            "the first worktree does not become the kernel workspace"
        );
    }

    #[test]
    fn a_parallel_slot_worktree_does_not_create_a_kernel() {
        let folder = temp_folder("slot-worktree");
        let repo = git_repository(&folder);
        let slot_path =
            crate::application::service::parallel_mode::parallel_pool_root(&repo).join("slot-1");
        std::fs::create_dir_all(slot_path.parent().expect("pool root")).expect("pool root");
        let slot = add_worktree(&repo, &slot_path, "akra-agent/slot-1/task");
        let registry = RepositoryKernelRegistry::default();

        let Err(error) = registry.attach(&slot) else {
            panic!("a slot worktree must not create a kernel");
        };

        assert!(
            format!("{error:#}").contains("parallel slot worktree"),
            "{error:#}"
        );
        assert!(
            production::acquire_repository_kernel_lock(&repo.display().to_string()).is_ok(),
            "the refused attach took no lock"
        );
    }

    #[test]
    fn a_slow_kernel_start_does_not_block_another_repository() {
        let slow = temp_folder("slow-repository");
        let other = temp_folder("other-repository");
        let registry = std::sync::Arc::new(RepositoryKernelRegistry::default());
        let release = hold_repository_for_test(&registry, &slow.path);

        let (attached, finished) = std::sync::mpsc::channel();
        let attaching = registry.clone();
        let other_path = other.path.clone();
        std::thread::spawn(move || {
            let _ = attached.send(
                attaching
                    .attach(&other_path)
                    .map(|(client, _)| client.client_id()),
            );
        });

        let result = finished
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("another repository attaches while the first one is still starting");
        assert!(result.is_ok(), "{result:?}");
        drop(release);
    }

    #[test]
    fn the_production_kernel_runs_without_a_screen() {
        // The host only ticks: no startup checks run, so no app-server starts.
        let folder = temp_folder("production");
        let kernel = build_kernel_for_workspace(&folder.path, None).expect("kernel should start");
        let client = kernel.attach().expect("a client attaches");

        assert!(
            client.query().is_some(),
            "the kernel thread answers queries"
        );
        assert!(
            production::acquire_repository_kernel_lock(&folder.path).is_err(),
            "the running kernel holds the repository lock"
        );
    }
}
