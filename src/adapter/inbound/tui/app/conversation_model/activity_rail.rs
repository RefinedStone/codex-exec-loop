use crate::domain::turn_terminal::{
    ConversationTurnApplicationDelivery, ConversationTurnTerminalOutcome,
    ConversationTurnTerminalReceipt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivityRailTerminalState {
    RecoveryPending,
    Interrupted,
    Failed,
    Unknown,
    RuntimeFailed,
}

impl ActivityRailTerminalState {
    pub(crate) fn from_receipt(receipt: &ConversationTurnTerminalReceipt) -> Option<Self> {
        if receipt.application_delivery != ConversationTurnApplicationDelivery::Confirmed {
            return Some(Self::RecoveryPending);
        }

        match &receipt.outcome {
            ConversationTurnTerminalOutcome::Completed => None,
            ConversationTurnTerminalOutcome::Interrupted => Some(Self::Interrupted),
            ConversationTurnTerminalOutcome::Failed { .. } => Some(Self::Failed),
            ConversationTurnTerminalOutcome::Unknown { .. } => Some(Self::Unknown),
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::RecoveryPending => "recovery-pending",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::RuntimeFailed => "runtime-failed",
        }
    }

    pub(crate) const fn compact_label(self) -> &'static str {
        match self {
            Self::RecoveryPending => "recover",
            Self::Interrupted => "interrupt",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::RuntimeFailed => "runtime-fail",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDeliveryFailure, ConversationTurnError,
        ConversationTurnTerminalUncertainty,
    };

    const SECRET: &str = "ACTIVITY_RAIL_TERMINAL_SECRET";

    fn receipt(outcome: ConversationTurnTerminalOutcome) -> ConversationTurnTerminalReceipt {
        ConversationTurnTerminalReceipt::new("thread-1", "turn-1", outcome)
            .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed)
    }

    #[test]
    fn confirmed_receipt_maps_only_non_completed_terminal_outcomes() {
        assert_eq!(
            ActivityRailTerminalState::from_receipt(&receipt(
                ConversationTurnTerminalOutcome::Completed
            )),
            None
        );
        assert_eq!(
            ActivityRailTerminalState::from_receipt(&receipt(
                ConversationTurnTerminalOutcome::Interrupted
            )),
            Some(ActivityRailTerminalState::Interrupted)
        );
        assert_eq!(
            ActivityRailTerminalState::from_receipt(&receipt(
                ConversationTurnTerminalOutcome::Failed {
                    error: ConversationTurnError::new(SECRET, None::<&str>, None),
                }
            )),
            Some(ActivityRailTerminalState::Failed)
        );
        assert_eq!(
            ActivityRailTerminalState::from_receipt(&receipt(
                ConversationTurnTerminalOutcome::Unknown {
                    reason: ConversationTurnTerminalUncertainty::protocol_inconsistency(SECRET),
                    observed_error: None,
                }
            )),
            Some(ActivityRailTerminalState::Unknown)
        );
    }

    #[test]
    fn unconfirmed_application_delivery_overrides_upstream_outcome() {
        for receipt in [
            ConversationTurnTerminalReceipt::completed("thread-1", "turn-pending", Vec::new()),
            receipt(ConversationTurnTerminalOutcome::Failed {
                error: ConversationTurnError::new(SECRET, None::<&str>, None),
            })
            .with_application_delivery(
                ConversationTurnApplicationDelivery::Unconfirmed(
                    ConversationTurnApplicationDeliveryFailure::Disconnected,
                ),
            ),
        ] {
            assert_eq!(
                ActivityRailTerminalState::from_receipt(&receipt),
                Some(ActivityRailTerminalState::RecoveryPending)
            );
        }
    }

    #[test]
    fn labels_are_short_ascii_terminal_facts() {
        fn assert_marker_traits<T: Clone + Copy + Eq>() {}

        assert_marker_traits::<ActivityRailTerminalState>();
        let cases = [
            (
                ActivityRailTerminalState::RecoveryPending,
                "recovery-pending",
            ),
            (ActivityRailTerminalState::Interrupted, "interrupted"),
            (ActivityRailTerminalState::Failed, "failed"),
            (ActivityRailTerminalState::Unknown, "unknown"),
            (ActivityRailTerminalState::RuntimeFailed, "runtime-failed"),
        ];

        for (state, expected) in cases {
            assert_eq!(state.label(), expected);
            assert!(state.label().is_ascii());
            assert!(state.compact_label().is_ascii());
            assert!(state.compact_label().len() <= 12);
        }
    }

    #[test]
    fn mapped_state_debug_cannot_retain_terminal_error_detail() {
        let state = ActivityRailTerminalState::from_receipt(&receipt(
            ConversationTurnTerminalOutcome::Failed {
                error: ConversationTurnError::new(SECRET, Some(SECRET), None),
            },
        ))
        .expect("failed receipt should produce a terminal fact");

        assert_eq!(format!("{state:?}"), "Failed");
        assert!(!format!("{state:?}").contains(SECRET));
    }
}
