use super::*;
use crate::domain::issue_work::IssueFieldHash;

fn target() -> IssueConfirmationTarget {
    IssueConfirmationTarget::Decision {
        workspace: "/workspace/repo".into(),
        decision_id: IssueDecisionId::new("decision-1").unwrap(),
        option: "accept".into(),
        basis: IssueDecisionBasis {
            delivery: None,
            authorization_epoch: 0,
            policy_version: "a".repeat(64),
            field_hash: Some(IssueFieldHash::of_fields("shared", "body")),
            work_order_id: None,
            gate_revision: 1,
        },
        decision_revision: 1,
    }
}

fn reference() -> String {
    format!("desktop-confirmation:{}", "b".repeat(32))
}

// Serialize/Deserialize가 추가되면 두 impl이 겹쳐 아래 증명이 컴파일되지 않는다.
trait NotSerialize<Marker> {
    fn proven() {}
}
impl<T: ?Sized> NotSerialize<()> for T {}
impl<T: ?Sized + Serialize> NotSerialize<u8> for T {}
trait NotDeserialize<Marker> {
    fn proven() {}
}
impl<T: ?Sized> NotDeserialize<()> for T {}
impl<T: serde::de::DeserializeOwned> NotDeserialize<u8> for T {}

#[test]
fn verified_capabilities_cannot_be_encoded_or_created_from_transport_json() {
    <VerifiedHumanConfirmation as NotSerialize<_>>::proven();
    <VerifiedHumanConfirmation as NotDeserialize<_>>::proven();
    let confirmed = VerifiedHumanConfirmation::from_verified(target(), reference()).unwrap();
    assert_eq!(confirmed.target(), &target());
    assert_eq!(confirmed.reference(), reference());
    let debug = format!("{confirmed:?}");
    assert!(!debug.contains("decision-1"));
    assert!(!debug.contains(&reference()));
}

#[test]
fn a_verified_capability_remains_bound_to_every_action_coordinate() {
    let confirmed = VerifiedHumanConfirmation::from_verified(target(), reference()).unwrap();
    confirmed.require_target(&target()).unwrap();
    let mut changed = target();
    if let IssueConfirmationTarget::Decision { option, .. } = &mut changed {
        *option = "reject".into();
    }
    assert_eq!(
        confirmed.require_target(&changed),
        Err(IssueConfirmationError::ScopeMismatch)
    );
    let mode = IssueConfirmationTarget::Mode {
        workspace: "/workspace/repo".into(),
        from: Some(IssueCollectionMode::Off),
        epoch: Some(1),
        revision: Some(1),
        to: IssueCollectionMode::Observe,
    };
    assert_eq!(
        confirmed.require_target(&mode),
        Err(IssueConfirmationError::ScopeMismatch)
    );
}

#[test]
fn audit_references_are_distinct_from_raw_bearer_proofs() {
    for value in [
        "raw-proof",
        "desktop-confirmation:",
        "desktop-confirmation:secret",
    ] {
        assert_eq!(
            VerifiedHumanConfirmation::from_verified(target(), value.to_owned()),
            Err(IssueConfirmationError::Invalid),
        );
    }
}

#[test]
fn mode_scope_requires_one_complete_valid_control_snapshot() {
    let mut target = IssueConfirmationTarget::Mode {
        workspace: "/workspace/repo".into(),
        from: Some(IssueCollectionMode::Off),
        epoch: Some(2),
        revision: Some(5),
        to: IssueCollectionMode::Observe,
    };
    target.validate().unwrap();
    if let IssueConfirmationTarget::Mode { revision, .. } = &mut target {
        *revision = None;
    }
    assert_eq!(target.validate(), Err(IssueConfirmationError::Invalid));
    if let IssueConfirmationTarget::Mode { revision, .. } = &mut target {
        *revision = Some(1);
    }
    assert_eq!(target.validate(), Err(IssueConfirmationError::Invalid));
    let absent = IssueConfirmationTarget::Mode {
        workspace: "/workspace/repo".into(),
        from: None,
        epoch: None,
        revision: None,
        to: IssueCollectionMode::Observe,
    };
    absent.validate().unwrap();
}

#[test]
fn serialized_action_metadata_does_not_accept_unknown_authority_fields() {
    let encoded = serde_json::to_value(target()).unwrap();
    let mut forged = encoded.clone();
    forged["human"] = true.into();
    assert!(serde_json::from_value::<IssueConfirmationTarget>(forged).is_err());
    assert_eq!(
        serde_json::from_value::<IssueConfirmationTarget>(encoded).unwrap(),
        target()
    );
}

fn autonomy_target() -> IssueConfirmationTarget {
    IssueConfirmationTarget::Autonomy {
        workspace: "/workspace/repo".into(),
        command_id: "autonomy-1".into(),
        from: IssueAutomationAutonomy::Observe,
        revision: 0,
        policy_version: "a".repeat(64),
        cap: IssueAutomationAutonomy::Auto,
        to: IssueAutonomyLevel::Propose,
    }
}

#[test]
fn autonomy_confirmation_accepts_initial_revision_and_binds_every_authority_coordinate() {
    let target = autonomy_target();
    target.validate().unwrap();
    let verified = VerifiedHumanConfirmation::from_verified(target.clone(), reference()).unwrap();
    verified.require_target(&target).unwrap();
    let encoded = serde_json::to_value(&target).unwrap();
    for (field, changed) in [
        ("workspace", serde_json::json!("/workspace/other")),
        ("command_id", serde_json::json!("autonomy-2")),
        ("from", serde_json::json!("off")),
        ("revision", serde_json::json!(1)),
        ("policy_version", serde_json::json!("b".repeat(64))),
        ("cap", serde_json::json!("propose")),
        ("to", serde_json::json!("observe")),
    ] {
        let mut changed_target = encoded.clone();
        changed_target[field] = changed;
        let changed_target: IssueConfirmationTarget =
            serde_json::from_value(changed_target).unwrap();
        changed_target.validate().unwrap();
        assert_eq!(
            verified.require_target(&changed_target),
            Err(IssueConfirmationError::ScopeMismatch),
            "{field}"
        );
    }
}

#[test]
fn autonomy_confirmation_rejects_invalid_identifiers_policy_and_exhausted_revision() {
    for (field, invalid) in [
        ("command_id", serde_json::json!("")),
        ("command_id", serde_json::json!(" padded")),
        ("command_id", serde_json::json!("x".repeat(257))),
        ("policy_version", serde_json::json!("a".repeat(63))),
        ("policy_version", serde_json::json!("z".repeat(64))),
        ("revision", serde_json::json!(MAX_ISSUE_AUTOMATION_REVISION)),
    ] {
        let mut encoded = serde_json::to_value(autonomy_target()).unwrap();
        encoded[field] = invalid;
        let target: IssueConfirmationTarget = serde_json::from_value(encoded).unwrap();
        assert_eq!(
            target.validate(),
            Err(IssueConfirmationError::Invalid),
            "{field}"
        );
        assert_eq!(
            VerifiedHumanConfirmation::from_verified(target, reference()),
            Err(IssueConfirmationError::Invalid)
        );
    }
}
