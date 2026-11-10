//! thread 하나의 턴 뒤 상태이다 (plan 19 Q2-1). 평가 대기, 평가 중, 경로 대기, 끝남을 enum 하나로 둔다.
//!
//! 이전에는 같은 단계를 두 곳이 따로 가졌다. thread session 이 평가 대기와 평가 중을, runtime 권한이 평가 중·경로
//! 대기·끝남의 투영과 경로 데이터를 가졌다. 두 곳을 함께 바꿔야 했고, "경로 대기인데 경로 데이터가 없다" 같은 조합이
//! 타입으로는 가능했다. 이제 상태는 이 enum 하나이고, 클라이언트가 보는 `PostTurnAuthoritySnapshot` 은 그 투영이다.

use std::time::Instant;

use super::{
    ConversationRuntimeSnapshot, PostTurnAuthoritySnapshot, PostTurnEvaluationCorrelation,
    PostTurnRouteResolution,
};
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
 * 호스트가 다음에 할 턴 뒤 일이다 (plan 19 Q2-2). core 는 모든 session 의 `PostTurnState` 를 보고 0~1개를 고른다.
 * 호스트는 이벤트를 따라가지 않고 매 dispatch 뒤에 이 값을 묻는다. 그래서 사건 하나를 놓쳐도 다음에 물을 때 같은
 * 일이 다시 보인다. 단계를 하면 상태가 바뀌므로 같은 단계가 두 번 나오지 않는다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostTurnHostStep {
    /// 평가 대기 턴이 있다. 호스트가 병렬 값을 붙여 평가를 시작한다.
    StartEvaluation {
        planning_workspace_directory: String,
    },
    /// 경로를 기다리는 평가가 하나 있다. 호스트가 병렬 제어부와 그 thread 의 자동 후속 설정으로 경로를 고른다.
    RouteContinuation(Box<PostTurnRouteWork>),
    /*
     * 경로를 기다리는 평가가 둘 이상이다. 어느 것이 이어 갈 차례인지 정할 수 없으므로 모두 한 번에 이어 가지 않고
     * 끝낸다 (실패하면 닫는다). 하나씩 끝내면 남은 하나가 다음 비교에서 정상 경로로 가 버린다.
     */
    SettleWithoutContinuation {
        correlations: Vec<PostTurnEvaluationCorrelation>,
    },
    /*
     * 원격 준비가 큐에 남긴 프롬프트가 있고 커널이 쉰다 (plan 19 Q2-3). 턴 뒤 일을 모두 끝낸 뒤에만 나온다. 그래서
     * 자동 후속 턴이 먼저 이어 간다. 호스트가 `ResubmitQueuedThreadPrompt` 로 core 에 다시 내게 한다.
     */
    ResubmitQueuedThreadPrompt,
}

/// 경로를 고를 평가와, 그 평가를 가진 thread 의 runtime 값이다. 그 thread 는 화면이 보지 않을 수 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnRouteWork {
    pub correlation: PostTurnEvaluationCorrelation,
    pub execution: Box<PostTurnExecution>,
    pub runtime: ConversationRuntimeSnapshot,
}

/*
 * 전이 (`may_follow` 가 같은 규칙을 검사한다):
 *   Idle | Due | Settled --턴 종료--> Due            (이전 Due 는 덮어쓴다)
 *   Due --평가 시작 또는 버림--> Idle
 *   Idle | Due | AwaitingRoute | Settled --평가 입장--> Evaluating
 *   Evaluating --평가 완료--> AwaitingRoute --경로 결정--> Settled
 *   Evaluating | AwaitingRoute --정책 변경--> Settled (이어 가지 않음)
 *   Evaluating | AwaitingRoute | Settled --새 턴, 대화 무효화, 평가 취소--> Idle (Due 는 남는다)
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

    /// 아직 끝나지 않은 턴 뒤 일이 있다 (평가 대기, 평가 중, 경로 대기). 대화를 바꿔도 이어서 처리한다.
    pub(super) fn has_unsettled_work(&self) -> bool {
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

    pub(super) fn evaluates(&self, correlation: &PostTurnEvaluationCorrelation) -> bool {
        self.active_evaluation()
            .is_some_and(|active| &active.correlation == correlation)
    }

    pub(super) fn due_evaluation(&self) -> Option<&PendingPostTurnEvaluation> {
        match self {
            Self::Due(pending) => Some(pending),
            _ => None,
        }
    }

    /// 경로를 기다리는 평가와 그 결과이다.
    pub(super) fn awaiting_route(
        &self,
    ) -> Option<(&PostTurnEvaluationCorrelation, &PostTurnExecution)> {
        match self {
            Self::AwaitingRoute {
                correlation,
                execution,
                ..
            } => Some((correlation, execution)),
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

    /// 이 상태가 `previous` 다음에 올 수 있다. 위 전이 규칙이다.
    pub(super) fn may_follow(&self, previous: &Self) -> bool {
        match self {
            Self::Idle => true,
            Self::Due(_) => previous.in_flight_correlation().is_none(),
            Self::Evaluating { .. } => !matches!(previous, Self::Evaluating { .. }),
            Self::AwaitingRoute { .. } => matches!(previous, Self::Evaluating { .. }),
            Self::Settled { .. } => previous.in_flight_correlation().is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::planning::{
        PlanningWorkerPanelState, PostTurnAutoFollowSkipReason, PostTurnContinuationAction,
        PostTurnOutcome, PostTurnProvenance, RuntimeProjection,
    };

    fn correlation() -> PostTurnEvaluationCorrelation {
        PostTurnEvaluationCorrelation::new(1, "thread-1", "turn-1", "/workspace", "/workspace")
    }

    fn execution() -> PostTurnExecution {
        PostTurnExecution {
            thread_id: "thread-1".to_string(),
            completed_turn_id: "turn-1".to_string(),
            runtime_projection_workspace_directory: "/workspace".to_string(),
            evaluation: PostTurnOutcome {
                provenance: PostTurnProvenance::new("turn-1".to_string()),
                runtime_projection: RuntimeProjection::uninitialized(),
                planning_repair_state: None,
                runtime_notices: Vec::new(),
                action: PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
                },
                operator_alerts: Vec::new(),
            },
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
        }
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
        assert!(state.has_unsettled_work());
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
        assert!(!state.has_unsettled_work());
        assert!(!state.awaits_route(&correlation()));
        assert_eq!(
            state.projection(),
            PostTurnAuthoritySnapshot::Settled {
                correlation: correlation(),
                resolution: PostTurnRouteResolution::NoContinuation,
            }
        );
    }

    #[test]
    fn only_an_in_flight_evaluation_can_settle_or_await_a_route() {
        let evaluating = PostTurnState::Evaluating {
            active: ActivePostTurnEvaluation::for_test(correlation()),
            started_at: Instant::now(),
        };
        let settled = PostTurnState::Settled {
            correlation: correlation(),
            resolution: PostTurnRouteResolution::NoContinuation,
        };
        let awaiting = PostTurnState::AwaitingRoute {
            correlation: correlation(),
            execution: Box::new(execution()),
            started_at: Instant::now(),
        };
        assert!(settled.may_follow(&evaluating));
        assert!(awaiting.may_follow(&evaluating));
        assert!(!settled.may_follow(&PostTurnState::Idle));
        assert!(!awaiting.may_follow(&PostTurnState::Idle));
        assert!(!PostTurnState::Due(Box::new(pending())).may_follow(&evaluating));
        assert!(!evaluating.may_follow(&evaluating));
        assert!(PostTurnState::Idle.may_follow(&awaiting));
    }
}
