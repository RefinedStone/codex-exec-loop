use super::*;
use crate::application::port::inbound::vault_restore_session_port::{
    VaultRestoreReconnectRequest, VaultRestoreSessionField, VaultRestoreSessionMapping,
    VaultRestoreSessionPlace, VaultRestoreSessionReview,
};
use crate::application::service::evidence_vault_restore::VaultReconnectApproval;
use crate::application::service::evidence_vault_unlock::{
    VaultUnlockOutcome, VaultUnlockRequest, VaultUserDecision,
};

mod cancellation;
#[cfg(unix)]
mod filesystem;

fn changed_fixture() -> Fixture {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let mut fields = fixture.remote.fields.lock().unwrap();
    fields.0.insert_str(0, "제목 수정 ");
    fields.1.insert_str(0, "본문 수정 ");
    drop(fields);
    fixture
}
fn review_request() -> Value {
    json!({"version":2,"id":"test-session","command":"review"})
}
fn reconnect_request(review: &Value) -> Value {
    let mappings = review["markers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|marker| {
            let candidate = marker["candidates"].as_array().unwrap().first()?;
            Some(json!({
                "marker":marker["marker"],
                "field":candidate["field"],
                "position":candidate["position"],
            }))
        })
        .collect::<Vec<_>>();
    json!({
        "version":2,"id":"test-session","command":"reconnect",
        "reviewed_hash":review["reviewed_hash"],"mappings":mappings,
    })
}
fn port_request(review: &VaultRestoreSessionReview) -> VaultRestoreReconnectRequest {
    VaultRestoreReconnectRequest {
        reviewed_hash: review.reviewed_hash.clone(),
        mappings: review
            .markers
            .iter()
            .filter_map(|marker| {
                let candidate = marker.candidates.first()?;
                Some(VaultRestoreSessionMapping {
                    marker: marker.marker.clone(),
                    place: candidate.place.clone(),
                })
            })
            .collect(),
    }
}
async fn begin_held(
    dependencies: DesktopVaultRestoreDependencies,
    duration: Duration,
) -> (
    BufReader<DuplexStream>,
    tokio::task::JoinHandle<io::Result<()>>,
) {
    let (mut client, task) = start(dependencies, duration);
    send(&mut client, &prepare_request()).await;
    assert_eq!(
        response(&mut client).await["result"],
        json!({"kind":"prepared","capabilities":{"reconnect":true}})
    );
    send(&mut client, &restore_request()).await;
    assert_eq!(
        response(&mut client).await["result"],
        json!({"kind":"held","reason":"published_copy_changed","can_reconnect":true})
    );
    (client, task)
}
async fn open_review(
    dependencies: DesktopVaultRestoreDependencies,
    duration: Duration,
) -> (
    BufReader<DuplexStream>,
    tokio::task::JoinHandle<io::Result<()>>,
    Value,
) {
    let (mut client, task) = begin_held(dependencies, duration).await;
    send(&mut client, &review_request()).await;
    let reply = response(&mut client).await;
    assert_eq!(reply["result"]["kind"], "review");
    let review = reply["result"]["review"].clone();
    (client, task, review)
}
fn direct_review(
    fixture: &Fixture,
) -> (Arc<dyn VaultRestoreSessionPort>, VaultRestoreSessionReview) {
    let port = session_port(fixture);
    assert!(port.prepare(session_request()).unwrap().reconnect);
    assert!(port.restore().unwrap_err().can_reconnect);
    let review = port.review().unwrap();
    (port, review)
}
async fn expect_terminal_hold(
    client: &mut BufReader<DuplexStream>,
    task: tokio::task::JoinHandle<io::Result<()>>,
    reason: &str,
) {
    assert_eq!(
        response(client).await["result"],
        json!({"kind":"held","reason":reason,"can_reconnect":false})
    );
    no_more_output(client).await;
    task.await.unwrap().unwrap();
}
async fn gate_started(gate: &Gate) {
    tokio::time::timeout(Duration::from_secs(3), gate.started.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn native_reconnect_requires_a_second_purpose_and_restores_once_with_byte_positions() {
    let fixture = changed_fixture();
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
    let review_text = review.to_string();
    assert!(!review_text.contains("synthetic-title-secret"));
    assert!(!review_text.contains("synthetic-body-secret"));
    assert_eq!(review["markers"].as_array().unwrap().len(), 2);
    assert!(review["reviewed_hash"].as_str().unwrap().starts_with("1:"));
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
    assert_eq!(
        *fixture.confirmation.purposes.lock().unwrap(),
        vec![VaultNativeConfirmationPurpose::ViewOriginal]
    );
    for marker in review["markers"].as_array().unwrap() {
        let candidate = &marker["candidates"][0];
        let text = if candidate["field"] == "title" {
            review["published_title"].as_str().unwrap()
        } else {
            review["published_body"].as_str().unwrap()
        };
        let position = candidate["position"].as_u64().unwrap() as usize;
        assert!(text[position..].starts_with(marker["marker"].as_str().unwrap()));
        assert!(position > marker["previous"]["position"].as_u64().unwrap() as usize);
    }
    send(&mut client, &reconnect_request(&review)).await;
    assert_eq!(
        response(&mut client).await["result"],
        json!({"kind":"reconnected"})
    );
    assert_eq!(
        *fixture.confirmation.purposes.lock().unwrap(),
        vec![
            VaultNativeConfirmationPurpose::ViewOriginal,
            VaultNativeConfirmationPurpose::RewriteBinding
        ]
    );
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes + 3);
    send(&mut client, &restore_request()).await;
    let restored = response(&mut client).await;
    assert_eq!(restored["result"]["title"], format!("제목 수정 {TITLE}"));
    assert_eq!(restored["result"]["body"], format!("본문 수정 {BODY}"));
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes + 3);
    assert_eq!(fixture.keys.stores.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn empty_selection_keeps_markers_and_final_copy_change_cannot_reconnect_again() {
    for change_after_reconnect in [false, true] {
        let fixture = changed_fixture();
        let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
        let mut request = reconnect_request(&review);
        request["mappings"] = json!([]);
        send(&mut client, &request).await;
        assert_eq!(response(&mut client).await["result"]["kind"], "reconnected");
        if change_after_reconnect {
            fixture
                .remote
                .fields
                .lock()
                .unwrap()
                .1
                .push_str(" later edit");
        }
        send(&mut client, &restore_request()).await;
        let reply = response(&mut client).await;
        if change_after_reconnect {
            assert_eq!(
                reply["result"],
                json!({"kind":"held","reason":"published_copy_changed","can_reconnect":false})
            );
        } else {
            assert_eq!(reply["result"]["title"], review["published_title"]);
            assert_eq!(reply["result"]["body"], review["published_body"]);
        }
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn stale_hash_or_invalid_selection_is_terminal_before_the_write_prompt() {
    for case in 0..5 {
        let fixture = changed_fixture();
        let writes = fixture.records.writes.load(Ordering::SeqCst);
        let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
        let mut request = reconnect_request(&review);
        match case {
            0 => request["reviewed_hash"] = json!(format!("1:{}", "0".repeat(64))),
            1 => request["mappings"][0]["marker"] = json!("[[akra-mask:unknown]]"),
            2 => request["mappings"][0]["position"] = json!(0),
            3 => {
                let first = request["mappings"][0].clone();
                request["mappings"].as_array_mut().unwrap().push(first);
            }
            _ => request["mappings"][0]["field"] = json!("body"),
        }
        send(&mut client, &request).await;
        expect_terminal_hold(
            &mut client,
            task,
            if case == 0 {
                "changed_during_review"
            } else {
                "mapping_rejected"
            },
        )
        .await;
        assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
        assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn native_write_cancellation_never_changes_the_binding() {
    let fixture = changed_fixture();
    *fixture.confirmation.write_outcome.lock().unwrap() =
        Some(VaultNativeConfirmationOutcome::Cancelled);
    let before = fixture.records.values.lock().unwrap().clone();
    let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
    send(&mut client, &reconnect_request(&review)).await;
    expect_terminal_hold(&mut client, task, "confirmation_cancelled").await;
    assert_eq!(*fixture.records.values.lock().unwrap(), before);
    assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn remote_and_local_changes_after_review_preserve_the_current_binding() {
    for local in [false, true] {
        let fixture = changed_fixture();
        let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
        if local {
            let session = fixture.unlock.begin_session();
            let VaultUnlockOutcome::Unlocked(grant) = fixture.unlock.unlock(VaultUnlockRequest {
                issue: issue(),
                session,
                decision: VaultUserDecision::Confirmed,
            }) else {
                panic!("synthetic grant should open");
            };
            let service = EvidenceVaultRestoreService::new(
                fixture.remote.clone(),
                fixture.records.clone(),
                fixture.unlock.clone(),
            );
            let latest = service.review_reconnect(&grant).unwrap();
            let approval = VaultReconnectApproval::from_review(&latest, Vec::new()).unwrap();
            service.reconnect(&grant, &approval).unwrap();
        } else {
            fixture
                .remote
                .fields
                .lock()
                .unwrap()
                .1
                .push_str(" edited during review");
        }
        let before = fixture.records.values.lock().unwrap().clone();
        let writes = fixture.records.writes.load(Ordering::SeqCst);
        send(&mut client, &reconnect_request(&review)).await;
        expect_terminal_hold(&mut client, task, "changed_during_review").await;
        assert_eq!(*fixture.records.values.lock().unwrap(), before);
        assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
    }
}

#[tokio::test]
async fn uncertain_commit_is_terminal_and_reopening_reads_the_committed_binding_without_retry() {
    let fixture = changed_fixture();
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
    fixture
        .records
        .lose_commit_reply
        .store(true, Ordering::SeqCst);
    send(&mut client, &reconnect_request(&review)).await;
    expect_terminal_hold(&mut client, task, "write_outcome_unknown").await;
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes + 3);
    let port = session_port(&fixture);
    port.prepare(session_request()).unwrap();
    assert_eq!(
        port.restore().unwrap().body.expose(),
        format!("본문 수정 {BODY}")
    );
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes + 3);
    assert!(port.review().is_err());
}

#[tokio::test]
async fn malformed_reconnect_frames_close_without_prompting_or_writing() {
    for case in 0..10 {
        let fixture = changed_fixture();
        let writes = fixture.records.writes.load(Ordering::SeqCst);
        let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
        let mut request = reconnect_request(&review);
        match case {
            0 => request["approved"] = json!(true),
            1 => request["id"] = json!("other-session"),
            2 => request["version"] = json!(1),
            3 => request["issue"] = prepare_request()["issue"].clone(),
            4 => request["mappings"][0]["approved"] = json!(true),
            5 => request["mappings"][0]["position"] = json!(-1),
            6 => request["mappings"][0]["position"] = json!(1.5),
            7 => request["mappings"][0]["position"] = json!(9_007_199_254_740_992u64),
            8 => request["mappings"][0]["field"] = json!("comments"),
            _ => request["reviewed_hash"] = json!("01:bad-hash"),
        }
        send(&mut client, &request).await;
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
        assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
        assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn v1_early_review_and_pipelined_commands_never_unlock() {
    for case in 0..4 {
        let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
        let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
        if case == 0 {
            let mut request = prepare_request();
            request["version"] = json!(1);
            send(&mut client, &request).await;
        } else if case == 1 {
            send(&mut client, &review_request()).await;
        } else if case == 2 {
            let bytes = format!("{}\n{}\n", prepare_request(), restore_request());
            client.get_mut().write_all(bytes.as_bytes()).await.unwrap();
        } else {
            send(&mut client, &prepare_request()).await;
            response(&mut client).await;
            send(&mut client, &review_request()).await;
        }
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
        fixture.assert_no_key_access();
    }
}

#[tokio::test]
async fn out_of_order_or_repeated_commands_do_not_repeat_a_reconnect() {
    for stage in 0..3 {
        let fixture = changed_fixture();
        let writes = fixture.records.writes.load(Ordering::SeqCst);
        let (mut client, task) = begin_held(fixture.services(), SESSION_LIFETIME).await;
        if stage == 0 {
            send(&mut client, &restore_request()).await;
        } else {
            send(&mut client, &review_request()).await;
            let review = response(&mut client).await["result"]["review"].clone();
            if stage == 1 {
                send(&mut client, &review_request()).await;
            } else {
                let request = reconnect_request(&review);
                send(&mut client, &request).await;
                assert_eq!(response(&mut client).await["result"]["kind"], "reconnected");
                send(&mut client, &request).await;
            }
        }
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
        assert_eq!(
            fixture.records.writes.load(Ordering::SeqCst),
            writes + if stage == 2 { 3 } else { 0 }
        );
    }
}

#[test]
fn output_budget_counts_all_frames_and_escaped_bytes() {
    let mut budget = protocol::OutputBudget::new();
    let body = "x".repeat(protocol::MAX_OUTPUT_BYTES / 2);
    let first = budget
        .encode(
            "test",
            Reply::Restored {
                title: "",
                body: &body,
            },
            false,
        )
        .unwrap();
    assert!(!first.terminal);
    let second = budget
        .encode(
            "test",
            Reply::Restored {
                title: "",
                body: &body,
            },
            false,
        )
        .unwrap();
    assert!(second.terminal);
    assert_eq!(
        serde_json::from_slice::<Value>(&second.bytes).unwrap()["result"],
        json!({"kind":"held","reason":"source_unavailable","can_reconnect":false})
    );
    assert!(first.bytes.len() + second.bytes.len() <= protocol::MAX_OUTPUT_BYTES);
}

#[test]
fn direct_port_rejects_review_before_hold_and_reconnect_before_review() {
    for stage in 0..3 {
        let fixture = changed_fixture();
        let port = session_port(&fixture);
        let writes = fixture.records.writes.load(Ordering::SeqCst);
        if stage > 0 {
            port.prepare(session_request()).unwrap();
        }
        if stage == 2 {
            assert!(port.restore().unwrap_err().can_reconnect);
        }
        if stage < 2 {
            assert_eq!(port.review().unwrap_err(), VaultRestoreSessionHold::Locked);
        } else {
            assert_eq!(
                port.reconnect(VaultRestoreReconnectRequest {
                    reviewed_hash: format!("1:{}", "0".repeat(64)),
                    mappings: Vec::new(),
                }),
                Err(VaultRestoreSessionHold::Locked)
            );
        }
        assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
    }
}

#[test]
fn direct_port_owns_the_original_review_and_consumes_reconnect_errors() {
    let fixture = changed_fixture();
    let (port, mut review) = direct_review(&fixture);
    review.reviewed_hash = format!("1:{}", "0".repeat(64));
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    assert_eq!(
        port.reconnect(port_request(&review)),
        Err(VaultRestoreSessionHold::ChangedDuringReview)
    );
    assert_eq!(
        port.reconnect(port_request(&review)),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert_eq!(port.review().unwrap_err(), VaultRestoreSessionHold::Locked);
    assert!(!port.restore().unwrap_err().can_reconnect);
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
}

#[test]
fn direct_port_cannot_reuse_a_projection_or_restore_twice_after_reconnect() {
    let fixture = changed_fixture();
    let (port, review) = direct_review(&fixture);
    port.reconnect(port_request(&review)).unwrap();
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    assert_eq!(
        port.reconnect(port_request(&review)),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert_eq!(
        port.restore().unwrap().title.expose(),
        format!("제목 수정 {TITLE}")
    );
    assert!(!port.restore().unwrap_err().can_reconnect);
    assert_eq!(port.review().unwrap_err(), VaultRestoreSessionHold::Locked);
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
}

#[test]
fn direct_port_invalid_mapping_does_not_request_native_write_confirmation() {
    let fixture = changed_fixture();
    let (port, review) = direct_review(&fixture);
    let mut request = port_request(&review);
    request.mappings.push(VaultRestoreSessionMapping {
        marker: review.markers[0].marker.clone(),
        place: VaultRestoreSessionPlace {
            field: VaultRestoreSessionField::Body,
            position: usize::MAX,
        },
    });
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    assert_eq!(
        port.reconnect(request),
        Err(VaultRestoreSessionHold::MappingRejected)
    );
    assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
}
