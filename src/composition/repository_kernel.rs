//! 저장소당 커널 하나이다 (plan 18 K3-4). 커널 단위는 planning 기준 데이터와 같은 저장소이다. 한 저장소의
//! worktree 들은 큐와 병렬 상태를 함께 쓰므로 한 커널을 쓴다. 병렬 슬롯 worktree(`akra-agent/slot-*`)는 이
//! 커널이 관리하는 작업 공간이고, 커널을 만들지 않는다.
//!
//! 조립 순서: 저장소 잠금을 잡고, 런타임을 조립하고, 병렬 보기를 정하고, 커널 호스트를 시작한다.
//!
//! 저장소별 커널 목록(`RepositoryKernelRegistry`, K3-5)은 저장소마다 커널 하나를 처음 붙을 때 만든다. 클라이언트가
//! 모두 떠나도 커널은 남는다. 자동 후속과 병렬 배분은 클라이언트 없이도 돌아야 하기 때문이다. 목록을 버리면
//! (Gateway 가 끝나면) 커널마다 병렬 작업 스레드를 멈추고, 호스트 스레드를 끝내고, 잠금을 푼다.

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::{Context, Result};

use crate::adapter::outbound::app_server::GatewayUpstreamSocket;

use crate::adapter::outbound::filesystem::RepositoryKernelLock;
use crate::composition::kernel_host::{KernelClientHandle, KernelHost};
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
}

impl RepositoryKernel {
    /// 저장소 신원이다. 같은 저장소의 worktree 들은 같은 값을 갖는다.
    pub(crate) fn repository_key(&self) -> &str {
        self.lock.repository_key()
    }

    pub(crate) fn attach(&self) -> Option<KernelClientHandle> {
        self.host.attach()
    }
}

/// 저장소별 커널 목록이다. Gateway 가 비동기 접속 사이에서 함께 쓴다.
#[derive(Default)]
pub(crate) struct RepositoryKernelRegistry {
    kernels: Mutex<BTreeMap<String, RepositoryKernel>>,
    /// 커널이 붙을 app-server 이다. 없으면 커널마다 자기 app-server 를 띄운다 (시험).
    app_server: Option<GatewayUpstreamSocket>,
}

impl RepositoryKernelRegistry {
    /// 커널들이 Gateway upstream app-server 를 Desktop 과 함께 쓴다 (K3-5, Q-K3-6).
    pub(crate) fn sharing_app_server(app_server: GatewayUpstreamSocket) -> Self {
        Self {
            kernels: Mutex::default(),
            app_server: Some(app_server),
        }
    }

    /// `workspace_dir` 가 속한 저장소의 커널에 클라이언트 하나를 붙이고, 커널의 저장소 신원과 함께 돌려준다. 그
    /// 저장소의 커널이 없으면 만든다. 같은 저장소의 다른 worktree 는 같은 커널에 붙는다. 다른 프로세스가 그
    /// 저장소의 커널을 돌리면 `RepositoryKernelLockHeld` 오류이다.
    pub(crate) fn attach(&self, workspace_dir: &str) -> Result<(KernelClientHandle, String)> {
        let repository_key = production::repository_kernel_key(workspace_dir)?;
        // shortcut: 커널을 만드는 동안 목록을 잠가 다른 저장소의 접속도 기다린다. 접속이 잦아지면 저장소별로 잠근다.
        let mut kernels = self
            .kernels
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(client) = kernels
            .get(&repository_key)
            .and_then(RepositoryKernel::attach)
        {
            return Ok((client, repository_key));
        }
        // 호스트 스레드가 끝난 커널은 먼저 버려 잠금을 푼 뒤 다시 만든다.
        kernels.remove(&repository_key);
        let kernel = build_kernel_for_workspace(workspace_dir, self.app_server.clone())?;
        let client = kernel
            .attach()
            .context("the new kernel stopped before a client attached")?;
        let repository_key = kernel.repository_key().to_string();
        kernels.insert(repository_key.clone(), kernel);
        Ok((client, repository_key))
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
    Ok(RepositoryKernel { host, lock })
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
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::adapter::outbound::filesystem::RepositoryKernelLockHeld;
    use crate::composition::native_client_runtime::tests::recording_runtime_for_test;

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
            production::repository_kernel_key(&folder.path).expect("repository key")
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
