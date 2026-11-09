//! 저장소당 커널 하나이다 (plan 18 K3-4). 커널 단위는 planning 기준 데이터와 같은 저장소이다. 한 저장소의
//! worktree 들은 큐와 병렬 상태를 함께 쓰므로 한 커널을 쓴다. 병렬 슬롯 worktree(`akra-agent/slot-*`)는 이
//! 커널이 관리하는 작업 공간이고, 커널을 만들지 않는다.
//!
//! 조립 순서: 저장소 잠금을 잡고, 런타임을 조립하고, 병렬 보기를 정하고, 커널 호스트를 시작한다. 저장소별
//! 커널 목록과 커널의 수명(클라이언트가 모두 떠난 뒤)은 Gateway 연결(K3-5)에서 정한다.

use anyhow::{Context, Result};

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

/// `workspace_dir` 가 속한 저장소의 커널을 조립하고 시작한다. 다른 프로세스가 그 저장소의 커널을 돌리면
/// 아무것도 시작하지 않고 `RepositoryKernelLockHeld` 오류를 돌려준다.
pub(crate) fn build_kernel_for_workspace(workspace_dir: &str) -> Result<RepositoryKernel> {
    // 잠금을 먼저 잡는다. 런타임 조립이 PR 검증 스케줄러 스레드를 시작하기 때문이다.
    let lock = production::acquire_repository_kernel_lock(workspace_dir)?;
    let runtime = production::build_headless_kernel_runtime(workspace_dir);
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

    fn temp_folder(label: &str) -> String {
        static NEXT_FOLDER: AtomicU64 = AtomicU64::new(0);
        let folder: PathBuf = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-repository-kernel-{label}-{}-{}",
            std::process::id(),
            NEXT_FOLDER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("temp folder should be created");
        folder.display().to_string()
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
        let lock = production::acquire_repository_kernel_lock(&folder).expect("first lock");
        let (runtime, _turns) = recording_runtime_for_test();
        let kernel = start_repository_kernel(lock, runtime, &folder).expect("kernel should start");
        let client = kernel
            .attach()
            .expect("a client attaches to the headless kernel");
        assert!(
            client.query().is_some(),
            "the kernel thread answers queries"
        );

        let refused = production::acquire_repository_kernel_lock(&folder)
            .expect_err("a second kernel for the repository is refused");
        assert!(
            refused.downcast_ref::<RepositoryKernelLockHeld>().is_some(),
            "{refused:#}"
        );

        let repository_key = kernel.repository_key().to_string();
        drop(client);
        drop(kernel);
        let next = production::acquire_repository_kernel_lock(&folder)
            .expect("the lock is free after the kernel stops");
        assert_eq!(next.repository_key(), repository_key);
    }

    #[test]
    fn a_second_kernel_for_the_repository_is_refused_before_it_builds_anything() {
        let folder = temp_folder("refused");
        let _held = production::acquire_repository_kernel_lock(&folder).expect("first lock");

        let Err(error) = build_kernel_for_workspace(&folder) else {
            panic!("a second kernel for the repository must not start");
        };

        assert!(
            error.downcast_ref::<RepositoryKernelLockHeld>().is_some(),
            "{error:#}"
        );
    }

    #[test]
    fn the_production_kernel_runs_without_a_screen() {
        // The host only ticks: no startup checks run, so no app-server starts.
        let folder = temp_folder("production");
        let kernel = build_kernel_for_workspace(&folder).expect("kernel should start");
        let client = kernel.attach().expect("a client attaches");

        assert!(
            client.query().is_some(),
            "the kernel thread answers queries"
        );
        assert!(
            production::acquire_repository_kernel_lock(&folder).is_err(),
            "the running kernel holds the repository lock"
        );
    }
}
