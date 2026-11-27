//! admin 의 planning 변경이 그 저장소의 커널을 쥔 Gateway 로 간다 (plan 18 K4-9d, #2801).
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::application::port::inbound::planning_admin_port::{
    PlanningAdminMutationRefusal, PlanningAdminPort, PlanningAdminTaskMutationRequest,
};
use crate::application::port::outbound::kernel_gateway_port::{
    KernelGatewayPort, KernelGatewayReply,
};
use crate::composition::kernel_command_execution::CALLER_ADMIN;
use crate::composition::repository_kernel::RepositoryKernelRegistry;
use crate::domain::kernel_event::{KernelEvent, MAX_KERNEL_EVENT_PAGE};
use crate::domain::kernel_spec::ReasonCode;

/// 프로세스 안의 Gateway 이다. 실제 클라이언트와 같이 요청마다 새로 붙는다. 답한 명령을 센다.
struct InProcessGateway {
    kernels: Arc<RepositoryKernelRegistry>,
    workspace: String,
    runtime: tokio::runtime::Handle,
    answered_commands: AtomicUsize,
}

impl KernelGatewayPort for InProcessGateway {
    fn request(&self, method: &str, params: Value, wait: bool) -> Result<KernelGatewayReply> {
        let reply = self.runtime.block_on(async {
            let mut client = connect_kernel_client(self.kernels.clone()).await;
            crate::adapter::outbound::gateway_kernel_client::request_over_socket(
                &mut client,
                &self.workspace,
                CALLER_ADMIN,
                method,
                params,
                wait,
            )
            .await
        })?;
        if method == "akra/kernel/command" && matches!(reply, KernelGatewayReply::Answer(_)) {
            self.answered_commands.fetch_add(1, Ordering::Relaxed);
        }
        Ok(reply)
    }
}

fn task(id: &str, title: &str, revision: Option<i64>) -> PlanningAdminTaskMutationRequest {
    PlanningAdminTaskMutationRequest {
        acceptance: None,
        id: id.to_string(),
        direction_id: String::new(),
        title: title.to_string(),
        description: String::new(),
        status: String::new(),
        base_priority: String::new(),
        dynamic_priority_delta: String::new(),
        priority_reason: String::new(),
        depends_on_text: String::new(),
        blocked_by_text: String::new(),
        expected_planning_revision: revision.map(|value| value.to_string()).unwrap_or_default(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn admin_task_changes_run_in_the_kernel_that_the_gateway_holds() {
    // 9d-1, 9d-2 이 남긴 시험이다: 명령은 명령 전용 조립이 아니라 Gateway 의 커널이 실행한다.
    let folder = KernelTestFolder::new("admin-planning");
    let workspace = folder
        .path
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let kernels = Arc::new(RepositoryKernelRegistry::default());
    let gateway = Arc::new(InProcessGateway {
        kernels: kernels.clone(),
        workspace: workspace.clone(),
        runtime: tokio::runtime::Handle::current(),
        answered_commands: AtomicUsize::new(0),
    });
    let admin = crate::composition::production::build_admin_planning(workspace.clone())
        .through_gateway(gateway.clone());

    let (created, stale) = tokio::task::spawn_blocking(move || {
        let created = admin
            .upsert_task(task("", "Ship through the Gateway", None))
            .unwrap();
        let id = created
            .management
            .tasks
            .iter()
            .find(|task| task.title == "Ship through the Gateway")
            .map(|task| task.id.clone())
            .expect("the task is in the management view");
        let stale = admin
            .upsert_task(task(
                &id,
                "Changed",
                Some(created.management.planning_revision - 1),
            ))
            .unwrap_err();
        (created, stale)
    })
    .await
    .unwrap();

    assert!(created.notice.ends_with("added"), "{}", created.notice);
    let refusal = PlanningAdminMutationRefusal::find_in(&stale).expect("a refusal");
    assert_eq!(refusal.reason, ReasonCode::PlanningRevisionStale);
    assert_eq!(gateway.answered_commands.load(Ordering::Relaxed), 2);
    // The kernel may write the record of the rejection just after it answers.
    let deadline = std::time::Instant::now() + TEST_TIMEOUT;
    let records = loop {
        let records = command_records(&workspace);
        if records.len() >= 4 || std::time::Instant::now() > deadline {
            break records;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        records,
        [
            "received create_task by akra-admin",
            "completed Done",
            "received update_task by akra-admin",
            "rejected planning_revision_stale",
        ]
    );
    kernels.shutdown();
}

fn command_records(workspace: &str) -> Vec<String> {
    crate::composition::production::kernel_event_log_port()
        .load_kernel_events_after(workspace, 0, MAX_KERNEL_EVENT_PAGE)
        .unwrap()
        .records
        .into_iter()
        .filter_map(|record| match record.event {
            KernelEvent::CommandReceived {
                command, caller, ..
            } => Some(format!(
                "received {command} by {}",
                caller.unwrap_or_default()
            )),
            KernelEvent::CommandCompleted { result, .. } => Some(format!("completed {result:?}")),
            KernelEvent::CommandRejected { reason, .. } => {
                Some(format!("rejected {}", reason.name()))
            }
            _ => None,
        })
        .collect()
}
