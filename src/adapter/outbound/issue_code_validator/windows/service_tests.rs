use super::tests::Fixture;
use crate::adapter::outbound::issue_code_validator::SandboxedIssueCodeValidatorRunnerAdapter;
use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorCancellation, IssueCodeValidatorNotRun as NotRun,
    IssueCodeValidatorRunOutcome as Outcome,
};
use crate::application::service::issue_code_validator::{
    IssueCodeValidatorResult, gate_issue_publish_with_code_validators,
};
use crate::application::service::issue_validation::IssuePublishBlocked;
use crate::domain::issue_policy::{
    IssueApprovedCodeValidator, IssueCodeValidatorSelection, IssueDisclosureTarget, IssuePolicy,
};
use crate::domain::issue_validation::IssueRequirements;
use crate::domain::issue_work::IssueSharedCopy;
use sha2::{Digest, Sha256};

#[test]
fn native_windows_required_validator_service_gate_uses_real_sandbox_outcomes() {
    let fixture = Fixture::create();
    let request = fixture.request("pass");
    let mut policy = IssuePolicy::builtin();
    policy.code_validators = vec![IssueCodeValidatorSelection {
        id: "native-windows-canary".to_owned(),
        required: true,
        targets: vec![IssueDisclosureTarget::Publish],
    }];
    policy.approved_code_validators = vec![IssueApprovedCodeValidator {
        id: "native-windows-canary".to_owned(),
        command: request.command.clone(),
        sha256: request.sha256,
        timeout_seconds: 1,
    }];
    let requirements = IssueRequirements {
        template_fields: Vec::new(),
        required_sections: vec!["Steps".to_owned()],
    };
    let runner = SandboxedIssueCodeValidatorRunnerAdapter::new();
    let copy = |mode: &str| IssueSharedCopy {
        title: mode.to_owned(),
        body: "## Steps\n\n1. run [masked-1]\n".to_owned(),
    };
    let passed = gate_issue_publish_with_code_validators(
        &copy("pass"),
        Some(&requirements),
        &policy,
        &runner,
        &IssueCodeValidatorCancellation::new(),
    )
    .expect("actual native validator passes");
    assert_eq!(passed.shared_copy(), &copy("pass"));
    for (mode, expected) in [
        (
            "reject",
            Outcome::Rejected {
                message: "synthetic validation failed".to_owned(),
            },
        ),
        ("crash", Outcome::NotRun(NotRun::Crashed)),
        ("timeout", Outcome::NotRun(NotRun::TimedOut)),
    ] {
        let blocked = gate_issue_publish_with_code_validators(
            &copy(mode),
            Some(&requirements),
            &policy,
            &runner,
            &IssueCodeValidatorCancellation::new(),
        )
        .unwrap_err();
        assert_eq!(
            blocked,
            IssuePublishBlocked::CodeValidator(vec![IssueCodeValidatorResult {
                id: "native-windows-canary".to_owned(),
                required: true,
                outcome: expected,
            }])
        );
    }
    policy.approved_code_validators[0].sha256 = "0".repeat(64);
    let assert_blocked = |policy: &IssuePolicy, expected| {
        assert_eq!(
            gate_issue_publish_with_code_validators(
                &copy("pass"),
                Some(&requirements),
                policy,
                &runner,
                &IssueCodeValidatorCancellation::new(),
            )
            .unwrap_err(),
            IssuePublishBlocked::CodeValidator(vec![IssueCodeValidatorResult {
                id: "native-windows-canary".to_owned(),
                required: true,
                outcome: Outcome::NotRun(expected),
            }])
        );
    };
    assert_blocked(&policy, NotRun::HashMismatch);
    let script = b"@echo off\r\nexit 0\r\n";
    std::fs::write(&request.command, script).unwrap();
    policy.approved_code_validators[0].sha256 = format!("{:x}", Sha256::digest(script));
    assert_blocked(&policy, NotRun::IsolationUnavailable);
}
