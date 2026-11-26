use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::Serialize;

use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorCancellation, IssueCodeValidatorNotRun, IssueCodeValidatorRunOutcome,
    IssueCodeValidatorRunRequest, IssueCodeValidatorRunnerPort,
};
use crate::application::service::issue_validation::{IssuePublishBlocked, gate_issue_publish};
use crate::domain::issue_policy::{IssueDisclosureTarget, IssuePolicy};
use crate::domain::issue_validation::{IssueRequirements, IssueValidationPassed};
use crate::domain::issue_work::IssueSharedCopy;

/*
 * 사용자가 승인한 코드 검증기를 정책에 따라 실행한다 (#2431).
 *
 * 입력은 `IssueValidationPassed` 뿐이다: 필수 검증을 통과한 공유본만 검증기에 간다. 정책은 ID 만 고르고,
 * 명령과 내용 해시는 사용자 파일의 승인 목록에서 온다. 필수 검증기가 통과하지 않으면 게시 관문이 막는다.
 *
 * 사용자 취소 (#2723): 클라이언트가 게시 요청에 검증 ID 를 주면 같은 ID 로 실행 중인 검증을 취소할 수 있다.
 * 취소를 받으면 실행 중인 검증기를 끝내고, 결과 확정 전에 받은 취소는 늦게 끝난 성공도 버리고 게시를 막는다.
 * 결과를 확정한 뒤의 취소는 받지 않는다 (`false`). 그 뒤의 원격 쓰기는 이 취소로 멈추지 않는다.
 */

/// 검증기 stdin JSON 의 판이다. 칸의 뜻이 바뀌면 올린다.
pub const ISSUE_CODE_VALIDATOR_INPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IssueCodeValidatorInput<'a> {
    schema_version: u32,
    target: IssueDisclosureTarget,
    title: &'a str,
    body: &'a str,
}

/// 정책이 고른 검증기 하나의 결과이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueCodeValidatorResult {
    pub id: String,
    pub required: bool,
    pub outcome: IssueCodeValidatorRunOutcome,
}

impl IssueCodeValidatorResult {
    /// 필수 검증기가 통과하지 않았다. 이 결과는 게시를 막는다.
    pub fn blocks_publish(&self) -> bool {
        self.required && self.outcome != IssueCodeValidatorRunOutcome::Passed
    }
}

/// `target` 에 적용하는 검증기를 정책의 순서로 실행한다. 승인 목록에 없는 ID 는 실행하지 않는다.
pub fn run_issue_code_validators(
    passed: &IssueValidationPassed,
    policy: &IssuePolicy,
    target: IssueDisclosureTarget,
    runner: &dyn IssueCodeValidatorRunnerPort,
    cancellation: &IssueCodeValidatorCancellation,
) -> Vec<IssueCodeValidatorResult> {
    let shared_copy = passed.shared_copy();
    let input_json = serde_json::to_string(&IssueCodeValidatorInput {
        schema_version: ISSUE_CODE_VALIDATOR_INPUT_SCHEMA_VERSION,
        target,
        title: &shared_copy.title,
        body: &shared_copy.body,
    })
    .expect("issue code validator input is plain text fields");

    policy
        .code_validators
        .iter()
        .filter(|selection| selection.targets.contains(&target))
        .map(|selection| {
            let approved = policy
                .approved_code_validators
                .iter()
                .find(|approved| approved.id == selection.id);
            let outcome = match approved {
                None => IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::NotApproved),
                // 취소한 검증의 남은 검증기는 시작하지 않는다.
                Some(_) if cancellation.is_cancelled() => {
                    IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Cancelled)
                }
                Some(approved) => runner.run(
                    IssueCodeValidatorRunRequest {
                        command: approved.command.clone(),
                        sha256: approved.sha256.clone(),
                        timeout: Duration::from_secs(approved.timeout_seconds),
                        input_json: input_json.clone(),
                    },
                    cancellation,
                ),
            };
            IssueCodeValidatorResult {
                id: selection.id.clone(),
                required: selection.required,
                outcome,
            }
        })
        .collect()
}

/// 코드 검증기가 있는 게시 관문이다. 먼저 필수 규약 관문(`gate_issue_publish`)을 지나고, 그 통과 결과로
/// 검증기를 실행한다. 필수 검증기가 하나라도 통과하지 않으면 게시를 막는다.
/// 결과를 확정하기 전에 `cancellation` 이 취소를 받았으면 결과와 상관없이 `Cancelled` 로 막는다.
pub fn gate_issue_publish_with_code_validators(
    shared_copy: &IssueSharedCopy,
    requirements: Option<&IssueRequirements>,
    policy: &IssuePolicy,
    runner: &dyn IssueCodeValidatorRunnerPort,
    cancellation: &IssueCodeValidatorCancellation,
) -> std::result::Result<IssueValidationPassed, IssuePublishBlocked> {
    let passed = gate_issue_publish(shared_copy, requirements)?;
    let results = run_issue_code_validators(
        &passed,
        policy,
        IssueDisclosureTarget::Publish,
        runner,
        cancellation,
    );
    if !cancellation.finish() {
        return Err(IssuePublishBlocked::Cancelled);
    }
    let blocking: Vec<IssueCodeValidatorResult> = results
        .into_iter()
        .filter(IssueCodeValidatorResult::blocks_publish)
        .collect();
    if blocking.is_empty() {
        Ok(passed)
    } else {
        Err(IssuePublishBlocked::CodeValidator(blocking))
    }
}

/// 실행 중인 게시 검증을 클라이언트의 검증 ID 로 찾는다. 게시 요청이 끝나면 항목을 지운다.
#[derive(Debug, Default)]
pub struct IssueCodeValidationRegistry {
    running: Mutex<HashMap<String, Arc<IssueCodeValidatorCancellation>>>,
}

impl IssueCodeValidationRegistry {
    /// 게시 검증 하나를 시작한다. ID 가 없으면 취소할 수 없는 검증이다. 같은 ID 가 실행 중이면 거절한다.
    pub fn start(&self, validation_id: Option<&str>) -> Result<IssueCodeValidation<'_>> {
        let cancellation = Arc::new(IssueCodeValidatorCancellation::new());
        if let Some(id) = validation_id {
            let mut running = self.lock();
            if running.contains_key(id) {
                return Err(anyhow!("the issue validation id is already running"));
            }
            running.insert(id.to_string(), cancellation.clone());
        }
        Ok(IssueCodeValidation {
            registry: self,
            validation_id: validation_id.map(str::to_string),
            cancellation,
        })
    }

    /// 실행 중인 검증이 취소를 받으면 `true` 이다. 이미 끝났거나 없는 ID 이면 `false` 이다.
    pub fn cancel(&self, validation_id: &str) -> bool {
        self.lock()
            .get(validation_id)
            .is_some_and(|cancellation| cancellation.cancel())
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, Arc<IssueCodeValidatorCancellation>>> {
        self.running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 시작한 게시 검증이다. 사라질 때 등록을 지운다.
pub struct IssueCodeValidation<'a> {
    registry: &'a IssueCodeValidationRegistry,
    validation_id: Option<String>,
    cancellation: Arc<IssueCodeValidatorCancellation>,
}

impl IssueCodeValidation<'_> {
    pub fn cancellation(&self) -> &IssueCodeValidatorCancellation {
        &self.cancellation
    }
}

impl Drop for IssueCodeValidation<'_> {
    fn drop(&mut self) {
        if let Some(id) = &self.validation_id {
            self.registry.lock().remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::domain::issue_policy::{IssueApprovedCodeValidator, IssueCodeValidatorSelection};

    /// 가짜 실행기이다. 명령마다 정한 결과를 주고 받은 요청을 기록한다.
    struct FakeRunner {
        outcome: IssueCodeValidatorRunOutcome,
        requests: Mutex<Vec<IssueCodeValidatorRunRequest>>,
    }

    impl FakeRunner {
        fn new(outcome: IssueCodeValidatorRunOutcome) -> Self {
            Self {
                outcome,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl IssueCodeValidatorRunnerPort for FakeRunner {
        fn run(
            &self,
            request: IssueCodeValidatorRunRequest,
            _: &IssueCodeValidatorCancellation,
        ) -> IssueCodeValidatorRunOutcome {
            self.requests.lock().expect("requests").push(request);
            self.outcome.clone()
        }
    }

    /// 실행 중에 취소를 받는 가짜 실행기이다. 받은 뒤에 `outcome` 으로 끝난다: 취소와 겹친 늦은 결과이다.
    struct CancelledWhileRunning {
        outcome: IssueCodeValidatorRunOutcome,
        accepted: Mutex<Vec<bool>>,
    }

    impl IssueCodeValidatorRunnerPort for CancelledWhileRunning {
        fn run(
            &self,
            _: IssueCodeValidatorRunRequest,
            cancellation: &IssueCodeValidatorCancellation,
        ) -> IssueCodeValidatorRunOutcome {
            self.accepted
                .lock()
                .expect("accepted")
                .push(cancellation.cancel());
            self.outcome.clone()
        }
    }

    fn gate(
        policy: &IssuePolicy,
        runner: &dyn IssueCodeValidatorRunnerPort,
    ) -> std::result::Result<IssueValidationPassed, IssuePublishBlocked> {
        gate_issue_publish_with_code_validators(
            &valid_copy(),
            Some(&requirements()),
            policy,
            runner,
            &IssueCodeValidatorCancellation::new(),
        )
    }

    fn valid_copy() -> IssueSharedCopy {
        IssueSharedCopy {
            title: "Title".to_string(),
            body: "## Steps\n\n1. run [masked-1]\n".to_string(),
        }
    }

    fn requirements() -> IssueRequirements {
        IssueRequirements {
            template_fields: Vec::new(),
            required_sections: vec!["Steps".to_string()],
        }
    }

    fn policy(required: bool, approved: bool, targets: Vec<IssueDisclosureTarget>) -> IssuePolicy {
        let mut policy = IssuePolicy::builtin();
        policy.code_validators = vec![IssueCodeValidatorSelection {
            id: "team-rules".to_string(),
            targets,
            required,
        }];
        if approved {
            policy.approved_code_validators = vec![IssueApprovedCodeValidator {
                id: "team-rules".to_string(),
                command: "/opt/validators/team-rules".to_string(),
                sha256: "a".repeat(64),
                timeout_seconds: 7,
            }];
        }
        policy
    }

    fn publish_policy(required: bool) -> IssuePolicy {
        policy(required, true, vec![IssueDisclosureTarget::Publish])
    }

    #[test]
    fn approved_validator_gets_only_the_validated_shared_copy() {
        let runner = FakeRunner::new(IssueCodeValidatorRunOutcome::Passed);

        let passed = gate(&publish_policy(true), &runner).expect("allowed");

        assert_eq!(passed.shared_copy(), &valid_copy());
        let requests = runner.requests.lock().expect("requests");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].command, "/opt/validators/team-rules");
        assert_eq!(requests[0].sha256, "a".repeat(64));
        assert_eq!(requests[0].timeout, Duration::from_secs(7));
        assert_eq!(
            requests[0].input_json,
            r###"{"schemaVersion":1,"target":"publish","title":"Title","body":"## Steps\n\n1. run [masked-1]\n"}"###
        );
    }

    #[test]
    fn required_validator_that_rejects_or_does_not_run_blocks_publish() {
        let outcomes = [
            IssueCodeValidatorRunOutcome::Rejected {
                message: "no ticket".to_string(),
            },
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Missing),
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::HashMismatch),
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::IsolationUnavailable),
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::TimedOut),
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Crashed),
        ];

        for outcome in outcomes {
            let runner = FakeRunner::new(outcome.clone());

            let blocked = gate(&publish_policy(true), &runner).expect_err("blocked");

            assert_eq!(
                blocked,
                IssuePublishBlocked::CodeValidator(vec![IssueCodeValidatorResult {
                    id: "team-rules".to_string(),
                    required: true,
                    outcome,
                }])
            );
        }
    }

    #[test]
    fn selected_validator_that_the_user_did_not_approve_never_runs() {
        let runner = FakeRunner::new(IssueCodeValidatorRunOutcome::Passed);
        let policy = policy(true, false, vec![IssueDisclosureTarget::Publish]);

        let blocked = gate(&policy, &runner).expect_err("blocked");

        assert!(runner.requests.lock().expect("requests").is_empty());
        assert_eq!(
            blocked,
            IssuePublishBlocked::CodeValidator(vec![IssueCodeValidatorResult {
                id: "team-rules".to_string(),
                required: true,
                outcome: IssueCodeValidatorRunOutcome::NotRun(
                    IssueCodeValidatorNotRun::NotApproved
                ),
            }])
        );
    }

    #[test]
    fn optional_validator_failure_does_not_block_publish() {
        let runner = FakeRunner::new(IssueCodeValidatorRunOutcome::NotRun(
            IssueCodeValidatorNotRun::TimedOut,
        ));

        let allowed = gate(&publish_policy(false), &runner);

        assert!(allowed.is_ok());
    }

    #[test]
    fn validators_do_not_run_when_mandatory_validation_fails_or_the_target_differs() {
        let runner = FakeRunner::new(IssueCodeValidatorRunOutcome::Passed);
        let invalid = IssueSharedCopy {
            title: "Title".to_string(),
            body: "no sections".to_string(),
        };

        let blocked = gate_issue_publish_with_code_validators(
            &invalid,
            Some(&requirements()),
            &publish_policy(true),
            &runner,
            &IssueCodeValidatorCancellation::new(),
        )
        .expect_err("blocked");
        assert!(matches!(blocked, IssuePublishBlocked::Invalid(_)));

        // AI 전달에만 고른 검증기는 게시에서 실행하지 않는다.
        let ai_only = policy(true, true, vec![IssueDisclosureTarget::AiHandoff]);
        assert!(gate(&ai_only, &runner).is_ok());
        assert!(runner.requests.lock().expect("requests").is_empty());
    }

    #[test]
    fn cancel_during_the_run_blocks_publish_even_when_the_validator_then_passes() {
        for outcome in [
            IssueCodeValidatorRunOutcome::NotRun(IssueCodeValidatorNotRun::Cancelled),
            // 취소와 같은 때에 끝난 성공이다. 취소를 먼저 받았으므로 버린다.
            IssueCodeValidatorRunOutcome::Passed,
        ] {
            let runner = CancelledWhileRunning {
                outcome,
                accepted: Mutex::new(Vec::new()),
            };
            for required in [true, false] {
                assert_eq!(
                    gate(&publish_policy(required), &runner),
                    Err(IssuePublishBlocked::Cancelled)
                );
            }
            assert_eq!(*runner.accepted.lock().expect("accepted"), [true, true]);
        }
    }

    #[test]
    fn cancel_before_the_run_starts_no_validator_and_blocks_publish() {
        let runner = FakeRunner::new(IssueCodeValidatorRunOutcome::Passed);
        let cancellation = IssueCodeValidatorCancellation::new();
        assert!(cancellation.cancel());

        let blocked = gate_issue_publish_with_code_validators(
            &valid_copy(),
            Some(&requirements()),
            &publish_policy(true),
            &runner,
            &cancellation,
        );

        assert_eq!(blocked, Err(IssuePublishBlocked::Cancelled));
        assert!(runner.requests.lock().expect("requests").is_empty());
    }

    #[test]
    fn cancel_after_the_result_is_final_is_refused_and_keeps_the_result() {
        let runner = FakeRunner::new(IssueCodeValidatorRunOutcome::Passed);
        let cancellation = IssueCodeValidatorCancellation::new();

        let passed = gate_issue_publish_with_code_validators(
            &valid_copy(),
            Some(&requirements()),
            &publish_policy(true),
            &runner,
            &cancellation,
        );

        assert!(passed.is_ok());
        assert!(!cancellation.cancel());
    }

    #[test]
    fn registry_cancels_only_a_running_validation_and_forgets_it_when_done() {
        let registry = IssueCodeValidationRegistry::default();
        assert!(!registry.cancel("v1"));
        {
            let validation = registry.start(Some("v1")).expect("started");
            assert!(registry.start(Some("v1")).is_err());
            assert!(registry.cancel("v1"));
            assert!(validation.cancellation().is_cancelled());
        }
        assert!(!registry.cancel("v1"));
        {
            let validation = registry.start(Some("v1")).expect("the id is free again");
            assert!(validation.cancellation().finish());
            assert!(!registry.cancel("v1"));
        }
        let anonymous = registry.start(None).expect("started");
        assert!(!anonymous.cancellation().is_cancelled());
    }
}
