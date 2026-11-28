//! The real stored command and coordinator share the SQLite quota revision.
use super::*;
use crate::application::port::outbound::issue_account_quota_port::{
    IssueAccountQuota, IssueAccountQuotaPort, IssueQuotaHold,
};
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkExecutionPort, IssueWorkExecutionSnapshot, IssueWorkPermitRequest,
};
use crate::application::service::issue_coordinator::IssueCoordinatorService;
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use std::collections::VecDeque;

#[derive(Default)]
struct Executor;

impl IssueWorkExecutionPort for Executor {
    fn snapshot(&self, _: &str, _: &str) -> anyhow::Result<IssueWorkExecutionSnapshot> {
        Ok(IssueWorkExecutionSnapshot::default())
    }

    fn submit(&self, _: &str, _: &IssueWorkOrder) -> anyhow::Result<Vec<String>> {
        Ok(vec!["quota-fixture-task".to_owned()])
    }

    fn execution_limit(&self, _: &str, _: &str, _: u32) -> anyhow::Result<()> {
        Ok(())
    }

    fn grant(&self, _: &str, _: &IssueWorkPermitRequest) -> anyhow::Result<()> {
        panic!("the fixture never supplies a source or starts a turn");
    }

    fn revoke(&self, _: &str, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

struct QuotaFixture(Mutex<VecDeque<IssueAccountQuota>>);

impl IssueAccountQuotaPort for QuotaFixture {
    fn read(&self) -> IssueAccountQuota {
        self.0
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected quota read")
    }
}

const POLICY: &str = r#"
[binding.github]
mode = "gated"
[authority]
autonomy_cap = "gated"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_decision"}]
[gates.gated.normal]
start = "auto"
"#;

fn retry(sim: &mut SimKernel, workspace: &str, revision: u64) -> bool {
    let request: CommandRequest = serde_json::from_value(serde_json::json!({
        "command": {"type":"retry_issue_account_quota","lane":"start","expected_revision":revision}
    }))
    .unwrap();
    let descriptor = request.descriptor().unwrap();
    let KernelCommandInput::Stored(command) = request.command.into_input(workspace).unwrap() else {
        panic!("the retry command must be stored");
    };
    let (reply, response) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Command {
        command: KernelClientCommand::Stored(Box::new(command)),
        descriptor,
        reply,
    });
    let outcome = response.try_recv().unwrap();
    let KernelCommandOutcome::Handled {
        stored:
            Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueAccountQuotaRetryRequested { accepted },
            ))),
        ..
    } = outcome
    else {
        panic!("expected a quota command result");
    };
    accepted
}

#[test]
fn kernel_quota_retry_is_revision_scoped_and_never_replays_a_lost_read() {
    let fixture = Fixture::new();
    let source = Source::new();
    let mut services = fixture.services_with_policy(source.clone(), POLICY);
    services.seed();
    services
        .issues
        .set_mode_for_test(IssueCollectionMode::Observe)
        .unwrap();
    let quota = Arc::new(QuotaFixture(Mutex::new(VecDeque::from([
        IssueAccountQuota::Exhausted,
        IssueAccountQuota::Unknown,
        IssueAccountQuota::Allowed,
    ]))));
    let coordinator = Arc::new(
        IssueCoordinatorService::new(
            fixture.workspace.clone(),
            services.store.clone(),
            source.clone(),
            services.triage.clone(),
            services.decisions.clone(),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
            Arc::new(Executor),
        )
        .with_account_quota(quota.clone()),
    );
    services.issues = Arc::new(
        IssueCollectionService::new(
            fixture.workspace.clone(),
            services.store.clone(),
            source,
            Arc::new(Origin),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        )
        .with_triage(services.triage.clone())
        .with_coordinator(coordinator.clone()),
    );
    services
        .triage
        .triage_all(&IssueCollectionCancellation::new(), &|_| {})
        .unwrap();
    let mut sim = fixture.boot(&services, Arc::new(SimKernelLog::default()));
    let tick = || {
        coordinator
            .tick(&IssueCollectionCancellation::new(), 1_000, false, &|_| {})
            .unwrap()
    };
    let wait = || {
        services
            .issues
            .projection()
            .unwrap()
            .account_quota
            .start
            .unwrap()
    };
    let work = || services.store.list_issue_work(&fixture.workspace).unwrap();

    tick();
    assert!(work().is_empty(), "exhausted quota must not reserve work");
    let exhausted = wait();
    assert_eq!(exhausted.hold, Some(IssueQuotaHold::Exhausted));
    assert!(!retry(&mut sim, &fixture.workspace, exhausted.revision - 1));
    tick();
    assert_eq!(quota.0.lock().unwrap().len(), 2);
    assert!(retry(&mut sim, &fixture.workspace, exhausted.revision));
    tick();
    assert!(work().is_empty(), "a lost response must not reserve work");
    let unknown = wait();
    assert_eq!(unknown.hold, Some(IssueQuotaHold::Unknown));
    assert!(!retry(&mut sim, &fixture.workspace, exhausted.revision));
    tick();
    assert_eq!(quota.0.lock().unwrap().len(), 1);

    assert!(retry(&mut sim, &fixture.workspace, unknown.revision));
    tick();
    assert_eq!(work().len(), 1);
    assert_eq!(work()[0].task_ids, ["quota-fixture-task"]);
    assert!(
        services
            .issues
            .projection()
            .unwrap()
            .account_quota
            .start
            .is_none()
    );
    assert!(quota.0.lock().unwrap().is_empty());
    assert!(sim.effects.take().is_empty());
}
