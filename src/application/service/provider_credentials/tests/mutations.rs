use super::*;
use crate::application::port::outbound::provider_credential_mutation_port::ProviderCredentialMutationPort;

fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "credential fence was not committed"
        );
        std::thread::yield_now();
    }
}

#[test]
fn prepared_send_cannot_adopt_a_replacement_or_disconnected_revision() {
    let fixture = Harness::new();
    fixture.connect("first", 0);
    let first = fixture.active();
    let replacement_race = fixture.service.authorize_mutation(&first).unwrap();
    fixture.connect("second", 1);
    assert!(replacement_race.begin_send().is_err());
    let second = fixture.active();
    let disconnect_race = fixture.service.authorize_mutation(&second).unwrap();
    fixture.service.disconnect(disconnect("remove", 2)).unwrap();
    assert!(disconnect_race.begin_send().is_err());
    assert!(fixture.service.authorize_mutation(&second).is_err());
}

#[test]
fn disconnect_commits_its_fence_but_returns_only_after_the_admitted_response() {
    let fixture = Harness::new();
    fixture.connect("first", 0);
    let first = fixture.active();
    let authorization = fixture.service.authorize_mutation(&first).unwrap();
    let response_guard = authorization.begin_send().unwrap();
    assert!(
        authorization.begin_send().is_err(),
        "a transport cannot retry the mutation"
    );
    let late = fixture.service.authorize_mutation(&first).unwrap();
    let (done, observed) = mpsc::sync_channel(1);
    let service = fixture.service.clone();
    let worker = std::thread::spawn(move || {
        done.send(service.disconnect(disconnect("remove", 1)))
            .unwrap();
    });
    wait_for(|| {
        fixture
            .service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    });
    assert!(late.begin_send().is_err());
    assert!(observed.recv_timeout(Duration::from_millis(25)).is_err());
    drop(response_guard); // An ACK, transport failure, or unknown outcome all release this guard.
    assert_eq!(
        observed
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap()
            .status,
        CredentialOperationStatus::Committed
    );
    worker.join().unwrap();
    assert_eq!(
        fixture
            .service
            .disconnect(disconnect("remove", 1))
            .unwrap()
            .status,
        CredentialOperationStatus::Committed
    );
}

#[test]
fn rotation_keeps_new_reads_current_while_old_mutation_completion_is_drained() {
    let fixture = Harness::new();
    fixture.connect("first", 0);
    let old = fixture.active();
    let response_guard = fixture
        .service
        .authorize_mutation(&old)
        .unwrap()
        .begin_send()
        .unwrap();
    let delayed = fixture.service.authorize_mutation(&old).unwrap();
    let service = fixture.service.clone();
    let (done, observed) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        done.send(service.start_connect(request("second", 1), owner("second")))
            .unwrap();
    });
    wait_for(|| fixture.active().revision == 2);
    assert!(delayed.begin_send().is_err());
    fixture.service.validate_current(&fixture.active()).unwrap();
    assert!(observed.recv_timeout(Duration::from_millis(25)).is_err());
    drop(response_guard);
    assert_eq!(
        observed
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap()
            .status,
        CredentialOperationStatus::Committed
    );
    worker.join().unwrap();
}

#[test]
fn shutdown_closes_prepared_sends_and_drains_the_owned_in_flight_guard() {
    let fixture = Harness::new();
    fixture.connect("first", 0);
    let first = fixture.active();
    let admitted = fixture
        .service
        .authorize_mutation(&first)
        .unwrap()
        .begin_send()
        .unwrap();
    let late = fixture.service.authorize_mutation(&first).unwrap();
    let service = fixture.service.clone();
    let (done, observed) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || done.send(service.shutdown()).unwrap());
    wait_for(|| fixture.service.closing.load(Ordering::SeqCst));
    assert!(late.begin_send().is_err());
    assert!(observed.recv_timeout(Duration::from_millis(25)).is_err());
    drop(admitted);
    observed
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
}
