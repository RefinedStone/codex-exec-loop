//! Persist only usage from accepted, correctly correlated stream snapshots.
use crate::core::app::{AppEvent, TurnStreamUpdate};
use crate::domain::conversation_progressive_activity::ConversationProgressiveActivityPayload;
use crate::domain::kernel_event::{KernelEvent, ThreadTurn};

pub(super) fn usage_events(events: &[AppEvent], turns: &[ThreadTurn]) -> Vec<KernelEvent> {
    let mut result = Vec::new();
    for event in events {
        let AppEvent::TurnStreamSnapshotChanged(snapshot) = event else {
            continue;
        };
        let TurnStreamUpdate::ProgressiveActivityObserved {
            activity,
            rejection: None,
        } = &snapshot.update
        else {
            continue;
        };
        let (Some(first), Some(last)) = (activity.first_sequence, activity.last_sequence) else {
            continue;
        };
        for record in &snapshot.progressive_activity.records {
            if record.last_sequence() < first || record.last_sequence() > last {
                continue;
            }
            let observation = record.observation();
            let ConversationProgressiveActivityPayload::TokenUsage { usage } = &observation.payload
            else {
                continue;
            };
            let Some(turn_id) = &observation.turn_id else {
                continue;
            };
            if snapshot.thread_id.as_ref() != Some(&observation.thread_id)
                || snapshot.active_turn_id.as_ref() != Some(turn_id)
            {
                continue;
            }
            let op_id = turns
                .iter()
                .find(|turn| {
                    turn.thread_id == observation.thread_id
                        && turn.turn_id.as_ref() == Some(turn_id)
                })
                .and_then(|turn| turn.op_id);
            result.push(KernelEvent::TurnUsageObserved {
                thread_id: observation.thread_id.clone(),
                turn_id: turn_id.clone(),
                op_id,
                last: usage.last,
                total: usage.total,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::usage_events;
    use crate::core::app::{
        AppEvent, TurnStreamProgressiveActivityUpdate, TurnStreamSnapshot, TurnStreamUpdate,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
        ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
        ConversationProgressiveActivityProjection, ConversationProgressiveActivityRejection,
        ConversationProgressiveTokenUsage, ConversationProgressiveTokenUsageBreakdown,
    };
    use crate::domain::kernel_event::{KernelEvent, ThreadTurn};

    #[test]
    fn task_usage_captures_only_accepted_current_turn_usage() {
        let tokens = ConversationProgressiveTokenUsageBreakdown {
            input_tokens: 10,
            total_tokens: 10,
            ..Default::default()
        };
        let batch = ConversationProgressiveActivityBatch::single(
            ConversationProgressiveActivityObservation {
                sequence: 1,
                thread_id: "thread".into(),
                turn_id: Some("turn".into()),
                item_id: None,
                kind: ConversationProgressiveActivityKind::TokenUsage,
                payload: ConversationProgressiveActivityPayload::TokenUsage {
                    usage: ConversationProgressiveTokenUsage {
                        last: tokens,
                        total: tokens,
                        model_context_window: None,
                    },
                },
            },
        )
        .unwrap();
        let mut projection = ConversationProgressiveActivityProjection::default();
        projection
            .apply_batch_correlated(Some("thread"), Some("turn"), batch)
            .unwrap();
        let activity = TurnStreamProgressiveActivityUpdate {
            first_sequence: Some(1),
            last_sequence: Some(1),
            source_observation_count: 1,
            superseded_publication_count: 0,
            payload_truncation_count: 0,
            dropped_observation_count: 0,
            invalid_observation_count: 0,
            unknown_observation_count: 0,
        };
        let mut snapshot = TurnStreamSnapshot {
            revision: 1,
            thread_id: Some("thread".into()),
            title: None,
            cwd: None,
            runtime_envelope: None,
            item_lifecycle: Default::default(),
            progressive_activity: projection.snapshot(),
            active_turn_id: Some("turn".into()),
            status_text: None,
            terminal: None,
            update: TurnStreamUpdate::ProgressiveActivityObserved {
                activity,
                rejection: None,
            },
        };
        let turns = vec![ThreadTurn {
            thread_id: "thread".into(),
            turn_id: Some("turn".into()),
            op_id: Some(7),
        }];
        let captured = usage_events(
            &[AppEvent::turn_stream_snapshot_changed(snapshot.clone())],
            &turns,
        );
        assert_eq!(
            captured,
            vec![KernelEvent::TurnUsageObserved {
                thread_id: "thread".into(),
                turn_id: "turn".into(),
                op_id: Some(7),
                last: tokens,
                total: tokens,
            }]
        );
        snapshot.update = TurnStreamUpdate::ProgressiveActivityObserved {
            activity,
            rejection: Some(ConversationProgressiveActivityRejection::ThreadMismatch),
        };
        assert!(
            usage_events(
                &[AppEvent::turn_stream_snapshot_changed(snapshot.clone())],
                &turns
            )
            .is_empty()
        );
        snapshot.update = TurnStreamUpdate::ProgressiveActivityObserved {
            activity,
            rejection: None,
        };
        snapshot.active_turn_id = Some("other".into());
        assert!(
            usage_events(&[AppEvent::turn_stream_snapshot_changed(snapshot)], &turns).is_empty()
        );
    }
}
