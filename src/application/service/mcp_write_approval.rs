/*
 * MCP hub 쓰기 도구 호출의 승인 중개이다 (#2830). Gateway 의 local 접속(Desktop)마다 구독자가 하나 있다. 요청은 모든
 * 구독자에게 알리고, 먼저 온 답 하나로 끝난다. 끝나면 모든 구독자에게 끝났다고 알린다. 제한 시간이 지난 뒤에 온 답은
 * 버린다. 그래서 한 번의 요청은 많아야 한 번 실행된다. 다시 시도하면 새 요청이고 새 승인이 필요하다.
 */
use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::application::port::inbound::mcp_write_approval_inbox_port::{
    McpWriteApprovalEvent, McpWriteApprovalInboxPort, McpWriteApprovalSubscription,
};
use crate::application::port::outbound::mcp_write_approval_port::{
    McpWriteApproval, McpWriteApprovalPort, McpWriteApprovalRequest,
};

/// Desktop 의 답을 기다리는 시간이다. `akra mcp` 의 Gateway 요청 제한(300초)보다 짧다.
pub const MCP_WRITE_APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

type Listener = Arc<dyn Fn(&McpWriteApprovalEvent) + Send + Sync>;

#[derive(Default)]
struct Inner {
    next_listener: u64,
    listeners: HashMap<u64, Listener>,
    pending: HashMap<String, mpsc::Sender<bool>>,
}

pub struct McpWriteApprovalBroker {
    inner: Arc<Mutex<Inner>>,
    timeout: Duration,
}

impl McpWriteApprovalBroker {
    pub fn new(timeout: Duration) -> Self {
        Self {
            inner: Arc::default(),
            timeout,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        lock(&self.inner)
    }

    fn notify(listeners: &[Listener], event: &McpWriteApprovalEvent) {
        for listener in listeners {
            listener(event);
        }
    }
}

impl McpWriteApprovalPort for McpWriteApprovalBroker {
    fn request_approval(&self, request: &McpWriteApprovalRequest) -> McpWriteApproval {
        let (sender, receiver) = mpsc::channel();
        let id = format!("mcp-write-{:032x}", rand::random::<u128>());
        let listeners: Vec<Listener> = {
            let mut inner = self.lock();
            if inner.listeners.is_empty() {
                return McpWriteApproval::NoApprover;
            }
            inner.pending.insert(id.clone(), sender);
            inner.listeners.values().cloned().collect()
        };
        Self::notify(
            &listeners,
            &McpWriteApprovalEvent::Requested {
                id: id.clone(),
                server: request.server.clone(),
                tool: request.tool.clone(),
                arguments: request.arguments.clone(),
            },
        );
        let outcome = match receiver.recv_timeout(self.timeout) {
            Ok(true) => McpWriteApproval::Approved,
            Ok(false) => McpWriteApproval::Declined,
            Err(_) => McpWriteApproval::TimedOut,
        };
        let listeners: Vec<Listener> = {
            let mut inner = self.lock();
            inner.pending.remove(&id);
            inner.listeners.values().cloned().collect()
        };
        Self::notify(&listeners, &McpWriteApprovalEvent::Resolved { id });
        outcome
    }
}

fn lock(inner: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl McpWriteApprovalInboxPort for McpWriteApprovalBroker {
    fn subscribe(
        &self,
        listener: Box<dyn Fn(&McpWriteApprovalEvent) + Send + Sync>,
    ) -> McpWriteApprovalSubscription {
        let mut inner = self.lock();
        inner.next_listener += 1;
        let id = inner.next_listener;
        inner.listeners.insert(id, Arc::from(listener));
        Box::new(Subscription {
            inner: self.inner.clone(),
            id,
        })
    }

    fn answer(&self, id: &str, approve: bool) -> bool {
        let sender = self.lock().pending.remove(id);
        sender.is_some_and(|sender| sender.send(approve).is_ok())
    }
}

struct Subscription {
    inner: Arc<Mutex<Inner>>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        lock(&self.inner).listeners.remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> McpWriteApprovalRequest {
        McpWriteApprovalRequest {
            server: "notion".into(),
            tool: "notion-update-page".into(),
            arguments: json!({"page": "p1"}),
        }
    }

    /// Records events and answers each request from another thread, like a Desktop client.
    fn answering(
        broker: &Arc<McpWriteApprovalBroker>,
        approve: bool,
    ) -> (
        McpWriteApprovalSubscription,
        Arc<Mutex<Vec<McpWriteApprovalEvent>>>,
    ) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let seen = events.clone();
        let answerer = Arc::downgrade(broker);
        let subscription = broker.subscribe(Box::new(move |event| {
            seen.lock().unwrap().push(event.clone());
            if let McpWriteApprovalEvent::Requested { id, .. } = event {
                let (answerer, id) = (answerer.clone(), id.clone());
                std::thread::spawn(move || {
                    if let Some(broker) = answerer.upgrade() {
                        broker.answer(&id, approve);
                    }
                });
            }
        }));
        (subscription, events)
    }

    #[test]
    fn no_subscriber_means_no_approver() {
        let broker = Arc::new(McpWriteApprovalBroker::new(Duration::from_secs(5)));
        assert_eq!(
            broker.request_approval(&request()),
            McpWriteApproval::NoApprover
        );
    }

    #[test]
    fn the_first_answer_decides_and_every_subscriber_sees_the_end() {
        let broker = Arc::new(McpWriteApprovalBroker::new(Duration::from_secs(5)));
        let (_approve, events) = answering(&broker, true);
        assert_eq!(
            broker.request_approval(&request()),
            McpWriteApproval::Approved
        );
        let events = events.lock().unwrap().clone();
        let [
            McpWriteApprovalEvent::Requested {
                id, server, tool, ..
            },
            McpWriteApprovalEvent::Resolved { id: resolved },
        ] = events.as_slice()
        else {
            panic!("unexpected events: {events:?}");
        };
        assert_eq!(
            (server.as_str(), tool.as_str()),
            ("notion", "notion-update-page")
        );
        assert_eq!(id, resolved);

        let (_decline, _) = answering(&broker, false);
        drop(_approve);
        assert_eq!(
            broker.request_approval(&request()),
            McpWriteApproval::Declined
        );
    }

    #[test]
    fn a_late_answer_after_the_timeout_is_dropped() {
        let broker = Arc::new(McpWriteApprovalBroker::new(Duration::from_millis(50)));
        let ids = Arc::new(Mutex::new(Vec::new()));
        let seen = ids.clone();
        let _silent = broker.subscribe(Box::new(move |event| {
            if let McpWriteApprovalEvent::Requested { id, .. } = event {
                seen.lock().unwrap().push(id.clone());
            }
        }));
        assert_eq!(
            broker.request_approval(&request()),
            McpWriteApproval::TimedOut
        );
        let id = ids.lock().unwrap()[0].clone();
        assert!(
            !broker.answer(&id, true),
            "a late approval must not reach any call"
        );
    }
}
