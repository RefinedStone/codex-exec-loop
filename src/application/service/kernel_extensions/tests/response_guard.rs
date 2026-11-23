use super::*;
use registry::ExtensionResponseGuard;
use std::sync::mpsc;

#[derive(Default)]
struct GuardedHandler {
    permission: Mutex<bool>,
    invalid_output: AtomicBool,
    pause_guard: AtomicBool,
    guard_started: AtomicBool,
    pause_validation: AtomicBool,
    validation_started: AtomicBool,
    guards: AtomicUsize,
    values: Mutex<Option<(Value, Value)>>,
}

struct PermissionGuard<'a> {
    handler: &'a GuardedHandler,
    permission: MutexGuard<'a, bool>,
}
impl ExtensionResponseGuard for PermissionGuard<'_> {
    fn validate(&self) -> Result<()> {
        self.handler
            .validation_started
            .store(true, Ordering::Release);
        // A test-only pause makes revoke/disable race with the exact response commit boundary.
        wait_until(|| !self.handler.pause_validation.load(Ordering::Acquire));
        ensure!(*self.permission, "test share was revoked");
        Ok(())
    }
}
impl ExtensionHandler for GuardedHandler {
    fn operations(&self) -> &'static [&'static str] {
        &["echo"]
    }

    fn invoke(&self, _: &ExtensionContext, _: &str, input: Value) -> Result<Value> {
        if self.invalid_output.load(Ordering::Acquire) {
            Ok(json!({"wrong":true}))
        } else {
            Ok(input)
        }
    }

    fn response_guard<'a>(
        &'a self,
        _: &ExtensionContext,
        action: &str,
        input: &Value,
        output: &Value,
    ) -> Result<Option<Box<dyn ExtensionResponseGuard + 'a>>> {
        assert_eq!(action, "echo");
        self.guards.fetch_add(1, Ordering::AcqRel);
        *self.values.lock().unwrap() = Some((input.clone(), output.clone()));
        self.guard_started.store(true, Ordering::Release);
        wait_until(|| !self.pause_guard.load(Ordering::Acquire));
        Ok(Some(Box::new(PermissionGuard {
            handler: self,
            permission: self.permission.lock().unwrap(),
        })))
    }
}
struct ReleasePauses(Arc<GuardedHandler>);
impl Drop for ReleasePauses {
    fn drop(&mut self) {
        self.0.pause_guard.store(false, Ordering::Release);
        self.0.pause_validation.store(false, Ordering::Release);
    }
}

fn guarded_host(handler: Arc<GuardedHandler>) -> Arc<ExtensionService> {
    *handler.permission.lock().unwrap() = true;
    let mut entry = registration(
        "akra.test",
        ExtensionScope::LocalUser,
        Arc::new(TestHandler::default()),
    );
    entry
        .manifest
        .operations
        .retain(|operation| operation.name == "echo");
    entry.handler = handler;
    let host = ExtensionService::new(
        ExtensionRegistry::build(vec![entry], ExtensionScope::LocalUser).unwrap(),
        Arc::new(MemoryStore::default()),
        ExtensionScope::LocalUser,
        None,
        BTreeSet::from(["test.run".into()]),
    )
    .unwrap();
    enable(&host);
    host
}

#[test]
fn schema_validation_precedes_guard_and_the_original_input_is_preserved() {
    let handler = Arc::new(GuardedHandler::default());
    let host = guarded_host(handler.clone());
    handler.invalid_output.store(true, Ordering::Release);
    assert!(call(&host, "echo", json!({"text":"original"}), None).is_err());
    assert_eq!(handler.guards.load(Ordering::Acquire), 0);
    handler.invalid_output.store(false, Ordering::Release);
    let expected = json!({"text":"original"});
    assert_eq!(
        call(&host, "echo", expected.clone(), None).unwrap(),
        expected
    );
    assert_eq!(handler.guards.load(Ordering::Acquire), 1);
    assert_eq!(
        *handler.values.lock().unwrap(),
        Some((expected.clone(), expected))
    );
    assert!(
        handler.permission.try_lock().is_ok(),
        "permission guard is released before return"
    );
    assert!(
        host.state.try_lock().is_ok(),
        "host lock is released before return"
    );
}

#[test]
fn revoke_disable_and_shutdown_during_guard_acquisition_reject_old_output() {
    for change in ["revoke", "disable", "shutdown"] {
        let handler = Arc::new(GuardedHandler::default());
        let _release = ReleasePauses(handler.clone());
        let host = guarded_host(handler.clone());
        handler.pause_guard.store(true, Ordering::Release);
        let querying = host.clone();
        let query =
            std::thread::spawn(move || call(&querying, "echo", json!({"text":"private"}), None));
        wait_until(|| handler.guard_started.load(Ordering::Acquire));
        match change {
            "revoke" => *handler.permission.lock().unwrap() = false,
            "disable" => {
                let revision = host.catalog()[0].revision;
                call(
                    &host,
                    "configure",
                    json!({"enabled":false,"expected_revision":revision}),
                    None,
                )
                .unwrap();
            }
            _ => host.shutdown(),
        }
        handler.pause_guard.store(false, Ordering::Release);
        assert!(
            query.join().unwrap().is_err(),
            "{change} must reject the old response"
        );
        if change == "disable" {
            wait_until(|| host.catalog()[0].lifecycle == ExtensionLifecycle::Disabled);
        }
        assert!(
            handler.permission.try_lock().is_ok(),
            "{change} releases the permission guard"
        );
        assert!(
            host.state.try_lock().is_ok(),
            "{change} releases the host lock"
        );
    }
}

#[test]
fn response_commit_serializes_both_permission_revocation_and_disable() {
    let handler = Arc::new(GuardedHandler::default());
    let _release = ReleasePauses(handler.clone());
    let host = guarded_host(handler.clone());
    let revision = host.catalog()[0].revision;
    handler.pause_validation.store(true, Ordering::Release);
    let querying = host.clone();
    let query =
        std::thread::spawn(move || call(&querying, "echo", json!({"text":"committed"}), None));
    wait_until(|| handler.validation_started.load(Ordering::Acquire));

    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let revoking = handler.clone();
    let revoke_started = started_tx.clone();
    let revoke_done = done_tx.clone();
    let revoke = std::thread::spawn(move || {
        revoke_started.send("revoke").unwrap();
        *revoking.permission.lock().unwrap() = false;
        revoke_done.send("revoke").unwrap();
    });
    let disabling = host.clone();
    let disable = std::thread::spawn(move || {
        started_tx.send("disable").unwrap();
        call(
            &disabling,
            "configure",
            json!({"enabled":false,"expected_revision":revision}),
            None,
        )
        .unwrap();
        done_tx.send("disable").unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        done_rx.recv_timeout(Duration::from_millis(40)).is_err(),
        "neither authority can change while the response is being committed"
    );
    handler.pause_validation.store(false, Ordering::Release);
    assert_eq!(query.join().unwrap().unwrap()["text"], "committed");
    done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    revoke.join().unwrap();
    disable.join().unwrap();
    wait_until(|| host.catalog()[0].lifecycle == ExtensionLifecycle::Disabled);
}
