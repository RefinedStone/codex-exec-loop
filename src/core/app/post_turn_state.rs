//! thread 하나의 턴 뒤 상태이다 (plan 19 Q2-1). 평가 대기, 평가 중, 경로 대기, 끝남을 enum 하나로 둔다.
//!
//! 이전에는 같은 단계를 두 곳이 따로 가졌다. thread session 이 평가 대기와 평가 중을, runtime 권한이 평가 중·경로
//! 대기·끝남의 투영과 경로 데이터를 가졌다. 두 곳을 함께 바꿔야 했고, "경로 대기인데 경로 데이터가 없다" 같은 조합이
//! 타입으로는 가능했다. 이제 상태는 이 enum 하나이고, 클라이언트가 보는 `PostTurnAuthoritySnapshot` 은 그 투영이다.

use std::time::Instant;

use super::{PostTurnAuthoritySnapshot, PostTurnEvaluationCorrelation, PostTurnRouteResolution};
use crate::domain::planning::{PostTurnContinuationPermit, PostTurnExecution, TurnSnapshotCapture};

/// 턴이 끝났고 평가를 기다리는 턴이다. 호스트가 병렬 값을 붙여서 평가를 시작한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingPostTurnEvaluation {
    pub(super) thread_id: String,
    pub(super) completed_turn_id: String,
    pub(super) workspace_directory: String,
    pub(super) planning_workspace_directory: String,
    pub(super) changed_planning_file_paths: Vec<String>,
    pub(super) execution_snapshot_capture: Option<TurnSnapshotCapture>,
    pub(super) latest_user_message: Option<String>,
    pub(super) latest_agent_reply: Option<String>,
}

/// 평가 worker 가 돌고 있다. permit 은 정책이 바뀌면 worker 의 결과를 무효로 만든다.
#[derive(Debug, Clone)]
pub(super) struct ActivePostTurnEvaluation {
    pub(super) correlation: PostTurnEvaluationCorrelation,
    pub(super) continuation_permit: PostTurnContinuationPermit,
}

/*
 * 전이:
 *   Idle | Settled --턴 종료--> Due --평가 시작--> Evaluating --평가 완료--> AwaitingRoute --경로 결정--> Settled
 *   Evaluating | AwaitingRoute --정책 변경--> Settled (이어 가지 않음)
 *   Evaluating | AwaitingRoute | Settled --취소--> Idle (Due 는 남는다)
 */
#[derive(Debug, Clone, Default)]
pub(super) enum PostTurnState {
    #[default]
    Idle,
    Due(Box<PendingPostTurnEvaluation>),
    Evaluating {
        active: ActivePostTurnEvaluation,
        started_at: Instant,
    },
    AwaitingRoute {
        correlation: PostTurnEvaluationCorrelation,
        execution: Box<PostTurnExecution>,
        started_at: Instant,
    },
    Settled {
        correlation: PostTurnEvaluationCorrelation,
        resolution: PostTurnRouteResolution,
    },
}

impl ActivePostTurnEvaluation {
    /// 시험용이다. 새 gate 의 permit 으로 평가 중 상태를 만든다.
    #[cfg(test)]
    pub(super) fn for_test(correlation: PostTurnEvaluationCorrelation) -> Self {
        Self {
            correlation,
            continuation_permit: crate::domain::planning::PostTurnContinuationGate::default()
                .capture(),
        }
    }
}

impl PostTurnState {
    /// 클라이언트가 보는 투영이다. 평가 대기는 아직 평가가 아니므로 `Idle` 로 보인다.
    pub(super) fn projection(&self) -> PostTurnAuthoritySnapshot {
        match self {
            Self::Idle | Self::Due(_) => PostTurnAuthoritySnapshot::Idle,
            Self::Evaluating { active, started_at } => PostTurnAuthoritySnapshot::Evaluating {
                correlation: active.correlation.clone(),
                started_at: *started_at,
            },
            Self::AwaitingRoute {
                correlation,
                started_at,
                ..
            } => PostTurnAuthoritySnapshot::AwaitingRoute {
                correlation: correlation.clone(),
                started_at: *started_at,
            },
            Self::Settled {
                correlation,
                resolution,
            } => PostTurnAuthoritySnapshot::Settled {
                correlation: correlation.clone(),
                resolution: *resolution,
            },
        }
    }

    /// 대화를 바꿔도 이어서 처리해야 할 턴 뒤 일이 있다 (평가 대기, 평가 중, 경로 대기).
    pub(super) fn has_work(&self) -> bool {
        matches!(
            self,
            Self::Due(_) | Self::Evaluating { .. } | Self::AwaitingRoute { .. }
        )
    }

    pub(super) fn is_due(&self) -> bool {
        matches!(self, Self::Due(_))
    }

    pub(super) fn active_evaluation(&self) -> Option<&ActivePostTurnEvaluation> {
        match self {
            Self::Evaluating { active, .. } => Some(active),
            _ => None,
        }
    }

    pub(super) fn awaits_route(&self, correlation: &PostTurnEvaluationCorrelation) -> bool {
        matches!(self, Self::AwaitingRoute { correlation: awaiting, .. } if awaiting == correlation)
    }

    /// 평가 중이거나 경로를 기다리는 평가의 correlation 이다.
    pub(super) fn in_flight_correlation(&self) -> Option<&PostTurnEvaluationCorrelation> {
        match self {
            Self::Evaluating { active, .. } => Some(&active.correlation),
            Self::AwaitingRoute { correlation, .. } => Some(correlation),
            Self::Idle | Self::Due(_) | Self::Settled { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correlation() -> PostTurnEvaluationCorrelation {
        PostTurnEvaluationCorrelation::new(1, "thread-1", "turn-1", "/workspace", "/workspace")
    }

    fn pending() -> PendingPostTurnEvaluation {
        PendingPostTurnEvaluation {
            thread_id: "thread-1".to_string(),
            completed_turn_id: "turn-1".to_string(),
            workspace_directory: "/workspace".to_string(),
            planning_workspace_directory: "/workspace".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
            latest_user_message: None,
            latest_agent_reply: None,
        }
    }

    #[test]
    fn a_due_evaluation_is_work_but_projects_as_idle() {
        let state = PostTurnState::Due(Box::new(pending()));
        assert!(state.has_work());
        assert!(state.is_due());
        assert_eq!(state.projection(), PostTurnAuthoritySnapshot::Idle);
        assert!(state.in_flight_correlation().is_none());
    }

    #[test]
    fn a_settled_evaluation_is_no_longer_work() {
        let state = PostTurnState::Settled {
            correlation: correlation(),
            resolution: PostTurnRouteResolution::NoContinuation,
        };
        assert!(!state.has_work());
        assert!(!state.awaits_route(&correlation()));
        assert_eq!(
            state.projection(),
            PostTurnAuthoritySnapshot::Settled {
                correlation: correlation(),
                resolution: PostTurnRouteResolution::NoContinuation,
            }
        );
    }
}
