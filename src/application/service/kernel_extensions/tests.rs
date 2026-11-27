mod admission;
mod lifecycle;
mod response_guard;
use super::*;
use registry::{ExtensionHandler, ExtensionRegistration};
use std::sync::atomic::AtomicUsize;
use std::time::{Duration, Instant};

#[derive(Default)]
struct MemoryStore {
    value: Mutex<ExtensionDurableState>,
    fail: AtomicBool,
}
impl ExtensionStateStorePort for MemoryStore {
    fn load(&self) -> Result<ExtensionDurableState> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn save(&self, state: &ExtensionDurableState) -> Result<()> {
        ensure!(
            !self.fail.load(Ordering::Acquire),
            "injected persistence failure"
        );
        *self.value.lock().unwrap() = state.clone();
        Ok(())
    }
}
#[derive(Default)]
struct TestHandler {
    entered: AtomicUsize,
    release: AtomicBool,
    invalid: AtomicBool,
    panic_worker: AtomicBool,
    hold_cancelled_worker: AtomicBool,
    hold_cleanup: AtomicBool,
    fail_cleanup: AtomicBool,
    cleanup_entered: AtomicUsize,
    cleanup_barriers: Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
    cleanups: Mutex<Vec<(String, String)>>,
    /// Zero means no per-extension bound.
    max_active: AtomicUsize,
}
impl ExtensionHandler for TestHandler {
    fn operations(&self) -> &'static [&'static str] {
        &["echo", "work"]
    }
    fn invoke(&self, context: &ExtensionContext, _action: &str, input: Value) -> Result<Value> {
        assert!(context.activation_revision > 0);
        if self.invalid.load(Ordering::Acquire) {
            Ok(json!({"wrong":true}))
        } else {
            Ok(input)
        }
    }
    fn run(&self, context: &ExtensionJobContext, _action: &str, input: Value) -> Result<Value> {
        self.entered.fetch_add(1, Ordering::AcqRel);
        assert!(
            !self.panic_worker.load(Ordering::Acquire),
            "injected worker panic"
        );
        while !self.release.load(Ordering::Acquire)
            && (self.hold_cancelled_worker.load(Ordering::Acquire) || !context.cancelled())
        {
            std::thread::sleep(Duration::from_millis(2));
        }
        if !self.hold_cancelled_worker.load(Ordering::Acquire) {
            ensure!(!context.cancelled(), "test worker cancelled");
        }
        Ok(input)
    }
    fn dedupe_key(&self, _action: &str, input: &Value) -> Result<Option<String>> {
        Ok(Some(input.to_string()))
    }
    fn max_active_jobs(&self) -> Option<usize> {
        Some(self.max_active.load(Ordering::Acquire)).filter(|limit| *limit > 0)
    }
    fn cleanup(&self, _context: &ExtensionContext, job: &ExtensionJob) -> Result<()> {
        self.cleanup_entered.fetch_add(1, Ordering::AcqRel);
        let barriers = self.cleanup_barriers.lock().unwrap().clone();
        if let Some((entered, release)) = barriers {
            entered.wait();
            release.wait();
        }
        while self.hold_cleanup.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(2));
        }
        ensure!(
            !self.fail_cleanup.load(Ordering::Acquire),
            "injected cleanup failure"
        );
        self.cleanups
            .lock()
            .unwrap()
            .push((job.id.clone(), job.generation.clone()));
        Ok(())
    }
}
fn registration(
    id: &str,
    scope: ExtensionScope,
    handler: Arc<TestHandler>,
) -> ExtensionRegistration {
    let schema = json!({"type":"object","properties":{"text":{"type":"string","maxLength":20}},"required":["text"],"additionalProperties":false});
    ExtensionRegistration {
        manifest: ExtensionDescriptor {
            id: id.into(),
            version: 1,
            api_version: 1,
            scope,
            label: id.into(),
            panel: None,
            permissions: vec!["test.run".into()],
            operations: vec![
                ExtensionOperation {
                    name: "echo".into(),
                    kind: ExtensionOperationKind::Query,
                    input_schema: schema.clone(),
                    output_schema: schema.clone(),
                    progress_schema: None,
                },
                ExtensionOperation {
                    name: "work".into(),
                    kind: ExtensionOperationKind::Job,
                    input_schema: schema.clone(),
                    output_schema: schema,
                    progress_schema: None,
                },
            ],
        },
        handler,
    }
}
fn service(store: Arc<MemoryStore>, handler: Arc<TestHandler>) -> Arc<ExtensionService> {
    ExtensionService::new(
        ExtensionRegistry::build(
            vec![registration(
                "akra.test",
                ExtensionScope::LocalUser,
                handler,
            )],
            ExtensionScope::LocalUser,
        )
        .unwrap(),
        store,
        ExtensionScope::LocalUser,
        None,
        BTreeSet::from(["test.run".into()]),
    )
    .unwrap()
}
fn context(host: &ExtensionService) -> ExtensionContext {
    host.context("test-connection".into(), None)
}
fn call(
    host: &Arc<ExtensionService>,
    action: &str,
    input: Value,
    key: Option<&str>,
) -> Result<Value> {
    host.call(
        context(host),
        "akra.test",
        1,
        action,
        input,
        key.map(str::to_owned),
    )
}
fn enable(host: &Arc<ExtensionService>) {
    let revision = host.catalog()[0].revision;
    call(
        host,
        "configure",
        json!({"enabled":true,"expected_revision":revision}),
        None,
    )
    .unwrap();
}
fn wait_until(mut check: impl FnMut() -> bool) {
    let start = Instant::now();
    while !check() {
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "condition timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_failed_package_does_not_replace_or_disable_a_valid_registration() {
    let handler = Arc::new(TestHandler::default());
    let valid = registration("akra.test", ExtensionScope::LocalUser, handler.clone());
    let mut bad = registration("akra.bad", ExtensionScope::LocalUser, handler.clone());
    bad.manifest.operations[0].input_schema = json!({"$ref":"https://untrusted.invalid/schema"});
    let duplicate = registration("akra.test", ExtensionScope::LocalUser, handler.clone());
    let mut mismatch = registration("akra.mismatch", ExtensionScope::LocalUser, handler);
    mismatch.manifest.operations.pop();
    let registry = ExtensionRegistry::build(
        vec![valid, bad, duplicate, mismatch],
        ExtensionScope::LocalUser,
    )
    .unwrap();
    assert!(registry.get("akra.test").is_ok());
    assert!(registry.get("akra.bad").is_err());
    assert!(registry.get("akra.mismatch").is_err());
    assert_eq!(registry.errors("akra.test").len(), 1);
    let host = ExtensionService::new(
        registry,
        Arc::new(MemoryStore::default()),
        ExtensionScope::LocalUser,
        None,
        BTreeSet::from(["test.run".into()]),
    )
    .unwrap();
    assert_eq!(host.catalog().len(), 3);
    assert!(
        !host
            .catalog()
            .iter()
            .find(|e| e.id == "akra.bad")
            .unwrap()
            .compatible
    );
}

#[test]
fn schemas_with_the_same_local_definition_name_do_not_collide() {
    let handler = Arc::new(TestHandler::default());
    let mut a = registration("akra.first", ExtensionScope::LocalUser, handler.clone());
    let mut b = registration("akra.second", ExtensionScope::LocalUser, handler);
    a.manifest.operations[0].input_schema =
        json!({"$defs":{"Input":{"type":"string"}},"$ref":"#/$defs/Input"});
    b.manifest.operations[0].input_schema =
        json!({"$defs":{"Input":{"type":"integer"}},"$ref":"#/$defs/Input"});
    let registry = ExtensionRegistry::build(vec![a, b], ExtensionScope::LocalUser).unwrap();
    assert!(
        registry.get("akra.first").unwrap().operations["echo"]
            .input
            .is_valid(&json!("one"))
    );
    assert!(
        !registry.get("akra.second").unwrap().operations["echo"]
            .input
            .is_valid(&json!("one"))
    );
}

#[test]
fn activation_is_durable_cas_and_failed_persistence_never_becomes_enabled() {
    let store = Arc::new(MemoryStore::default());
    let host = service(store.clone(), Arc::new(TestHandler::default()));
    assert!(call(&host, "echo", json!({"text":"hello"}), None).is_err());
    let mut events = host.subscribe();
    store.fail.store(true, Ordering::Release);
    assert!(
        call(
            &host,
            "configure",
            json!({"enabled":true,"expected_revision":0}),
            None
        )
        .is_err()
    );
    assert!(!host.catalog()[0].enabled);
    assert!(events.try_recv().is_err());
    store.fail.store(false, Ordering::Release);
    enable(&host);
    assert!(host.catalog()[0].enabled);
    assert!(events.try_recv().is_ok());
    assert!(
        call(
            &host,
            "configure",
            json!({"enabled":false,"expected_revision":0}),
            None
        )
        .is_err()
    );
    assert_eq!(
        call(&host, "echo", json!({"text":"hello"}), None).unwrap(),
        json!({"text":"hello"})
    );
    assert!(call(&host, "echo", json!({"text":12}), None).is_err());
}

#[test]
fn incompatible_api_and_missing_permissions_stay_visible_but_cannot_execute() {
    let handler = Arc::new(TestHandler::default());
    let mut entry = registration("akra.test", ExtensionScope::LocalUser, handler);
    entry.manifest.api_version = 2;
    let host = ExtensionService::new(
        ExtensionRegistry::build(vec![entry], ExtensionScope::LocalUser).unwrap(),
        Arc::new(MemoryStore::default()),
        ExtensionScope::LocalUser,
        None,
        BTreeSet::new(),
    )
    .unwrap();
    assert!(!host.catalog()[0].compatible);
    assert!(
        call(
            &host,
            "configure",
            json!({"enabled":true,"expected_revision":0}),
            None
        )
        .is_err()
    );
    let handler = Arc::new(TestHandler::default());
    let host = ExtensionService::new(
        ExtensionRegistry::build(
            vec![registration(
                "akra.test",
                ExtensionScope::LocalUser,
                handler,
            )],
            ExtensionScope::LocalUser,
        )
        .unwrap(),
        Arc::new(MemoryStore::default()),
        ExtensionScope::LocalUser,
        None,
        BTreeSet::new(),
    )
    .unwrap();
    assert!(
        call(
            &host,
            "configure",
            json!({"enabled":true,"expected_revision":0}),
            None
        )
        .is_err()
    );
}

#[test]
fn retry_dedupe_and_completed_rescan_have_different_semantics() {
    let handler = Arc::new(TestHandler::default());
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let first = call(&host, "work", json!({"text":"same"}), Some("retry")).unwrap();
    wait_until(|| handler.entered.load(Ordering::Acquire) == 1);
    let concurrent = call(&host, "work", json!({"text":"same"}), Some("second-key")).unwrap();
    assert_eq!(first["id"], concurrent["id"]);
    assert!(call(&host, "work", json!({"text":"different"}), Some("retry")).is_err());
    handler.release.store(true, Ordering::Release);
    let id = first["id"].as_str().unwrap();
    wait_until(|| host.job("akra.test", id).unwrap().state == ExtensionJobState::Completed);
    let retry = call(&host, "work", json!({"text":"same"}), Some("retry")).unwrap();
    assert_eq!(retry["id"], first["id"]);
    let rescan = call(&host, "work", json!({"text":"same"}), Some("new-scan")).unwrap();
    assert_ne!(rescan["id"], first["id"]);
    wait_until(|| {
        host.job("akra.test", rescan["id"].as_str().unwrap())
            .unwrap()
            .state
            == ExtensionJobState::Completed
    });
}

#[test]
fn disable_blocks_admission_and_waits_for_worker_and_cleanup() {
    let handler = Arc::new(TestHandler::default());
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let job = call(&host, "work", json!({"text":"private"}), Some("job")).unwrap();
    wait_until(|| handler.entered.load(Ordering::Acquire) == 1);
    let revision = host.catalog()[0].revision;
    call(
        &host,
        "configure",
        json!({"enabled":false,"expected_revision":revision}),
        None,
    )
    .unwrap();
    assert!(call(&host, "work", json!({"text":"new"}), Some("new")).is_err());
    wait_until(|| host.catalog()[0].lifecycle == ExtensionLifecycle::Disabled);
    let stored = call(&host, "job", json!({"job_id":job["id"]}), None).unwrap();
    assert_eq!(stored["state"], "cancelled");
    assert!(stored["result"].is_null());
    assert!(stored["progress"].is_null());
    assert_eq!(stored["cleanup"], "complete");
    assert_eq!(handler.cleanups.lock().unwrap().len(), 1);
}

#[test]
fn restart_marks_unfinished_work_interrupted_without_restarting_it() {
    let store = Arc::new(MemoryStore::default());
    let first = service(store.clone(), Arc::new(TestHandler::default()));
    enable(&first);
    let old_generation = first.generation().to_owned();
    let mut state = store.value.lock().unwrap();
    state.jobs.insert(
        "old-job".into(),
        ExtensionJob {
            id: "old-job".into(),
            extension: "akra.test".into(),
            action: "work".into(),
            state: ExtensionJobState::Running,
            generation: old_generation.clone(),
            revision: 1,
            sequence: 1,
            progress: json!({"private":"path"}),
            result: None,
            error: None,
            cleanup: ExtensionCleanupState::Pending,
            created_at_ms: 1,
            updated_at_ms: 1,
            result_expires_at_ms: 1,
            dedupe_key: None,
        },
    );
    drop(state);
    drop(first);
    let handler = Arc::new(TestHandler::default());
    let restarted = service(store, handler.clone());
    assert_ne!(restarted.generation(), old_generation);
    assert!(restarted.catalog()[0].enabled);
    assert_eq!(
        restarted.job("akra.test", "old-job").unwrap().state,
        ExtensionJobState::Interrupted
    );
    assert_eq!(handler.entered.load(Ordering::Acquire), 0);
    assert_eq!(
        handler.cleanups.lock().unwrap()[0],
        ("old-job".into(), old_generation)
    );
}

#[test]
fn a_wrong_host_or_old_generation_cannot_reuse_an_attachment() {
    let first = service(
        Arc::new(MemoryStore::default()),
        Arc::new(TestHandler::default()),
    );
    let second = service(
        Arc::new(MemoryStore::default()),
        Arc::new(TestHandler::default()),
    );
    enable(&second);
    assert!(
        second
            .call(
                context(&first),
                "akra.test",
                1,
                "echo",
                json!({"text":"x"}),
                None
            )
            .is_err()
    );
    let mut stale = context(&second);
    stale.generation = "old".into();
    assert!(
        second
            .call(stale, "akra.test", 1, "echo", json!({"text":"x"}), None)
            .is_err()
    );
}
#[test]
fn registered_output_validation_is_enforced() {
    let handler = Arc::new(TestHandler::default());
    handler.invalid.store(true, Ordering::Release);
    let host = service(Arc::new(MemoryStore::default()), handler);
    enable(&host);
    assert!(call(&host, "echo", json!({"text":"valid input"}), None).is_err());
}
#[test]
fn catalog_exposes_management_schemas_and_bounded_event_resync_sequence() {
    let host = service(
        Arc::new(MemoryStore::default()),
        Arc::new(TestHandler::default()),
    );
    let catalog = host.catalog();
    for operation in &catalog[0].operations {
        jsonschema::validator_for(&operation.input_schema).unwrap();
        jsonschema::validator_for(&operation.output_schema).unwrap();
    }
    let jobs = call(&host, "jobs", json!({}), None).unwrap();
    let jobs_schema = &catalog[0]
        .operations
        .iter()
        .find(|o| o.name == "jobs")
        .unwrap()
        .output_schema;
    assert!(
        jsonschema::validator_for(jobs_schema)
            .unwrap()
            .is_valid(&jobs)
    );
    let configure = catalog[0]
        .operations
        .iter()
        .find(|o| o.name == "configure")
        .unwrap();
    let validator = jsonschema::validator_for(&configure.input_schema).unwrap();
    assert!(validator.is_valid(&json!({"enabled":true,"expected_revision":0})));
    assert!(!validator.is_valid(&json!({"enabled":true})));
    let mut receiver = host.subscribe();
    {
        let mut state = host.lock();
        for _ in 0..80 {
            host.publish(&mut state, "akra.test", None);
        }
    }
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Lagged(_))
    ));
    assert_eq!(host.catalog()[0].sequence, 80);
}

#[test]
fn concurrent_starts_converge_and_expiry_cleans_only_the_original_job() {
    let handler = Arc::new(TestHandler::default());
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let answers = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            call(&host, "work", json!({"text":"parallel"}), Some("a")).unwrap()
        });
        let b = scope.spawn(|| {
            barrier.wait();
            call(&host, "work", json!({"text":"parallel"}), Some("b")).unwrap()
        });
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(answers.0["id"], answers.1["id"]);
    wait_until(|| handler.entered.load(Ordering::Acquire) == 1);
    handler.release.store(true, Ordering::Release);
    let id = answers.0["id"].as_str().unwrap();
    wait_until(|| host.job("akra.test", id).unwrap().state == ExtensionJobState::Completed);
    {
        let mut state = host.lock();
        let mut next = state.durable.clone();
        next.jobs.get_mut(id).unwrap().result_expires_at_ms = 1;
        host.persist(&mut state, next).unwrap();
    }
    host.reap_expired().unwrap();
    let expired = host.job("akra.test", id).unwrap();
    assert_eq!(expired.cleanup, ExtensionCleanupState::Complete);
    assert!(expired.result.is_none());
    assert!(expired.progress.is_null());
    assert_eq!(handler.cleanups.lock().unwrap().len(), 1);
    let retry = call(&host, "work", json!({"text":"parallel"}), Some("a")).unwrap();
    assert_eq!(retry["id"], id);
    assert_eq!(handler.entered.load(Ordering::Acquire), 1);
}
