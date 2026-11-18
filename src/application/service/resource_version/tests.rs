use std::sync::Arc;

use super::*;

fn next_request(
    receiver: &mut async_mpsc::Receiver<ResourceVersionRequest>,
) -> ResourceVersionRequest {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap()
        })
}

fn resource() -> ExternalResourceRef {
    ExternalResourceRef {
        kind: "jira".into(),
        key: "acme.example/APP-1".into(),
        url: "https://acme.example/browse/APP-1".into(),
        title_hint: None,
    }
}

#[test]
fn only_the_selected_session_can_answer() {
    let service = Arc::new(ResourceVersionService::default());
    let (first, mut requests) = service.open().unwrap();
    let (second, _standby) = service.open().unwrap();
    service
        .register(first, Some("https://acme.example"))
        .unwrap();
    service
        .register(second, Some("https://acme.example"))
        .unwrap();
    let worker = service.clone();
    let lookup = std::thread::spawn(move || worker.current_version(&resource()));
    let request = next_request(&mut requests);
    service.answer(second, &request.id, Ok("2026-11-01T00:00:00Z".into()));
    service.answer(first, &request.id, Ok("2026-11-17T00:00:00Z".into()));
    assert_eq!(lookup.join().unwrap().unwrap(), "2026-11-17T00:00:00Z");
}

#[test]
fn disconnect_cancels_pending_and_the_next_request_uses_a_standby() {
    let service = Arc::new(ResourceVersionService::default());
    let (first, mut requests) = service.open().unwrap();
    let (second, mut standby) = service.open().unwrap();
    service
        .register(first, Some("https://acme.example"))
        .unwrap();
    service
        .register(second, Some("https://acme.example"))
        .unwrap();
    let worker = service.clone();
    let lookup = std::thread::spawn(move || worker.current_version(&resource()));
    let request = next_request(&mut requests);
    service.close(first);
    service.answer(first, &request.id, Ok("2026-11-01T00:00:00Z".into()));
    assert!(lookup.join().unwrap().is_err());
    let worker = service.clone();
    let lookup = std::thread::spawn(move || worker.current_version(&resource()));
    let request = next_request(&mut standby);
    service.answer(second, &request.id, Ok("2026-11-17T00:00:00Z".into()));
    assert!(lookup.join().unwrap().is_ok());
}

#[test]
fn replacing_a_registration_invalidates_its_pending_answer() {
    let service = Arc::new(ResourceVersionService::default());
    let (session, mut requests) = service.open().unwrap();
    service
        .register(session, Some("https://acme.example"))
        .unwrap();
    let worker = service.clone();
    let lookup = std::thread::spawn(move || worker.current_version(&resource()));
    let request = next_request(&mut requests);
    service
        .register(session, Some("https://other.example"))
        .unwrap();
    service.answer(session, &request.id, Ok("2026-11-17T00:00:00Z".into()));
    assert!(lookup.join().unwrap().is_err());
    assert!(!service.available(&resource()));
}

#[test]
fn expired_requests_are_removed_and_late_answers_are_ignored() {
    let service = ResourceVersionService::default();
    let (session, mut requests) = service.open().unwrap();
    service
        .register(session, Some("https://acme.example"))
        .unwrap();
    assert!(service.lookup(&resource(), Duration::ZERO).is_err());
    let request = requests.try_recv().unwrap();
    service.answer(session, &request.id, Ok("2026-11-17T00:00:00Z".into()));
    assert!(service.state.lock().unwrap().pending.is_empty());
}

#[test]
fn invalid_update_times_do_not_become_versions() {
    let service = Arc::new(ResourceVersionService::default());
    let (session, mut requests) = service.open().unwrap();
    service
        .register(session, Some("https://acme.example"))
        .unwrap();
    let worker = service.clone();
    let lookup = std::thread::spawn(move || worker.current_version(&resource()));
    let request = next_request(&mut requests);
    service.answer(session, &request.id, Ok("not a timestamp".into()));
    assert!(lookup.join().unwrap().is_err());
}
