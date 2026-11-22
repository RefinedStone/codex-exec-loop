use super::*;

#[tokio::test]
async fn eof_extra_byte_and_expiry_during_the_write_prompt_prevent_all_binding_writes() {
    for ending in 0..3 {
        let fixture = changed_fixture();
        let gate = Arc::new(Gate::default());
        *fixture.confirmation.write_gate.lock().unwrap() = Some(gate.clone());
        let writes = fixture.records.writes.load(Ordering::SeqCst);
        let duration = if ending == 2 {
            Duration::from_secs(1)
        } else {
            SESSION_LIFETIME
        };
        let (mut client, task, review) = open_review(fixture.services(), duration).await;
        send(&mut client, &reconnect_request(&review)).await;
        gate_started(&gate).await;
        match ending {
            0 => client.get_mut().shutdown().await.unwrap(),
            1 => client.get_mut().write_all(b"{").await.unwrap(),
            _ => {}
        }
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
        gate.release();
        assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
        assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 2);
    }
}

#[tokio::test]
async fn eof_during_review_drops_the_projection_and_cannot_write() {
    let fixture = changed_fixture();
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let (mut client, task) = begin_held(fixture.services(), SESSION_LIFETIME).await;
    let gate = Arc::new(Gate::default());
    *fixture.remote.gate.lock().unwrap() = Some(gate.clone());
    send(&mut client, &review_request()).await;
    gate_started(&gate).await;
    client.get_mut().shutdown().await.unwrap();
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    gate.release();
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
    assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancellation_during_the_last_remote_read_wins_before_commit_admission() {
    let fixture = changed_fixture();
    let (port, review) = direct_review(&fixture);
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let before = fixture.records.values.lock().unwrap().clone();
    let gate = Arc::new(Gate::default());
    *fixture.remote.gate.lock().unwrap() = Some(gate.clone());
    let request = port_request(&review);
    let reconnecting_port = port.clone();
    let reconnecting = tokio::task::spawn_blocking(move || reconnecting_port.reconnect(request));
    gate_started(&gate).await;
    let cancelling_port = port.clone();
    let cancelling = tokio::task::spawn_blocking(move || cancelling_port.cancel());
    tokio::time::timeout(Duration::from_secs(2), cancelling)
        .await
        .unwrap()
        .unwrap();
    gate.release();
    assert_eq!(
        reconnecting.await.unwrap(),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert_eq!(*fixture.records.values.lock().unwrap(), before);
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
    assert!(port.review().is_err());
    assert!(port.restore().is_err());
}

#[tokio::test]
async fn cancellation_after_commit_admission_closes_the_pipe_and_late_commit_does_not_reply() {
    let fixture = changed_fixture();
    let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let commits = fixture.records.commits.load(Ordering::SeqCst);
    let gate = Arc::new(Gate::default());
    *fixture.records.write_gate.lock().unwrap() = Some(gate.clone());
    send(&mut client, &reconnect_request(&review)).await;
    gate_started(&gate).await;
    client.get_mut().shutdown().await.unwrap();
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    // 저장 gate를 통과한 작업은 취소가 되돌리지 않는다. 출력 채널은 이미 닫혔다.
    gate.release();
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.records.commits.load(Ordering::SeqCst) == commits {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes + 3);
    let port = session_port(&fixture);
    port.prepare(session_request()).unwrap();
    assert_eq!(
        port.restore().unwrap().body.expose(),
        format!("본문 수정 {BODY}")
    );
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes + 3);
}

#[tokio::test]
async fn direct_port_allows_only_one_reconnect_while_a_commit_is_in_progress() {
    let fixture = changed_fixture();
    let (port, review) = direct_review(&fixture);
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let gate = Arc::new(Gate::default());
    *fixture.records.write_gate.lock().unwrap() = Some(gate.clone());
    let request = port_request(&review);
    let reconnecting_port = port.clone();
    let reconnecting = tokio::task::spawn_blocking(move || reconnecting_port.reconnect(request));
    gate_started(&gate).await;
    let duplicate_port = port.clone();
    let duplicate_request = port_request(&review);
    let duplicate =
        tokio::task::spawn_blocking(move || duplicate_port.reconnect(duplicate_request));
    let duplicate_result = tokio::time::timeout(Duration::from_secs(2), duplicate).await;
    gate.release();
    assert_eq!(
        duplicate_result.unwrap().unwrap(),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert_eq!(reconnecting.await.unwrap(), Ok(()));
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes + 3);
    assert_eq!(
        port.restore().unwrap().body.expose(),
        format!("본문 수정 {BODY}")
    );
}

#[tokio::test]
async fn direct_port_cancellation_does_not_revive_a_late_write_confirmation() {
    struct LateWriteConfirmation(Arc<Gate>);
    impl VaultNativeConfirmationPort for LateWriteConfirmation {
        fn confirm(
            &self,
            request: &VaultNativeConfirmationRequest,
            _: &AtomicBool,
            _: Instant,
        ) -> VaultNativeConfirmationOutcome {
            if request.purpose == VaultNativeConfirmationPurpose::RewriteBinding {
                self.0.wait();
            }
            VaultNativeConfirmationOutcome::Confirmed
        }
    }
    let fixture = changed_fixture();
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let gate = Arc::new(Gate::default());
    let mut dependencies = fixture.services();
    dependencies.confirmation = Arc::new(LateWriteConfirmation(gate.clone()));
    let port: Arc<dyn VaultRestoreSessionPort> = Arc::new(DesktopVaultRestoreService::new(
        dependencies,
        SESSION_LIFETIME,
    ));
    port.prepare(session_request()).unwrap();
    assert!(port.restore().unwrap_err().can_reconnect);
    let review = port.review().unwrap();
    let request = port_request(&review);
    let reconnecting_port = port.clone();
    let reconnecting = tokio::task::spawn_blocking(move || reconnecting_port.reconnect(request));
    gate_started(&gate).await;
    port.cancel();
    gate.release();
    assert_eq!(
        reconnecting.await.unwrap(),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
    assert!(port.review().is_err());
    assert!(port.restore().is_err());
}

#[tokio::test]
async fn oversized_review_is_a_terminal_hold_without_exposing_the_raw_copy() {
    let fixture = changed_fixture();
    fixture
        .remote
        .fields
        .lock()
        .unwrap()
        .1
        .push_str(&"\0".repeat(protocol::MAX_OUTPUT_BYTES / 3));
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let (mut client, task) = begin_held(fixture.services(), SESSION_LIFETIME).await;
    send(&mut client, &review_request()).await;
    expect_terminal_hold(&mut client, task, "source_unavailable").await;
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
    assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn total_output_budget_includes_review_and_the_final_restore() {
    let fixture = changed_fixture();
    fixture
        .remote
        .fields
        .lock()
        .unwrap()
        .1
        .push_str(&"x".repeat(protocol::MAX_OUTPUT_BYTES / 2));
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let (mut client, task, review) = open_review(fixture.services(), SESSION_LIFETIME).await;
    send(&mut client, &reconnect_request(&review)).await;
    assert_eq!(response(&mut client).await["result"]["kind"], "reconnected");
    send(&mut client, &restore_request()).await;
    expect_terminal_hold(&mut client, task, "source_unavailable").await;
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes + 3);
}

#[tokio::test]
async fn the_next_request_after_a_complete_response_is_valid_while_flush_is_pending() {
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};

    #[derive(Default)]
    struct FlushGate {
        started: Notify,
        released: AtomicBool,
        waker: Mutex<Option<Waker>>,
    }
    impl FlushGate {
        fn release(&self) {
            self.released.store(true, Ordering::SeqCst);
            let waker = self.waker.lock().unwrap().take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
    struct DelayedFlush<W> {
        inner: W,
        gate: Arc<FlushGate>,
    }
    impl<W: AsyncWrite + Unpin> AsyncWrite for DelayedFlush<W> {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.inner).poll_write(cx, bytes)
        }
        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            if !self.gate.released.load(Ordering::SeqCst) {
                *self.gate.waker.lock().unwrap() = Some(cx.waker().clone());
                if !self.gate.released.load(Ordering::SeqCst) {
                    self.gate.started.notify_one();
                    return Poll::Pending;
                }
            }
            Pin::new(&mut self.inner).poll_flush(cx)
        }
        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.inner).poll_shutdown(cx)
        }
    }

    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let gate = Arc::new(FlushGate::default());
    let (client, server) = tokio::io::duplex(128 * 1024);
    let (input, output) = tokio::io::split(server);
    let mut client = BufReader::new(client);
    let task = tokio::spawn(run_test_io(
        input,
        DelayedFlush {
            inner: output,
            gate: gate.clone(),
        },
        fixture.services(),
        SESSION_LIFETIME,
    ));
    send(&mut client, &prepare_request()).await;
    assert_eq!(response(&mut client).await["result"]["kind"], "prepared");
    tokio::time::timeout(Duration::from_secs(2), gate.started.notified())
        .await
        .unwrap();
    send(&mut client, &restore_request()).await;
    gate.release();
    assert_eq!(response(&mut client).await["result"]["body"], BODY);
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
}
