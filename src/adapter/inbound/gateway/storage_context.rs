//! Conversation authority is correlated with successful responses from the trusted upstream.
//! Client thread strings, duplicate request IDs and lifecycle notifications cannot grant authority.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const MAX_PENDING: usize = 64;

#[derive(Clone)]
pub(super) struct VerifiedConversation {
    pub thread_id: String,
    pub epoch: u64,
    pub current_epoch: Arc<AtomicU64>,
}
#[derive(Default)]
struct State {
    pending: BTreeMap<String, Pending>,
    ambiguous: BTreeSet<String>,
    thread: Option<String>,
    tracking_lost: bool,
}
struct Pending {
    epoch: u64,
    expected_thread: Option<String>,
    verifies: bool,
}
#[derive(Default)]
pub(super) struct StorageConversationContext {
    epoch: Arc<AtomicU64>,
    state: Mutex<State>,
}
impl StorageConversationContext {
    pub(super) fn client_message(&self, text: &str) {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let Some(method) = value["method"].as_str() else {
            return;
        };
        let mut state = self.state.lock().unwrap();
        if state.tracking_lost {
            return;
        }
        if matches!(method, "thread/unsubscribe" | "thread/archive") {
            let requested = value["params"]["threadId"].as_str();
            if requested.is_some_and(|thread| Self::matches_thread(&state, thread)) {
                self.advance(&mut state);
            }
        }
        let Some(id) = value
            .get("id")
            .filter(|id| id.is_string() || id.is_number())
        else {
            return;
        };
        let id = id.to_string();
        // Track every forwarded request, including thread/read. Otherwise its response can be
        // confused with a resume when a client reuses an outstanding JSON-RPC identifier.
        if state.ambiguous.contains(&id) || state.pending.contains_key(&id) {
            self.advance(&mut state);
            state.pending.remove(&id);
            if state.ambiguous.len() >= MAX_PENDING {
                Self::lose_tracking(&mut state);
            } else {
                // Keep the tombstone for this connection: multiple old replies can arrive.
                state.ambiguous.insert(id);
            }
            return;
        }
        if state.pending.len() >= MAX_PENDING {
            self.advance(&mut state);
            Self::lose_tracking(&mut state);
            return;
        }
        let verifies = matches!(method, "thread/start" | "thread/resume");
        let expected_thread = if method == "thread/resume" {
            value["params"]["threadId"]
                .as_str()
                .filter(|thread| !thread.is_empty() && thread.len() <= 256)
                .map(str::to_owned)
        } else {
            None
        };
        let verifies = verifies && (method == "thread/start" || expected_thread.is_some());
        let epoch = if verifies {
            self.advance(&mut state)
        } else {
            self.epoch.load(Ordering::Acquire)
        };
        state.pending.insert(
            id,
            Pending {
                epoch,
                expected_thread,
                verifies,
            },
        );
    }

    pub(super) fn upstream_message(&self, text: &str) {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let mut state = self.state.lock().unwrap();
        if state.tracking_lost {
            return;
        }
        if let Some(method) = value["method"].as_str() {
            if matches!(
                method,
                "thread/closed" | "thread/archived" | "thread/deleted"
            ) && value["params"]["threadId"]
                .as_str()
                .is_some_and(|thread| Self::matches_thread(&state, thread))
            {
                self.advance(&mut state);
            }
            return;
        }
        let Some(id) = value.get("id") else { return };
        let id = id.to_string();
        if state.ambiguous.contains(&id) {
            return;
        }
        let Some(pending) = state.pending.remove(&id) else {
            return;
        };
        if !pending.verifies
            || pending.epoch != self.epoch.load(Ordering::Acquire)
            || value.get("error").is_some()
        {
            return;
        }
        let Some(thread) = value
            .pointer("/result/thread/id")
            .and_then(Value::as_str)
            .filter(|thread| !thread.is_empty() && thread.len() <= 256)
        else {
            return;
        };
        if pending
            .expected_thread
            .as_deref()
            .is_some_and(|expected| expected != thread)
        {
            return;
        }
        state.thread = Some(thread.to_owned());
    }
    pub(super) fn current(&self) -> Option<VerifiedConversation> {
        let state = self.state.lock().unwrap();
        if state.tracking_lost {
            return None;
        }
        Some(VerifiedConversation {
            thread_id: state.thread.clone()?,
            epoch: self.epoch.load(Ordering::Acquire),
            current_epoch: self.epoch.clone(),
        })
    }
    pub(super) fn invalidate(&self) {
        let mut state = self.state.lock().unwrap();
        self.advance(&mut state);
        Self::lose_tracking(&mut state);
    }
    fn matches_thread(state: &State, thread: &str) -> bool {
        state.thread.as_deref() == Some(thread)
            || state.pending.values().any(|pending| {
                pending.verifies
                    && (pending.expected_thread.is_none()
                        || pending.expected_thread.as_deref() == Some(thread))
            })
    }
    fn lose_tracking(state: &mut State) {
        // A fresh connection is required after bounded correlation state is exhausted.
        state.tracking_lost = true;
        state.pending.clear();
        state.ambiguous.clear();
        state.thread = None;
    }
    fn advance(&self, state: &mut State) -> u64 {
        state.thread = None;
        self.epoch.fetch_add(1, Ordering::AcqRel) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(context: &StorageConversationContext, id: u64, thread: &str) {
        context.client_message(
            &json!({"id":id,"method":"thread/resume","params":{"threadId":thread}}).to_string(),
        );
    }
    fn response(context: &StorageConversationContext, id: u64, thread: &str) {
        context.upstream_message(&json!({"id":id,"result":{"thread":{"id":thread}}}).to_string());
    }

    #[test]
    fn only_successful_correlated_upstream_responses_authorize() {
        let context = StorageConversationContext::default();
        request(&context, 1, "a");
        assert!(context.current().is_none());
        response(&context, 77, "a");
        assert!(context.current().is_none());
        response(&context, 1, "wrong");
        assert!(context.current().is_none());
        request(&context, 2, "a");
        response(&context, 2, "a");
        assert_eq!(context.current().unwrap().thread_id, "a");
    }

    #[test]
    fn switching_threads_and_disconnect_invalidate_inflight_authority() {
        let context = StorageConversationContext::default();
        request(&context, 1, "a");
        response(&context, 1, "a");
        let old = context.current().unwrap();
        request(&context, 2, "b");
        request(&context, 3, "a");
        response(&context, 2, "b");
        assert!(context.current().is_none());
        response(&context, 3, "a");
        assert_eq!(context.current().unwrap().thread_id, "a");
        assert_ne!(old.epoch, old.current_epoch.load(Ordering::Acquire));
        context.invalidate();
        assert!(context.current().is_none());
    }

    #[test]
    fn background_thread_reads_do_not_replace_the_verified_conversation() {
        let context = StorageConversationContext::default();
        request(&context, 1, "selected");
        response(&context, 1, "selected");
        let before = context.current().unwrap();
        context.client_message(
            r#"{"id":2,"method":"thread/read","params":{"threadId":"background"}}"#,
        );
        response(&context, 2, "background");
        assert_eq!(context.current().unwrap().thread_id, "selected");
        assert_eq!(context.current().unwrap().epoch, before.epoch);
    }

    #[test]
    fn unsubscribe_during_resume_cannot_authorize_a_late_response() {
        let context = StorageConversationContext::default();
        request(&context, 1, "a");
        context
            .client_message(r#"{"id":2,"method":"thread/unsubscribe","params":{"threadId":"a"}}"#);
        response(&context, 1, "a");
        assert!(context.current().is_none());
    }

    #[test]
    fn failed_resume_does_not_keep_previous_thread_authority() {
        let context = StorageConversationContext::default();
        request(&context, 1, "a");
        response(&context, 1, "a");
        request(&context, 2, "b");
        context.upstream_message(r#"{"id":2,"error":{"code":-1}}"#);
        assert!(context.current().is_none());
    }
}

#[cfg(test)]
mod correlation_regressions {
    use super::*;
    use serde_json::json;
    fn request(c: &StorageConversationContext, id: u64, method: &str, thread: &str) {
        c.client_message(
            &json!({"id":id,"method":method,"params":{"threadId":thread}}).to_string(),
        );
    }
    fn response(c: &StorageConversationContext, id: u64, thread: &str) {
        c.upstream_message(&json!({"id":id,"result":{"thread":{"id":thread}}}).to_string());
    }
    #[test]
    fn a_read_response_with_a_duplicate_resume_id_never_verifies_a_thread() {
        for (first, second) in [
            ("thread/resume", "thread/read"),
            ("thread/read", "thread/resume"),
            ("thread/start", "thread/start"),
        ] {
            let c = StorageConversationContext::default();
            request(&c, 7, first, "a");
            request(&c, 7, second, "a");
            response(&c, 7, "a");
            response(&c, 7, "a");
            assert!(c.current().is_none());
            request(&c, 7, "thread/resume", "a");
            response(&c, 7, "a");
            assert!(c.current().is_none(), "ambiguous ID remains poisoned");
            request(&c, 8, "thread/resume", "a");
            response(&c, 8, "a");
            assert_eq!(c.current().unwrap().thread_id, "a");
        }
    }
    #[test]
    fn trusted_lifecycle_notifications_invalidate_current_and_pending_threads() {
        for method in ["thread/closed", "thread/archived", "thread/deleted"] {
            let c = StorageConversationContext::default();
            request(&c, 1, "thread/resume", "a");
            response(&c, 1, "a");
            let old = c.current().unwrap();
            c.upstream_message(&json!({"method":method,"params":{"threadId":"a"}}).to_string());
            assert!(c.current().is_none());
            assert_ne!(old.epoch, old.current_epoch.load(Ordering::Acquire));
            request(&c, 2, "thread/resume", "a");
            c.upstream_message(&json!({"method":method,"params":{"threadId":"a"}}).to_string());
            response(&c, 2, "a");
            assert!(c.current().is_none());
        }
    }
    #[test]
    fn overflowing_outstanding_correlation_fails_closed_until_reconnect() {
        let c = StorageConversationContext::default();
        for id in 0..=MAX_PENDING as u64 {
            request(&c, id, "thread/read", "a");
        }
        request(&c, 100, "thread/resume", "a");
        response(&c, 100, "a");
        assert!(c.current().is_none());
        assert!(c.state.lock().unwrap().pending.is_empty());
    }
}
