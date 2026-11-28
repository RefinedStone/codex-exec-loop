use super::*;
use crate::application::port::inbound::stored_command_port::issue_confirmation::VerifiedHumanConfirmation;
use crate::application::port::outbound::issue_work_execution_port::{
    ConfirmedIssueControl, IssueWorkExecutionPort, IssueWorkExecutionSnapshot,
    IssueWorkPermitRequest,
};
use crate::domain::issue_automation::comment_command::{
    IssueCommentRequestKey, IssueCommentRequestStatus, StoredIssueCommentRequest,
};
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::work_control::{IssueWorkControl, IssueWorkControlAction};

#[derive(Default)]
struct ControlFixture {
    // Shared independently of the request service, like the planning ledger on restart.
    control: Mutex<IssueWorkControl>,
    receipts: Mutex<BTreeMap<String, IssueWorkControl>>,
    unreadable: AtomicBool,
}

impl IssueWorkExecutionPort for ControlFixture {
    fn control(&self, _: &str, _: &str) -> Result<IssueWorkControl> {
        Ok(self.control.lock().unwrap().clone())
    }

    fn control_command_result(
        &self,
        _: &str,
        _: &str,
        command_id: &str,
        _: IssueWorkControlAction,
    ) -> Result<Option<IssueWorkControl>> {
        if self.unreadable.load(Ordering::SeqCst) {
            anyhow::bail!("planning outcome is temporarily unreadable");
        }
        Ok(self.receipts.lock().unwrap().get(command_id).cloned())
    }

    fn apply_confirmed_control(
        &self,
        _: &str,
        request: &ConfirmedIssueControl<'_>,
    ) -> Result<Option<IssueWorkControl>> {
        let mut control = self.control.lock().unwrap();
        if control.authorization_epoch != request.expected_epoch || !(request.authorize)()? {
            return Ok(None);
        }
        control.authorization_epoch += 1;
        self.receipts
            .lock()
            .unwrap()
            .insert(request.command_id.into(), control.clone());
        Ok(Some(control.clone()))
    }

    fn snapshot(&self, _: &str, _: &str) -> Result<IssueWorkExecutionSnapshot> {
        panic!("comment requests must not read worker state")
    }
    fn submit(&self, _: &str, _: &IssueWorkOrder) -> Result<Vec<String>> {
        panic!("comment requests must not submit work directly")
    }
    fn grant(&self, _: &str, _: &IssueWorkPermitRequest) -> Result<()> {
        panic!("comment requests must not grant worker approval")
    }
    fn revoke(&self, _: &str, _: &str) -> Result<()> {
        panic!("the control commit owns permit fencing")
    }
}

fn service(fixture: &Fixture, execution: &Arc<ControlFixture>) -> IssueCommentCommandService {
    IssueCommentCommandService::new(
        fixture.workspace.clone(),
        fixture.store.clone(),
        fixture.store.clone(),
        fixture.source.clone(),
        IssueAutomationPolicyService::new(fixture.policy.clone()),
        fixture.interventions.clone(),
    )
    .with_request_execution(execution.clone())
}

fn record(
    fixture: &Fixture,
    execution: &Arc<ControlFixture>,
    id: &str,
    observed_at: u64,
) -> StoredIssueCommentRequest {
    fixture.comment(id, TRUSTED, IssueCommentCommandKind::Retriage);
    service(fixture, execution)
        .poll(&IssueCollectionCancellation::new(), observed_at, &|_| {})
        .unwrap();
    let key = IssueCommentRequestKey {
        issue: fixture.issue.clone(),
        comment_id: id.into(),
    };
    let (basis, _) = fixture.store.request_state(&key).unwrap().unwrap();
    StoredIssueCommentRequest { key, basis }
}

fn now() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap()
}

fn projected(
    fixture: &Fixture,
    execution: &Arc<ControlFixture>,
) -> Vec<crate::domain::issue_automation::comment_command::IssueCommentRequestDetail> {
    let mut items = fixture
        .store
        .read_triage_batch(&fixture.workspace, None, 10)
        .unwrap()
        .items;
    let item = items.pop().unwrap();
    service(fixture, execution)
        .recorded_requests(&item)
        .unwrap()
}

#[test]
fn native_confirmation_applies_once_and_links_only_the_selected_comment() {
    let (fixture, execution) = (Fixture::new(), Arc::new(ControlFixture::default()));
    let first = record(&fixture, &execution, "first", now());
    let second = record(&fixture, &execution, "second", now());
    let service = service(&fixture, &execution);
    assert!(
        projected(&fixture, &execution)
            .iter()
            .all(|item| item.status == IssueCommentRequestStatus::Pending)
    );
    assert_eq!(execution.control.lock().unwrap().authorization_epoch, 0);
    let verified = VerifiedHumanConfirmation::from_verified(
        service.prepare_confirmation(&second.key).unwrap().target,
        format!("desktop-confirmation:{}", "a".repeat(32)),
    )
    .unwrap();
    let context = StoredCommandContext {
        human_confirmation: Some(verified),
        ..comment_context(now())
    };
    assert!(service.confirm_request(&first.key, &context).is_err());
    let receipt = service.confirm_request(&second.key, &context).unwrap();
    assert!(service.confirm_request(&second.key, &context).is_err());
    let rows = projected(&fixture, &execution);
    assert_eq!(rows[0].status, IssueCommentRequestStatus::Stale);
    assert_eq!(rows[0].receipt, None);
    assert_eq!(rows[1].receipt, Some(receipt));
    assert_eq!(execution.control.lock().unwrap().authorization_epoch, 1);
}

#[test]
fn recovered_pre_effect_request_rechecks_expiry_and_trust() {
    for (observed_at, revoke, expected_epoch) in
        [(now(), false, 1), (1, false, 0), (now(), true, 0)]
    {
        let (fixture, execution) = (Fixture::new(), Arc::new(ControlFixture::default()));
        let request = record(&fixture, &execution, "accepted-before-effect", observed_at);
        assert!(fixture.store.claim_request(&request).unwrap());
        if revoke {
            *fixture.policy.0.lock().unwrap() = None;
        }
        service(&fixture, &execution).recover_requests().unwrap();
        service(&fixture, &execution).recover_requests().unwrap();
        assert_eq!(
            execution.control.lock().unwrap().authorization_epoch,
            expected_epoch
        );
    }
}

#[test]
fn recovered_post_effect_request_retains_original_receipt_after_revocation() {
    let (fixture, execution) = (Fixture::new(), Arc::new(ControlFixture::default()));
    let request = record(&fixture, &execution, "after-effect", now());
    let live = service(&fixture, &execution);
    let proof = VerifiedHumanConfirmation::from_verified(
        live.prepare_confirmation(&request.key).unwrap().target,
        format!("desktop-confirmation:{}", "a".repeat(32)),
    )
    .unwrap();
    let receipt = live
        .confirm_request(
            &request.key,
            &StoredCommandContext {
                human_confirmation: Some(proof),
                ..comment_context(now())
            },
        )
        .unwrap();
    drop(live);
    *fixture.policy.0.lock().unwrap() = None;
    execution.control.lock().unwrap().authorization_epoch = 8;
    service(&fixture, &execution).recover_requests().unwrap();
    assert_eq!(projected(&fixture, &execution)[0].receipt, Some(receipt));
    assert_eq!(execution.control.lock().unwrap().authorization_epoch, 8);
}

#[test]
fn uncertain_recovery_never_replays_after_the_ledger_becomes_readable() {
    let (fixture, execution) = (Fixture::new(), Arc::new(ControlFixture::default()));
    let request = record(&fixture, &execution, "uncertain", now());
    assert!(fixture.store.claim_request(&request).unwrap());
    execution.unreadable.store(true, Ordering::SeqCst);
    service(&fixture, &execution).recover_requests().unwrap();
    execution.unreadable.store(false, Ordering::SeqCst);
    service(&fixture, &execution).recover_requests().unwrap();
    assert_eq!(
        projected(&fixture, &execution)[0].status,
        IssueCommentRequestStatus::Uncertain
    );
    assert_eq!(execution.control.lock().unwrap().authorization_epoch, 0);
}
