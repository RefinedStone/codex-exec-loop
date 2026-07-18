use std::fmt;
use std::sync::{Arc, Weak};

use super::progressive_activity_cards::{
    ProgressiveActivityCard, card_detail_text, project_activity_cards,
};
use crate::domain::conversation_progressive_activity::{
    ConversationProgressiveActivityPayload, ConversationProgressiveActivityProjectionSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityDetailKind {
    Diff,
    Output,
}

pub(crate) struct ProgressiveActivityDocument {
    pub(crate) kind: ProgressiveActivityDetailKind,
    pub(crate) sequence: u64,
    pub(crate) source_bytes: u64,
    pub(crate) retained_bytes: u64,
    pub(crate) truncated_bytes: u64,
    pub(crate) history_incomplete: bool,
    snapshot: Arc<ConversationProgressiveActivityProjectionSnapshot>,
    record_index: usize,
    owned_text: Option<String>,
}

impl ProgressiveActivityDocument {
    pub(crate) fn text(&self) -> &str {
        if let Some(owned_text) = self.owned_text.as_deref() {
            return owned_text;
        }
        detail_payload(
            &self.snapshot.records[self.record_index]
                .observation()
                .payload,
            self.kind,
        )
        .expect("document kind must match its retained record")
        .0
    }
}

impl fmt::Debug for ProgressiveActivityDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProgressiveActivityDocument")
            .field("kind", &self.kind)
            .field("sequence", &self.sequence)
            .field("source_bytes", &self.source_bytes)
            .field("retained_bytes", &self.retained_bytes)
            .field("truncated_bytes", &self.truncated_bytes)
            .field("history_incomplete", &self.history_incomplete)
            .finish()
    }
}

#[derive(Clone, Default)]
pub(crate) struct ProgressiveActivityDetailState {
    // A strong reference would force Core's next Arc::make_mut to clone up to
    // the full retained projection. The document guard upgrades only for one frame.
    snapshot: Weak<ConversationProgressiveActivityProjectionSnapshot>,
    lifecycle_epoch: u64,
}

impl ProgressiveActivityDetailState {
    pub(crate) fn replace_snapshot(
        &mut self,
        snapshot: &Arc<ConversationProgressiveActivityProjectionSnapshot>,
    ) {
        self.snapshot = Arc::downgrade(snapshot);
    }

    pub(crate) fn document(
        &self,
        kind: ProgressiveActivityDetailKind,
    ) -> Option<ProgressiveActivityDocument> {
        let snapshot = self.snapshot.upgrade()?;
        let (record_index, sequence, source_bytes, retained_bytes, truncated_bytes) = snapshot
            .records
            .iter()
            .enumerate()
            .filter_map(|(record_index, record)| {
                let (text, source_bytes, truncated_bytes) =
                    detail_payload(&record.observation().payload, kind)?;
                Some((
                    record_index,
                    record.last_sequence(),
                    source_bytes,
                    u64::try_from(text.len()).unwrap_or(u64::MAX),
                    truncated_bytes,
                ))
            })
            .max_by_key(|(_, sequence, _, _, _)| *sequence)?;
        let history_incomplete = snapshot.history_incomplete();
        Some(ProgressiveActivityDocument {
            kind,
            sequence,
            source_bytes,
            retained_bytes,
            truncated_bytes,
            history_incomplete,
            snapshot,
            record_index,
            owned_text: None,
        })
    }

    pub(crate) fn cards(&self) -> Vec<ProgressiveActivityCard> {
        let Some(snapshot) = self.snapshot.upgrade() else {
            return Vec::new();
        };
        project_activity_cards(&snapshot)
    }

    pub(crate) fn card_document(
        &self,
        card: &ProgressiveActivityCard,
    ) -> Option<ProgressiveActivityDocument> {
        let snapshot = self.snapshot.upgrade()?;
        if card.record_index >= snapshot.records.len() {
            return None;
        }
        let detail = card_detail_text(&snapshot, card.record_index)?;
        let history_incomplete = snapshot.history_incomplete();
        // Card documents synthesize detail for every progressive kind. Diff/Output
        // kind is only a legacy tab label; card body text lives in owned_text.
        Some(ProgressiveActivityDocument {
            kind: ProgressiveActivityDetailKind::Output,
            sequence: card.key.sequence,
            source_bytes: card.source_bytes,
            retained_bytes: card.retained_bytes,
            truncated_bytes: card.truncated_bytes,
            history_incomplete,
            snapshot,
            record_index: card.record_index,
            owned_text: Some(detail),
        })
    }

    pub(crate) fn reset(&mut self) {
        self.snapshot = Weak::new();
        self.lifecycle_epoch = self.lifecycle_epoch.wrapping_add(1);
    }

    pub(crate) const fn lifecycle_epoch(&self) -> u64 {
        self.lifecycle_epoch
    }
}

fn detail_payload(
    payload: &ConversationProgressiveActivityPayload,
    kind: ProgressiveActivityDetailKind,
) -> Option<(&str, u64, u64)> {
    match (kind, payload) {
        (
            ProgressiveActivityDetailKind::Diff,
            ConversationProgressiveActivityPayload::TurnDiff {
                detail,
                source_bytes,
                truncated_bytes,
                ..
            },
        ) => Some((detail, *source_bytes, *truncated_bytes)),
        (
            ProgressiveActivityDetailKind::Output,
            ConversationProgressiveActivityPayload::CommandOutput {
                tail,
                source_bytes,
                truncated_bytes,
                ..
            },
        ) => Some((tail, *source_bytes, *truncated_bytes)),
        _ => None,
    }
}

impl fmt::Debug for ProgressiveActivityDetailState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let snapshot = self.snapshot.upgrade();
        formatter
            .debug_struct("ProgressiveActivityDetailState")
            .field("lifecycle_epoch", &self.lifecycle_epoch)
            .field("snapshot_present", &snapshot.is_some())
            .field(
                "last_sequence",
                &snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.last_sequence),
            )
            .field(
                "history_incomplete",
                &snapshot.is_some_and(|snapshot| snapshot.history_incomplete()),
            )
            .field(
                "diff_available",
                &self.document(ProgressiveActivityDetailKind::Diff).is_some(),
            )
            .field(
                "output_available",
                &self
                    .document(ProgressiveActivityDetailKind::Output)
                    .is_some(),
            )
            .finish()
    }
}

#[cfg(test)]
#[path = "progressive_activity_detail_tests.rs"]
mod tests;
