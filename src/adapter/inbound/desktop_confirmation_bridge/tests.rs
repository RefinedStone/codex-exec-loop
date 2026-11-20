use super::*;
use crate::application::port::inbound::stored_command_port::issue_confirmation::IssueConfirmationTarget;
use crate::domain::issue_automation::collection::IssueCollectionMode;

fn fixture() -> (Broker, PathBuf) {
    let root =
        crate::test_utils::platform_safe_temp_dir().join(format!("hc-{}", random_id(6).unwrap()));
    let files = SessionFiles::create(&root).unwrap();
    let broker = Broker {
        endpoint: files.endpoint(),
        files,
        registrations: HashMap::new(),
        proofs: HashMap::new(),
    };
    (broker, root)
}
fn intent(registration_id: &str) -> IssueConfirmationIntent {
    IssueConfirmationIntent {
        binding: IssueConfirmationBinding {
            registration_id: registration_id.to_owned(),
            connection_id: "a".repeat(32),
            attachment_id: "b".repeat(32),
            client_id: "client-1".into(),
            workspace: "/workspace".into(),
        },
        target: IssueConfirmationTarget::Mode {
            workspace: "/workspace".into(),
            from: None,
            epoch: None,
            revision: None,
            to: IssueCollectionMode::Observe,
        },
        key: "mode-key".into(),
        fingerprint: "c".repeat(32),
    }
}
fn result(reply: Reply) -> Value {
    reply.into_result().unwrap()
}
fn consume(
    broker: &mut Broker,
    registration_id: &str,
    token: &str,
    proof_id: &str,
    intent: IssueConfirmationIntent,
) -> Reply {
    match broker.admit(ClientRequest::Consume {
        registration_id: registration_id.into(),
        token: token.into(),
        proof_id: proof_id.into(),
        intent: Box::new(intent),
    }) {
        Admission::Reply(reply) => reply,
        _ => panic!("consume cannot watch"),
    }
}
#[test]
fn external_ipc_cannot_approve_even_with_a_registration_token() {
    assert!(serde_json::from_value::<ClientRequest>(json!({
        "kind":"approve","registration_id":"a".repeat(32),"token":"b".repeat(64),"intent":intent(&"a".repeat(32))
    })).is_err());
    assert!(serde_json::from_value::<ClientRequest>(json!({
        "kind":"bind","registration_id":"a".repeat(32),"token":"b".repeat(64),"binding":intent(&"a".repeat(32)).binding,
        "approve":true
    })).is_err());
}
#[test]
fn approval_requires_a_live_exact_binding_and_consumption_only_retries_the_identical_action() {
    let (mut broker, root) = fixture();
    let prepared = result(broker.prepare().unwrap());
    let id = prepared["registration_id"].as_str().unwrap().to_owned();
    let token = broker.registrations[&id].token.clone();
    let expected = intent(&id);
    assert!(
        broker
            .approve(&id, expected.clone())
            .unwrap()
            .into_result()
            .is_err()
    );
    assert!(matches!(
        broker.admit(ClientRequest::Bind {
            registration_id: id.clone(),
            token: token.clone(),
            binding: expected.binding.clone()
        }),
        Admission::Reply(Reply::Result { .. })
    ));
    assert!(
        broker
            .approve(&id, expected.clone())
            .unwrap()
            .into_result()
            .is_err()
    );
    let watch = broker.admit(ClientRequest::Watch {
        registration_id: id.clone(),
        token: token.clone(),
        binding: expected.binding.clone(),
    });
    assert!(matches!(watch, Admission::Watch { .. }));
    let proof_id = result(broker.approve(&id, expected.clone()).unwrap())["proof_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let first = result(consume(
        &mut broker,
        &id,
        &token,
        &proof_id,
        expected.clone(),
    ));
    assert_ne!(first["confirmation_ref"], proof_id);
    assert_eq!(
        first["confirmation_ref"].as_str().unwrap().len(),
        "desktop-confirmation:".len() + 32
    );
    assert_eq!(
        first,
        result(consume(
            &mut broker,
            &id,
            &token,
            &proof_id,
            expected.clone()
        ))
    );
    let mut variants = Vec::new();
    let mut changed = expected.clone();
    changed.key = "other-key".into();
    variants.push(changed);
    let mut changed = expected.clone();
    changed.fingerprint = "d".repeat(32);
    variants.push(changed);
    let mut changed = expected.clone();
    changed.binding.client_id = "other-client".into();
    variants.push(changed);
    let mut changed = expected.clone();
    changed.binding.attachment_id = "d".repeat(32);
    variants.push(changed);
    let mut changed = expected.clone();
    changed.binding.connection_id = "d".repeat(32);
    variants.push(changed);
    let mut changed = expected.clone();
    changed.binding.workspace = "/other".into();
    variants.push(changed);
    let mut changed = expected.clone();
    changed.target = IssueConfirmationTarget::Mode {
        workspace: "/workspace".into(),
        from: Some(IssueCollectionMode::Off),
        epoch: Some(1),
        revision: Some(1),
        to: IssueCollectionMode::Observe,
    };
    variants.push(changed);
    for changed in variants {
        assert!(
            consume(&mut broker, &id, &token, &proof_id, changed)
                .into_result()
                .is_err()
        );
    }
    let denied = serde_json::to_string(&consume(
        &mut broker,
        &id,
        "raw-secret-canary",
        &proof_id,
        expected.clone(),
    ))
    .unwrap();
    assert!(!denied.contains("raw-secret-canary"));
    broker.revoke(&id).unwrap();
    assert!(
        consume(&mut broker, &id, &token, &proof_id, expected)
            .into_result()
            .is_err()
    );
    assert!(broker.proofs.is_empty());
    drop(watch);
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn expired_proof_is_not_reused_or_retained() {
    let (mut broker, root) = fixture();
    let prepared = result(broker.prepare().unwrap());
    let id = prepared["registration_id"].as_str().unwrap().to_owned();
    let expected = intent(&id);
    let registration = broker.registrations.get_mut(&id).unwrap();
    registration.binding = Some(expected.binding.clone());
    registration.watching = true;
    let proof_id = result(broker.approve(&id, expected).unwrap())["proof_id"]
        .as_str()
        .unwrap()
        .to_owned();
    broker.proofs.get_mut(&proof_id).unwrap().expires = Instant::now() - Duration::from_secs(1);
    broker.expire();
    assert!(broker.proofs.is_empty());
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn registration_reader_rejects_browser_authority_and_other_runtime_locations() {
    let (mut broker, root) = fixture();
    let prepared = result(broker.prepare().unwrap());
    let id = prepared["registration_id"].as_str().unwrap();
    let path = PathBuf::from(prepared["session_file"].as_str().unwrap());
    assert!(storage::read_session(&path, id, &root).is_ok());
    let outside = root.join("other-runtime");
    std::fs::create_dir(&outside).unwrap();
    assert!(storage::read_session(&path, id, &outside).is_err());
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["purpose"] = json!("desktop-browser");
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(storage::read_session(&path, id, &root).is_err());
    std::fs::remove_dir(outside).unwrap();
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn registration_reader_rejects_shared_permissions_links_and_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let (mut broker, root) = fixture();
    let prepared = result(broker.prepare().unwrap());
    let id = prepared["registration_id"].as_str().unwrap();
    let path = PathBuf::from(prepared["session_file"].as_str().unwrap());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(storage::read_session(&path, id, &root).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let alias = root.join("linked-session");
    std::fs::hard_link(&path, &alias).unwrap();
    assert!(storage::read_session(&path, id, &root).is_err());
    std::fs::remove_file(&alias).unwrap();
    let original = root.join("original-session");
    std::fs::rename(&path, &original).unwrap();
    symlink(&original, &path).unwrap();
    assert!(storage::read_session(&path, id, &root).is_err());
    std::fs::remove_file(original).unwrap();
    drop(broker);
    std::fs::remove_dir_all(root).unwrap();
}
