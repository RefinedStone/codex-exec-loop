//! The local Desktop reverse RPC. Request bodies and responses bypass session recording.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::future::BoxFuture;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::application::port::outbound::agent_backend_port::{
    AgentBackendPort, BackendCapabilities, BackendHealth, Dispatch, Observation,
    TextDispatchProfile,
};
use crate::application::port::outbound::delegation_store_port::{DelegationStorePort, Job};
use crate::application::port::outbound::pr_review_port::PrReviewModelCall;
use crate::domain::delegation::ISSUE_DRAFT_REVIEW_FEATURE;
use crate::domain::delegation::policy::{CHAT, CODEX, OPENAI_TEXT};
use crate::domain::delegation::{
    Attempt, BackendState, ChatBackendStatus, Circuit, Delivery, EndReason, Evidence,
    EvidenceSource, Fallback, NextAction, Phase, Recovery, State, can_fall_back, next_action,
    select_backend,
};

pub(super) const PREFIX: &str = "akra/delegationDriver/";
const ID_PREFIX: &str = "akra-delegation-driver:";
const RPC_TIMEOUT: Duration = Duration::from_secs(4);
type PendingReplies = HashMap<u64, (u64, oneshot::Sender<Value>)>;
pub(super) type DelegationStore = Arc<dyn DelegationStorePort>;

pub(crate) struct Delegations {
    sequence: AtomicU64,
    driver: Mutex<Option<(u64, mpsc::Sender<Message>)>>,
    pending: Mutex<PendingReplies>,
    workers: Mutex<BTreeMap<PathBuf, tokio::task::AbortHandle>>,
    codex: Arc<dyn AgentBackendPort>,
    read_account: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    issue_review_profile: TextDispatchProfile,
}

#[cfg(test)]
impl Default for Delegations {
    fn default() -> Self {
        crate::composition::production::build_delegations()
    }
}

impl Drop for Delegations {
    fn drop(&mut self) {
        for worker in self.workers.get_mut().unwrap().values() {
            worker.abort();
        }
    }
}

impl Delegations {
    pub(crate) fn new(
        codex: Arc<dyn AgentBackendPort>,
        read_account: Arc<dyn Fn() -> Option<String> + Send + Sync>,
        issue_review_profile: TextDispatchProfile,
    ) -> Self {
        Self {
            sequence: AtomicU64::default(),
            driver: Mutex::default(),
            pending: Mutex::default(),
            workers: Mutex::default(),
            codex,
            read_account,
            issue_review_profile,
        }
    }

    #[cfg(test)]
    pub(super) fn set_test_account_reader(
        &mut self,
        read_account: impl Fn() -> Option<String> + Send + Sync + 'static,
    ) {
        self.read_account = Arc::new(read_account);
    }

    fn next_id(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::Relaxed)
    }

    pub(super) fn connection(self: &Arc<Self>, outbox: mpsc::Sender<Message>) -> DriverConnection {
        DriverConnection {
            service: self.clone(),
            id: self.next_id(),
            outbox,
        }
    }

    pub(super) fn track(
        self: &Arc<Self>,
        runtime: PathBuf,
        store: DelegationStore,
        workspace: String,
    ) {
        self.workers
            .lock()
            .unwrap()
            .entry(runtime)
            .or_insert_with(|| {
                let service = Arc::downgrade(self);
                let store = store.clone();
                tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(2));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tick.tick().await;
                    let Some(service) = service.upgrade() else {
                        break;
                    };
                    if let Err(error) = service.reconcile_workspace(&store, &workspace).await {
                        tracing::warn!(error = %error, "delegation ledger reconciliation failed");
                    }
                }
            })
            .abort_handle()
            });
    }

    pub(super) async fn account_id(&self) -> Option<String> {
        let read_account = self.read_account.clone();
        tokio::task::spawn_blocking(move || read_account())
            .await
            .ok()
            .flatten()
    }

    pub(super) async fn status(&self) -> ChatBackendStatus {
        let params = self
            .account_id()
            .await
            .map_or_else(|| json!({}), |id| json!({"expectedAccountId": id}));
        self.call("status", params)
            .await
            .unwrap_or_else(|_| ChatBackendStatus::unavailable("desktop_unavailable"))
    }

    pub(super) async fn circuits(
        &self,
        store: DelegationStore,
        workspace: String,
    ) -> Result<(Circuit, Circuit)> {
        db(store, move |store| {
            let route = crate::composition::production::delegation_route(&workspace, "manual")?;
            let now = chrono::Utc::now().timestamp_millis();
            let (mut chat, mut codex) = (Circuit::Closed, Circuit::Closed);
            for backend in route.backends {
                if store
                    .circuit_until(&backend.name)?
                    .is_some_and(|at| at > now)
                {
                    match backend.kind.as_str() {
                        CHAT => chat = Circuit::Open,
                        CODEX => codex = Circuit::Open,
                        _ => {}
                    }
                }
            }
            Ok((chat, codex))
        })
        .await
    }

    pub(super) async fn codex_status(&self) -> crate::domain::delegation::CodexBackendStatus {
        crate::domain::delegation::CodexBackendStatus::from_status(
            self.codex.health().await.status,
            self.codex.capabilities().defensive_isolation,
        )
    }

    async fn call<T: DeserializeOwned>(&self, name: &str, params: Value) -> Result<T> {
        let id = self.next_id();
        let (tx, rx) = oneshot::channel();
        let outbox = {
            let driver = self.driver.lock().unwrap();
            let (connection, outbox) = driver.as_ref().context("Desktop driver not registered")?;
            self.pending.lock().unwrap().insert(id, (*connection, tx));
            outbox.clone()
        };
        let _pending = PendingCall(self, id);
        let message = json!({
            "jsonrpc": "2.0", "id": format!("{ID_PREFIX}{id}"),
            "method": format!("{PREFIX}{name}"), "params": params,
        });
        let response = tokio::time::timeout(RPC_TIMEOUT, async {
            outbox
                .send(Message::text(message.to_string()))
                .await
                .context("Desktop driver closed")?;
            rx.await.context("Desktop driver closed")
        })
        .await??;
        if response.get("error").is_some() {
            bail!("Desktop driver rejected the request");
        }
        let result = response
            .get("result")
            .context("Desktop driver result missing")?;
        serde_json::from_value(result.clone()).context("Invalid Desktop driver result")
    }

    #[cfg(test)]
    async fn reconcile(&self, store: &DelegationStore) -> Result<()> {
        self.reconcile_workspace(store, "/fixture").await
    }

    async fn reconcile_workspace(&self, store: &DelegationStore, workspace: &str) -> Result<()> {
        let active = db(store.clone(), |store| store.active()).await?;
        if active.is_empty() {
            return Ok(());
        }
        for job in active {
            if let Err(error) = self.advance(store, job, None, workspace).await {
                tracing::warn!(error = %error, "delegation job reconciliation failed");
            }
        }
        Ok(())
    }

    async fn advance(
        &self,
        store: &DelegationStore,
        job: Job,
        review: Option<&dyn PrReviewModelCall>,
        workspace: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        let mut old = job.view;
        let action = next_action(&old, job.stop_pending, now);
        let account_id = if action == NextAction::Select && old.session.selected_url().is_some() {
            let current = self.account_id().await;
            if !current.as_ref().is_some_and(|id| {
                job.account_digest.as_deref() == Some(account_fingerprint(id).as_slice())
            }) {
                let mut next = old.clone();
                next.state = State::AwaitingHuman;
                next.phase = Phase::Blocked;
                next.reason = Some(
                    if current.is_some() {
                        "chat_account_mismatch"
                    } else {
                        "chat_account_unknown"
                    }
                    .into(),
                );
                save(store, old, next, None).await?;
                return Ok(());
            }
            current
        } else {
            None
        };
        if action == NextAction::Observe
            && old.state == State::AwaitingHuman
            && old.attempts.is_empty()
            && old.session.selected_url().is_some()
        {
            return Ok(());
        }
        // Only the repository watcher holds fresh PR data. Background reconciliation
        // may expire or release these jobs, but must never dispatch or resend them.
        if job.route.requires_defensive_isolation
            && job.route.backends.iter().any(|b| b.kind == OPENAI_TEXT)
            && review.is_none()
            && !matches!(action, NextAction::Expire | NextAction::Stop)
        {
            return Ok(());
        }
        let mut rpc_id = old
            .attempts
            .last()
            .map_or_else(|| old.id.clone(), |a| a.id.clone());
        match action {
            NextAction::Stop => {
                let id = old.id;
                if old
                    .attempts
                    .last()
                    .is_some_and(|a| a.backend_kind == OPENAI_TEXT)
                {
                    // The process owning the POST releases capacity when it drops the
                    // request. A restarted process can release an orphan at the deadline.
                    if old.deadline.is_some_and(|deadline| now >= deadline) {
                        store.take_stop(&id)?;
                    }
                    return Ok(());
                }
                if db(store.clone(), move |store| store.take_stop(&id)).await? {
                    if old
                        .attempts
                        .last()
                        .is_some_and(|attempt| attempt.backend_kind == CODEX)
                    {
                        self.codex.stop(&rpc_id).await?;
                    } else {
                        let _ = self.stop(&rpc_id).await;
                    }
                }
                return Ok(());
            }
            NextAction::Expire => {
                let mut next = old.clone();
                next.state = State::Expired;
                next.reason = Some("deadline_exceeded".into());
                let stop = old.state != State::Queued
                    && old.delivery != Delivery::NotSent
                    && old
                        .attempts
                        .last()
                        .is_none_or(|a| a.backend_kind != OPENAI_TEXT);
                db(store.clone(), move |store| {
                    store.update(&old, &mut next, None, stop)
                })
                .await?;
                return Ok(());
            }
            NextAction::None => return Ok(()),
            _ => {}
        }
        let submitting = action == NextAction::Select;
        if !submitting
            && old
                .attempts
                .last()
                .is_some_and(|a| a.backend_kind == OPENAI_TEXT)
        {
            // A second watcher or restart must never replay an ambiguous HTTP POST.
            return Ok(());
        }
        let observed: Result<Observation> = if submitting {
            let mut tried = old.clone();
            let mut selected = None;
            for backend in &job.route.backends {
                if old
                    .attempts
                    .iter()
                    .any(|attempt| attempt.backend == backend.name)
                {
                    continue;
                }
                let name = backend.name.clone();
                let circuit = db(store.clone(), move |store| store.circuit_until(&name)).await?;
                let capable = match backend.kind.as_str() {
                    CODEX => self.codex.capabilities().defensive_isolation,
                    CHAT => self.capabilities().defensive_isolation,
                    OPENAI_TEXT => review.is_some() && job.route.requires_defensive_isolation,
                    _ => false,
                };
                let status = if (job.route.requires_defensive_isolation && !capable)
                    || circuit.is_some_and(|until| now < until)
                {
                    None
                } else if backend.kind == OPENAI_TEXT && capable {
                    Some(BackendHealth {
                        status: crate::domain::delegation::BackendStatus {
                            state: BackendState::Ready,
                            reason: None,
                        },
                        retry_at: None,
                    })
                } else if backend.kind == CODEX
                    && job.route.effect == crate::domain::delegation::Effect::ReadOnly
                {
                    Some(self.codex.health().await)
                } else if backend.kind == CHAT {
                    Some(self.health().await)
                } else {
                    None
                };
                let reason = if circuit.is_some_and(|until| now < until) {
                    EndReason::Unavailable
                } else {
                    status
                        .as_ref()
                        .and_then(|s| s.status.reason.as_deref())
                        .map_or(EndReason::Unavailable, |reason| {
                            match classify_reason(Some(reason), Phase::Unknown) {
                                EndReason::Unknown => EndReason::Unavailable,
                                classified => classified,
                            }
                        })
                };
                let id = if tried.attempts.is_empty() {
                    old.id.clone()
                } else {
                    format!("{}-{}", old.id, tried.attempts.len() + 1)
                };
                let ready = status
                    .as_ref()
                    .is_some_and(|health| health.status.state == BackendState::Ready);
                let choice = select_backend([(backend, ready, circuit)], now);
                if let Some(previous) = tried.attempts.last()
                    && let Some(reason) = previous.reason
                {
                    tried.fallbacks.push(Fallback {
                        from: previous.backend.clone(),
                        reason,
                        to: backend.name.clone(),
                        at: now,
                    });
                }
                tried.attempts.push(Attempt {
                    id,
                    backend: backend.name.clone(),
                    backend_kind: backend.kind.clone(),
                    model: backend.model.clone(),
                    delivery: if choice.is_some() {
                        Delivery::Uncertain
                    } else {
                        Delivery::NotSent
                    },
                    phase: Phase::Unknown,
                    reason: choice.is_none().then_some(reason),
                    retry_at: status.and_then(|s| s.retry_at),
                    recovery: None,
                    resumes: None,
                    observations: vec![Evidence {
                        source: EvidenceSource::Coordinator,
                        at: now,
                        delivery: if choice.is_some() {
                            Delivery::Uncertain
                        } else {
                            Delivery::NotSent
                        },
                        phase: Phase::Unknown,
                    }],
                });
                if choice.is_some() {
                    selected = choice;
                    break;
                }
                let allow_fallback = job.route.fall_back_on.contains(&reason);
                if !allow_fallback {
                    tried.state = State::Failed;
                    tried.reason = Some("route_exhausted".into());
                }
                // Persist each skipped candidate so its metadata and circuit update belong to
                // the same compare-and-swap as the observation that justified skipping it.
                if !save(store, old.clone(), tried.clone(), None).await? {
                    return Ok(());
                }
                if !allow_fallback {
                    return Ok(());
                }
                old = db(store.clone(), {
                    let id = old.id.clone();
                    move |store| store.find(&id)
                })
                .await?
                .context("delegation disappeared after backend selection")?;
                if old.state != State::Queued {
                    return Ok(());
                }
                tried = old.clone();
            }
            let Some(backend) = selected else {
                tried.state = State::Failed;
                tried.reason = Some("route_exhausted".into());
                save(store, old, tried, None).await?;
                return Ok(());
            };
            rpc_id = tried.attempts.last().unwrap().id.clone();
            let mut claimed = tried;
            // A persisted claim is ambiguous after a crash. Future ticks only observe.
            claimed.state = State::Dispatching;
            claimed.delivery = Delivery::Uncertain;
            claimed.outcome_unknown = true;
            let id = old.id.clone();
            if !save(store, old, claimed, None).await? {
                return Ok(());
            }
            old = db(store.clone(), move |store| store.find(&id))
                .await?
                .context("delegation disappeared after claim")?;
            if old.state.terminal() {
                return Ok(());
            }
            let mut request = Dispatch {
                attempt_id: &rpc_id,
                job_id: &old.id,
                instruction: &job.instruction,
                max_resumes: if backend.kind == CHAT {
                    Some(backend.max_resumes.unwrap_or(2).min(2))
                } else {
                    backend.max_resumes
                },
                chat_session: account_id.as_deref().map(|id| (&old.session, id)),
                callback_token: "",
                callback_workspace: workspace,
                model: backend.model.as_deref(),
                profile: (old.caller == ISSUE_DRAFT_REVIEW_FEATURE)
                    .then_some(self.issue_review_profile),
                timeout: Duration::from_millis(
                    old.deadline
                        .unwrap_or(now)
                        .saturating_sub(chrono::Utc::now().timestamp_millis())
                        .max(0) as u64,
                ),
            };
            if backend.kind == CODEX {
                self.codex.dispatch(request).await
            } else if backend.kind == OPENAI_TEXT {
                let answer = {
                    let post = review.context("review input unavailable")?.complete(
                        request.model.context("review model missing")?,
                        request.timeout,
                    );
                    tokio::pin!(post);
                    loop {
                        tokio::select! {
                            response = &mut post => break Some(response),
                            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                                if store.find(&old.id)?.is_none_or(|job| job.state.terminal()) { break None; }
                            }
                        }
                    }
                };
                let Some(answer) = answer else {
                    store.take_stop(&old.id)?;
                    return Ok(());
                };
                let mut next = old.clone();
                let mut result = None;
                let at = chrono::Utc::now().timestamp_millis();
                let reason = match answer {
                    Ok(text) => {
                        next.apply_observation(Delivery::Observed, Phase::Complete);
                        next.state = State::Succeeded;
                        result = Some(text);
                        None
                    }
                    Err(reason) => {
                        let delivery = match reason {
                            EndReason::Unavailable => Delivery::NotSent,
                            EndReason::Unknown => Delivery::Uncertain,
                            _ => Delivery::Observed,
                        };
                        next.delivery = delivery;
                        next.apply_observation(delivery, Phase::Failed);
                        next.reason = Some(format!(
                            "review_{}",
                            serde_json::to_value(reason)?.as_str().unwrap_or("failed")
                        ));
                        if job.route.fall_back_on.contains(&reason)
                            && can_fall_back(job.route.effect, delivery, true, false)
                        {
                            next.state = State::Queued;
                        }
                        Some(reason)
                    }
                };
                if let Some(a) = next.attempts.last_mut() {
                    a.reason = reason;
                    a.record(Evidence {
                        source: EvidenceSource::Coordinator,
                        at,
                        delivery: next.delivery,
                        phase: next.phase,
                    });
                }
                if next.state == State::Queued {
                    next.delivery = Delivery::NotSent;
                    next.phase = Phase::Unknown;
                }
                if old.deadline.is_some_and(|d| at >= d) {
                    next.state = State::Expired;
                    next.reason = Some("deadline_exceeded".into());
                    result = None;
                }
                let id = old.id.clone();
                if !save(store, old, next, result).await? {
                    store.take_stop(&id)?;
                }
                return Ok(());
            } else {
                let id = old.id.clone();
                let Some(token) = db(store.clone(), move |store| {
                    store.issue_callback_capability(&id)
                })
                .await?
                else {
                    return Ok(());
                };
                request.callback_token = &token;
                self.dispatch(request).await
            }
        } else if old
            .attempts
            .last()
            .is_some_and(|attempt| attempt.backend_kind == CODEX)
        {
            self.codex.observe(&rpc_id).await
        } else {
            self.observe(&rpc_id).await
        };
        let codex = old
            .attempts
            .last()
            .is_some_and(|attempt| attempt.backend_kind == CODEX);
        let mut next = old.clone();
        let mut result = None;
        let mut end = None;
        let mut retry_at = None;
        let mut restart = false;
        let (mut recovery, mut resumes) = (None, None);
        if let Ok(observation) = observed {
            if codex
                && observation.delivery == Delivery::Uncertain
                && observation.phase == Phase::Working
            {
                return Ok(());
            }
            restart = codex
                && observation.reason.as_deref() == Some("codex_restart")
                && observation.delivery == Delivery::Uncertain;
            // Desktop owns this job once it queues the send. Its page has no delivery
            // evidence yet. Retain the attempt and observe it, including after restart.
            let desktop_queued = !codex
                && observation.delivery == Delivery::NotSent
                && observation.phase == Phase::Unknown
                && observation.reason.is_none();
            let observed_delivery = if desktop_queued {
                Delivery::Accepted
            } else if restart && old.delivery == Delivery::Observed {
                Delivery::Observed
            } else {
                observation.delivery
            };
            recovery = observation.recovery.or(observation
                .recovery_exhausted
                .then_some(Recovery::Exhausted));
            resumes = observation.resumes.filter(|count| *count <= 2);
            let recovery_exhausted =
                recovery == Some(Recovery::Exhausted) || observation.phase == Phase::Stopped;
            if (submitting || codex || observation.phase == Phase::Blocked)
                && observed_delivery == Delivery::NotSent
                && !matches!(old.delivery, Delivery::Accepted | Delivery::Observed)
            {
                next.delivery = Delivery::NotSent;
                next.outcome_unknown = false;
                next.phase = observation.phase;
            }
            next.apply_observation(observed_delivery, observation.phase);
            if next.phase == Phase::Failed && !recovery_exhausted && !next.outcome_unknown {
                next.state = State::Interrupted;
            }
            next.reason = observation.reason;
            retry_at = observation.retry_at;
            if next.reason.as_deref() == Some("chat_job_missing") {
                next.state = State::AwaitingHuman;
                next.delivery = Delivery::Uncertain;
                next.outcome_unknown = true;
            }
            if let Some(url) = observation
                .conversation_url
                .filter(|url| safe_chat_url(url))
            {
                next.conversation_url = Some(url);
            }
            if next.phase == Phase::Complete
                && matches!(next.delivery, Delivery::Accepted | Delivery::Observed)
            {
                let collected = if codex {
                    self.codex.collect(&rpc_id).await
                } else {
                    self.collect(&rpc_id).await
                };
                if let Ok(text) = collected
                    && !text.trim().is_empty()
                    && text.len() <= 1_048_576
                {
                    next.state = State::Succeeded;
                    next.reason = None;
                    result = Some(text);
                } else {
                    next.reason = Some("result_unavailable".to_owned());
                }
            }
            if next.phase == Phase::Working && !next.outcome_unknown {
                next.last_progress_at = Some(now);
            }
            if next.outcome_unknown {
                next.state = State::AwaitingHuman;
                end = Some(if restart {
                    EndReason::Interrupted
                } else {
                    EndReason::Unknown
                });
            } else if !codex && next.phase == Phase::Blocked {
                next.state = State::AwaitingHuman;
                end = Some(EndReason::NeedsHuman);
            } else if next.state != State::Succeeded
                && (next.delivery == Delivery::NotSent
                    || recovery_exhausted
                    || next.phase == Phase::Blocked)
            {
                let reason = classify_reason(next.reason.as_deref(), next.phase);
                end = Some(reason);
                if reason != EndReason::Unknown
                    && recovery != Some(Recovery::InProgress)
                    && (old.session.selected_url().is_none() || next.delivery == Delivery::NotSent)
                    && job.route.fall_back_on.contains(&reason)
                    && can_fall_back(
                        job.route.effect,
                        next.delivery,
                        recovery_exhausted,
                        next.phase == Phase::Stopped,
                    )
                {
                    next.state = State::Queued;
                } else if next.delivery == Delivery::NotSent
                    || (codex && reason != EndReason::NeedsHuman)
                {
                    next.state = State::Failed;
                } else if recovery_exhausted
                    && (job.route.effect == crate::domain::delegation::Effect::SideEffecting
                        || old.session.selected_url().is_some())
                {
                    next.state = State::AwaitingHuman;
                    if next.reason.is_none() {
                        next.reason = Some("chat_recovery_exhausted".into());
                    }
                }
            }
        } else {
            next.phase = Phase::Unreachable;
            next.state = if next.outcome_unknown {
                State::AwaitingHuman
            } else {
                State::Interrupted
            };
            next.reason = Some("desktop_unreachable".to_owned());
            if next.outcome_unknown {
                end = Some(EndReason::Unknown);
            }
        }
        let delivery = next.delivery;
        let phase = next.phase;
        if let Some(attempt) = next.attempts.last_mut() {
            attempt.reason = end;
            attempt.retry_at = retry_at;
            if recovery.is_some() {
                attempt.recovery = recovery;
            }
            if resumes.is_some() {
                attempt.resumes = resumes;
            }
            attempt.record(Evidence {
                source: if codex {
                    if delivery == Delivery::Observed && !restart {
                        EvidenceSource::Stream
                    } else {
                        EvidenceSource::Coordinator
                    }
                } else {
                    EvidenceSource::Browser
                },
                at: now,
                delivery,
                phase,
            });
        }
        if next.state == State::Queued {
            next.delivery = Delivery::NotSent;
            next.phase = Phase::Unknown;
            next.conversation_url = None;
            next.last_progress_at = None;
        }
        if matches!(
            next_action(&old, false, chrono::Utc::now().timestamp_millis()),
            NextAction::Expire
        ) {
            next.state = State::Expired;
            next.reason = Some("deadline_exceeded".into());
            let stop = delivery != Delivery::NotSent;
            db(store.clone(), move |store| {
                store.update(&old, &mut next, None, stop)
            })
            .await?;
            return Ok(());
        }
        let forget_codex = codex && (end.is_some() || result.is_some());
        if save(store, old, next, result).await? && forget_codex {
            self.codex.forget(&rpc_id);
        }
        Ok(())
    }
}

/// Drives only the keyed job submitted by the watcher, never unrelated gateway work.
pub(crate) async fn advance_pr_review(
    store: DelegationStore,
    id: &str,
    review: &dyn PrReviewModelCall,
) -> Result<()> {
    if let Some(job) = store.active()?.into_iter().find(|j| j.view.id == id) {
        anyhow::ensure!(
            job.route.requires_defensive_isolation,
            "invalid review route"
        );
        crate::composition::production::build_delegations()
            .advance(&store, job, Some(review), "")
            .await?;
    }
    Ok(())
}

/// The watcher can run without a gateway session. Release expired review claims
/// before admitting work, including claims left behind by a stopped watcher.
pub(crate) async fn expire_pr_reviews(store: &DelegationStore) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let coordinator = crate::composition::production::build_delegations();
    for job in store.active()? {
        if job.route.requires_defensive_isolation
            && job.route.backends.iter().any(|b| b.kind == OPENAI_TEXT)
            && matches!(
                next_action(&job.view, job.stop_pending, now),
                NextAction::Expire | NextAction::Stop
            )
        {
            coordinator.advance(store, job, None, "").await?;
        }
    }
    Ok(())
}

impl AgentBackendPort for Delegations {
    fn capabilities(&self) -> BackendCapabilities {
        // Text-only reverse RPC does not attest to the browser account's
        // available tools or enforce remote network isolation.
        BackendCapabilities {
            defensive_isolation: false,
        }
    }

    fn health(&self) -> BoxFuture<'_, BackendHealth> {
        Box::pin(async move {
            self.call("status", json!({}))
                .await
                .unwrap_or(BackendHealth {
                    status: ChatBackendStatus::unavailable("desktop_unavailable").status,
                    retry_at: None,
                })
        })
    }

    fn dispatch<'a>(&'a self, request: Dispatch<'a>) -> BoxFuture<'a, Result<Observation>> {
        Box::pin(async move {
            let mut params = json!({
                "jobId": request.attempt_id,
                "instruction": request.instruction,
                "callback": {"jobId": request.job_id, "token": request.callback_token, "workspace": request.callback_workspace},
            });
            if let Some(limit) = request.max_resumes {
                params["maxResumes"] = json!(limit);
            }
            if let Some((session, account_id)) = request.chat_session {
                params["session"] = json!({"mode":"continue", "chatUrl": session.selected_url()});
                params["expectedAccountId"] = json!(account_id);
            }
            self.call("submit", params).await
        })
    }

    fn observe<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<Observation>> {
        Box::pin(async move { self.call("observe", json!({"jobId": attempt_id})).await })
    }

    fn collect<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let response: TextResult = self
                .call("readResult", json!({"jobId": attempt_id}))
                .await?;
            Ok(response.text)
        })
    }

    fn stop<'a>(&'a self, attempt_id: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.call::<Value>("stop", json!({"jobId": attempt_id}))
                .await?;
            Ok(())
        })
    }
}

pub(super) fn account_fingerprint(account: &str) -> [u8; 32] {
    Sha256::digest(account.as_bytes()).into()
}

fn classify_reason(reason: Option<&str>, phase: Phase) -> EndReason {
    match (reason, phase) {
        (Some("quota" | "rate_limited" | "usage_limit_exceeded"), _) => EndReason::Quota,
        (Some("transient" | "chat_queue_full" | "chat_host_busy"), _) => EndReason::Transient,
        (Some("rejected"), _) => EndReason::Rejected,
        (Some("result_invalid"), _) => EndReason::ResultInvalid,
        (Some("desktop_unavailable" | "codex_unavailable" | "unavailable"), _) => {
            EndReason::Unavailable
        }
        (Some("unknown"), _) => EndReason::Unknown,
        (Some("codex_interrupted" | "codex_restart"), _) => EndReason::Interrupted,
        (_, Phase::Blocked) => EndReason::NeedsHuman,
        (_, Phase::Failed | Phase::Stopped | Phase::Disconnected) => EndReason::Interrupted,
        _ => EndReason::Unknown,
    }
}

pub(super) async fn db<T: Send + 'static>(
    store: DelegationStore,
    operation: impl FnOnce(DelegationStore) -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(move || operation(store)).await?
}

async fn save(
    store: &DelegationStore,
    old: crate::domain::delegation::Delegation,
    mut next: crate::domain::delegation::Delegation,
    result: Option<String>,
) -> Result<bool> {
    db(store.clone(), move |store| {
        store.update(&old, &mut next, result.as_deref(), false)
    })
    .await
}

fn safe_chat_url(url: &str) -> bool {
    url.strip_prefix("https://chatgpt.com/c/")
        .is_some_and(|id| {
            !id.is_empty()
                && id.len() <= 128
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[derive(Deserialize)]
struct TextResult {
    text: String,
}

struct PendingCall<'a>(&'a Delegations, u64);

impl Drop for PendingCall<'_> {
    fn drop(&mut self) {
        self.0.pending.lock().unwrap().remove(&self.1);
    }
}

pub(super) struct DriverConnection {
    service: Arc<Delegations>,
    id: u64,
    outbox: mpsc::Sender<Message>,
}

impl DriverConnection {
    pub(super) fn request(&self, id: Value, method: &str) -> String {
        let result = match method.strip_prefix(PREFIX) {
            Some("register") => {
                let mut driver = self.service.driver.lock().unwrap();
                if driver
                    .as_ref()
                    .is_some_and(|(id, outbox)| *id != self.id && !outbox.is_closed())
                {
                    Err("another Desktop driver is already registered")
                } else {
                    *driver = Some((self.id, self.outbox.clone()));
                    Ok(())
                }
            }
            Some("unregister") => {
                self.disconnect();
                Ok(())
            }
            _ => Err("unknown Desktop driver registration method"),
        };
        match result {
            Ok(()) => json!({"jsonrpc":"2.0","id":id,"result":{}}),
            Err(message) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":message}})
            }
        }
        .to_string()
    }

    pub(super) fn answer(&self, id: u64, response: Value) {
        let mut pending = self.service.pending.lock().unwrap();
        if pending
            .get(&id)
            .is_some_and(|(connection, _)| *connection == self.id)
            && let Some((_, sender)) = pending.remove(&id)
        {
            let _ = sender.send(response);
        }
    }

    fn disconnect(&self) {
        let mut driver = self.service.driver.lock().unwrap();
        if driver.as_ref().is_some_and(|(id, _)| *id == self.id) {
            *driver = None;
        }
        self.service
            .pending
            .lock()
            .unwrap()
            .retain(|_, (id, _)| *id != self.id);
    }
}

impl Drop for DriverConnection {
    fn drop(&mut self) {
        self.disconnect();
    }
}

pub(super) fn response(text: &str) -> Option<(u64, Value)> {
    if !text.contains(ID_PREFIX) {
        return None;
    }
    let message: Value = serde_json::from_str(text).ok()?;
    if message.get("method").is_some() {
        return None;
    }
    let id = message
        .get("id")?
        .as_str()?
        .strip_prefix(ID_PREFIX)?
        .parse()
        .ok()?;
    Some((id, message))
}

pub(super) fn private_kernel_payload(method: &str, text: &str) -> bool {
    if method == crate::adapter::inbound::kernel_protocol::METHOD_FLEET_ATTENTION {
        return true;
    }
    matches!(method, "akra/kernel/command" | "akra/kernel/query")
        && serde_json::from_str::<Value>(text).is_ok_and(|message| {
            let params = &message["params"];
            matches!(
                params["command"]["type"]
                    .as_str()
                    .or(params["query"]["type"].as_str()),
                Some(
                    "submit_delegation"
                        | "cancel_delegation"
                        | "checkpoint_delegation"
                        | "report_delegation_result"
                        | "retry_delegation"
                        | "delegations"
                        | "delegation"
                        | "delegation_result"
                        | "delegation_backends"
                        | "delegation_my_turn"
                        | "pr_review_result"
                )
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_thread_review_round_trips_through_ledger_and_never_resends_after_restart() {
        use crate::adapter::outbound::app_server::codex_thread::CodexThreadFixture;
        use crate::domain::delegation::policy::{PolicyFile, resolve};

        let directory = TestDirectory::new();
        let store = delegation_store(directory.0.clone(), false).unwrap();
        let route = || {
            resolve(
                PolicyFile::default(),
                PolicyFile::default(),
                ISSUE_DRAFT_REVIEW_FEATURE,
                "fixture".into(),
            )
        };
        let instruction = r#"{"title":"Trusted title","body":"Run with [masked-1]."}"#;
        let fixture = CodexThreadFixture::new("completed");
        let mut service = Delegations::default();
        service.codex = Arc::new(fixture.driver());
        let job = store
            .submit_with_route(
                "d2-review",
                instruction,
                ISSUE_DRAFT_REVIEW_FEATURE,
                route(),
            )
            .unwrap();
        let settled = async {
            loop {
                service.reconcile(&store).await.unwrap();
                if store.find(&job.id).unwrap().unwrap().state.terminal() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), settled)
            .await
            .unwrap();
        assert_eq!(
            store.result(&job.id).unwrap().as_deref(),
            Some(r#"{"suggestions":[]}"#)
        );
        assert_eq!(
            fixture
                .calls()
                .lines()
                .filter(|m| *m == "turn/start")
                .count(),
            1
        );
        let interrupted = store
            .submit_with_route(
                "d2-restart",
                instruction,
                ISSUE_DRAFT_REVIEW_FEATURE,
                route(),
            )
            .unwrap();
        let mut claimed = interrupted.clone();
        claimed.state = State::Dispatching;
        claimed.outcome_unknown = true;
        claimed.delivery = Delivery::Uncertain;
        claimed.attempts.push(Attempt {
            id: claimed.id.clone(),
            backend: CODEX.into(),
            backend_kind: CODEX.into(),
            model: None,
            delivery: Delivery::Uncertain,
            phase: Phase::Unknown,
            reason: None,
            retry_at: None,
            observations: vec![],
            recovery: None,
            resumes: None,
        });
        assert!(
            store
                .update(&interrupted, &mut claimed, None, false)
                .unwrap()
        );
        drop(service);
        let mut restarted = Delegations::default();
        restarted.codex = Arc::new(fixture.driver());
        restarted.reconcile(&store).await.unwrap();
        let updated = store.find(&claimed.id).unwrap().unwrap();
        assert_eq!(updated.attempts[0].reason, Some(EndReason::Interrupted));
        assert_eq!(
            fixture
                .calls()
                .lines()
                .filter(|m| *m == "turn/start")
                .count(),
            1
        );
    }
    use crate::composition::production::delegation_store;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = crate::test_utils::platform_safe_temp_dir()
                .join(format!("akra-delegation-{:016x}", rand::random::<u64>()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn request(messages: &mut mpsc::Receiver<Message>, method: &str) -> Value {
        let message = tokio::time::timeout(Duration::from_secs(3), messages.recv())
            .await
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_eq!(value["method"], format!("{PREFIX}{method}"));
        value
    }

    fn reply(driver: &DriverConnection, request: &Value, result: Value) {
        let text = json!({"jsonrpc":"2.0", "id":request["id"], "result":result}).to_string();
        let (id, value) = response(&text).unwrap();
        driver.answer(id, value);
    }

    #[tokio::test]
    async fn delegation_driver_registration_and_replies_are_connection_bound() {
        let service = Arc::new(Delegations::default());
        let (outbox, mut messages) = mpsc::channel(4);
        let driver = service.connection(outbox);
        let (unrelated_outbox, _unrelated_messages) = mpsc::channel(4);
        let unrelated = service.connection(unrelated_outbox);
        assert_eq!(
            service.status().await.status.state,
            BackendState::Unavailable
        );
        driver.request(json!(1), &format!("{PREFIX}register"));
        let registered_twice: Value =
            serde_json::from_str(&unrelated.request(json!(2), &format!("{PREFIX}register")))
                .unwrap();
        assert!(registered_twice.get("error").is_some());
        unrelated.request(json!(3), &format!("{PREFIX}unregister"));
        let browser = async {
            let status = request(&mut messages, "status").await;
            reply(&unrelated, &status, json!({"state":"unavailable"}));
            reply(&driver, &status, json!({"state":"ready"}));
        };
        let (status, ()) = tokio::join!(service.status(), browser);
        assert_eq!(status.status.state, BackendState::Ready);
    }

    #[tokio::test]
    async fn delegation_queued_by_desktop_is_observed_until_result() {
        let directory = TestDirectory::new();
        let store = delegation_store(directory.0.clone(), false).unwrap();
        let service = Arc::new(Delegations::default());
        let (outbox, mut messages) = mpsc::channel(4);
        let driver = service.connection(outbox);
        driver.request(json!(1), &format!("{PREFIX}register"));
        let job = store.submit("queue", "text", "test").unwrap();
        for step in 0..3 {
            let browser = async {
                if step == 0 {
                    let status = request(&mut messages, "status").await;
                    reply(&driver, &status, json!({"state":"ready"}));
                }
                let call =
                    request(&mut messages, if step == 0 { "submit" } else { "observe" }).await;
                assert_eq!(call["params"]["jobId"], job.id);
                if step < 2 {
                    reply(
                        &driver,
                        &call,
                        json!({"delivery":"not_sent","phase":"unknown"}),
                    );
                } else {
                    reply(
                        &driver,
                        &call,
                        json!({"delivery":"observed","phase":"complete"}),
                    );
                    let read = request(&mut messages, "readResult").await;
                    reply(&driver, &read, json!({"text":"queued result"}));
                }
            };
            let (result, ()) = tokio::join!(service.reconcile(&store), browser);
            result.unwrap();
        }
        assert_eq!(
            store.result(&job.id).unwrap().as_deref(),
            Some("queued result")
        );
    }

    #[tokio::test]
    async fn delegation_lost_submit_ack_is_observed_without_resend_and_result_survives_restart() {
        let directory = TestDirectory::new();
        let store = delegation_store(directory.0.clone(), false).unwrap();
        let service = Arc::new(Delegations::default());
        let (outbox, mut messages) = mpsc::channel(4);
        let driver = service.connection(outbox);
        driver.request(json!(1), &format!("{PREFIX}register"));
        let job = store.submit("key", "text", "test").unwrap();
        let tick = service.reconcile(&store);
        let browser = async {
            let status = request(&mut messages, "status").await;
            reply(&driver, &status, json!({"state":"ready"}));
            let submit = request(&mut messages, "submit").await;
            assert_eq!(submit["params"]["jobId"], job.id);
            assert_eq!(submit["params"]["instruction"], "text");
            assert_eq!(submit["params"]["callback"]["jobId"], job.id);
            assert_eq!(
                submit["params"]["callback"]["token"]
                    .as_str()
                    .unwrap()
                    .len(),
                64
            );
            assert!(store.find(&job.id).unwrap().unwrap().outcome_unknown);
            drop(driver);
        };
        let (result, ()) = tokio::join!(tick, browser);
        result.unwrap();
        drop(service);

        let store = delegation_store(directory.0.clone(), true).unwrap();
        assert_eq!(store.submit("key", "text", "test").unwrap().id, job.id);
        assert!(store.submit("key", "different", "test").is_err());
        assert_eq!(store.list().unwrap().len(), 1);
        let service = Arc::new(Delegations::default());
        let (outbox, mut messages) = mpsc::channel(4);
        let driver = service.connection(outbox);
        driver.request(json!(1), &format!("{PREFIX}register"));
        let tick = service.reconcile(&store);
        let browser = async {
            let observe = request(&mut messages, "observe").await;
            assert_eq!(observe["params"]["jobId"], job.id);
            reply(
                &driver,
                &observe,
                json!({
                    "delivery":"observed", "phase":"complete", "conversationUrl":"https://chatgpt.com/c/abc"
                }),
            );
            let read = request(&mut messages, "readResult").await;
            reply(&driver, &read, json!({"text":"stored answer"}));
        };
        let (result, ()) = tokio::join!(tick, browser);
        result.unwrap();
        let store = delegation_store(directory.0.clone(), true).unwrap();
        assert_eq!(
            store.find(&job.id).unwrap().unwrap().state,
            State::Succeeded
        );
        assert_eq!(
            store.result(&job.id).unwrap().as_deref(),
            Some("stored answer")
        );
    }

    #[tokio::test]
    async fn delegation_cancellation_survives_restart_and_stops_once_when_acknowledged() {
        let directory = TestDirectory::new();
        let store = delegation_store(directory.0.clone(), false).unwrap();
        let job = store.submit("key", "text", "test").unwrap();
        let mut started = job.clone();
        started.state = State::Dispatching;
        assert!(store.update(&job, &mut started, None, false).unwrap());
        assert_eq!(
            store.cancel(&job.id).unwrap().unwrap().state,
            State::Cancelled
        );
        let mut stale = started.clone();
        stale.state = State::Succeeded;
        assert!(
            !store
                .update(&started, &mut stale, Some("stale"), false)
                .unwrap()
        );
        assert!(store.result(&job.id).unwrap().is_none());

        let store = delegation_store(directory.0.clone(), true).unwrap();
        let service = Arc::new(Delegations::default());
        let (outbox, mut messages) = mpsc::channel(4);
        let driver = service.connection(outbox);
        driver.request(json!(1), &format!("{PREFIX}register"));
        let tick = service.reconcile(&store);
        let browser = async {
            let stop = request(&mut messages, "stop").await;
            assert_eq!(stop["params"]["jobId"], job.id);
            reply(&driver, &stop, json!({}));
        };
        let (result, ()) = tokio::join!(tick, browser);
        result.unwrap();
        assert!(store.active().unwrap().is_empty());
        service.reconcile(&store).await.unwrap();
        assert!(messages.try_recv().is_err());
    }

    #[tokio::test]
    async fn delegation_skips_quota_backend_and_persists_circuit_fallback_and_evidence() {
        use crate::domain::delegation::policy::{Backend, Route};

        let directory = TestDirectory::new();
        let store = delegation_store(directory.0.clone(), false).unwrap();
        let mut route = Route::default();
        route.backends.push(Backend {
            name: "second".into(),
            kind: CHAT.into(),
            model: None,
            max_concurrent: 1,
            max_resumes: None,
        });
        let job = store
            .submit_with_route("quota", "text", "test", Ok(route))
            .unwrap();
        let service = Arc::new(Delegations::default());
        let (outbox, mut messages) = mpsc::channel(4);
        let driver = service.connection(outbox);
        driver.request(json!(1), &format!("{PREFIX}register"));
        let retry_at = chrono::Utc::now().timestamp_millis() + 111_000;
        let browser = async {
            let status = request(&mut messages, "status").await;
            reply(
                &driver,
                &status,
                json!({"state":"unavailable","reason":"quota","retryAt":retry_at}),
            );
            let status = request(&mut messages, "status").await;
            reply(&driver, &status, json!({"state":"ready"}));
            let submit = request(&mut messages, "submit").await;
            assert_eq!(submit["params"]["jobId"], format!("{}-2", job.id));
            reply(
                &driver,
                &submit,
                json!({"delivery":"accepted","phase":"working"}),
            );
        };
        let (result, ()) = tokio::join!(service.reconcile(&store), browser);
        result.unwrap();
        let store = delegation_store(directory.0.clone(), true).unwrap();
        assert_eq!(store.circuit_until(CHAT).unwrap(), Some(retry_at));
        let persisted = store.find(&job.id).unwrap().unwrap();
        assert_eq!(persisted.state, State::Running);
        assert_eq!(persisted.attempts[0].delivery, Delivery::NotSent);
        assert_eq!(persisted.attempts[1].delivery, Delivery::Accepted);
        assert_eq!(persisted.fallbacks[0].reason, EndReason::Quota);
        assert_eq!(persisted.fallbacks[0].to, "second");
    }

    #[tokio::test]
    async fn delegation_passed_deadline_expires_without_backend_io() {
        let directory = TestDirectory::new();
        let store = delegation_store(directory.0.clone(), false).unwrap();
        let job = store.submit("expired", "text", "test").unwrap();
        let mut next = job.clone();
        next.deadline = Some(job.created_at - 1);
        assert!(store.update(&job, &mut next, None, false).unwrap());
        let service = Arc::new(Delegations::default());
        service.reconcile(&store).await.unwrap();
        assert_eq!(store.find(&job.id).unwrap().unwrap().state, State::Expired);
        assert!(store.active().unwrap().is_empty());
    }

    #[tokio::test]
    async fn delegation_defensive_route_never_consults_or_dispatches_to_desktop() {
        use crate::domain::delegation::policy::Route;

        let directory = TestDirectory::new();
        let store = delegation_store(directory.0.clone(), false).unwrap();
        let route = Route {
            requires_defensive_isolation: true,
            ..Route::default()
        };
        let job = store
            .submit_with_route("review", "untrusted content", "pr_review", Ok(route))
            .unwrap();
        let service = Arc::new(Delegations::default());
        let (outbox, mut messages) = mpsc::channel(4);
        let driver = service.connection(outbox);
        driver.request(json!(1), &format!("{PREFIX}register"));
        service.reconcile(&store).await.unwrap();
        assert!(messages.try_recv().is_err());
        let failed = store.find(&job.id).unwrap().unwrap();
        assert_eq!(failed.state, State::Failed);
        assert_eq!(failed.attempts[0].delivery, Delivery::NotSent);
    }
}
