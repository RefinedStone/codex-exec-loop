//! The local Desktop reverse RPC. Request bodies and responses bypass session recording.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::application::port::outbound::delegation_store_port::{DelegationStorePort, Job};
use crate::domain::delegation::{BackendState, BackendStatus, Delivery, Phase, State};

pub(super) const PREFIX: &str = "akra/delegationDriver/";
const ID_PREFIX: &str = "akra-delegation-driver:";
const RPC_TIMEOUT: Duration = Duration::from_secs(4);
type PendingReplies = HashMap<u64, (u64, oneshot::Sender<Value>)>;
pub(super) type DelegationStore = Arc<dyn DelegationStorePort>;

#[derive(Default)]
pub(super) struct Delegations {
    sequence: AtomicU64,
    driver: Mutex<Option<(u64, mpsc::Sender<Message>)>>,
    pending: Mutex<PendingReplies>,
    workers: Mutex<BTreeMap<PathBuf, tokio::task::AbortHandle>>,
}

impl Drop for Delegations {
    fn drop(&mut self) {
        for worker in self.workers.get_mut().unwrap().values() {
            worker.abort();
        }
    }
}

impl Delegations {
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

    pub(super) fn track(self: &Arc<Self>, runtime: PathBuf, store: DelegationStore) {
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
                    if let Err(error) = service.reconcile(&store).await {
                        tracing::warn!(error = %error, "delegation ledger reconciliation failed");
                    }
                }
            })
            .abort_handle()
            });
    }

    pub(super) async fn status(&self) -> BackendStatus {
        self.call("status", json!({}))
            .await
            .unwrap_or(BackendStatus {
                state: BackendState::Unavailable,
                reason: Some("desktop_unavailable".to_owned()),
            })
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

    async fn reconcile(&self, store: &DelegationStore) -> Result<()> {
        let active = db(store.clone(), |store| store.active()).await?;
        if active.is_empty() {
            return Ok(());
        }
        for job in active {
            self.advance(store, job).await?;
        }
        Ok(())
    }

    async fn advance(&self, store: &DelegationStore, job: Job) -> Result<()> {
        let mut old = job.view;
        let id = old.id.clone();
        if job.stop_pending {
            if self
                .call::<Value>("stop", json!({"jobId": id}))
                .await
                .is_ok()
            {
                return db(store.clone(), move |store| store.stopped(&id)).await;
            }
            return Ok(());
        }
        let submitting = old.state == State::Queued;
        let observed: Result<Observation> = if submitting {
            let status = self.status().await;
            if status.state != BackendState::Ready {
                let mut next = old.clone();
                next.reason = status.reason;
                save(store, old, next, None).await?;
                return Ok(());
            }
            let mut claimed = old.clone();
            // A persisted claim is ambiguous after a crash. Future ticks only observe.
            claimed.state = State::Dispatching;
            claimed.delivery = Delivery::Uncertain;
            claimed.outcome_unknown = true;
            if !save(store, old, claimed.clone(), None).await? {
                return Ok(());
            }
            old = db(store.clone(), move |store| store.find(&id))
                .await?
                .context("delegation disappeared after claim")?;
            if old.state.terminal() {
                return Ok(());
            }
            self.call(
                "submit",
                json!({"jobId": old.id, "instruction": job.instruction}),
            )
            .await
        } else {
            self.call("observe", json!({"jobId": id})).await
        };
        let mut next = old.clone();
        let mut result = None;
        if let Ok(observation) = observed {
            if submitting
                && observation.delivery == Delivery::NotSent
                && observation.phase == Phase::Unknown
            {
                next.delivery = Delivery::NotSent;
                next.outcome_unknown = false;
            }
            next.apply_observation(observation.delivery, observation.phase);
            next.reason = observation.reason;
            if submitting
                && observation.delivery == Delivery::NotSent
                && matches!(
                    next.reason.as_deref(),
                    Some("chat_queue_full" | "chat_host_busy")
                )
            {
                next.state = State::Failed;
                next.delivery = Delivery::NotSent;
                next.outcome_unknown = false;
            }
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
                if let Ok(text) = self
                    .call::<TextResult>("readResult", json!({"jobId": old.id}))
                    .await
                    && !text.text.trim().is_empty()
                    && text.text.len() <= 1_048_576
                {
                    next.state = State::Succeeded;
                    next.reason = None;
                    result = Some(text.text);
                } else {
                    next.reason = Some("result_unavailable".to_owned());
                }
            }
        } else {
            next.phase = Phase::Unreachable;
            next.state = State::Interrupted;
            next.reason = Some("desktop_unreachable".to_owned());
        }
        save(store, old, next, result).await?;
        Ok(())
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
#[serde(rename_all = "camelCase")]
struct Observation {
    delivery: Delivery,
    phase: Phase,
    reason: Option<String>,
    conversation_url: Option<String>,
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
                        | "delegations"
                        | "delegation"
                        | "delegation_result"
                        | "delegation_backends"
                )
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(service.status().await.state, BackendState::Unavailable);
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
        assert_eq!(status.state, BackendState::Ready);
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
            assert_eq!(
                submit["params"],
                json!({"jobId": job.id, "instruction": "text"})
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
}
