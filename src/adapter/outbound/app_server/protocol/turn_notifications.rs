use std::io::Write;

use anyhow::Result;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::adapter::outbound::app_server::{AppServerEventSender, send_required_app_server_event};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::canonical_active_planning_file_path;
use crate::domain::conversation::{
    ConversationApprovalReview, ConversationApprovalReviewStatus, ConversationMessage,
    ConversationMessageKind, ConversationToolActivity, ConversationToolActivityKind,
};
use crate::domain::conversation_item_lifecycle::{
    ConversationItemKind, ConversationItemLifecycleConsistency, ConversationItemLifecyclePhase,
    ConversationItemLifecycleProjection, ConversationItemOutcome,
    MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS,
};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeEnvelopeObservation, ConversationRuntimeObservationGap,
};
use crate::domain::turn_terminal::{
    ConversationTurnError, ConversationTurnErrorInfo, ConversationTurnItemsView,
    ConversationTurnObservations, ConversationTurnTerminalOutcome, ConversationTurnTerminalReceipt,
    ConversationTurnTerminalUncertainty,
};

const MAX_TERMINAL_PROTOCOL_TEXT_BYTES: usize = 4 * 1024;
pub(super) const MAX_RETAINED_ITEM_EFFECT_IDENTITIES: usize =
    MAX_RETAINED_CONVERSATION_ITEM_LIFECYCLE_RECORDS * 2;

/*
 * turn_notifications.rs owns the app-server notification stream translation. connection.rs only reads JSON-RPC
 * messages, while mod.rs drives the active turn loop; this module decides which notifications belong to that
 * active thread/turn and how raw app-server item payloads become ConversationStreamEvent or snapshot messages.
 */
#[derive(Debug, Clone, PartialEq)]
pub(in crate::adapter::outbound::app_server) struct AppServerNotification {
    // method is the JSON-RPC notification method, for example `item/completed` or `turn/completed`.
    method: String,
    // params stays as raw JSON because item schemas evolve faster than the domain events we expose.
    params: Value,
}

impl AppServerNotification {
    pub(in crate::adapter::outbound::app_server) fn from_value(value: Value) -> Option<Self> {
        // Non-notification JSON-RPC messages have no method and are handled by request/response paths instead.
        let method = value.get("method").and_then(Value::as_str)?.to_string();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        Some(Self { method, params })
    }

    pub(in crate::adapter::outbound::app_server) fn method(&self) -> &str {
        &self.method
    }

    pub(in crate::adapter::outbound::app_server) fn params(&self) -> &Value {
        &self.params
    }

    pub(in crate::adapter::outbound::app_server) fn encoded_size_bytes(&self) -> usize {
        // Pending notifications outlive the transport line that carried them. Re-encode
        // the retained method/params value so that queue admission is bounded by bytes,
        // not only by an entry count that small deltas or one huge item can game.
        self.method
            .len()
            .saturating_add(serde_json::to_vec(&self.params).map_or(usize::MAX, |body| body.len()))
    }

    pub(in crate::adapter::outbound::app_server) fn should_defer_to_turn_stream(&self) -> bool {
        /*
         * These notification families are owned by the active turn reducer even when
         * they arrive while a JSON-RPC request is waiting for its response. Deferring
         * them keeps `turn/start` races from turning valid early deltas into generic
         * connection warnings.
         */
        self.method == "error"
            || self.method == "thread/status/changed"
            || self.method == "thread/settings/updated"
            || self.method == "model/rerouted"
            || self.method.starts_with("turn/")
            || self.method.starts_with("item/")
    }

    pub(in crate::adapter::outbound::app_server) fn warning_text(&self, context: &str) -> String {
        /*
         * Dropped notifications are not silent because app-server schemas are still a
         * moving boundary. The warning copy keeps method identity plus call-site
         * context so diagnostics can distinguish stale-turn noise from actual schema
         * drift.
         */
        match self.method.as_str() {
            "configWarning" => format!(
                "app-server sent a configuration warning {context}; provider details were redacted"
            ),
            "error" => format!(
                "app-server reported an error {context}: {}",
                self.params
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .map(|message| {
                        bounded_terminal_protocol_text(message, MAX_TERMINAL_PROTOCOL_TEXT_BYTES)
                    })
                    .unwrap_or_else(|| "unknown error".to_string())
            ),
            _ => format!("app-server sent notification `{}` {context}", self.method),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::outbound::app_server) enum TurnNotificationHandling {
    // Consumed means the notification updated stream state but the turn should keep reading.
    Consumed,
    // Retrying errors are facts for the live stream but do not end the active turn.
    RetryObserved {
        error: ConversationTurnError,
    },
    // Non-retrying errors start a bounded grace period for an authoritative turn/completed receipt.
    NonRetryErrorCandidate {
        error: ConversationTurnError,
    },
    // Terminal is the first terminal receipt accepted for this active turn.
    Terminal {
        receipt: ConversationTurnTerminalReceipt,
    },
    // DuplicateTerminal preserves the first receipt and prevents a later notification from overwriting it.
    DuplicateTerminal {
        receipt: ConversationTurnTerminalReceipt,
    },
    // Dropped keeps the loop alive while preserving a warning for out-of-scope or unknown notifications.
    Dropped(String),
}

#[derive(Debug, Clone, Default)]
pub(in crate::adapter::outbound::app_server) struct ActiveTurnNotificationState {
    changed_planning_file_paths: Vec<String>,
    terminal_receipt: Option<ConversationTurnTerminalReceipt>,
    runtime_envelope_observation_gap: ConversationRuntimeObservationGap,
    item_lifecycle: ConversationItemLifecycleProjection,
    item_effect_identities: ItemEffectIdentityLedger,
}

#[derive(Debug, Clone, Default)]
struct ItemEffectIdentityLedger {
    records: Vec<ItemEffectIdentityRecord>,
    exhausted: bool,
}

#[derive(Debug, Clone)]
struct ItemEffectIdentityRecord {
    identity_fingerprint: [u8; 32],
    kind_fingerprint: [u8; 32],
    completion_seen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemEffectIdentityObservation {
    Accepted,
    FirstCompletion,
    DuplicateCompletion,
    KindMismatch,
    Exhausted,
}

impl ItemEffectIdentityLedger {
    fn observe(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        item_id: &str,
        kind: &ConversationItemKind,
        phase: ConversationItemLifecyclePhase,
    ) -> ItemEffectIdentityObservation {
        let identity_fingerprint = item_identity_fingerprint(thread_id, turn_id, item_id);
        let kind_fingerprint = item_kind_fingerprint(kind);
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|record| record.identity_fingerprint == identity_fingerprint)
        {
            if record.kind_fingerprint != kind_fingerprint {
                return ItemEffectIdentityObservation::KindMismatch;
            }
            if phase == ConversationItemLifecyclePhase::Completed {
                if record.completion_seen {
                    return ItemEffectIdentityObservation::DuplicateCompletion;
                }
                record.completion_seen = true;
                return ItemEffectIdentityObservation::FirstCompletion;
            }
            return ItemEffectIdentityObservation::Accepted;
        }
        if self.exhausted || self.records.len() == MAX_RETAINED_ITEM_EFFECT_IDENTITIES {
            self.exhausted = true;
            return ItemEffectIdentityObservation::Exhausted;
        }
        self.records.push(ItemEffectIdentityRecord {
            identity_fingerprint,
            kind_fingerprint,
            completion_seen: phase == ConversationItemLifecyclePhase::Completed,
        });
        match phase {
            ConversationItemLifecyclePhase::Started => ItemEffectIdentityObservation::Accepted,
            ConversationItemLifecyclePhase::Completed => {
                ItemEffectIdentityObservation::FirstCompletion
            }
            ConversationItemLifecyclePhase::SnapshotObserved => {
                ItemEffectIdentityObservation::Accepted
            }
        }
    }
}

fn item_identity_fingerprint(thread_id: &str, turn_id: &str, item_id: &str) -> [u8; 32] {
    let mut digest = Sha256::new();
    for value in [thread_id, turn_id, item_id] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    digest.finalize().into()
}

fn item_kind_fingerprint(kind: &ConversationItemKind) -> [u8; 32] {
    let label = kind.stable_wire_label().unwrap_or_else(|| match kind {
        ConversationItemKind::Unknown(label) => label,
        _ => unreachable!("stable item kinds always expose a wire label"),
    });
    Sha256::digest(label.as_bytes()).into()
}

impl ActiveTurnNotificationState {
    pub(in crate::adapter::outbound::app_server) fn new() -> Self {
        Self::default()
    }

    pub(in crate::adapter::outbound::app_server) fn changed_planning_file_paths(
        &self,
    ) -> &[String] {
        &self.changed_planning_file_paths
    }

    pub(in crate::adapter::outbound::app_server) fn runtime_envelope_gap_observation(
        &self,
        thread_id: &str,
    ) -> Option<ConversationRuntimeEnvelopeObservation> {
        (!self.runtime_envelope_observation_gap.is_empty()).then(|| {
            ConversationRuntimeEnvelopeObservation::ProjectionGap {
                thread_id: thread_id.to_string(),
                gap: self.runtime_envelope_observation_gap,
            }
        })
    }

    pub(in crate::adapter::outbound::app_server) fn clear_runtime_envelope_gap(&mut self) {
        self.runtime_envelope_observation_gap = ConversationRuntimeObservationGap::default();
    }

    fn record_runtime_envelope_gap(&mut self, gap: ConversationRuntimeObservationGap) {
        self.runtime_envelope_observation_gap =
            self.runtime_envelope_observation_gap.merged_with(gap);
    }

    fn resolve_runtime_envelope_gap(&mut self, observation: ConversationRuntimeObservationGap) {
        if observation.settings_may_be_stale {
            self.runtime_envelope_observation_gap.settings_may_be_stale = false;
            self.runtime_envelope_observation_gap.model_may_be_stale = false;
        } else if observation.model_may_be_stale {
            self.runtime_envelope_observation_gap.model_may_be_stale = false;
        }
        if observation.status_may_be_stale {
            self.runtime_envelope_observation_gap.status_may_be_stale = false;
        }
    }

    #[cfg(test)]
    fn terminal_receipt(&self) -> Option<&ConversationTurnTerminalReceipt> {
        self.terminal_receipt.as_ref()
    }

    fn record_terminal(
        &mut self,
        receipt: ConversationTurnTerminalReceipt,
    ) -> TurnNotificationHandling {
        if let Some(first_receipt) = self.terminal_receipt.as_ref() {
            return TurnNotificationHandling::DuplicateTerminal {
                receipt: first_receipt.clone(),
            };
        }

        self.terminal_receipt = Some(receipt.clone());
        TurnNotificationHandling::Terminal { receipt }
    }
}

pub(in crate::adapter::outbound::app_server) fn handle_turn_notification(
    notification: &AppServerNotification,
    thread_id: &str,
    turn_id: &str,
    state: &mut ActiveTurnNotificationState,
    event_sender: &dyn AppServerEventSender,
) -> Result<TurnNotificationHandling> {
    /*
     * This function is the live stream reducer. Every branch first verifies thread/turn identity before emitting
     * domain events so a shared connection cannot leak notifications from a stale or concurrent turn into the
     * TUI transcript. ActiveTurnNotificationState owns changed-file observations plus the first terminal receipt,
     * making terminal classification idempotent even when app-server repeats or reorders notifications.
     */
    let params = notification.params();
    if state.terminal_receipt.is_some() && notification.method() != "turn/completed" {
        return Ok(TurnNotificationHandling::Dropped(
            notification.warning_text("after the active turn already had a terminal receipt"),
        ));
    }

    match notification.method() {
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed" => {
            // Approval review events are attached to tool items, so malformed payloads are warnings rather than fatal errors.
            if !matches_active_turn(params, thread_id, turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            if let Some(event) = parse_approval_review_event(notification, thread_id, turn_id) {
                let _ = event_sender.send(event);
                return Ok(TurnNotificationHandling::Consumed);
            }

            Ok(TurnNotificationHandling::Dropped(
                notification.warning_text(
                    "with an approval review payload the adapter could not translate",
                ),
            ))
        }
        "thread/settings/updated" => {
            if params.get("threadId").and_then(Value::as_str) != Some(thread_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let settings = match super::settings_observation(params) {
                Ok(settings) => settings,
                Err(error) => {
                    state
                        .record_runtime_envelope_gap(ConversationRuntimeObservationGap::settings());
                    return Ok(TurnNotificationHandling::Dropped(
                        notification
                            .warning_text(&format!("with an invalid settings envelope ({error})")),
                    ));
                }
            };
            Ok(send_runtime_envelope_observation(
                event_sender,
                ConversationRuntimeEnvelopeObservation::SettingsUpdated {
                    thread_id: thread_id.to_string(),
                    settings: Box::new(settings),
                },
                notification,
                state,
            ))
        }
        "model/rerouted" => {
            if !matches_active_turn(params, thread_id, turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }
            let Some(reroute) = super::model_reroute(params) else {
                state.record_runtime_envelope_gap(ConversationRuntimeObservationGap::model());
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("with a malformed model reroute envelope"),
                ));
            };

            Ok(send_runtime_envelope_observation(
                event_sender,
                ConversationRuntimeEnvelopeObservation::ModelRerouted {
                    thread_id: thread_id.to_string(),
                    turn_id: turn_id.to_string(),
                    reroute,
                },
                notification,
                state,
            ))
        }
        "thread/status/changed" => {
            if params.get("threadId").and_then(Value::as_str) != Some(thread_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            Ok(send_runtime_envelope_observation(
                event_sender,
                ConversationRuntimeEnvelopeObservation::ThreadStatusChanged {
                    thread_id: thread_id.to_string(),
                    status: super::status_observation(params.get("status")),
                },
                notification,
                state,
            ))
        }
        "turn/started" => {
            // The nested turn id is authoritative; missing identity must never be filled from caller-owned state.
            if !matches_started_turn(params, thread_id, turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            // turn/start response already emitted the required TurnStarted event
            // together with the exact request envelope. The notification confirms
            // correlation but must not replace that request with inferred defaults.
            Ok(TurnNotificationHandling::Consumed)
        }
        "item/agentMessage/delta" => {
            /*
             * Delta items update the live transcript only. The completed agent message
             * is emitted by `item/completed`, so replay and final transcript state do
             * not depend on reconstructing text from a possibly missing delta stream.
             */
            if !matches_active_turn(params, thread_id, turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let item_id = params
                .get("itemId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let delta = params
                .get("delta")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let _ = event_sender.send(ConversationStreamEvent::AgentMessageDelta {
                item_id,
                phase: params
                    .get("phase")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                delta,
            });
            Ok(TurnNotificationHandling::Consumed)
        }
        "item/started" => {
            if !matches_active_turn(params, thread_id, turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let observation = match super::parse_live_item_lifecycle(
                params,
                ConversationItemLifecyclePhase::Started,
            ) {
                Ok(observation) => observation,
                Err(error) => {
                    return Ok(TurnNotificationHandling::Dropped(
                        notification.warning_text(&format!("with {}", error.notice_label())),
                    ));
                }
            };
            if state
                .item_lifecycle
                .apply_correlated(Some(thread_id), Some(turn_id), observation.clone())
                .is_err()
            {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("with an invalid item lifecycle observation"),
                ));
            }
            let identity_observation = state.item_effect_identities.observe(
                thread_id,
                turn_id,
                &observation.item_id,
                &observation.kind,
                observation.phase,
            );
            send_required_app_server_event(
                event_sender,
                ConversationStreamEvent::ItemLifecycleObserved {
                    observation: Box::new(observation),
                },
                "item/started",
            )?;
            if identity_observation == ItemEffectIdentityObservation::Exhausted {
                anyhow::bail!(
                    "bounded item identity ledger exhausted after retaining lifecycle observation"
                );
            }
            Ok(TurnNotificationHandling::Consumed)
        }
        "item/completed" => {
            /*
             * Completed items are the live stream's finalization point for transcript
             * records and tool summaries. Planning changed-file tracking is recorded
             * before UI fan-out so the later `turn/completed` event can carry the full
             * turn-level planning refresh summary.
             */
            let observed_thread_id = params.get("threadId").and_then(Value::as_str);
            let observed_turn_id = params.get("turnId").and_then(Value::as_str);
            if observed_thread_id.is_none_or(str::is_empty)
                || observed_turn_id.is_none_or(str::is_empty)
            {
                anyhow::bail!(
                    "active item/completed had missing or invalid lifecycle correlation identity"
                );
            }
            if observed_thread_id != Some(thread_id) || observed_turn_id != Some(turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let observation = match super::parse_live_item_lifecycle(
                params,
                ConversationItemLifecyclePhase::Completed,
            ) {
                Ok(observation) => observation,
                Err(error) => {
                    anyhow::bail!(
                        "active item/completed violated the lifecycle contract: {}",
                        error.notice_label()
                    );
                }
            };
            let consistency = match state.item_lifecycle.apply_correlated(
                Some(thread_id),
                Some(turn_id),
                observation.clone(),
            ) {
                Ok(consistency) => consistency,
                Err(_) => {
                    anyhow::bail!(
                        "active item/completed violated lifecycle correlation after parsing"
                    );
                }
            };
            let identity_observation = state.item_effect_identities.observe(
                thread_id,
                turn_id,
                &observation.item_id,
                &observation.kind,
                observation.phase,
            );
            let permits_outcome_side_effect =
                permits_completed_item_side_effect(&observation.kind, &observation.outcome);
            send_required_app_server_event(
                event_sender,
                ConversationStreamEvent::ItemLifecycleObserved {
                    observation: Box::new(observation),
                },
                "item/completed/lifecycle",
            )?;

            let consistency_permits_legacy_side_effect =
                matches!(consistency, ConversationItemLifecycleConsistency::Accepted)
                    || matches!(
                        consistency,
                        ConversationItemLifecycleConsistency::CompletionWithoutStart
                            | ConversationItemLifecycleConsistency::TimestampRegression
                    );
            match identity_observation {
                ItemEffectIdentityObservation::DuplicateCompletion => {
                    return Ok(TurnNotificationHandling::Consumed);
                }
                ItemEffectIdentityObservation::KindMismatch => {
                    anyhow::bail!(
                        "active item identity changed kind after retaining lifecycle observation"
                    );
                }
                ItemEffectIdentityObservation::Exhausted => {
                    anyhow::bail!(
                        "bounded item identity ledger exhausted after retaining lifecycle observation"
                    );
                }
                ItemEffectIdentityObservation::FirstCompletion => {}
                ItemEffectIdentityObservation::Accepted => {
                    return Ok(TurnNotificationHandling::Consumed);
                }
            }
            if !consistency_permits_legacy_side_effect || !permits_outcome_side_effect {
                return Ok(TurnNotificationHandling::Consumed);
            }

            record_changed_planning_file_paths(
                params.get("item"),
                &mut state.changed_planning_file_paths,
            );
            handle_completed_item(params.get("item"), event_sender)?;
            Ok(TurnNotificationHandling::Consumed)
        }
        "error" => {
            if !matches_active_turn(params, thread_id, turn_id) {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }

            let error = parse_error_notification(params);
            if params.get("willRetry").and_then(Value::as_bool) == Some(true) {
                send_required_app_server_event(
                    event_sender,
                    ConversationStreamEvent::TurnRetrying {
                        thread_id: thread_id.to_string(),
                        turn_id: turn_id.to_string(),
                        error: error.clone(),
                    },
                    "turn/retrying",
                )?;
                Ok(TurnNotificationHandling::RetryObserved { error })
            } else {
                Ok(TurnNotificationHandling::NonRetryErrorCandidate { error })
            }
        }
        "turn/completed" => {
            /*
             * `turn/completed` is the only notification that may stop the read loop.
             * It also transfers the side-band planning-file summary accumulated from
             * earlier fileChange items, letting post-turn planning refresh run after
             * the transcript has seen the complete app-server turn.
             */
            let observed_thread_id = params
                .get("threadId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty());
            let observed_turn_id = params
                .get("turn")
                .and_then(Value::as_object)
                .and_then(|turn| turn.get("id"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty());
            if observed_thread_id.is_some_and(|observed| observed != thread_id)
                || observed_turn_id.is_some_and(|observed| observed != turn_id)
            {
                return Ok(TurnNotificationHandling::Dropped(
                    notification.warning_text("that did not match the active turn stream"),
                ));
            }
            let identity_uncertainty = if observed_thread_id.is_none() {
                Some(ConversationTurnTerminalUncertainty::missing_required_identity("threadId"))
            } else if observed_turn_id.is_none() {
                Some(ConversationTurnTerminalUncertainty::missing_required_identity("turn.id"))
            } else {
                None
            };

            let receipt = parse_terminal_receipt(
                params,
                thread_id,
                turn_id,
                state.changed_planning_file_paths.clone(),
                identity_uncertainty,
            );
            Ok(state.record_terminal(receipt))
        }
        _ => Ok(TurnNotificationHandling::Dropped(
            notification.warning_text("that has no adapter translation for the active turn stream"),
        )),
    }
}

fn send_runtime_envelope_observation(
    event_sender: &dyn AppServerEventSender,
    observation: ConversationRuntimeEnvelopeObservation,
    notification: &AppServerNotification,
    state: &mut ActiveTurnNotificationState,
) -> TurnNotificationHandling {
    let gap = match &observation {
        ConversationRuntimeEnvelopeObservation::SettingsUpdated { .. } => {
            ConversationRuntimeObservationGap::settings()
        }
        ConversationRuntimeEnvelopeObservation::ModelRerouted { .. } => {
            ConversationRuntimeObservationGap::model()
        }
        ConversationRuntimeEnvelopeObservation::ThreadStatusChanged { .. } => {
            ConversationRuntimeObservationGap::status()
        }
        ConversationRuntimeEnvelopeObservation::ProjectionGap { gap, .. } => *gap,
    };
    match event_sender.send(ConversationStreamEvent::RuntimeEnvelopeObserved {
        observation: Box::new(observation),
    }) {
        Ok(()) => {
            state.resolve_runtime_envelope_gap(gap);
            TurnNotificationHandling::Consumed
        }
        Err(failure) => {
            state.record_runtime_envelope_gap(gap);
            TurnNotificationHandling::Dropped(notification.warning_text(&format!(
                "whose typed runtime-envelope projection was not admitted ({failure:?})"
            )))
        }
    }
}

fn parse_error_notification(params: &Value) -> ConversationTurnError {
    parse_turn_error(params.get("error")).unwrap_or_else(|| {
        ConversationTurnError::new(
            "app-server error notification omitted error.message",
            None::<&str>,
            None,
        )
    })
}

fn parse_turn_error(value: Option<&Value>) -> Option<ConversationTurnError> {
    let error = value?.as_object()?;
    let message = error.get("message")?.as_str()?;
    let additional_details = error
        .get("additionalDetails")
        .and_then(Value::as_str)
        .map(|details| bounded_terminal_protocol_text(details, MAX_TERMINAL_PROTOCOL_TEXT_BYTES));
    let info = error
        .get("codexErrorInfo")
        .filter(|value| !value.is_null())
        .map(parse_error_info);

    Some(ConversationTurnError::new(
        bounded_terminal_protocol_text(message, MAX_TERMINAL_PROTOCOL_TEXT_BYTES),
        additional_details,
        info,
    ))
}

fn parse_error_info(value: &Value) -> ConversationTurnErrorInfo {
    if let Some(label) = value.as_str() {
        return match label {
            "contextWindowExceeded" => ConversationTurnErrorInfo::ContextWindowExceeded,
            "sessionBudgetExceeded" => ConversationTurnErrorInfo::SessionBudgetExceeded,
            "usageLimitExceeded" => ConversationTurnErrorInfo::UsageLimitExceeded,
            "serverOverloaded" => ConversationTurnErrorInfo::ServerOverloaded,
            "cyberPolicy" => ConversationTurnErrorInfo::CyberPolicy,
            "internalServerError" => ConversationTurnErrorInfo::InternalServerError,
            "unauthorized" => ConversationTurnErrorInfo::Unauthorized,
            "badRequest" => ConversationTurnErrorInfo::BadRequest,
            "threadRollbackFailed" => ConversationTurnErrorInfo::ThreadRollbackFailed,
            "sandboxError" => ConversationTurnErrorInfo::SandboxError,
            "other" => ConversationTurnErrorInfo::Other,
            unknown => ConversationTurnErrorInfo::unknown(bounded_terminal_protocol_text(
                unknown,
                MAX_TERMINAL_PROTOCOL_TEXT_BYTES,
            )),
        };
    }

    let Some(info) = value.as_object() else {
        return ConversationTurnErrorInfo::unknown(bounded_terminal_protocol_json(value));
    };
    if info.len() != 1 {
        return ConversationTurnErrorInfo::unknown(bounded_terminal_protocol_json(value));
    }
    if let Some(details) = info.get("httpConnectionFailed") {
        return parse_http_status_code(details).map_or_else(
            || ConversationTurnErrorInfo::unknown(bounded_terminal_protocol_json(value)),
            |http_status_code| ConversationTurnErrorInfo::HttpConnectionFailed { http_status_code },
        );
    }
    if let Some(details) = info.get("responseStreamConnectionFailed") {
        return parse_http_status_code(details).map_or_else(
            || ConversationTurnErrorInfo::unknown(bounded_terminal_protocol_json(value)),
            |http_status_code| ConversationTurnErrorInfo::ResponseStreamConnectionFailed {
                http_status_code,
            },
        );
    }
    if let Some(details) = info.get("responseStreamDisconnected") {
        return parse_http_status_code(details).map_or_else(
            || ConversationTurnErrorInfo::unknown(bounded_terminal_protocol_json(value)),
            |http_status_code| ConversationTurnErrorInfo::ResponseStreamDisconnected {
                http_status_code,
            },
        );
    }
    if let Some(details) = info.get("responseTooManyFailedAttempts") {
        return parse_http_status_code(details).map_or_else(
            || ConversationTurnErrorInfo::unknown(bounded_terminal_protocol_json(value)),
            |http_status_code| ConversationTurnErrorInfo::ResponseTooManyFailedAttempts {
                http_status_code,
            },
        );
    }

    if let Some(details) = info.get("activeTurnNotSteerable")
        && let Some(turn_kind) = details.get("turnKind").and_then(Value::as_str)
    {
        return ConversationTurnErrorInfo::active_turn_not_steerable(
            bounded_terminal_protocol_text(turn_kind, MAX_TERMINAL_PROTOCOL_TEXT_BYTES),
        );
    }

    ConversationTurnErrorInfo::unknown(bounded_terminal_protocol_json(value))
}

fn parse_http_status_code(value: &Value) -> Option<Option<u16>> {
    let details = value.as_object()?;
    match details.get("httpStatusCode") {
        None | Some(Value::Null) => Some(None),
        Some(status) => u16::try_from(status.as_u64()?).ok().map(Some),
    }
}

fn parse_terminal_receipt(
    params: &Value,
    thread_id: &str,
    turn_id: &str,
    changed_planning_file_paths: Vec<String>,
    identity_uncertainty: Option<ConversationTurnTerminalUncertainty>,
) -> ConversationTurnTerminalReceipt {
    let turn = params.get("turn");
    let parsed_error = parse_turn_error(turn.and_then(|value| value.get("error")));
    let has_error = turn
        .and_then(|value| value.get("error"))
        .is_some_and(|value| !value.is_null());
    let uncertainty = identity_uncertainty.or_else(|| terminal_payload_uncertainty(turn));
    let outcome = uncertainty.map_or_else(
        || terminal_outcome(turn, has_error, parsed_error.clone()),
        |reason| ConversationTurnTerminalOutcome::Unknown {
            reason,
            observed_error: parsed_error.clone(),
        },
    );

    ConversationTurnTerminalReceipt::new(thread_id, turn_id, outcome)
        .with_turn_metadata(
            parse_items_view(turn.and_then(|value| value.get("itemsView"))),
            turn.and_then(|value| value.get("startedAt"))
                .and_then(Value::as_i64),
            turn.and_then(|value| value.get("completedAt"))
                .and_then(Value::as_i64),
            turn.and_then(|value| value.get("durationMs"))
                .and_then(Value::as_i64),
        )
        .with_observations(ConversationTurnObservations::new(
            changed_planning_file_paths,
        ))
}

fn terminal_payload_uncertainty(
    turn: Option<&Value>,
) -> Option<ConversationTurnTerminalUncertainty> {
    let Some(turn) = turn.and_then(Value::as_object) else {
        return Some(ConversationTurnTerminalUncertainty::protocol_inconsistency(
            "terminal notification omitted required turn object",
        ));
    };
    if !turn.get("items").is_some_and(Value::is_array) {
        return Some(ConversationTurnTerminalUncertainty::protocol_inconsistency(
            "terminal notification omitted required turn.items array",
        ));
    }
    if turn.get("status").and_then(Value::as_str).is_none() {
        return Some(ConversationTurnTerminalUncertainty::protocol_inconsistency(
            "terminal notification omitted required turn.status string",
        ));
    }

    for field in ["startedAt", "completedAt", "durationMs"] {
        if turn
            .get(field)
            .is_some_and(|value| !value.is_null() && value.as_i64().is_none())
        {
            return Some(ConversationTurnTerminalUncertainty::protocol_inconsistency(
                format!("terminal turn.{field} was not an integer or null"),
            ));
        }
    }

    None
}

fn terminal_outcome(
    turn: Option<&Value>,
    has_error: bool,
    parsed_error: Option<ConversationTurnError>,
) -> ConversationTurnTerminalOutcome {
    let status = turn
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    match status {
        "completed" if !has_error => ConversationTurnTerminalOutcome::Completed,
        "interrupted" if !has_error => ConversationTurnTerminalOutcome::Interrupted,
        "failed" => parsed_error.map_or_else(
            || ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::MissingFailedError,
                observed_error: None,
            },
            |error| ConversationTurnTerminalOutcome::Failed { error },
        ),
        "completed" | "interrupted" => ConversationTurnTerminalOutcome::Unknown {
            reason: ConversationTurnTerminalUncertainty::status_error_contradiction(status),
            observed_error: parsed_error,
        },
        "inProgress" => ConversationTurnTerminalOutcome::Unknown {
            reason: ConversationTurnTerminalUncertainty::InProgressStatus,
            observed_error: parsed_error,
        },
        unknown => ConversationTurnTerminalOutcome::Unknown {
            reason: ConversationTurnTerminalUncertainty::unknown_status(
                bounded_terminal_protocol_text(unknown, MAX_TERMINAL_PROTOCOL_TEXT_BYTES),
            ),
            observed_error: parsed_error,
        },
    }
}

fn parse_items_view(value: Option<&Value>) -> ConversationTurnItemsView {
    match value.and_then(Value::as_str) {
        None if value.is_none() => ConversationTurnItemsView::Full,
        Some("notLoaded") => ConversationTurnItemsView::NotLoaded,
        Some("summary") => ConversationTurnItemsView::Summary,
        Some("full") => ConversationTurnItemsView::Full,
        Some(unknown) => ConversationTurnItemsView::unknown(bounded_terminal_protocol_text(
            unknown,
            MAX_TERMINAL_PROTOCOL_TEXT_BYTES,
        )),
        None => ConversationTurnItemsView::unknown(
            value.map_or_else(|| "null".to_string(), bounded_terminal_protocol_json),
        ),
    }
}

fn bounded_terminal_protocol_json(value: &Value) -> String {
    let mut writer = BoundedProtocolJsonWriter::new(MAX_TERMINAL_PROTOCOL_TEXT_BYTES);
    if serde_json::to_writer(&mut writer, value).is_err() {
        return "unencodable protocol value".to_string();
    }
    writer.finish()
}

struct BoundedProtocolJsonWriter {
    bytes: Vec<u8>,
    maximum_bytes: usize,
    truncated: bool,
}

impl BoundedProtocolJsonWriter {
    fn new(maximum_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(maximum_bytes),
            maximum_bytes,
            truncated: false,
        }
    }

    fn finish(self) -> String {
        let body = String::from_utf8_lossy(&self.bytes);
        if !self.truncated {
            return body.into_owned();
        }

        let marker = "...";
        let prefix =
            bounded_terminal_protocol_text(&body, self.maximum_bytes.saturating_sub(marker.len()));
        format!("{prefix}{marker}")
    }
}

impl Write for BoundedProtocolJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let retained = buffer
            .len()
            .min(self.maximum_bytes.saturating_sub(self.bytes.len()));
        self.bytes.extend_from_slice(&buffer[..retained]);
        self.truncated |= retained < buffer.len();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn bounded_terminal_protocol_text(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_string();
    }

    let mut boundary = maximum_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

pub(super) fn to_conversation_message(item: Value) -> Option<ConversationMessage> {
    /*
     * Snapshot reads replay historical turn items rather than live notifications. This parser intentionally mirrors
     * handle_completed_item so resumed sessions and live streams render agent messages, tool summaries, and user
     * messages with the same ConversationMessage vocabulary.
     */
    let item_type = item.get("type")?.as_str()?;
    match item_type {
        "userMessage" => {
            let text = item
                .get("content")
                .and_then(Value::as_array)
                .map(|content| extract_user_input_text(content.as_slice()))
                .filter(|value| !value.trim().is_empty())?;

            Some(ConversationMessage::new(
                ConversationMessageKind::User,
                text,
                None,
                item.get("id").and_then(Value::as_str).map(str::to_string),
            ))
        }
        "agentMessage" => Some(ConversationMessage::new(
            ConversationMessageKind::Agent,
            item.get("text").and_then(Value::as_str).unwrap_or_default(),
            item.get("phase")
                .and_then(Value::as_str)
                .map(str::to_string),
            item.get("id").and_then(Value::as_str).map(str::to_string),
        )),
        "fileChange" if item.get("status").and_then(Value::as_str) == Some("completed") => {
            Some(ConversationMessage::new(
                ConversationMessageKind::Tool,
                format_file_change_summary(&item),
                None,
                item.get("id").and_then(Value::as_str).map(str::to_string),
            ))
        }
        "commandExecution" => Some(ConversationMessage::new(
            ConversationMessageKind::Tool,
            format_command_execution_summary(&item),
            None,
            item.get("id").and_then(Value::as_str).map(str::to_string),
        )),
        _ => None,
    }
}

fn permits_completed_item_side_effect(
    kind: &ConversationItemKind,
    outcome: &ConversationItemOutcome,
) -> bool {
    match kind {
        ConversationItemKind::AgentMessage | ConversationItemKind::CommandExecution => true,
        ConversationItemKind::FileChange => matches!(outcome, ConversationItemOutcome::Completed),
        _ => false,
    }
}

fn changed_planning_file_paths(item: &Value) -> Vec<String> {
    /*
     * Only canonical planning runtime files should trigger planning refresh/repair
     * follow-up after a turn completes. The app-server may report repo-relative,
     * absolute, or legacy-looking paths; canonical_active_planning_file_path is the
     * single gate that keeps arbitrary file edits from waking planning automation.
     */
    let mut paths = Vec::new();

    for path in changed_file_paths(item) {
        if let Some(canonical_path) = canonical_active_planning_file_path(&path) {
            push_unique_path(&mut paths, canonical_path.to_string());
        }
    }

    paths
}

fn format_file_change_summary(item: &Value) -> String {
    /*
     * Tool activity copy stays compact because it appears inline in the live TUI
     * activity area. The detailed diff remains in the underlying app-server item;
     * this summary is only a scan-friendly progress signal.
     */
    let changes = item
        .get("changes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if changes.is_empty() {
        return "file change completed".to_string();
    }

    let entries = changes
        .iter()
        .map(|change| {
            let path = change
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("unknown-path");
            let kind = change
                .get("kind")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("update");
            format!("{kind} {path}")
        })
        .collect::<Vec<_>>();

    format!("file change: {}", entries.join(", "))
}

fn count_file_changes(item: &Value) -> usize {
    // The count is separate from text so UI components can choose badge/count rendering without reparsing the summary.
    item.get("changes")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default()
}

fn format_command_execution_summary(item: &Value) -> String {
    // Command execution payloads are reduced to command plus status; stdout/stderr detail is left to app-server transcript items.
    let command = item
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or("command");
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    format!("command: {command} [{status}]")
}

fn extract_user_input_text(items: &[Value]) -> String {
    // userMessage content is an array; only text fragments become transcript text and multiple fragments keep line breaks.
    items
        .iter()
        .filter_map(|content| {
            if content.get("type").and_then(Value::as_str) == Some("text") {
                content.get("text").and_then(Value::as_str)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn changed_file_paths(item: &Value) -> Vec<String> {
    /*
     * fileChange items can report both the original path and a move destination.
     * Planning tracking has to inspect both because moving an active planning doc out
     * of or into the canonical location is just as relevant as editing it in place.
     */
    let mut paths = Vec::new();

    for change in item
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(path) = change.get("path").and_then(Value::as_str) {
            paths.push(path.to_string());
        }
        if let Some(move_path) = change
            .get("kind")
            .and_then(|kind| kind.get("move_path"))
            .and_then(Value::as_str)
        {
            paths.push(move_path.to_string());
        }
    }

    paths
}

fn push_unique_path(target: &mut Vec<String>, path: String) {
    // app-server may report repeated changes in one item; downstream planning refresh only needs each canonical path once.
    if !target.iter().any(|existing| existing == &path) {
        target.push(path);
    }
}

fn parse_approval_review_event(
    notification: &AppServerNotification,
    thread_id: &str,
    turn_id: &str,
) -> Option<ConversationStreamEvent> {
    /*
     * Approval review parsing is intentionally optional. A malformed review payload
     * should surface as a dropped-notification warning, while the surrounding tool
     * execution and message stream continue reducing normally.
     */
    if !matches!(
        notification.method(),
        "item/autoApprovalReview/started" | "item/autoApprovalReview/completed"
    ) {
        return None;
    }

    let params = notification.params();
    if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
        || params.get("turnId").and_then(Value::as_str) != Some(turn_id)
    {
        return None;
    }

    let review = params.get("review")?;
    let status = review
        .get("status")
        .and_then(Value::as_str)
        .map(parse_approval_review_status)?;
    let target_item_id = params.get("targetItemId").and_then(Value::as_str)?;

    Some(ConversationStreamEvent::ApprovalReviewUpdated {
        review: ConversationApprovalReview {
            target_item_id: target_item_id.to_string(),
            status,
            risk_level: review
                .get("riskLevel")
                .and_then(Value::as_str)
                .map(str::to_string),
            rationale: review
                .get("rationale")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
    })
}

fn parse_approval_review_status(value: &str) -> ConversationApprovalReviewStatus {
    // Unknown statuses are preserved verbatim so UI/debug output can reveal new app-server vocabulary.
    match value {
        "inProgress" => ConversationApprovalReviewStatus::InProgress,
        "approved" => ConversationApprovalReviewStatus::Approved,
        "denied" => ConversationApprovalReviewStatus::Denied,
        "aborted" => ConversationApprovalReviewStatus::Aborted,
        other => ConversationApprovalReviewStatus::Unknown(other.to_string()),
    }
}

fn handle_completed_item(
    item: Option<&Value>,
    event_sender: &dyn AppServerEventSender,
) -> Result<()> {
    // Live item/completed notifications fan out to either final message events or tool activity events.
    let Some(item) = item else {
        return Ok(());
    };

    let item_type = item.get("type").and_then(Value::as_str);
    match item_type {
        Some("agentMessage") => {
            let item_id = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let phase = item
                .get("phase")
                .and_then(Value::as_str)
                .map(str::to_string);
            send_required_app_server_event(
                event_sender,
                ConversationStreamEvent::AgentMessageCompleted {
                    item_id,
                    phase,
                    text,
                },
                "item/agentMessage/completed",
            )?;
        }
        Some("fileChange") => {
            send_required_app_server_event(
                event_sender,
                ConversationStreamEvent::ToolActivity {
                    activity: ConversationToolActivity {
                        kind: ConversationToolActivityKind::FileChange,
                        text: format_file_change_summary(item),
                        file_change_count: count_file_changes(item),
                    },
                },
                "item/fileChange/completed",
            )?;
        }
        Some("commandExecution") => {
            send_required_app_server_event(
                event_sender,
                ConversationStreamEvent::ToolActivity {
                    activity: ConversationToolActivity {
                        kind: ConversationToolActivityKind::CommandExecution,
                        text: format_command_execution_summary(item),
                        file_change_count: 0,
                    },
                },
                "item/commandExecution/completed",
            )?;
        }
        _ => {}
    }

    Ok(())
}

fn record_changed_planning_file_paths(
    item: Option<&Value>,
    changed_file_paths_for_turn: &mut Vec<String>,
) {
    /*
     * Planning file tracking is side-band state because fileChange tool activity is
     * emitted before the turn is known to be complete. Holding only canonical paths
     * here lets TurnCompleted remain the single post-turn trigger for planning
     * refreshes.
     */
    let Some(item) = item else {
        return;
    };
    if item.get("type").and_then(Value::as_str) != Some("fileChange") {
        return;
    }

    for path in changed_planning_file_paths(item) {
        push_unique_path(changed_file_paths_for_turn, path);
    }
}

fn matches_active_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    // Most item notifications include both identifiers, which is the strongest guard against stale shared-connection events.
    !thread_id.is_empty()
        && !turn_id.is_empty()
        && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
        && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
}

fn matches_started_turn(params: &Value, thread_id: &str, turn_id: &str) -> bool {
    !thread_id.is_empty()
        && !turn_id.is_empty()
        && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
        && params
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            == Some(turn_id)
}

#[cfg(test)]
mod terminal_receipt_tests {
    use std::sync::mpsc::{channel, sync_channel};

    use serde_json::json;

    use super::*;
    use crate::domain::turn_terminal::ConversationTurnApplicationDelivery;

    const THREAD_ID: &str = "thread-live";
    const TURN_ID: &str = "turn-live";
    const RESULT_OUTPUT_PATH: &str = ".codex-exec-loop/planning/result-output.md";

    #[test]
    fn terminal_statuses_reduce_to_typed_outcomes_and_metadata() {
        let cases = [
            ("completed", Value::Null, ExpectedTerminalOutcome::Completed),
            (
                "interrupted",
                Value::Null,
                ExpectedTerminalOutcome::Interrupted,
            ),
            (
                "failed",
                json!({
                    "message": "provider rejected the request",
                    "additionalDetails": "quota is exhausted",
                    "codexErrorInfo": "usageLimitExceeded"
                }),
                ExpectedTerminalOutcome::Failed,
            ),
            (
                "inProgress",
                Value::Null,
                ExpectedTerminalOutcome::InProgress,
            ),
        ];

        for (status, error, expected) in cases {
            let mut state = ActiveTurnNotificationState::new();
            let handling = reduce(
                terminal_notification(json!({
                    "id": TURN_ID,
                    "items": [],
                    "itemsView": "summary",
                    "status": status,
                    "error": error,
                    "startedAt": 10,
                    "completedAt": 12,
                    "durationMs": 2_000
                })),
                &mut state,
            );
            let receipt = terminal_receipt(handling);

            assert_eq!(receipt.thread_id, THREAD_ID);
            assert_eq!(receipt.turn_id, TURN_ID);
            assert_eq!(receipt.items_view, ConversationTurnItemsView::Summary);
            assert_eq!(receipt.started_at, Some(10));
            assert_eq!(receipt.completed_at, Some(12));
            assert_eq!(receipt.duration_ms, Some(2_000));
            assert_eq!(
                receipt.application_delivery,
                ConversationTurnApplicationDelivery::Pending
            );
            assert_expected_outcome(&receipt.outcome, expected);
        }
    }

    #[test]
    fn retrying_error_emits_typed_fact_while_non_retry_error_becomes_candidate() {
        let retrying = error_notification(
            true,
            json!({
                "message": "response stream disconnected",
                "additionalDetails": "reconnecting",
                "codexErrorInfo": {
                    "responseStreamDisconnected": { "httpStatusCode": 503 }
                }
            }),
        );
        let (sender, receiver) = channel();
        let mut state = ActiveTurnNotificationState::new();
        let handling = handle_turn_notification(&retrying, THREAD_ID, TURN_ID, &mut state, &sender)
            .expect("retrying error should remain in the live stream");

        let TurnNotificationHandling::RetryObserved { error } = handling else {
            panic!("expected retry observation");
        };
        assert_eq!(error.message, "response stream disconnected");
        assert_eq!(error.additional_details.as_deref(), Some("reconnecting"));
        assert_eq!(
            error.info,
            Some(ConversationTurnErrorInfo::ResponseStreamDisconnected {
                http_status_code: Some(503)
            })
        );
        assert_eq!(
            receiver.recv().expect("retry event should be emitted"),
            ConversationStreamEvent::TurnRetrying {
                thread_id: THREAD_ID.to_string(),
                turn_id: TURN_ID.to_string(),
                error: error.clone(),
            }
        );

        let non_retrying = error_notification(
            false,
            json!({
                "message": "request rejected",
                "codexErrorInfo": "badRequest"
            }),
        );
        let (sender, receiver) = channel();
        let handling =
            handle_turn_notification(&non_retrying, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("non-retry error should become a grace-period candidate");
        let TurnNotificationHandling::NonRetryErrorCandidate { error } = handling else {
            panic!("expected non-retry candidate");
        };
        assert_eq!(error.info, Some(ConversationTurnErrorInfo::BadRequest));
        assert!(receiver.try_recv().is_err());
        assert!(state.terminal_receipt().is_none());
    }

    #[test]
    fn retry_fact_rejected_by_full_sink_fails_closed() {
        let (sender, _receiver) = sync_channel(1);
        sender
            .try_send(ConversationStreamEvent::StatusUpdated {
                text: "occupy application sink".to_string(),
            })
            .expect("fixture should fill the bounded sink");
        let mut state = ActiveTurnNotificationState::new();

        let error = handle_turn_notification(
            &error_notification(true, json!({ "message": "retry me" })),
            THREAD_ID,
            TURN_ID,
            &mut state,
            &sender,
        )
        .expect_err("a retry fact that cannot be projected must fail closed");

        assert!(error.to_string().contains("turn/retrying"));
        assert!(state.terminal_receipt().is_none());
    }

    #[test]
    fn completed_message_rejected_by_full_sink_fails_closed() {
        let notification = AppServerNotification::from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": THREAD_ID,
                "turnId": TURN_ID,
                "completedAtMs": 1,
                "item": {
                    "id": "agent-1",
                    "type": "agentMessage",
                    "text": "authoritative final text"
                }
            }
        }))
        .expect("notification");
        let (sender, _receiver) = sync_channel(1);
        sender
            .try_send(ConversationStreamEvent::StatusUpdated {
                text: "occupy application sink".to_string(),
            })
            .expect("fixture should fill the bounded sink");
        let mut state = ActiveTurnNotificationState::new();

        let error =
            handle_turn_notification(&notification, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect_err("a completed message that cannot be projected must fail closed");

        assert!(error.to_string().contains("item/completed/lifecycle"));
        assert!(state.terminal_receipt().is_none());
    }

    #[test]
    fn turn_started_requires_exact_nonempty_thread_and_nested_turn_ids() {
        let valid = AppServerNotification::from_value(json!({
            "method": "turn/started",
            "params": {
                "threadId": THREAD_ID,
                "turn": { "id": TURN_ID }
            }
        }))
        .expect("notification");
        let (sender, receiver) = channel();
        let mut state = ActiveTurnNotificationState::new();
        assert_eq!(
            handle_turn_notification(&valid, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("matching start should reduce"),
            TurnNotificationHandling::Consumed
        );
        assert!(
            receiver.try_recv().is_err(),
            "turn/start response owns the single request-bearing start event"
        );

        let invalid_cases = [
            (
                "stale thread",
                THREAD_ID,
                TURN_ID,
                json!({ "threadId": "thread-stale", "turn": { "id": TURN_ID } }),
            ),
            (
                "stale nested turn",
                THREAD_ID,
                TURN_ID,
                json!({ "threadId": THREAD_ID, "turn": { "id": "turn-stale" } }),
            ),
            (
                "missing nested turn id",
                THREAD_ID,
                TURN_ID,
                json!({ "threadId": THREAD_ID, "turn": {} }),
            ),
            (
                "blank nested turn id",
                THREAD_ID,
                TURN_ID,
                json!({ "threadId": THREAD_ID, "turn": { "id": "" } }),
            ),
            (
                "blank active thread id",
                "",
                TURN_ID,
                json!({ "threadId": "", "turn": { "id": TURN_ID } }),
            ),
            (
                "blank active turn id",
                THREAD_ID,
                "",
                json!({ "threadId": THREAD_ID, "turn": { "id": "" } }),
            ),
        ];
        for (label, active_thread_id, active_turn_id, params) in invalid_cases {
            let notification = AppServerNotification::from_value(json!({
                "method": "turn/started",
                "params": params,
            }))
            .expect("notification");
            let (sender, receiver) = channel();
            let mut state = ActiveTurnNotificationState::new();
            let handling = handle_turn_notification(
                &notification,
                active_thread_id,
                active_turn_id,
                &mut state,
                &sender,
            )
            .expect("identity mismatch should be diagnostic");

            assert!(
                matches!(handling, TurnNotificationHandling::Dropped(_)),
                "case: {label}"
            );
            assert!(receiver.try_recv().is_err(), "case: {label}");
        }
    }

    #[test]
    fn runtime_envelope_notifications_require_exact_scope_and_valid_settings() {
        let settings = AppServerNotification::from_value(json!({
            "method": "thread/settings/updated",
            "params": valid_settings_params(THREAD_ID),
        }))
        .expect("settings notification");
        let (sender, receiver) = channel();
        let mut state = ActiveTurnNotificationState::new();

        assert_eq!(
            handle_turn_notification(&settings, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("valid settings should reduce"),
            TurnNotificationHandling::Consumed
        );
        assert!(matches!(
            receiver.recv().expect("settings event should be emitted"),
            ConversationStreamEvent::RuntimeEnvelopeObserved { observation }
                if matches!(
                    observation.as_ref(),
                    ConversationRuntimeEnvelopeObservation::SettingsUpdated { thread_id, .. }
                        if thread_id == THREAD_ID
                )
        ));

        let stale = AppServerNotification::from_value(json!({
            "method": "thread/settings/updated",
            "params": valid_settings_params("thread-stale"),
        }))
        .expect("stale settings notification");
        let (sender, receiver) = channel();
        assert!(matches!(
            handle_turn_notification(&stale, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("stale settings should be diagnostic"),
            TurnNotificationHandling::Dropped(_)
        ));
        assert!(receiver.try_recv().is_err());

        let malformed = AppServerNotification::from_value(json!({
            "method": "thread/settings/updated",
            "params": {
                "threadId": THREAD_ID,
                "threadSettings": {
                    "model": "gpt-next",
                    "modelProvider": "openai"
                }
            }
        }))
        .expect("malformed settings notification");
        let (sender, receiver) = channel();
        assert!(matches!(
            handle_turn_notification(&malformed, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("malformed settings should be diagnostic"),
            TurnNotificationHandling::Dropped(ref warning)
                if warning.contains("invalid settings envelope")
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn model_reroute_requires_exact_thread_and_turn_identity() {
        let valid = AppServerNotification::from_value(json!({
            "method": "model/rerouted",
            "params": {
                "threadId": THREAD_ID,
                "turnId": TURN_ID,
                "fromModel": "gpt-a",
                "toModel": "gpt-b",
                "reason": "highRiskCyberActivity"
            }
        }))
        .expect("reroute notification");
        let (sender, receiver) = channel();
        let mut state = ActiveTurnNotificationState::new();
        assert_eq!(
            handle_turn_notification(&valid, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("valid reroute should reduce"),
            TurnNotificationHandling::Consumed
        );
        assert!(matches!(
            receiver.recv().expect("reroute event should be emitted"),
            ConversationStreamEvent::RuntimeEnvelopeObserved { observation }
                if matches!(
                    observation.as_ref(),
                    ConversationRuntimeEnvelopeObservation::ModelRerouted {
                        thread_id,
                        turn_id,
                        ..
                    } if thread_id == THREAD_ID && turn_id == TURN_ID
                )
        ));

        for (thread_id, turn_id) in [("thread-stale", TURN_ID), (THREAD_ID, "turn-stale")] {
            let stale = AppServerNotification::from_value(json!({
                "method": "model/rerouted",
                "params": {
                    "threadId": thread_id,
                    "turnId": turn_id,
                    "fromModel": "gpt-a",
                    "toModel": "gpt-b",
                    "reason": "highRiskCyberActivity"
                }
            }))
            .expect("stale reroute notification");
            let (sender, receiver) = channel();
            assert!(matches!(
                handle_turn_notification(&stale, THREAD_ID, TURN_ID, &mut state, &sender)
                    .expect("stale reroute should be diagnostic"),
                TurnNotificationHandling::Dropped(_)
            ));
            assert!(receiver.try_recv().is_err());
        }
    }

    #[test]
    fn full_sink_records_envelope_gap_without_losing_terminal_truth() {
        let settings = AppServerNotification::from_value(json!({
            "method": "thread/settings/updated",
            "params": valid_settings_params(THREAD_ID),
        }))
        .expect("settings notification");
        let (sender, receiver) = sync_channel(1);
        sender
            .try_send(ConversationStreamEvent::StatusUpdated {
                text: "occupy application sink".to_string(),
            })
            .expect("fixture should fill the sink");
        let mut state = ActiveTurnNotificationState::new();

        assert!(matches!(
            handle_turn_notification(&settings, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("nonterminal projection pressure must not abort the stream"),
            TurnNotificationHandling::Dropped(ref warning)
                if warning.contains("not admitted")
        ));
        assert!(matches!(
            state.runtime_envelope_gap_observation(THREAD_ID),
            Some(ConversationRuntimeEnvelopeObservation::ProjectionGap { gap, .. })
                if gap == ConversationRuntimeObservationGap::settings()
        ));
        receiver.recv().expect("fixture entry should drain");
        let handling = reduce(
            terminal_notification(json!({
                "id": TURN_ID,
                "items": [],
                "status": "completed"
            })),
            &mut state,
        );
        assert!(matches!(
            terminal_receipt(handling).outcome,
            ConversationTurnTerminalOutcome::Completed
        ));
    }

    #[test]
    fn later_successful_settings_snapshot_clears_a_pending_settings_gap() {
        let settings = AppServerNotification::from_value(json!({
            "method": "thread/settings/updated",
            "params": valid_settings_params(THREAD_ID),
        }))
        .expect("settings notification");
        let (sender, receiver) = sync_channel(1);
        sender
            .try_send(ConversationStreamEvent::StatusUpdated {
                text: "occupy application sink".to_string(),
            })
            .expect("fixture should fill the sink");
        let mut state = ActiveTurnNotificationState::new();

        assert!(matches!(
            handle_turn_notification(&settings, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("first settings observation should remain nonterminal"),
            TurnNotificationHandling::Dropped(_)
        ));
        receiver.recv().expect("fixture entry should drain");
        assert_eq!(
            handle_turn_notification(&settings, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("later complete settings snapshot should recover projection truth"),
            TurnNotificationHandling::Consumed
        );
        assert!(matches!(
            receiver
                .recv()
                .expect("recovery snapshot should be emitted"),
            ConversationStreamEvent::RuntimeEnvelopeObserved { .. }
        ));
        assert!(state.runtime_envelope_gap_observation(THREAD_ID).is_none());
    }

    #[test]
    fn delta_and_completed_item_distinguish_stale_from_missing_active_ids() {
        let invalid_notifications = [
            json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "turnId": TURN_ID,
                    "itemId": "agent-1",
                    "delta": "missing thread"
                }
            }),
            json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "threadId": "thread-stale",
                    "turnId": TURN_ID,
                    "itemId": "agent-1",
                    "delta": "stale thread"
                }
            }),
            json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-stale",
                    "turnId": TURN_ID,
                    "completedAtMs": 1,
                    "item": { "id": "agent-1", "type": "agentMessage", "text": "stale thread" }
                }
            }),
        ];

        for value in invalid_notifications {
            let notification =
                AppServerNotification::from_value(value).expect("notification fixture");
            let (sender, receiver) = channel();
            let mut state = ActiveTurnNotificationState::new();
            let handling =
                handle_turn_notification(&notification, THREAD_ID, TURN_ID, &mut state, &sender)
                    .expect("stale or missing identity should be diagnostic");

            assert!(matches!(handling, TurnNotificationHandling::Dropped(_)));
            assert!(receiver.try_recv().is_err());
            assert!(state.changed_planning_file_paths().is_empty());
        }

        let missing_turn = AppServerNotification::from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": THREAD_ID,
                "completedAtMs": 1,
                "item": { "id": "agent-1", "type": "agentMessage", "text": "missing turn" }
            }
        }))
        .expect("missing-turn completion fixture");
        let (sender, receiver) = channel();
        let mut state = ActiveTurnNotificationState::new();

        let error =
            handle_turn_notification(&missing_turn, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect_err("missing completion identity must fail the active stream");

        assert!(error.to_string().contains("lifecycle correlation identity"));
        assert!(receiver.try_recv().is_err());
        assert!(state.changed_planning_file_paths().is_empty());
    }

    #[test]
    fn codex_error_info_vocabulary_is_translated_without_flattening() {
        let simple_cases = [
            (
                "contextWindowExceeded",
                ConversationTurnErrorInfo::ContextWindowExceeded,
            ),
            (
                "sessionBudgetExceeded",
                ConversationTurnErrorInfo::SessionBudgetExceeded,
            ),
            (
                "usageLimitExceeded",
                ConversationTurnErrorInfo::UsageLimitExceeded,
            ),
            (
                "serverOverloaded",
                ConversationTurnErrorInfo::ServerOverloaded,
            ),
            ("cyberPolicy", ConversationTurnErrorInfo::CyberPolicy),
            (
                "internalServerError",
                ConversationTurnErrorInfo::InternalServerError,
            ),
            ("unauthorized", ConversationTurnErrorInfo::Unauthorized),
            ("badRequest", ConversationTurnErrorInfo::BadRequest),
            (
                "threadRollbackFailed",
                ConversationTurnErrorInfo::ThreadRollbackFailed,
            ),
            ("sandboxError", ConversationTurnErrorInfo::SandboxError),
            ("other", ConversationTurnErrorInfo::Other),
        ];
        for (raw, expected) in simple_cases {
            assert_eq!(parse_error_info(&json!(raw)), expected);
        }

        assert_eq!(
            parse_error_info(&json!({
                "httpConnectionFailed": { "httpStatusCode": 429 }
            })),
            ConversationTurnErrorInfo::HttpConnectionFailed {
                http_status_code: Some(429)
            }
        );
        assert_eq!(
            parse_error_info(&json!({
                "responseStreamConnectionFailed": { "httpStatusCode": null }
            })),
            ConversationTurnErrorInfo::ResponseStreamConnectionFailed {
                http_status_code: None
            }
        );
        assert_eq!(
            parse_error_info(&json!({
                "responseTooManyFailedAttempts": { "httpStatusCode": 500 }
            })),
            ConversationTurnErrorInfo::ResponseTooManyFailedAttempts {
                http_status_code: Some(500)
            }
        );
        assert_eq!(
            parse_error_info(&json!({
                "activeTurnNotSteerable": { "turnKind": "review" }
            })),
            ConversationTurnErrorInfo::ActiveTurnNotSteerable {
                turn_kind: "review".to_string()
            }
        );

        let unknown = parse_error_info(&json!({
            "futureError": { "payload": "x".repeat(MAX_TERMINAL_PROTOCOL_TEXT_BYTES * 2) }
        }));
        let ConversationTurnErrorInfo::Unknown(value) = unknown else {
            panic!("future structured error should stay visible as bounded unknown data");
        };
        assert!(value.len() <= 512);
        assert!(value.ends_with("..."));
    }

    #[test]
    fn missing_nested_error_is_bounded_candidate_and_failed_without_error_is_unknown() {
        let notification = error_notification(false, Value::Null);
        let mut state = ActiveTurnNotificationState::new();
        let handling = reduce(notification, &mut state);
        let TurnNotificationHandling::NonRetryErrorCandidate { error } = handling else {
            panic!("missing nested error should still fail closed as a candidate");
        };
        assert!(error.message.contains("omitted error.message"));

        let receipt = terminal_receipt(reduce(
            terminal_notification(json!({
                "id": TURN_ID,
                "items": [],
                "status": "failed"
            })),
            &mut state,
        ));
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::MissingFailedError,
                observed_error: None,
            }
        ));
    }

    #[test]
    fn completed_or_interrupted_status_with_error_is_protocol_unknown() {
        for expected_status in ["completed", "interrupted"] {
            let mut state = ActiveTurnNotificationState::new();
            let receipt = terminal_receipt(reduce(
                terminal_notification(json!({
                    "id": TURN_ID,
                    "items": [],
                    "status": expected_status,
                    "error": { "message": "contradictory error" }
                })),
                &mut state,
            ));

            assert!(matches!(
                receipt.outcome,
                ConversationTurnTerminalOutcome::Unknown {
                    reason: ConversationTurnTerminalUncertainty::StatusErrorContradiction {
                        ref status,
                    },
                    observed_error: Some(_),
                } if status == expected_status
            ));
        }
    }

    #[test]
    fn terminal_identity_mismatch_is_dropped_but_missing_identity_is_unknown() {
        let mut state = ActiveTurnNotificationState::new();
        let mismatched = AppServerNotification::from_value(json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-stale",
                "turn": { "id": TURN_ID, "items": [], "status": "completed" }
            }
        }))
        .expect("notification");
        assert!(matches!(
            reduce(mismatched, &mut state),
            TurnNotificationHandling::Dropped(_)
        ));
        assert!(state.terminal_receipt().is_none());

        for (params, missing_field) in [
            (
                json!({
                    "turn": { "id": TURN_ID, "items": [], "status": "completed" }
                }),
                "threadId",
            ),
            (
                json!({
                    "threadId": null,
                    "turn": { "id": TURN_ID, "items": [], "status": "completed" }
                }),
                "threadId",
            ),
            (
                json!({
                    "threadId": "",
                    "turn": { "id": TURN_ID, "items": [], "status": "completed" }
                }),
                "threadId",
            ),
            (
                json!({
                    "threadId": 7,
                    "turn": { "id": TURN_ID, "items": [], "status": "completed" }
                }),
                "threadId",
            ),
            (
                json!({
                    "threadId": THREAD_ID,
                    "turn": { "items": [], "status": "completed" }
                }),
                "turn.id",
            ),
            (
                json!({
                    "threadId": THREAD_ID,
                    "turn": { "id": null, "items": [], "status": "completed" }
                }),
                "turn.id",
            ),
            (
                json!({
                    "threadId": THREAD_ID,
                    "turn": { "id": "", "items": [], "status": "completed" }
                }),
                "turn.id",
            ),
            (
                json!({
                    "threadId": THREAD_ID,
                    "turn": { "id": 7, "items": [], "status": "completed" }
                }),
                "turn.id",
            ),
            (
                json!({
                    "threadId": THREAD_ID,
                    "turn": null
                }),
                "turn.id",
            ),
        ] {
            let mut state = ActiveTurnNotificationState::new();
            let malformed = AppServerNotification::from_value(json!({
                "method": "turn/completed",
                "params": params,
            }))
            .expect("notification");
            let receipt = terminal_receipt(reduce(malformed, &mut state));
            assert_eq!(receipt.thread_id, THREAD_ID);
            assert_eq!(receipt.turn_id, TURN_ID);
            assert!(matches!(
                receipt.outcome,
                ConversationTurnTerminalOutcome::Unknown {
                    reason: ConversationTurnTerminalUncertainty::MissingRequiredIdentity {
                        ref field
                    },
                    ..
                } if field == missing_field
            ));
            assert_eq!(state.terminal_receipt(), Some(&receipt));
        }

        let mut state = ActiveTurnNotificationState::new();
        let nested_mismatch = terminal_notification(json!({
            "id": "turn-stale",
            "items": [],
            "status": "completed"
        }));
        assert!(matches!(
            reduce(nested_mismatch, &mut state),
            TurnNotificationHandling::Dropped(_)
        ));
    }

    #[test]
    fn unknown_status_and_missing_items_never_become_completed() {
        let mut state = ActiveTurnNotificationState::new();
        let receipt = terminal_receipt(reduce(
            terminal_notification(json!({
                "id": TURN_ID,
                "items": [],
                "status": "futureTerminalState"
            })),
            &mut state,
        ));
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::UnknownStatus(ref status),
                ..
            } if status == "futureTerminalState"
        ));

        let mut state = ActiveTurnNotificationState::new();
        let receipt = terminal_receipt(reduce(
            terminal_notification(json!({
                "id": TURN_ID,
                "status": "completed"
            })),
            &mut state,
        ));
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::ProtocolInconsistency(_),
                ..
            }
        ));
    }

    #[test]
    fn items_view_defaults_to_full_and_malformed_optional_metadata_fails_closed() {
        assert_eq!(parse_items_view(None), ConversationTurnItemsView::Full);
        assert_eq!(
            parse_items_view(Some(&json!("notLoaded"))),
            ConversationTurnItemsView::NotLoaded
        );
        assert_eq!(
            parse_items_view(Some(&json!("summary"))),
            ConversationTurnItemsView::Summary
        );
        assert_eq!(
            parse_items_view(Some(&json!("full"))),
            ConversationTurnItemsView::Full
        );

        let mut state = ActiveTurnNotificationState::new();
        let receipt = terminal_receipt(reduce(
            terminal_notification(json!({
                "id": TURN_ID,
                "items": [],
                "status": "completed",
                "durationMs": "not-an-integer"
            })),
            &mut state,
        ));
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Unknown {
                reason: ConversationTurnTerminalUncertainty::ProtocolInconsistency(_),
                ..
            }
        ));
    }

    #[test]
    fn first_terminal_receipt_is_idempotent_and_late_file_change_cannot_promote_it() {
        let mut state = ActiveTurnNotificationState::new();
        let first = terminal_receipt(reduce(
            terminal_notification(json!({
                "id": TURN_ID,
                "items": [],
                "status": "interrupted"
            })),
            &mut state,
        ));
        assert!(first.observations.changed_planning_file_paths.is_empty());

        let late_file_change = AppServerNotification::from_value(json!({
            "method": "item/completed",
            "params": {
                "threadId": THREAD_ID,
                "turnId": TURN_ID,
                "item": {
                    "id": "file-change-late",
                    "type": "fileChange",
                    "changes": [{
                        "path": RESULT_OUTPUT_PATH,
                        "kind": { "type": "update" }
                    }]
                }
            }
        }))
        .expect("notification");
        assert!(matches!(
            reduce(late_file_change, &mut state),
            TurnNotificationHandling::Dropped(_)
        ));
        assert!(state.changed_planning_file_paths().is_empty());

        let duplicate = reduce(
            terminal_notification(json!({
                "id": TURN_ID,
                "items": [],
                "status": "completed"
            })),
            &mut state,
        );
        let TurnNotificationHandling::DuplicateTerminal { receipt } = duplicate else {
            panic!("second terminal notification should be classified as a duplicate");
        };
        assert_eq!(receipt, first);
        assert!(matches!(
            receipt.outcome,
            ConversationTurnTerminalOutcome::Interrupted
        ));
        assert!(receipt.observations.changed_planning_file_paths.is_empty());
        assert_eq!(state.terminal_receipt(), Some(&first));
    }

    #[test]
    fn mismatched_error_does_not_emit_retry_or_create_terminal_state() {
        let notification = AppServerNotification::from_value(json!({
            "method": "error",
            "params": {
                "threadId": THREAD_ID,
                "turnId": "turn-stale",
                "willRetry": true,
                "error": { "message": "stale retry" }
            }
        }))
        .expect("notification");
        let (sender, receiver) = channel();
        let mut state = ActiveTurnNotificationState::new();
        let handling =
            handle_turn_notification(&notification, THREAD_ID, TURN_ID, &mut state, &sender)
                .expect("mismatch should be diagnostic, not fatal");

        assert!(matches!(handling, TurnNotificationHandling::Dropped(_)));
        assert!(receiver.try_recv().is_err());
        assert!(state.terminal_receipt().is_none());
    }

    #[test]
    fn unknown_protocol_strings_are_bounded_before_entering_domain_state() {
        let mut state = ActiveTurnNotificationState::new();
        let receipt = terminal_receipt(reduce(
            terminal_notification(json!({
                "id": TURN_ID,
                "items": [],
                "itemsView": "v".repeat(MAX_TERMINAL_PROTOCOL_TEXT_BYTES + 10),
                "status": "s".repeat(MAX_TERMINAL_PROTOCOL_TEXT_BYTES + 10)
            })),
            &mut state,
        ));
        let ConversationTurnItemsView::Unknown(items_view) = receipt.items_view else {
            panic!("unknown items view should be retained");
        };
        let ConversationTurnTerminalOutcome::Unknown {
            reason: ConversationTurnTerminalUncertainty::UnknownStatus(status),
            ..
        } = receipt.outcome
        else {
            panic!("unknown status should be retained");
        };
        assert!(items_view.len() <= 512);
        assert!(status.len() <= 512);
    }

    #[derive(Clone, Copy)]
    enum ExpectedTerminalOutcome {
        Completed,
        Interrupted,
        Failed,
        InProgress,
    }

    fn assert_expected_outcome(
        outcome: &ConversationTurnTerminalOutcome,
        expected: ExpectedTerminalOutcome,
    ) {
        match expected {
            ExpectedTerminalOutcome::Completed => {
                assert!(matches!(
                    outcome,
                    ConversationTurnTerminalOutcome::Completed
                ));
            }
            ExpectedTerminalOutcome::Interrupted => {
                assert!(matches!(
                    outcome,
                    ConversationTurnTerminalOutcome::Interrupted
                ));
            }
            ExpectedTerminalOutcome::Failed => {
                let ConversationTurnTerminalOutcome::Failed { error } = outcome else {
                    panic!("expected failed terminal outcome");
                };
                assert_eq!(
                    error.info,
                    Some(ConversationTurnErrorInfo::UsageLimitExceeded)
                );
            }
            ExpectedTerminalOutcome::InProgress => {
                assert!(matches!(
                    outcome,
                    ConversationTurnTerminalOutcome::Unknown {
                        reason: ConversationTurnTerminalUncertainty::InProgressStatus,
                        ..
                    }
                ));
            }
        }
    }

    fn terminal_receipt(handling: TurnNotificationHandling) -> ConversationTurnTerminalReceipt {
        let TurnNotificationHandling::Terminal { receipt } = handling else {
            panic!("expected first terminal receipt");
        };
        receipt
    }

    fn reduce(
        notification: AppServerNotification,
        state: &mut ActiveTurnNotificationState,
    ) -> TurnNotificationHandling {
        let (sender, _receiver) = channel();
        handle_turn_notification(&notification, THREAD_ID, TURN_ID, state, &sender)
            .expect("notification should reduce")
    }

    fn terminal_notification(turn: Value) -> AppServerNotification {
        AppServerNotification::from_value(json!({
            "method": "turn/completed",
            "params": {
                "threadId": THREAD_ID,
                "turn": turn
            }
        }))
        .expect("notification")
    }

    fn valid_settings_params(thread_id: &str) -> Value {
        json!({
            "threadId": thread_id,
            "threadSettings": {
                "model": "gpt-next",
                "modelProvider": "openai",
                "effort": "ultra",
                "serviceTier": null,
                "cwd": "/repo",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "sandboxPolicy": { "type": "workspaceWrite" },
                "activePermissionProfile": null,
                "collaborationMode": {}
            }
        })
    }

    fn error_notification(will_retry: bool, error: Value) -> AppServerNotification {
        AppServerNotification::from_value(json!({
            "method": "error",
            "params": {
                "threadId": THREAD_ID,
                "turnId": TURN_ID,
                "willRetry": will_retry,
                "error": error
            }
        }))
        .expect("notification")
    }
}
