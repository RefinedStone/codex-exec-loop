use super::*;
use registry::RegisteredExtension;
use sha2::{Digest, Sha256};
use std::panic::{AssertUnwindSafe, catch_unwind};

impl ExtensionService {
    pub(super) fn start_job(
        self: &Arc<Self>,
        context: ExtensionContext,
        entry: &RegisteredExtension,
        action: &str,
        input: Value,
        key: Option<String>,
        activation_revision: u64,
    ) -> Result<Value> {
        if let Some(key) = &key {
            ensure!(
                !key.is_empty() && key.len() <= 128,
                "invalid idempotency key"
            );
        }
        let dedupe = entry.handler.dedupe_key(action, &input)?;
        ensure!(
            dedupe.as_ref().is_none_or(|k| k.len() <= 1024),
            "deduplication key is too large"
        );
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&json!({
                "extension":entry.manifest.id,"version":entry.manifest.version,"action":action,"input":input
            }))?)
        );
        let receipt_key = key.map(|key| format!("{}:{key}", entry.manifest.id));
        self.reap_expired()?;
        let (job, cancel) = {
            let mut state = self.lock();
            ensure!(!state.stopped, "extension host is stopping");
            let activation = &state.durable.activations[&entry.manifest.id];
            ensure!(
                activation.revision == activation_revision
                    && activation.lifecycle == ExtensionLifecycle::Enabled
                    && activation.desired_enabled,
                "extension activation changed before admission"
            );
            if let Some(receipt) = receipt_key
                .as_ref()
                .and_then(|k| state.durable.receipts.get(k))
            {
                ensure!(
                    receipt.fingerprint == fingerprint,
                    "idempotency key was used for another request"
                );
                let previous = state
                    .durable
                    .jobs
                    .get(&receipt.job_id)
                    .context("job receipt has expired")?;
                return Ok(serde_json::to_value(previous)?);
            }
            if let Some(job) = state
                .durable
                .jobs
                .values()
                .find(|job| {
                    job.extension == entry.manifest.id
                        && job.action == action
                        && job.state.active()
                        && dedupe.is_some()
                        && job.dedupe_key == dedupe
                })
                .cloned()
            {
                if let Some(key) = receipt_key {
                    ensure!(
                        state.durable.receipts.len() < MAX_RECEIPTS,
                        "extension receipt limit reached"
                    );
                    let mut next = state.durable.clone();
                    next.receipts.insert(
                        key,
                        ExtensionReceipt {
                            recorded_at_ms: now_ms(),
                            fingerprint,
                            job_id: job.id.clone(),
                        },
                    );
                    self.persist(&mut state, next)?;
                }
                return Ok(serde_json::to_value(job)?);
            }
            ensure!(
                state
                    .durable
                    .jobs
                    .values()
                    .filter(|j| j.state.active())
                    .count()
                    < MAX_ACTIVE_JOBS,
                "extension concurrent job limit reached"
            );
            ensure!(
                receipt_key.is_none() || state.durable.receipts.len() < MAX_RECEIPTS,
                "extension receipt limit reached"
            );
            // Preserve unexpired results. Only already-cleaned terminal metadata can
            // be retired, atomically with the newly accepted job and its receipt.
            let retire = if state.durable.jobs.len() >= MAX_JOBS {
                Some(
                    state
                        .durable
                        .jobs
                        .values()
                        .filter(|job| {
                            !job.state.active() && job.cleanup == ExtensionCleanupState::Complete
                        })
                        .min_by_key(|job| job.updated_at_ms)
                        .context("extension job retention limit reached; wait for result expiry or disable to clear retained results")?
                        .clone(),
                )
            } else {
                None
            };
            let now = now_ms();
            let job = ExtensionJob {
                id: random_id(),
                extension: entry.manifest.id.clone(),
                action: action.into(),
                state: ExtensionJobState::Queued,
                generation: self.generation.clone(),
                revision: 1,
                sequence: state.sequence + 1 + u64::from(retire.is_some()),
                progress: Value::Null,
                result: None,
                error: None,
                cleanup: ExtensionCleanupState::Pending,
                created_at_ms: now,
                updated_at_ms: now,
                result_expires_at_ms: now.saturating_add(retention::RESULT_RETENTION_MS),
                dedupe_key: dedupe,
            };
            let mut next = state.durable.clone();
            if let Some(retired) = &retire {
                next.jobs.remove(&retired.id);
                // Unexpired receipts remain as tombstones and never rerun retired work.
            }
            next.jobs.insert(job.id.clone(), job.clone());
            if let Some(key) = receipt_key {
                next.receipts.insert(
                    key,
                    ExtensionReceipt {
                        recorded_at_ms: now_ms(),
                        fingerprint,
                        job_id: job.id.clone(),
                    },
                );
            }
            self.persist(&mut state, next)?;
            let cancel = Arc::new(AtomicBool::new(false));
            state.cancellations.insert(job.id.clone(), cancel.clone());
            if let Some(retired) = retire {
                self.publish(&mut state, &retired.extension, Some(retired.id));
            }
            self.publish(&mut state, &job.extension, Some(job.id.clone()));
            (job, cancel)
        };
        let service = self.clone();
        let id = job.id.clone();
        let worker_context = ExtensionJobContext {
            context,
            job_id: id.clone(),
            cancel,
            host: Arc::downgrade(self),
        };
        let started = std::thread::Builder::new()
            .name("akra-extension-job".into())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    service.run_job(&worker_context, input, activation_revision)
                }));
                let result = match result {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!("extension worker panicked")),
                };
                if let Err(error) = service.finish_job(&worker_context, result) {
                    // Failed persistence must never be advertised as success.
                    service.shutdown();
                    tracing::error!(error=%error,"extension completion could not be recorded");
                }
            });
        if let Err(error) = started {
            let context = self.context(
                "worker-start-failure".into(),
                (self.scope == ExtensionScope::Repository).then(|| self.host_id.clone()),
            );
            let worker_context = ExtensionJobContext {
                context,
                job_id: job.id.clone(),
                cancel: Arc::new(AtomicBool::new(false)),
                host: Arc::downgrade(self),
            };
            self.finish_job(&worker_context, Err(error.into()))?;
        }
        let job = self.job(&job.extension, &job.id)?;
        let activation = self.lock().durable.activations[&job.extension].clone();
        Ok(serde_json::to_value(visible_job(job, &activation))?)
    }

    fn run_job(&self, context: &ExtensionJobContext, input: Value, revision: u64) -> Result<Value> {
        let job = {
            let mut state = self.lock();
            let job = state
                .durable
                .jobs
                .get(&context.job_id)
                .context("extension job disappeared")?
                .clone();
            let activation = &state.durable.activations[&job.extension];
            ensure!(
                !state.stopped
                    && !context.cancelled()
                    && activation.revision == revision
                    && activation.desired_enabled
                    && activation.lifecycle == ExtensionLifecycle::Enabled,
                "extension job cancelled before dispatch"
            );
            let mut next = state.durable.clone();
            let running = next.jobs.get_mut(&job.id).expect("job");
            running.state = ExtensionJobState::Running;
            running.revision += 1;
            running.updated_at_ms = now_ms();
            running.sequence = state.sequence + 1;
            self.persist(&mut state, next)?;
            self.publish(&mut state, &job.extension, Some(job.id.clone()));
            job
        };
        let entry = self.registry.get(&job.extension)?;
        self.check_permissions(entry)?;
        let output = entry.handler.run(context, &job.action, input)?;
        validate_output(&entry.operations[&job.action].output, &output)?;
        Ok(output)
    }

    fn finish_job(&self, context: &ExtensionJobContext, result: Result<Value>) -> Result<()> {
        let extension = {
            let mut state = self.lock();
            let job = state
                .durable
                .jobs
                .get(&context.job_id)
                .context("extension job disappeared")?;
            ensure!(job.generation == self.generation, "stale worker completion");
            let extension = job.extension.clone();
            let activation = &state.durable.activations[&extension];
            let cancelled = context.cancelled() || !activation.desired_enabled || state.stopped;
            let mut next = state.durable.clone();
            let job = next.jobs.get_mut(&context.job_id).expect("job");
            job.revision += 1;
            job.updated_at_ms = now_ms();
            job.sequence = state.sequence + 1;
            job.result_expires_at_ms = now_ms().saturating_add(retention::RESULT_RETENTION_MS);
            if cancelled {
                job.state = ExtensionJobState::Cancelled;
                job.result = None;
                job.error = None;
            } else {
                match result {
                    Ok(value) => {
                        job.state = ExtensionJobState::Completed;
                        job.result = Some(value);
                    }
                    Err(error) => {
                        job.state = ExtensionJobState::Failed;
                        job.error = Some(error.to_string());
                    }
                }
            }
            self.persist(&mut state, next)?;
            state.cancellations.remove(&context.job_id);
            self.publish(&mut state, &extension, Some(context.job_id.clone()));
            extension
        };
        let job = self.job(&extension, &context.job_id)?;
        if job.state != ExtensionJobState::Completed {
            self.cleanup_job(&context.context, &job)?;
        }
        self.cleanup_disabled(&context.context, &extension)
    }

    pub(super) fn progress(&self, id: &str, progress: Value) -> Result<()> {
        ensure!(
            serde_json::to_vec(&progress)?.len() <= MAX_PAYLOAD,
            "extension progress is too large"
        );
        let mut state = self.lock();
        let job = state
            .durable
            .jobs
            .get(id)
            .context("extension job unavailable")?;
        ensure!(
            job.generation == self.generation && job.state == ExtensionJobState::Running,
            "extension job no longer accepts progress"
        );
        let entry = self.registry.get(&job.extension)?;
        let schema = entry.operations[&job.action]
            .progress
            .as_ref()
            .context("progress is not declared")?;
        ensure!(
            schema.is_valid(&progress),
            "progress does not match its registered schema"
        );
        // Bound persistent progress writes and notifications independently of worker throughput.
        if now_ms().saturating_sub(job.updated_at_ms) < 100 {
            return Ok(());
        }
        let extension = job.extension.clone();
        let mut next = state.durable.clone();
        let job = next.jobs.get_mut(id).expect("job");
        job.progress = progress;
        job.revision += 1;
        job.updated_at_ms = now_ms();
        job.sequence = state.sequence + 1;
        self.persist(&mut state, next)?;
        self.publish(&mut state, &extension, Some(id.into()));
        Ok(())
    }

    pub(super) fn cancel_job(&self, extension: &str, id: &str) -> Result<Value> {
        let mut state = self.lock();
        let job = state
            .durable
            .jobs
            .get(id)
            .filter(|job| job.extension == extension)
            .context("extension job unavailable")?
            .clone();
        if !job.state.active() {
            return Ok(serde_json::to_value(visible_job(
                job,
                &state.durable.activations[extension],
            ))?);
        }
        let mut next = state.durable.clone();
        let job = next.jobs.get_mut(id).expect("job");
        job.state = ExtensionJobState::Cancelling;
        job.sequence = state.sequence + 1;
        job.revision += 1;
        job.updated_at_ms = now_ms();
        self.persist(&mut state, next)?;
        if let Some(cancel) = state.cancellations.get(id) {
            cancel.store(true, Ordering::Release);
        }
        self.publish(&mut state, extension, Some(id.into()));
        Ok(serde_json::to_value(visible_job(
            state.durable.jobs[id].clone(),
            &state.durable.activations[extension],
        ))?)
    }

    pub(super) fn recover_cleanup(&self) -> Result<()> {
        let context = self.context(
            "host-recovery".into(),
            (self.scope == ExtensionScope::Repository).then(|| self.host_id.clone()),
        );
        let jobs = self
            .lock()
            .durable
            .jobs
            .values()
            .filter(|job| {
                matches!(
                    job.state,
                    ExtensionJobState::Interrupted
                        | ExtensionJobState::Failed
                        | ExtensionJobState::Cancelled
                ) && job.cleanup != ExtensionCleanupState::Complete
            })
            .cloned()
            .collect::<Vec<_>>();
        for job in jobs {
            if self.registry.get(&job.extension).is_ok() {
                self.cleanup_job(&context, &job)?;
            }
        }
        let ids = self
            .registry
            .entries()
            .map(|e| e.manifest.id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.cleanup_disabled(&context, &id)?;
        }
        Ok(())
    }
    pub(super) fn cleanup_disabled(
        &self,
        context: &ExtensionContext,
        extension: &str,
    ) -> Result<()> {
        if self.lock().durable.activations[extension].desired_enabled {
            return Ok(());
        }
        let jobs = self
            .lock()
            .durable
            .jobs
            .values()
            .filter(|job| {
                job.extension == extension
                    && !job.state.active()
                    && job.cleanup != ExtensionCleanupState::Complete
            })
            .cloned()
            .collect::<Vec<_>>();
        for job in jobs {
            self.cleanup_job(context, &job)?;
        }
        self.finish_disable_if_clean(extension)
    }

    pub(super) fn finish_disable_if_clean(&self, extension: &str) -> Result<()> {
        let mut state = self.lock();
        if state.durable.activations[extension].desired_enabled {
            return Ok(());
        }
        if state.durable.jobs.values().any(|j| {
            j.extension == extension
                && (j.state.active() || j.cleanup != ExtensionCleanupState::Complete)
        }) {
            return Ok(());
        }
        if state.durable.activations[extension].lifecycle == ExtensionLifecycle::Disabled {
            return Ok(());
        }
        let mut next = state.durable.clone();
        let activation = next.activations.get_mut(extension).expect("activation");
        activation.lifecycle = ExtensionLifecycle::Disabled;
        activation.error = None;
        activation.revision += 1;
        self.persist(&mut state, next)?;
        self.publish(&mut state, extension, None);
        Ok(())
    }
    pub(super) fn cleanup_job(&self, context: &ExtensionContext, job: &ExtensionJob) -> Result<()> {
        // Serial cleanup claims prevent disable and worker completion from racing the same index.
        {
            let mut state = self.lock();
            if state
                .durable
                .jobs
                .get(&job.id)
                .is_none_or(|j| j.cleanup == ExtensionCleanupState::Complete)
            {
                return Ok(());
            }
            if !state.cleanup_claims.insert(job.id.clone()) {
                return Ok(());
            }
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.registry
                .get(&job.extension)?
                .handler
                .cleanup(context, job)
        }));
        let result = match result {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("extension cleanup panicked")),
        };
        let mut state = self.lock();
        state.cleanup_claims.remove(&job.id);
        let mut next = state.durable.clone();
        let saved = next
            .jobs
            .get_mut(&job.id)
            .context("extension job disappeared")?;
        ensure!(
            saved.generation == job.generation,
            "stale cleanup generation"
        );
        saved.revision += 1;
        saved.updated_at_ms = now_ms();
        saved.sequence = state.sequence + 1;
        match result {
            Ok(()) => {
                saved.cleanup = ExtensionCleanupState::Complete;
                saved.result = None;
                saved.progress = Value::Null;
            }
            Err(error) => {
                saved.cleanup = ExtensionCleanupState::Failed;
                saved.error = Some(format!("cleanup failed: {error}"));
                let activation = next
                    .activations
                    .get_mut(&job.extension)
                    .context("activation disappeared")?;
                activation.lifecycle = ExtensionLifecycle::Error;
                activation.error = Some(error.to_string());
                activation.revision += 1;
            }
        }
        self.persist(&mut state, next)?;
        self.publish(&mut state, &job.extension, Some(job.id.clone()));
        Ok(())
    }
    pub(super) fn record_cleanup_error(&self, extension: &str, error: String) {
        let mut state = self.lock();
        let mut next = state.durable.clone();
        if let Some(activation) = next.activations.get_mut(extension) {
            activation.lifecycle = ExtensionLifecycle::Error;
            activation.error = Some(error);
            activation.revision += 1;
            if self.persist(&mut state, next).is_ok() {
                self.publish(&mut state, extension, None);
            }
        }
    }
}
