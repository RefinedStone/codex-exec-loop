use super::approval::{ApprovalReviewPersistenceCoordinator, ApprovalReviewPersistenceSettlement};
use super::conversation_runtime::{ConversationRuntimeAuthority, TurnAuthorityAdmission};
use super::{
    ApprovalDecisionAdmission, ApprovalDecisionCorrelation, ApprovalReviewPersistenceCorrelation,
    ConversationLoadCorrelation, ConversationRuntimeSnapshot, PostTurnEvaluationCorrelation,
    PostTurnRouteResolution, SessionRenameCorrelation, StopRequestAdmission, StopRequestAttempt,
    StopRequestCorrelation, ThreadActivitySummary, TurnSteerAdmission, TurnSteerCorrelation,
    TurnStreamEvent, TurnStreamSnapshot, TurnStreamStartRejection, TurnStreamState,
    TurnStreamUpdate, TurnSubmissionAdmission, TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequestIdentity,
    ConversationTurnSteerReceipt, ConversationTurnSteerRequest,
};
use crate::domain::conversation_item_lifecycle::ConversationItemLifecycleProjection;
use crate::domain::planning::{
    PostTurnContinuationGate, PostTurnContinuationPermit, PostTurnExecution, PostTurnRequest,
};

#[derive(Debug, Clone)]
struct DeferredConversationLoadIntent {
    thread_id: String,
    fallback_workspace_directory: String,
}

/*
 * 턴 종료 뒤 평가에 넘길 값이다 (plan 18 K1-4). 이전에는 TUI 가 화면 기록에서 꺼냈다. core 는 이 대화에서
 * 제출한 마지막 사용자 문장과 마지막 응답만 기억한다. 전체 기록은 갖지 않는다.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PostTurnMessages {
    latest_user_message: Option<String>,
    latest_agent_reply: Option<String>,
}

/// 턴이 끝났고 평가를 기다리는 턴이다. 호스트가 병렬 값을 붙여서 평가를 시작한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingPostTurnEvaluation {
    pub(super) thread_id: String,
    pub(super) completed_turn_id: String,
    pub(super) workspace_directory: String,
    pub(super) planning_workspace_directory: String,
    pub(super) changed_planning_file_paths: Vec<String>,
    pub(super) execution_snapshot_capture: Option<crate::domain::planning::TurnSnapshotCapture>,
    pub(super) latest_user_message: Option<String>,
    pub(super) latest_agent_reply: Option<String>,
}

#[derive(Debug, Clone)]
struct ActiveTurnSteer {
    correlation: TurnSteerCorrelation,
    expected_turn_id: String,
}

#[derive(Debug, Clone, Copy)]
struct ActiveStopRequest {
    correlation: StopRequestCorrelation,
    pending_attempt: Option<StopRequestAttempt>,
    synchronize_after_turn_started: bool,
    invalidated: bool,
}

#[derive(Debug, Clone)]
struct ActivePostTurnEvaluation {
    correlation: PostTurnEvaluationCorrelation,
    continuation_permit: PostTurnContinuationPermit,
}

#[derive(Debug)]
pub(super) struct LoadedConversationStreamIdentity {
    pub(super) thread_id: String,
    pub(super) title: String,
    pub(super) workspace_directory: String,
    pub(super) item_lifecycle: ConversationItemLifecycleProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopEffectIntent {
    Request {
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
    },
    Invalidate {
        correlation: StopRequestCorrelation,
    },
}

#[derive(Debug)]
pub(super) enum ConversationLoadAdmission {
    Deferred {
        stop_effects: Vec<StopEffectIntent>,
    },
    Started {
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
        stop_effects: Vec<StopEffectIntent>,
    },
}

#[derive(Debug)]
pub(super) struct ConversationLifecycleReduction {
    pub(super) stop_effects: Vec<StopEffectIntent>,
}

#[derive(Debug)]
pub(super) struct TurnStreamReduction {
    // 이 스트림 이벤트로 턴이 끝나서 평가를 기다린다. 값은 그 턴의 planning 작업 공간이다.
    pub(super) post_turn_evaluation_due_workspace: Option<String>,
    pub(super) snapshots: Vec<TurnStreamSnapshot>,
    pub(super) stop_effects: Vec<StopEffectIntent>,
    pub(super) approval_review_persistence: Option<ApprovalReviewPersistenceCorrelation>,
}

#[derive(Debug)]
pub(super) struct StopRequestCompletionReduction {
    pub(super) publish_completion: bool,
    pub(super) settlement_finished: bool,
    pub(super) stop_effects: Vec<StopEffectIntent>,
}

/*
 * thread 하나의 턴 상태이다 (plan 18 K2-2). 진행 중 턴, 스트림, 정지·steer, 승인 검토 저장, 턴 종료 후
 * 평가가 여기에 모인다. 세대 번호와 대화 로드 절차는 모든 thread 에 걸쳐야 하므로 reducer 에 남는다.
 */
#[derive(Clone)]
struct ThreadSession {
    turn_stream_state: TurnStreamState,
    conversation_runtime: ConversationRuntimeAuthority,
    guarded_session_rename_stream: Option<(TurnSubmissionCorrelation, SessionRenameCorrelation)>,
    post_turn_continuation_gate: PostTurnContinuationGate,
    in_flight_post_turn_evaluation: Option<ActivePostTurnEvaluation>,
    active_stop_request: Option<ActiveStopRequest>,
    active_turn_steer: Option<ActiveTurnSteer>,
    post_turn_messages: PostTurnMessages,
    pending_post_turn_evaluation: Option<PendingPostTurnEvaluation>,
    // 자동 후속 턴에 쓸 턴 옵션이다 (plan 18 K1-5). 대화 로드, 제출, 클라이언트 변경으로 갱신한다.
    turn_options: crate::domain::conversation::ConversationTurnOptions,
}

impl ThreadSession {
    fn new() -> Self {
        Self {
            turn_stream_state: TurnStreamState::new(),
            conversation_runtime: ConversationRuntimeAuthority::new(),
            guarded_session_rename_stream: None,
            post_turn_continuation_gate: PostTurnContinuationGate::default(),
            in_flight_post_turn_evaluation: None,
            active_stop_request: None,
            active_turn_steer: None,
            post_turn_messages: PostTurnMessages::default(),
            pending_post_turn_evaluation: None,
            turn_options: Default::default(),
        }
    }

    /// 대화를 다른 것으로 바꿔도 이어서 처리해야 할 일이 있다 (턴, 자동 후속, 평가, 정지, steer).
    fn is_busy(&self) -> bool {
        !self
            .conversation_runtime
            .snapshot()
            .can_accept_manual_prompt()
            || self.pending_post_turn_evaluation.is_some()
            || self.in_flight_post_turn_evaluation.is_some()
            || self.active_stop_request.is_some()
            || self.active_turn_steer.is_some()
    }

    /// 보는 자리에 둘 새 session 이다. 자동 후속 설정과 턴 옵션만 이어받는다. 평가 취소 gate 는 worker 와
    /// 공유되므로 새로 만든다.
    fn detached_view(&self) -> Self {
        let mut view = Self::new();
        view.conversation_runtime = self.conversation_runtime.clone();
        view.conversation_runtime.invalidate_conversation();
        view.turn_options = self.turn_options.clone();
        view
    }

    fn owns_turn(&self, correlation: TurnSubmissionCorrelation) -> bool {
        self.conversation_runtime.active_turn_correlation() == Some(correlation)
    }

    fn awaits_post_turn_route(&self, correlation: &PostTurnEvaluationCorrelation) -> bool {
        matches!(
            &self.conversation_runtime.snapshot().post_turn,
            super::PostTurnAuthoritySnapshot::AwaitingRoute { correlation: awaiting, .. }
                if awaiting == correlation
        )
    }

    fn activity_summary(&self) -> Option<ThreadActivitySummary> {
        let thread_id = self.turn_stream_state.thread_id()?.to_string();
        let runtime = self.conversation_runtime.snapshot();
        Some(ThreadActivitySummary {
            thread_id,
            running: runtime.active_turn.is_some(),
            active_turn_id: runtime
                .active_turn
                .as_ref()
                .and_then(|turn| turn.turn_id.clone()),
            approval: runtime
                .approval
                .as_ref()
                .map(|approval| approval.request.clone()),
            auto_follow_active: runtime.auto_follow.has_live_activity()
                || runtime.post_turn.is_in_flight()
                || self.pending_post_turn_evaluation.is_some(),
        })
    }

    fn streams_thread(&self, thread_id: &str) -> bool {
        self.turn_stream_state.matches_thread(thread_id)
    }
}

#[derive(Clone)]
pub(super) struct ConversationTurnFeatureReducer {
    // 클라이언트가 보는 thread 의 session 이다.
    session: ThreadSession,
    /*
     * 클라이언트가 다른 대화를 열 때 일이 남아 있던 session 이다 (plan 18 K2-3). correlation 으로 주인을 찾아
     * 끝까지 처리하고, 일이 끝나면 지운다. 보는 thread 를 다시 열면 보는 자리로 돌아온다.
     */
    background_sessions: Vec<ThreadSession>,
    // 뒤로 보낸 session 을 보는 자리에 잠시 넣고 처리하는 중이다. 화면용 스트림 투영은 내지 않는다.
    serving_background: bool,
    // 승인 검토 저장은 모든 thread 를 통틀어 한 줄로 차례대로 한다. 그래서 session 밖에 둔다.
    approval_review_persistence: ApprovalReviewPersistenceCoordinator,
    // 클라이언트에 마지막으로 알린 뒤의 thread 요약이다 (plan 18 K2-4).
    published_thread_activity: Vec<ThreadActivitySummary>,
    next_conversation_load_generation: u64,
    in_flight_conversation_load: Option<ConversationLoadCorrelation>,
    deferred_conversation_load: Option<DeferredConversationLoadIntent>,
    next_turn_submission_generation: u64,
    next_post_turn_evaluation_generation: u64,
    next_stop_request_generation: u64,
    next_turn_steer_generation: u64,
    next_approval_decision_generation: u64,
}

impl std::fmt::Debug for ConversationTurnFeatureReducer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConversationTurnFeatureReducer")
            .field("conversation_runtime", &self.session.conversation_runtime)
            .field(
                "in_flight_conversation_load",
                &self.in_flight_conversation_load,
            )
            .field(
                "in_flight_post_turn_evaluation",
                &self.session.in_flight_post_turn_evaluation,
            )
            .field("active_stop_request", &self.session.active_stop_request)
            .field("active_turn_steer", &self.session.active_turn_steer)
            .field("background_sessions", &self.background_sessions.len())
            .finish_non_exhaustive()
    }
}

impl ConversationTurnFeatureReducer {
    /*
     * `owns` 가 맞는 session 에서 `reduce` 를 실행한다 (plan 18 K2-3). 보는 session 을 먼저 본다. 뒤로 보낸
     * session 이면 보는 자리와 잠시 바꿔서 같은 규칙으로 처리한다. 그동안 나머지 session 은 모두
     * `background_sessions` 에 있다. 일이 끝난 session 은 지운다.
     */
    fn with_owning_session<R>(
        &mut self,
        owns: impl Fn(&ThreadSession) -> bool,
        reduce: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        if owns(&self.session) {
            return Some(reduce(self));
        }
        let index = self.background_sessions.iter().position(owns)?;
        let result = self.reduce_background_session(index, reduce);
        if !self.background_sessions[index].is_busy() {
            self.background_sessions.remove(index);
        }
        Some(result)
    }

    fn for_each_session(&mut self, mut reduce: impl FnMut(&mut Self)) {
        reduce(self);
        for index in 0..self.background_sessions.len() {
            self.reduce_background_session(index, &mut reduce);
        }
        self.background_sessions.retain(ThreadSession::is_busy);
    }

    /// 뒤의 session 을 보는 자리와 잠시 바꿔서 `reduce` 를 실행하고 되돌린다.
    fn reduce_background_session<R>(
        &mut self,
        index: usize,
        reduce: impl FnOnce(&mut Self) -> R,
    ) -> R {
        std::mem::swap(&mut self.session, &mut self.background_sessions[index]);
        self.serving_background = true;
        let result = reduce(self);
        self.serving_background = false;
        std::mem::swap(&mut self.session, &mut self.background_sessions[index]);
        result
    }

    /// 이 평가가 클라이언트가 보지 않는 thread 의 것이다. 그 결과는 클라이언트에 알리지 않는다.
    pub(super) fn post_turn_evaluation_is_background(
        &self,
        correlation: &PostTurnEvaluationCorrelation,
    ) -> bool {
        let owns = |session: &ThreadSession| {
            session
                .in_flight_post_turn_evaluation
                .as_ref()
                .is_some_and(|active| &active.correlation == correlation)
                || session.awaits_post_turn_route(correlation)
        };
        !owns(&self.session) && self.background_sessions.iter().any(owns)
    }

    /// 마지막으로 알린 뒤 일이 남은 thread 요약이 바뀌었으면 새 요약을 돌려준다 (plan 18 K2-4, K3-3a).
    /// 보는 thread 도 일이 남아 있으면 담는다. 원격 클라이언트는 보는 자리를 모르기 때문이다.
    pub(super) fn take_thread_activity_change(&mut self) -> Option<Vec<ThreadActivitySummary>> {
        let activity = std::iter::once(&self.session)
            .filter(|session| session.is_busy())
            .chain(&self.background_sessions)
            .filter_map(ThreadSession::activity_summary)
            .collect::<Vec<_>>();
        if activity == self.published_thread_activity {
            return None;
        }
        self.published_thread_activity = activity.clone();
        Some(activity)
    }

    /// 이 thread 는 클라이언트가 보지 않는 중이다.
    pub(super) fn thread_is_background(&self, thread_id: &str) -> bool {
        !self.session.streams_thread(thread_id)
            && self
                .background_sessions
                .iter()
                .any(|session| session.streams_thread(thread_id))
    }

    /// 보는 session 에 일이 남아 있으면 뒤로 보내고, 보는 자리에는 새 session 을 둔다.
    fn send_busy_viewed_session_to_background(&mut self) {
        if !self.session.is_busy() {
            return;
        }
        let view = self.session.detached_view();
        let busy = std::mem::replace(&mut self.session, view);
        self.background_sessions.push(busy);
    }

    fn other_session_active_turn(&self) -> Option<TurnSubmissionCorrelation> {
        self.background_sessions
            .iter()
            .find_map(|session| session.conversation_runtime.active_turn_correlation())
    }

    /// 경로 결정을 기다리는 session 의 runtime 값이다. 호스트가 그 thread 의 자동 후속 설정으로 경로를 고른다.
    pub(super) fn post_turn_route_runtime(
        &self,
        correlation: &PostTurnEvaluationCorrelation,
    ) -> Option<ConversationRuntimeSnapshot> {
        std::iter::once(&self.session)
            .chain(&self.background_sessions)
            .find(|session| session.awaits_post_turn_route(correlation))
            .map(|session| session.conversation_runtime.snapshot())
    }

    /// 자동 후속 턴을 만들 때 쓰는 그 thread 의 runtime 값과 턴 옵션이다.
    pub(super) fn turn_context_for_thread(
        &self,
        thread_id: &str,
    ) -> (
        ConversationRuntimeSnapshot,
        crate::domain::conversation::ConversationTurnOptions,
    ) {
        let session = if self.session.streams_thread(thread_id) {
            &self.session
        } else {
            self.background_sessions
                .iter()
                .find(|session| session.streams_thread(thread_id))
                .unwrap_or(&self.session)
        };
        (
            session.conversation_runtime.snapshot(),
            session.turn_options.clone(),
        )
    }

    /// 일이 남은 session 이 있다 (턴, 승인, 자동 후속, 평가, 정지, steer). 보는 session 과 뒤로 보낸 session 을 함께 본다.
    pub(super) fn has_busy_session(&self) -> bool {
        self.session.is_busy() || self.background_sessions.iter().any(ThreadSession::is_busy)
    }

    #[cfg(test)]
    pub(super) fn background_session_count(&self) -> usize {
        self.background_sessions.len()
    }

    pub(super) fn new() -> Self {
        Self {
            session: ThreadSession::new(),
            background_sessions: Vec::new(),
            serving_background: false,
            approval_review_persistence: ApprovalReviewPersistenceCoordinator::new(),
            published_thread_activity: Vec::new(),
            next_conversation_load_generation: 1,
            in_flight_conversation_load: None,
            deferred_conversation_load: None,
            next_turn_submission_generation: 1,
            next_post_turn_evaluation_generation: 1,
            next_stop_request_generation: 1,
            next_turn_steer_generation: 1,
            next_approval_decision_generation: 1,
        }
    }

    pub(super) fn set_turn_options(
        &mut self,
        turn_options: crate::domain::conversation::ConversationTurnOptions,
    ) {
        self.session.turn_options = turn_options;
    }

    /// 호스트가 평가를 시작할 때 가져간다. 한 번만 가져갈 수 있다.
    /*
     * 평가를 기다리는 턴을 꺼내서 바로 평가를 시작한다 (plan 18 K2-3). 꺼내기와 시작을 한 session 안에서
     * 함께 한다. 따로 하면 뒤에서 도는 session 이 그 사이에 "할 일 없음"으로 지워진다.
     */
    pub(super) fn start_due_post_turn_evaluation(
        &mut self,
        build_request: impl FnOnce(PendingPostTurnEvaluation) -> PostTurnRequest,
    ) -> Option<(PostTurnEvaluationCorrelation, PostTurnRequest)> {
        self.with_owning_session(
            |session| session.pending_post_turn_evaluation.is_some(),
            |reducer| {
                let pending = reducer.session.pending_post_turn_evaluation.take()?;
                let mut request = build_request(pending);
                let correlation = reducer.admit_post_turn_evaluation_in_session(&mut request)?;
                Some((correlation, request))
            },
        )
        .flatten()
    }

    pub(super) fn runtime_snapshot(&self) -> ConversationRuntimeSnapshot {
        self.session.conversation_runtime.snapshot()
    }

    pub(super) fn active_conversation_load_for_thread(
        &self,
        thread_id: &str,
    ) -> Option<ConversationLoadCorrelation> {
        self.in_flight_conversation_load
            .as_ref()
            .filter(|load| load.requested_thread_id == thread_id)
            .cloned()
    }

    pub(super) fn admit_conversation_load(
        &mut self,
        thread_id: String,
        fallback_workspace_directory: String,
        blocked_by_session_rename: bool,
    ) -> ConversationLoadAdmission {
        /*
         * 대화를 여는 일은 보는 thread 만 바꾼다 (plan 18 K2-3, KP-03). 일이 남은 session 은 뒤에서 계속 돈다.
         * 이름 바꾸기 때문에 로드가 미뤄져도 같다. 정지가 끝나기를 기다리는 session 만 보는 자리에 남는다.
         */
        if !self.stop_request_settlement_pending() {
            self.send_busy_viewed_session_to_background();
        }
        self.invalidate_post_turn_continuation();
        let mut stop_effects = Vec::new();
        self.invalidate_stop_request_for_lifecycle(&mut stop_effects);
        if self.stop_request_settlement_pending() || blocked_by_session_rename {
            self.deferred_conversation_load = Some(DeferredConversationLoadIntent {
                thread_id,
                fallback_workspace_directory,
            });
            return ConversationLoadAdmission::Deferred { stop_effects };
        }

        self.deferred_conversation_load = None;
        self.session.active_stop_request = None;
        self.session.active_turn_steer = None;
        self.session.conversation_runtime.invalidate_conversation();
        self.approval_review_persistence.invalidate_conversation();
        self.session.guarded_session_rename_stream = None;
        let correlation = ConversationLoadCorrelation::new(
            take_generation(
                &mut self.next_conversation_load_generation,
                "conversation load",
            ),
            thread_id,
        );
        self.in_flight_conversation_load = Some(correlation.clone());
        self.session.turn_stream_state = TurnStreamState::new();
        self.prune_post_turn_evaluation_for_lifecycle();
        ConversationLoadAdmission::Started {
            correlation,
            fallback_workspace_directory,
            stop_effects,
        }
    }

    pub(super) fn take_deferred_conversation_load(&mut self) -> Option<(String, String)> {
        self.deferred_conversation_load
            .take()
            .map(|intent| (intent.thread_id, intent.fallback_workspace_directory))
    }

    pub(super) fn reduce_conversation_invalidation(&mut self) -> ConversationLifecycleReduction {
        if !self.stop_request_settlement_pending() {
            self.send_busy_viewed_session_to_background();
        }
        self.invalidate_post_turn_continuation();
        self.deferred_conversation_load = None;
        self.in_flight_conversation_load = None;
        let mut stop_effects = Vec::new();
        self.invalidate_stop_request_for_lifecycle(&mut stop_effects);
        self.session.active_turn_steer = None;
        self.session.conversation_runtime.invalidate_conversation();
        self.approval_review_persistence.invalidate_conversation();
        self.session.guarded_session_rename_stream = None;
        self.session.turn_stream_state = TurnStreamState::new();
        ConversationLifecycleReduction { stop_effects }
    }

    pub(super) fn complete_conversation_load(
        &mut self,
        correlation: &ConversationLoadCorrelation,
        loaded_identity: Option<LoadedConversationStreamIdentity>,
    ) -> Option<ConversationLifecycleReduction> {
        if self.in_flight_conversation_load.as_ref() != Some(correlation) {
            return None;
        }
        self.invalidate_post_turn_continuation();
        self.in_flight_conversation_load = None;
        let mut stop_effects = Vec::new();
        self.invalidate_stop_request_for_lifecycle(&mut stop_effects);
        self.session.active_turn_steer = None;
        self.session.conversation_runtime.invalidate_conversation();
        self.approval_review_persistence.invalidate_conversation();
        self.session.guarded_session_rename_stream = None;
        self.session.turn_stream_state = TurnStreamState::new();
        // 다른 대화를 열면 이전 대화의 메시지와 평가 대기 턴은 버린다.
        self.session.post_turn_messages = PostTurnMessages::default();
        self.session.pending_post_turn_evaluation = None;
        let resumed_index = loaded_identity.as_ref().and_then(|identity| {
            self.background_sessions
                .iter()
                .position(|session| session.streams_thread(&identity.thread_id))
        });
        if let Some(index) = resumed_index {
            // 뒤에서 돌던 thread 를 다시 열었다. 진행 상태를 그대로 보는 자리로 가져온다.
            self.session = self.background_sessions.remove(index);
            if let Some(turn) = self.session.conversation_runtime.active_turn_correlation() {
                // 승인 검토 저장 실패를 다시 이 대화에 알릴 수 있게 한다.
                self.approval_review_persistence
                    .begin_conversation_turn(turn);
            }
        } else if let Some(identity) = loaded_identity {
            self.session
                .turn_stream_state
                .seed_loaded_thread_projection(
                    identity.thread_id,
                    identity.title,
                    identity.workspace_directory,
                    identity.item_lifecycle,
                );
        }
        self.prune_post_turn_evaluation_for_lifecycle();
        Some(ConversationLifecycleReduction { stop_effects })
    }

    pub(super) fn admit_turn_submission(
        &mut self,
        request: &TurnSubmissionRequest,
    ) -> TurnSubmissionAdmission {
        // 자동 후속 턴은 원래 thread 의 session 에 넣는다. 사람이 낸 턴은 보는 session 에 넣는다.
        let auto_follow_thread = request
            .auto_follow_source
            .as_ref()
            .filter(|_| request.prompt_origin == super::CorePromptOrigin::AutoFollow)
            .map(|source| source.thread_id.clone());
        match auto_follow_thread {
            Some(thread_id) => self
                .with_owning_session(
                    |session| session.streams_thread(&thread_id),
                    |reducer| reducer.admit_turn_submission_in_session(request),
                )
                .unwrap_or_else(|| self.admit_turn_submission_in_session(request)),
            None => self.admit_turn_submission_in_session(request),
        }
    }

    /// 이 thread 의 session 이 있다. 보는 thread 이거나 일이 남은 thread 이다 (plan 18 K3-3b).
    pub(super) fn has_session_for_thread(&self, thread_id: &str) -> bool {
        std::iter::once(&self.session)
            .chain(&self.background_sessions)
            .any(|session| session.streams_thread(thread_id))
    }

    /*
     * 이 thread 는 지금 새 프롬프트를 받을 수 없다 (plan 18 K3-3b). 그 thread 에 일이 남아 있거나, 다른 thread 에서
     * 턴이 돈다. 턴은 모든 thread 를 통틀어 하나만 돌기 때문이다. 준비는 planning 작업을 만들므로 그 전에 거절한다.
     */
    pub(super) fn thread_rejects_new_prompt(&self, thread_id: &str) -> bool {
        std::iter::once(&self.session)
            .chain(&self.background_sessions)
            .any(|session| {
                session
                    .conversation_runtime
                    .active_turn_correlation()
                    .is_some()
                    || (session.streams_thread(thread_id) && session.is_busy())
            })
    }

    /*
     * thread 를 지정한 턴을 그 thread 의 session 에 넣는다 (plan 18 K3-3b). 그 thread 의 session 이 없으면 보는
     * session 의 설정으로 뒤에 새로 만든다. 보는 자리는 바꾸지 않는다. 거절되면 새 session 은 일이 없어서 지워진다.
     */
    pub(super) fn admit_thread_turn_submission(
        &mut self,
        thread_id: &str,
        request: &TurnSubmissionRequest,
    ) -> TurnSubmissionAdmission {
        if !self.has_session_for_thread(thread_id) {
            let mut session = self.session.detached_view();
            session.turn_stream_state.seed_loaded_thread_projection(
                thread_id,
                "",
                request.workspace_directory.clone(),
                ConversationItemLifecycleProjection::default(),
            );
            self.background_sessions.push(session);
        }
        self.with_owning_session(
            |session| session.streams_thread(thread_id),
            |reducer| reducer.admit_turn_submission_in_session(request),
        )
        .unwrap_or(TurnSubmissionAdmission::RejectedUnavailable)
    }

    fn admit_turn_submission_in_session(
        &mut self,
        request: &TurnSubmissionRequest,
    ) -> TurnSubmissionAdmission {
        // 대화 턴은 모든 thread 를 통틀어 하나만 실행한다 (plan 18 K2-3).
        if let Some(active_correlation) = self
            .session
            .conversation_runtime
            .active_turn_correlation()
            .or_else(|| self.other_session_active_turn())
        {
            if request.prompt_origin == super::CorePromptOrigin::AutoFollow {
                self.session
                    .conversation_runtime
                    .cancel_queued_auto_follow_submission();
            }
            return TurnSubmissionAdmission::RejectedActive { active_correlation };
        }
        if let Some(active_stop) = self
            .session
            .active_stop_request
            .filter(|active| active.pending_attempt.is_some())
        {
            if request.prompt_origin == super::CorePromptOrigin::AutoFollow {
                self.session
                    .conversation_runtime
                    .cancel_queued_auto_follow_submission();
            }
            return TurnSubmissionAdmission::RejectedStopPending {
                stop_correlation: active_stop.correlation,
            };
        }
        if matches!(
            request.prompt_origin,
            super::CorePromptOrigin::Manual | super::CorePromptOrigin::ManualIntake
        ) && self
            .session
            .turn_stream_state
            .has_unapplied_confirmed_terminal()
        {
            return TurnSubmissionAdmission::RejectedUnavailable;
        }
        match self
            .session
            .conversation_runtime
            .classify_turn_admission(request)
        {
            TurnAuthorityAdmission::Accepted => {}
            TurnAuthorityAdmission::RejectedPreservingLease => {
                return TurnSubmissionAdmission::RejectedUnavailable;
            }
            TurnAuthorityAdmission::RejectedMalformedCurrentAutoFollowTarget => {
                self.session
                    .conversation_runtime
                    .cancel_queued_auto_follow_submission();
                return TurnSubmissionAdmission::RejectedUnavailable;
            }
        }
        let correlation = TurnSubmissionCorrelation::new(take_generation(
            &mut self.next_turn_submission_generation,
            "turn submission",
        ));
        self.session.guarded_session_rename_stream = None;
        if !request.transcript_text.trim().is_empty() {
            self.session.post_turn_messages.latest_user_message =
                Some(request.transcript_text.clone());
        }
        self.session.turn_options = request.turn_options.clone();
        self.session
            .conversation_runtime
            .begin_turn(correlation, request);
        self.approval_review_persistence
            .begin_conversation_turn(correlation);
        self.session.turn_stream_state.begin_submission();
        self.prune_post_turn_evaluation_for_lifecycle();
        TurnSubmissionAdmission::Accepted { correlation }
    }

    pub(super) fn reduce_session_rename_projection(
        &mut self,
        correlation: &SessionRenameCorrelation,
    ) -> Option<TurnStreamSnapshot> {
        if self
            .session
            .turn_stream_state
            .matches_thread(&correlation.request.thread_id)
            && let Some(turn_correlation) =
                self.session.conversation_runtime.active_turn_correlation()
        {
            self.session.guarded_session_rename_stream =
                Some((turn_correlation, correlation.clone()));
        }
        self.session
            .turn_stream_state
            .apply_session_rename(&correlation.request.thread_id, &correlation.request.name)
    }

    pub(super) fn apply_correlated_turn_stream_event(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        event: TurnStreamEvent,
    ) -> Option<TurnStreamReduction> {
        self.with_owning_session(
            |session| session.owns_turn(correlation),
            |reducer| reducer.apply_turn_stream_event_in_session(correlation, event),
        )
    }

    fn apply_turn_stream_event_in_session(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        mut event: TurnStreamEvent,
    ) -> TurnStreamReduction {
        if let TurnStreamEvent::ThreadPrepared {
            thread_id, title, ..
        } = &mut event
            && let Some(guarded_rename) = self
                .session
                .guarded_session_rename_stream
                .as_ref()
                .filter(|(turn, _)| *turn == correlation)
                .map(|(_, rename)| rename.clone())
        {
            if thread_id == &guarded_rename.request.thread_id {
                *title = guarded_rename.request.name;
            }
            self.session.guarded_session_rename_stream = None;
        }
        if let TurnStreamEvent::AgentMessageCompleted { text, .. } = &event
            && !text.trim().is_empty()
        {
            // 비어 있지 않은 마지막 응답을 쓴다. 병렬 슬롯 루프와 TUI 의 규칙과 같다.
            self.session.post_turn_messages.latest_agent_reply = Some(text.clone());
        }
        // 턴 종료 처리 전의 작업 공간이다. TUI 는 같은 순서로 골랐다 (활성 턴, 스트림, 실행 캡처).
        let active_turn_workspace = self
            .session
            .conversation_runtime
            .snapshot()
            .active_turn
            .as_ref()
            .map(|turn| turn.workspace_directory.clone())
            .filter(|workspace| !workspace.trim().is_empty());
        let stream_snapshot = self.session.turn_stream_state.apply_stream_event(event);
        let post_turn_evaluation_due_workspace = match &stream_snapshot.update {
            TurnStreamUpdate::TurnCompleted {
                turn_id,
                changed_planning_file_paths,
                execution_snapshot_capture,
                ..
            } => {
                let stream_workspace = stream_snapshot
                    .cwd
                    .clone()
                    .filter(|workspace| !workspace.trim().is_empty());
                let planning_workspace_directory = stream_workspace.clone().unwrap_or_default();
                let workspace_directory = active_turn_workspace
                    .or(stream_workspace)
                    .or_else(|| {
                        execution_snapshot_capture
                            .as_ref()
                            .map(|capture| capture.workspace_directory.clone())
                    })
                    .unwrap_or_else(|| planning_workspace_directory.clone());
                self.session.pending_post_turn_evaluation = Some(PendingPostTurnEvaluation {
                    thread_id: stream_snapshot.thread_id.clone().unwrap_or_default(),
                    completed_turn_id: turn_id.clone(),
                    workspace_directory,
                    planning_workspace_directory: planning_workspace_directory.clone(),
                    changed_planning_file_paths: changed_planning_file_paths.clone(),
                    execution_snapshot_capture: execution_snapshot_capture.clone(),
                    latest_user_message: self
                        .session
                        .post_turn_messages
                        .latest_user_message
                        .clone(),
                    latest_agent_reply: self.session.post_turn_messages.latest_agent_reply.clone(),
                });
                Some(planning_workspace_directory)
            }
            _ => None,
        };
        match &stream_snapshot.update {
            TurnStreamUpdate::TurnStarted { turn_id, .. } => {
                self.session
                    .conversation_runtime
                    .mark_turn_started(correlation, turn_id.clone());
            }
            TurnStreamUpdate::ApprovalRequested { request } => {
                self.session
                    .conversation_runtime
                    .set_pending_approval(request.clone());
            }
            TurnStreamUpdate::ApprovalReviewUpdated { review } => {
                self.session
                    .conversation_runtime
                    .set_approval_review(review.clone());
            }
            TurnStreamUpdate::ApprovalResolved {
                request_identity, ..
            } => {
                self.session
                    .conversation_runtime
                    .clear_pending_approval(request_identity);
            }
            TurnStreamUpdate::AttachmentObserved { .. }
            | TurnStreamUpdate::SessionRenamed { .. }
            | TurnStreamUpdate::ThreadPrepared { .. }
            | TurnStreamUpdate::TurnStartedIgnored { .. }
            | TurnStreamUpdate::RuntimeEnvelopeObserved { .. }
            | TurnStreamUpdate::ItemLifecycleObserved { .. }
            | TurnStreamUpdate::ProgressiveActivityObserved { .. }
            | TurnStreamUpdate::StatusUpdated { .. }
            | TurnStreamUpdate::AgentMessageCompleted { .. }
            | TurnStreamUpdate::ToolActivity { .. }
            | TurnStreamUpdate::ApprovalResolutionIgnored { .. }
            | TurnStreamUpdate::TurnInterruptRequestFailed { .. }
            | TurnStreamUpdate::TurnRetrying { .. }
            | TurnStreamUpdate::TurnCompleted { .. }
            | TurnStreamUpdate::TurnTerminal { .. }
            | TurnStreamUpdate::TurnTerminalIgnored { .. }
            | TurnStreamUpdate::Failed { .. }
            | TurnStreamUpdate::RuntimeFailureIgnored { .. }
            | TurnStreamUpdate::RuntimeNotice { .. } => {}
        }
        let closes_submission = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnCompleted { .. }
                | TurnStreamUpdate::TurnTerminal { .. }
                | TurnStreamUpdate::Failed { .. }
        );
        let rejected_terminal = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnTerminalIgnored { .. }
        );
        let rejected_start = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnStartedIgnored {
                rejection: TurnStreamStartRejection::TurnMismatch { .. },
                ..
            }
        );
        let turn_started = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnStarted { .. }
        );
        let retry_reopens_stop = matches!(
            &stream_snapshot.update,
            TurnStreamUpdate::TurnRetrying {
                correlation_failure: None,
                ..
            }
        );
        let approval_review_persistence = match &stream_snapshot.update {
            TurnStreamUpdate::ApprovalReviewUpdated { review } => stream_snapshot
                .cwd
                .clone()
                .zip(stream_snapshot.thread_id.clone())
                .and_then(|(workspace_directory, thread_id)| {
                    self.approval_review_persistence.enqueue(
                        correlation,
                        workspace_directory,
                        thread_id,
                        review.clone(),
                    )
                }),
            _ => None,
        };
        let mut snapshots = vec![stream_snapshot];
        let mut stop_effects = Vec::new();
        if rejected_start {
            snapshots.push(
                self.session
                    .turn_stream_state
                    .apply_stream_event(TurnStreamEvent::Failed {
                        message: "active turn returned a start event with mismatched turn identity"
                            .to_string(),
                    }),
            );
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
            self.session.conversation_runtime.finish_turn(correlation);
            self.session.guarded_session_rename_stream = None;
        } else if rejected_terminal {
            snapshots.push(
                self.session
                    .turn_stream_state
                    .apply_stream_event(TurnStreamEvent::Failed {
                        message: "active turn returned a terminal receipt with mismatched identity"
                            .to_string(),
                    }),
            );
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
            self.session.conversation_runtime.finish_turn(correlation);
            self.session.guarded_session_rename_stream = None;
        } else if closes_submission {
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
            self.session.conversation_runtime.finish_turn(correlation);
            self.session.guarded_session_rename_stream = None;
        } else if retry_reopens_stop {
            self.clear_stop_request_for_turn(correlation, &mut stop_effects);
        } else if turn_started {
            self.schedule_stop_synchronization_after_turn_started(&mut stop_effects);
        }
        self.prune_post_turn_evaluation_for_lifecycle();
        if self.serving_background {
            // 클라이언트는 이 thread 를 보지 않는다. 화면용 투영은 보내지 않는다.
            snapshots.clear();
        }
        TurnStreamReduction {
            post_turn_evaluation_due_workspace,
            snapshots,
            stop_effects,
            approval_review_persistence,
        }
    }

    pub(super) fn complete_approval_review_persistence(
        &mut self,
        correlation: &ApprovalReviewPersistenceCorrelation,
    ) -> Option<ApprovalReviewPersistenceSettlement> {
        self.approval_review_persistence.complete(correlation)
    }

    pub(super) fn apply_runtime_notice(&mut self, notice: String) -> TurnStreamSnapshot {
        self.session.turn_stream_state.apply_runtime_notice(notice)
    }

    pub(super) fn apply_correlated_runtime_notice(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        notice: String,
    ) -> Option<TurnStreamSnapshot> {
        self.with_owning_session(
            |session| session.owns_turn(correlation),
            |reducer| {
                let snapshot = reducer
                    .session
                    .turn_stream_state
                    .apply_runtime_notice(notice);
                (!reducer.serving_background).then_some(snapshot)
            },
        )
        .flatten()
    }

    pub(super) fn apply_turn_workspace_change(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        workspace_directory: String,
    ) -> bool {
        self.with_owning_session(
            |session| session.owns_turn(correlation),
            |reducer| {
                reducer
                    .session
                    .conversation_runtime
                    .replace_active_turn_workspace(correlation, workspace_directory)
                    && !reducer.serving_background
            },
        )
        .unwrap_or(false)
    }

    pub(super) fn admit_stop_request(&mut self) -> StopRequestAdmission {
        if let Some(active) = self.session.active_stop_request {
            return StopRequestAdmission::RejectedActive {
                active_correlation: active.correlation,
            };
        }
        let correlation = StopRequestCorrelation::new(
            take_generation(
                &mut self.next_stop_request_generation,
                "runtime stop request",
            ),
            self.session.conversation_runtime.active_turn_correlation(),
        );
        self.session.active_stop_request = Some(ActiveStopRequest {
            correlation,
            pending_attempt: Some(StopRequestAttempt::Initial),
            synchronize_after_turn_started: correlation.turn_submission.is_some()
                && !self.session.turn_stream_state.has_active_turn(),
            invalidated: false,
        });
        StopRequestAdmission::Accepted { correlation }
    }

    pub(super) fn complete_stop_request(
        &mut self,
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
        failed: bool,
    ) -> Option<StopRequestCompletionReduction> {
        self.with_owning_session(
            |session| {
                session
                    .active_stop_request
                    .is_some_and(|active| active.correlation == correlation)
            },
            |reducer| reducer.complete_stop_request_in_session(correlation, attempt, failed),
        )
        .flatten()
    }

    fn complete_stop_request_in_session(
        &mut self,
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
        failed: bool,
    ) -> Option<StopRequestCompletionReduction> {
        if self.session.active_stop_request.is_none_or(|active| {
            active.correlation != correlation || active.pending_attempt != Some(attempt)
        }) {
            return None;
        }
        self.session
            .active_stop_request
            .as_mut()
            .expect("exact active stop request must remain present")
            .pending_attempt = None;
        let invalidated = self
            .session
            .active_stop_request
            .is_some_and(|active| active.invalidated);
        if failed || invalidated || correlation.turn_submission.is_none() {
            self.session.active_stop_request = None;
        }
        let mut stop_effects = Vec::new();
        if !failed && !invalidated {
            self.schedule_stop_synchronization_after_turn_started(&mut stop_effects);
        }
        Some(StopRequestCompletionReduction {
            publish_completion: !invalidated,
            settlement_finished: self.session.active_stop_request.is_none(),
            stop_effects,
        })
    }

    pub(super) fn stop_request_settlement_pending(&self) -> bool {
        self.session
            .active_stop_request
            .is_some_and(|active| active.pending_attempt.is_some())
    }

    /// steer 는 thread id 로 그 thread 의 session 을 찾는다 (plan 18 K3-3a). 원격 클라이언트는 보지 않는 thread 도
    /// steer 할 수 있다.
    pub(super) fn admit_turn_steer(
        &mut self,
        request: &ConversationTurnSteerRequest,
    ) -> TurnSteerAdmission {
        self.with_owning_session(
            |session| session.streams_thread(&request.thread_id),
            |reducer| reducer.admit_turn_steer_in_session(request),
        )
        .unwrap_or(TurnSteerAdmission::RejectedUnavailable)
    }

    fn admit_turn_steer_in_session(
        &mut self,
        request: &ConversationTurnSteerRequest,
    ) -> TurnSteerAdmission {
        if let Some(active) = &self.session.active_turn_steer {
            return TurnSteerAdmission::RejectedActive {
                active_correlation: active.correlation,
            };
        }
        let Some(turn_submission) = self.session.conversation_runtime.active_turn_correlation()
        else {
            return TurnSteerAdmission::RejectedUnavailable;
        };
        if !self
            .session
            .turn_stream_state
            .matches_active_turn(&request.thread_id, &request.expected_turn_id)
        {
            return TurnSteerAdmission::RejectedUnavailable;
        }
        let correlation = TurnSteerCorrelation::new(
            take_generation(&mut self.next_turn_steer_generation, "active turn steer"),
            turn_submission,
        );
        self.session.active_turn_steer = Some(ActiveTurnSteer {
            correlation,
            expected_turn_id: request.expected_turn_id.clone(),
        });
        TurnSteerAdmission::Accepted { correlation }
    }

    pub(super) fn complete_turn_steer(
        &mut self,
        correlation: TurnSteerCorrelation,
        result: Result<ConversationTurnSteerReceipt, String>,
    ) -> Option<Result<ConversationTurnSteerReceipt, String>> {
        let active = self
            .with_owning_session(
                |session| {
                    session
                        .active_turn_steer
                        .as_ref()
                        .is_some_and(|active| active.correlation == correlation)
                },
                // 보지 않는 thread 의 steer 결과는 정리만 하고 클라이언트에 알리지 않는다.
                |reducer| {
                    let active = reducer.session.active_turn_steer.take();
                    active.filter(|_| !reducer.serving_background)
                },
            )
            .flatten()?;
        Some(result.and_then(|receipt| {
            if receipt.turn_id == active.expected_turn_id {
                Ok(receipt)
            } else {
                Err("turn steer provider returned a different turn".to_string())
            }
        }))
    }

    /// 승인 결정은 승인 id 로 그 승인을 기다리는 session 을 찾는다 (plan 18 K3-3a).
    pub(super) fn admit_approval_decision(
        &mut self,
        request_identity: ConversationApprovalRequestIdentity,
        decision: ConversationApprovalDecision,
    ) -> ApprovalDecisionAdmission {
        let owner_identity = request_identity.clone();
        self.with_owning_session(
            |session| {
                session
                    .turn_stream_state
                    .matches_pending_approval(&owner_identity)
            },
            |reducer| reducer.admit_approval_decision_in_session(request_identity, decision),
        )
        .unwrap_or(ApprovalDecisionAdmission::RejectedUnavailable)
    }

    fn admit_approval_decision_in_session(
        &mut self,
        request_identity: ConversationApprovalRequestIdentity,
        decision: ConversationApprovalDecision,
    ) -> ApprovalDecisionAdmission {
        if let Some(active) = self.session.conversation_runtime.active_approval_decision() {
            return ApprovalDecisionAdmission::RejectedActive {
                active_correlation: active.clone(),
            };
        }
        let Some(turn_submission) = self.session.conversation_runtime.active_turn_correlation()
        else {
            return ApprovalDecisionAdmission::RejectedUnavailable;
        };
        if !self
            .session
            .turn_stream_state
            .matches_pending_approval(&request_identity)
        {
            return ApprovalDecisionAdmission::RejectedUnavailable;
        }
        let correlation = ApprovalDecisionCorrelation::new(
            take_generation(
                &mut self.next_approval_decision_generation,
                "approval decision",
            ),
            turn_submission,
            request_identity,
            decision,
        );
        if !self
            .session
            .conversation_runtime
            .begin_approval_decision(correlation.clone())
        {
            return ApprovalDecisionAdmission::RejectedUnavailable;
        }
        ApprovalDecisionAdmission::Accepted { correlation }
    }

    pub(super) fn complete_approval_decision(
        &mut self,
        correlation: &ApprovalDecisionCorrelation,
        succeeded: bool,
    ) -> bool {
        self.with_owning_session(
            |session| session.conversation_runtime.active_approval_decision() == Some(correlation),
            |reducer| {
                // 보지 않는 thread 의 승인 결과는 상태에만 반영하고 클라이언트에 알리지 않는다.
                reducer
                    .session
                    .conversation_runtime
                    .complete_approval_decision(correlation, succeeded)
                    && !reducer.serving_background
            },
        )
        .unwrap_or(false)
    }

    pub(super) fn admit_post_turn_evaluation(
        &mut self,
        request: &mut PostTurnRequest,
    ) -> Option<PostTurnEvaluationCorrelation> {
        let thread_id = request.context.thread_id.clone();
        let completed_turn_id = request.completed_turn_id.clone();
        self.with_owning_session(
            |session| {
                session
                    .turn_stream_state
                    .can_start_post_turn_evaluation(&thread_id, &completed_turn_id)
            },
            |reducer| reducer.admit_post_turn_evaluation_in_session(request),
        )
        .flatten()
    }

    fn admit_post_turn_evaluation_in_session(
        &mut self,
        request: &mut PostTurnRequest,
    ) -> Option<PostTurnEvaluationCorrelation> {
        self.prune_post_turn_evaluation_for_lifecycle();
        if self.session.in_flight_post_turn_evaluation.is_some()
            || !self
                .session
                .turn_stream_state
                .can_start_post_turn_evaluation(
                    &request.context.thread_id,
                    &request.completed_turn_id,
                )
        {
            return None;
        }
        let runtime = self.session.conversation_runtime.snapshot();
        /*
         * The TUI may carry a compatibility copy while mapping the request,
         * but it is never authoritative. Bind evaluation to the handoff owned
         * by the exact admitted turn/post-turn route before any worker starts.
         */
        request.context.previous_handoff_task = runtime.planning_handoff.clone();
        let auto_follow = runtime.auto_follow;
        let parallel_continuation_enabled = request.context.parallel_mode_enabled
            && auto_follow.parallel_post_turn_continuation_allowed();
        request.context.planning_settlement_paused =
            auto_follow.continuation_paused && !parallel_continuation_enabled;
        request.context.continuation_paused = (!auto_follow.is_enabled()
            || auto_follow.continuation_paused)
            && !parallel_continuation_enabled;
        request.context.can_queue_next =
            auto_follow.can_queue_next() || parallel_continuation_enabled;
        request.context.stop_keyword = auto_follow.stop_keyword.clone();
        request.context.stop_keyword_matched = request
            .context
            .latest_main_reply
            .as_deref()
            .is_some_and(|reply| auto_follow.matches_stop_keyword(reply));
        request.context.no_file_changes_stop_matched =
            auto_follow.stop_on_no_file_changes && request.changed_planning_file_paths.is_empty();
        let correlation = PostTurnEvaluationCorrelation::new(
            take_generation(
                &mut self.next_post_turn_evaluation_generation,
                "post-turn evaluation",
            ),
            request.context.thread_id.clone(),
            request.completed_turn_id.clone(),
            request.workspace_directory.clone(),
            request.context.planning_workspace_directory.clone(),
        );
        request.continuation_permit = self.session.post_turn_continuation_gate.capture();
        self.session.in_flight_post_turn_evaluation = Some(ActivePostTurnEvaluation {
            correlation: correlation.clone(),
            continuation_permit: request.continuation_permit.clone(),
        });
        self.session
            .conversation_runtime
            .begin_post_turn_evaluation(correlation.clone());
        Some(correlation)
    }

    pub(super) fn complete_post_turn_evaluation(
        &mut self,
        correlation: &PostTurnEvaluationCorrelation,
        execution: &PostTurnExecution,
    ) -> bool {
        self.with_owning_session(
            |session| {
                session
                    .in_flight_post_turn_evaluation
                    .as_ref()
                    .is_some_and(|active| &active.correlation == correlation)
            },
            |reducer| reducer.complete_post_turn_evaluation_in_session(correlation, execution),
        )
        .unwrap_or(false)
    }

    fn complete_post_turn_evaluation_in_session(
        &mut self,
        correlation: &PostTurnEvaluationCorrelation,
        execution: &PostTurnExecution,
    ) -> bool {
        if self.active_post_turn_evaluation_correlation() != Some(correlation)
            || !correlation.matches_execution(execution)
        {
            return false;
        }
        // 대화를 여는 중에는 보는 session 의 평가 결과를 버린다. 뒤의 session 은 대화 로드와 관계없다.
        if self.in_flight_conversation_load.is_some() && !self.serving_background {
            return false;
        }
        if !self
            .session
            .turn_stream_state
            .accept_post_turn_evaluation_completion(execution)
        {
            return false;
        }
        self.session.in_flight_post_turn_evaluation = None;
        self.session
            .conversation_runtime
            .await_post_turn_route(correlation.clone(), Box::new(execution.clone()))
    }

    pub(super) fn resolve_post_turn_route(
        &mut self,
        correlation: &PostTurnEvaluationCorrelation,
        resolution: PostTurnRouteResolution,
    ) -> Option<(Box<PostTurnExecution>, PostTurnRouteResolution)> {
        self.with_owning_session(
            |session| session.awaits_post_turn_route(correlation),
            |reducer| {
                reducer
                    .session
                    .conversation_runtime
                    .resolve_post_turn_route(correlation, resolution)
            },
        )
        .flatten()
    }

    // 자동 후속 정책은 모든 session 에 같이 적용한다 (plan 18 K2-3). 뒤에서 도는 자동 후속도 멈춰야 한다.
    pub(super) fn set_auto_follow_max_turns(&mut self, value: usize) {
        self.for_each_session(|reducer| {
            reducer.settle_post_turn_continuation_for_policy_change();
            reducer
                .session
                .conversation_runtime
                .set_auto_follow_max_turns(value);
        });
    }

    pub(super) fn pause_post_turn_continuation(&mut self) {
        self.for_each_session(|reducer| {
            reducer.settle_post_turn_continuation_for_policy_change();
            reducer
                .session
                .conversation_runtime
                .pause_post_turn_continuation();
        });
    }

    pub(super) fn set_parallel_post_turn_rearm(&mut self, rearmed: bool) {
        self.for_each_session(|reducer| reducer.set_parallel_post_turn_rearm_in_session(rearmed));
    }

    fn set_parallel_post_turn_rearm_in_session(&mut self, rearmed: bool) {
        /*
         * Turning parallel routing off must not revoke an independently
         * authorized single-session continuation. In that case the exact
         * worker may finish and NativeClientRuntime will resolve its route to
         * AutoSubmit after the parallel control-plane declines it. Every other
         * parallel policy change still settles the in-flight correlation before
         * changing authority, so a late worker cannot regain a route.
         */
        let preserves_single_session_continuation =
            !rearmed && self.runtime_snapshot().auto_follow.can_queue_next();
        if !preserves_single_session_continuation {
            self.settle_post_turn_continuation_for_policy_change();
        }
        self.session
            .conversation_runtime
            .set_parallel_post_turn_rearm(rearmed);
    }

    pub(super) fn active_post_turn_evaluation_correlation(
        &self,
    ) -> Option<&PostTurnEvaluationCorrelation> {
        self.session
            .in_flight_post_turn_evaluation
            .as_ref()
            .map(|active| &active.correlation)
    }

    pub(super) fn matches_conversation(&self, workspace_directory: &str, thread_id: &str) -> bool {
        self.session
            .turn_stream_state
            .matches_conversation(workspace_directory, thread_id)
    }

    fn prune_post_turn_evaluation_for_lifecycle(&mut self) {
        let should_prune = self
            .session
            .in_flight_post_turn_evaluation
            .as_ref()
            .is_some_and(|active| {
                !self
                    .session
                    .turn_stream_state
                    .can_start_post_turn_evaluation(
                        &active.correlation.thread_id,
                        &active.correlation.completed_turn_id,
                    )
            });
        if should_prune {
            self.cancel_active_post_turn_evaluation();
        }
    }

    fn cancel_active_post_turn_evaluation(&mut self) {
        if let Some(active) = self.session.in_flight_post_turn_evaluation.take() {
            active.continuation_permit.invalidate_if_current();
        }
        self.session
            .conversation_runtime
            .cancel_post_turn_evaluation();
    }

    fn invalidate_post_turn_continuation(&mut self) {
        self.session.post_turn_continuation_gate.advance();
        self.cancel_active_post_turn_evaluation();
    }

    fn settle_post_turn_continuation_for_policy_change(&mut self) {
        self.session.post_turn_continuation_gate.advance();
        let active = self.session.in_flight_post_turn_evaluation.take();
        if let Some(active) = active.as_ref()
            && !self.session.turn_stream_state.settle_post_turn_terminal(
                &active.correlation.thread_id,
                &active.correlation.completed_turn_id,
            )
        {
            self.session
                .conversation_runtime
                .cancel_post_turn_evaluation();
            return;
        }
        let settled = self
            .session
            .conversation_runtime
            .settle_in_flight_post_turn_without_continuation();
        if let Some(active) = active {
            debug_assert_eq!(
                settled.as_ref(),
                Some(&active.correlation),
                "post-turn worker and runtime authority must settle the same correlation"
            );
        }
    }

    fn schedule_stop_synchronization_after_turn_started(
        &mut self,
        stop_effects: &mut Vec<StopEffectIntent>,
    ) {
        let Some(active) = self.session.active_stop_request.as_mut() else {
            return;
        };
        if active.invalidated
            || !active.synchronize_after_turn_started
            || active.pending_attempt.is_some()
            || active.correlation.turn_submission
                != self.session.conversation_runtime.active_turn_correlation()
            || !self.session.turn_stream_state.has_active_turn()
        {
            return;
        }
        active.synchronize_after_turn_started = false;
        active.pending_attempt = Some(StopRequestAttempt::AfterTurnStarted);
        stop_effects.push(StopEffectIntent::Request {
            correlation: active.correlation,
            attempt: StopRequestAttempt::AfterTurnStarted,
        });
    }

    fn clear_stop_request_for_turn(
        &mut self,
        correlation: TurnSubmissionCorrelation,
        stop_effects: &mut Vec<StopEffectIntent>,
    ) {
        if self
            .session
            .active_stop_request
            .is_none_or(|active| active.correlation.turn_submission != Some(correlation))
        {
            return;
        }
        self.invalidate_stop_request_for_lifecycle(stop_effects);
    }

    fn invalidate_stop_request_for_lifecycle(&mut self, stop_effects: &mut Vec<StopEffectIntent>) {
        let Some(active) = self.session.active_stop_request.as_mut() else {
            return;
        };
        if active.pending_attempt.is_none() {
            self.session.active_stop_request = None;
            return;
        }
        if active.invalidated {
            return;
        }
        active.invalidated = true;
        active.synchronize_after_turn_started = false;
        stop_effects.push(StopEffectIntent::Invalidate {
            correlation: active.correlation,
        });
    }

    #[cfg(test)]
    pub(super) fn begin_test_turn_submission(&mut self) -> TurnSubmissionCorrelation {
        assert!(
            self.session
                .conversation_runtime
                .active_turn_correlation()
                .is_none(),
            "test turn submission must not supersede an active generation"
        );
        let request = TurnSubmissionRequest {
            transcript_text: String::new(),
            workspace_directory: "/workspace".to_string(),
            thread_id: None,
            image_paths: Vec::new(),
            prompt: "test prompt".to_string(),
            prompt_origin: super::CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: crate::domain::conversation::ConversationTurnOptions::default(),
            slot_lease_handoff: None,
        };
        let TurnSubmissionAdmission::Accepted { correlation } =
            self.admit_turn_submission(&request)
        else {
            panic!("test turn submission should be admitted");
        };
        correlation
    }

    #[cfg(test)]
    pub(super) fn active_turn_submission_for_test(&self) -> Option<TurnSubmissionCorrelation> {
        self.session.conversation_runtime.active_turn_correlation()
    }

    #[cfg(test)]
    pub(super) fn active_turn_steer_for_test(&self) -> Option<TurnSteerCorrelation> {
        self.session
            .active_turn_steer
            .as_ref()
            .map(|active| active.correlation)
    }

    #[cfg(test)]
    pub(super) fn active_approval_decision_for_test(&self) -> Option<&ApprovalDecisionCorrelation> {
        self.session.conversation_runtime.active_approval_decision()
    }

    #[cfg(test)]
    pub(super) fn approval_decision_is_submitted_for_test(&self) -> bool {
        self.session
            .conversation_runtime
            .snapshot()
            .approval
            .as_ref()
            .is_some_and(|approval| approval.phase == super::ApprovalAuthorityPhase::Submitted)
    }

    #[cfg(test)]
    pub(super) fn exhaust_stop_generation_for_test(&mut self) {
        self.next_stop_request_generation = u64::MAX;
    }

    #[cfg(test)]
    pub(super) fn exhaust_approval_generation_for_test(&mut self) {
        self.next_approval_decision_generation = u64::MAX;
    }

    #[cfg(test)]
    pub(super) fn exhaust_post_turn_generation_for_test(&mut self) {
        self.next_post_turn_evaluation_generation = u64::MAX;
    }
}

impl Default for ConversationTurnFeatureReducer {
    fn default() -> Self {
        Self::new()
    }
}

fn take_generation(next_generation: &mut u64, operation: &str) -> u64 {
    let generation = *next_generation;
    *next_generation = generation
        .checked_add(1)
        .unwrap_or_else(|| panic!("{operation} generation exhausted"));
    generation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::planning::{
        PlanningWorkerPanelState, PostTurnAutoFollowSkipReason, PostTurnContext,
        PostTurnContinuationAction, PostTurnOutcome, PostTurnProvenance, RuntimeProjection,
        TaskHandoff,
    };

    fn turn_request() -> TurnSubmissionRequest {
        TurnSubmissionRequest {
            transcript_text: String::new(),
            workspace_directory: "/workspace".to_string(),
            thread_id: None,
            image_paths: Vec::new(),
            prompt: "test prompt".to_string(),
            prompt_origin: super::super::CorePromptOrigin::Manual,
            auto_follow_source: None,
            planning_handoff: None,
            turn_options: crate::domain::conversation::ConversationTurnOptions::default(),
            slot_lease_handoff: None,
        }
    }

    fn queued_auto_follow_authority(reducer: &mut ConversationTurnFeatureReducer) {
        let correlation =
            PostTurnEvaluationCorrelation::new(1, "thread-1", "turn-1", "/workspace", "/workspace");
        let execution = Box::new(PostTurnExecution {
            thread_id: "thread-1".to_string(),
            completed_turn_id: "turn-1".to_string(),
            runtime_projection_workspace_directory: "/workspace".to_string(),
            evaluation: PostTurnOutcome {
                provenance: PostTurnProvenance::new("turn-1".to_string()),
                runtime_projection: RuntimeProjection::invalid("planning blocked"),
                planning_repair_state: None,
                runtime_notices: Vec::new(),
                action: PostTurnContinuationAction::SkipAutoFollow {
                    reason: PostTurnAutoFollowSkipReason::PlanningBlocked,
                },
                operator_alerts: Vec::new(),
            },
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
        });
        reducer
            .session
            .conversation_runtime
            .set_auto_follow_max_turns(1);
        reducer
            .session
            .conversation_runtime
            .begin_post_turn_evaluation(correlation.clone());
        assert!(
            reducer
                .session
                .conversation_runtime
                .await_post_turn_route(correlation.clone(), execution)
        );
        assert!(
            reducer
                .session
                .conversation_runtime
                .resolve_post_turn_route(&correlation, PostTurnRouteResolution::AutoSubmit)
                .is_some()
        );
    }

    fn started_load(
        reducer: &mut ConversationTurnFeatureReducer,
        thread_id: &str,
    ) -> ConversationLoadCorrelation {
        let ConversationLoadAdmission::Started { correlation, .. } =
            reducer.admit_conversation_load(thread_id.to_string(), "/workspace".to_string(), false)
        else {
            panic!("conversation load should start");
        };
        correlation
    }

    #[test]
    fn conversation_load_completion_requires_the_latest_exact_generation_once() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        let first = started_load(&mut reducer, "thread-a");
        let second = started_load(&mut reducer, "thread-b");
        let current = started_load(&mut reducer, "thread-a");

        assert!(reducer.complete_conversation_load(&first, None).is_none());
        assert!(reducer.complete_conversation_load(&second, None).is_none());
        assert!(reducer.complete_conversation_load(&current, None).is_some());
        assert!(reducer.complete_conversation_load(&current, None).is_none());
    }

    #[test]
    fn deferred_load_and_invalidation_share_the_same_authority_slice() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        assert!(matches!(
            reducer.admit_conversation_load(
                "thread-a".to_string(),
                "/workspace-a".to_string(),
                true,
            ),
            ConversationLoadAdmission::Deferred { .. }
        ));
        assert_eq!(
            reducer.take_deferred_conversation_load(),
            Some(("thread-a".to_string(), "/workspace-a".to_string()))
        );

        let correlation = started_load(&mut reducer, "thread-a");
        reducer.reduce_conversation_invalidation();
        assert!(
            reducer
                .complete_conversation_load(&correlation, None)
                .is_none()
        );
        assert!(reducer.take_deferred_conversation_load().is_none());
    }

    #[test]
    fn turn_and_stop_admissions_cannot_bypass_each_other() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        let TurnSubmissionAdmission::Accepted { correlation: turn } =
            reducer.admit_turn_submission(&turn_request())
        else {
            panic!("first turn should start");
        };
        assert!(matches!(
            reducer.admit_turn_submission(&turn_request()),
            TurnSubmissionAdmission::RejectedActive {
                active_correlation
            } if active_correlation == turn
        ));

        let StopRequestAdmission::Accepted { correlation: stop } = reducer.admit_stop_request()
        else {
            panic!("first stop should start");
        };
        assert_eq!(stop.turn_submission, Some(turn));
        assert!(matches!(
            reducer.admit_stop_request(),
            StopRequestAdmission::RejectedActive {
                active_correlation
            } if active_correlation == stop
        ));
        assert!(matches!(
            reducer.admit_turn_submission(&turn_request()),
            TurnSubmissionAdmission::RejectedActive {
                active_correlation
            } if active_correlation == turn
        ));

        let mut idle = ConversationTurnFeatureReducer::new();
        let StopRequestAdmission::Accepted {
            correlation: idle_stop,
        } = idle.admit_stop_request()
        else {
            panic!("idle stop should start");
        };
        assert!(matches!(
            idle.admit_turn_submission(&turn_request()),
            TurnSubmissionAdmission::RejectedStopPending {
                stop_correlation
            } if stop_correlation == idle_stop
        ));

        let settlement = reducer
            .complete_stop_request(stop, StopRequestAttempt::Initial, true)
            .expect("exact stop completion should settle");
        assert!(settlement.publish_completion);
        assert!(settlement.settlement_finished);
        // plan 18 K2-3: leaving the conversation keeps the running turn in the background, and only
        // one conversation turn runs across all threads.
        reducer.reduce_conversation_invalidation();
        assert_eq!(reducer.background_session_count(), 1);
        assert!(matches!(
            reducer.admit_turn_submission(&turn_request()),
            TurnSubmissionAdmission::RejectedActive {
                active_correlation
            } if active_correlation == turn
        ));
    }

    #[test]
    fn stop_pending_rejection_settles_a_queued_auto_follow_submission() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        queued_auto_follow_authority(&mut reducer);
        assert!(matches!(
            reducer.runtime_snapshot().auto_follow.phase,
            super::super::AutoFollowPhase::Queued { .. }
        ));
        let StopRequestAdmission::Accepted { correlation: stop } = reducer.admit_stop_request()
        else {
            panic!("the stop request should own the idle runtime");
        };
        let mut auto_turn = turn_request();
        auto_turn.prompt_origin = super::super::CorePromptOrigin::AutoFollow;

        assert!(matches!(
            reducer.admit_turn_submission(&auto_turn),
            TurnSubmissionAdmission::RejectedStopPending {
                stop_correlation
            } if stop_correlation == stop
        ));
        assert!(matches!(
            reducer.runtime_snapshot().auto_follow.phase,
            super::super::AutoFollowPhase::Idle
        ));
        assert!(reducer.runtime_snapshot().can_accept_manual_prompt());
    }

    #[test]
    fn malformed_exact_auto_follow_target_cancels_only_the_owned_queue_lease() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        queued_auto_follow_authority(&mut reducer);
        let source =
            PostTurnEvaluationCorrelation::new(1, "thread-1", "turn-1", "/workspace", "/workspace");
        let mut malformed = turn_request();
        malformed.prompt_origin = super::super::CorePromptOrigin::AutoFollow;
        malformed.thread_id = Some("thread-1".to_string());
        malformed.workspace_directory = "/wrong-workspace".to_string();
        malformed.auto_follow_source = Some(source);

        assert_eq!(
            reducer.admit_turn_submission(&malformed),
            TurnSubmissionAdmission::RejectedUnavailable
        );
        assert!(matches!(
            reducer.runtime_snapshot().auto_follow.phase,
            super::super::AutoFollowPhase::Idle
        ));
        assert!(
            reducer.runtime_snapshot().can_accept_manual_prompt(),
            "a malformed request that owns the current lease must not leave manual input locked"
        );
    }

    #[test]
    fn duplicate_turn_start_is_ignored_and_conflicting_start_closes_the_generation() {
        let mut reducer = ConversationTurnFeatureReducer::new();
        let TurnSubmissionAdmission::Accepted { correlation } =
            reducer.admit_turn_submission(&turn_request())
        else {
            panic!("manual turn should be admitted");
        };
        reducer
            .apply_correlated_turn_stream_event(
                correlation,
                TurnStreamEvent::ThreadPrepared {
                    thread_id: "thread-1".to_string(),
                    title: "Core stream".to_string(),
                    cwd: "/workspace".to_string(),
                    runtime_envelope: Box::default(),
                },
            )
            .expect("thread preparation should be correlated");
        reducer
            .apply_correlated_turn_stream_event(
                correlation,
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-1".to_string(),
                    runtime_request: Box::default(),
                },
            )
            .expect("first turn start should be accepted");
        let running = reducer.runtime_snapshot();

        let duplicate = reducer
            .apply_correlated_turn_stream_event(
                correlation,
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-1".to_string(),
                    runtime_request: Box::default(),
                },
            )
            .expect("duplicate start should be reported as ignored");
        assert!(matches!(
            duplicate.snapshots.as_slice(),
            [TurnStreamSnapshot {
                update: TurnStreamUpdate::TurnStartedIgnored {
                    rejection: TurnStreamStartRejection::Duplicate,
                    ..
                },
                ..
            }]
        ));
        assert_eq!(reducer.runtime_snapshot(), running);

        let conflicting = reducer
            .apply_correlated_turn_stream_event(
                correlation,
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-forged".to_string(),
                    runtime_request: Box::default(),
                },
            )
            .expect("conflicting start should fail the active generation closed");
        assert!(matches!(
            conflicting.snapshots.as_slice(),
            [
                TurnStreamSnapshot {
                    update: TurnStreamUpdate::TurnStartedIgnored {
                        rejection: TurnStreamStartRejection::TurnMismatch { .. },
                        ..
                    },
                    ..
                },
                TurnStreamSnapshot {
                    update: TurnStreamUpdate::Failed { .. },
                    ..
                },
            ]
        ));
        assert!(reducer.runtime_snapshot().active_turn.is_none());
    }

    #[test]
    fn rejected_parallel_intake_cannot_replace_the_completed_turn_handoff() {
        fn handoff(task_id: &str) -> TaskHandoff {
            TaskHandoff {
                task_id: task_id.to_string(),
                task_title: format!("Task {task_id}"),
                direction_id: "direction-1".to_string(),
                combined_priority: 10,
                updated_at: "2026-07-27T00:00:00Z".to_string(),
                status_label: "Ready".to_string(),
            }
        }

        let mut reducer = ConversationTurnFeatureReducer::new();
        let task_a = handoff("task-a");
        let task_b = handoff("task-b");
        let mut turn_a = turn_request();
        turn_a.prompt_origin = super::super::CorePromptOrigin::ManualIntake;
        turn_a.planning_handoff = Some(task_a.clone());
        let TurnSubmissionAdmission::Accepted {
            correlation: turn_a_correlation,
        } = reducer.admit_turn_submission(&turn_a)
        else {
            panic!("manual intake A should be admitted");
        };

        let mut parallel_intake_b = turn_request();
        parallel_intake_b.prompt_origin = super::super::CorePromptOrigin::ManualIntake;
        parallel_intake_b.planning_handoff = Some(task_b.clone());
        assert!(matches!(
            reducer.admit_turn_submission(&parallel_intake_b),
            TurnSubmissionAdmission::RejectedActive { .. }
        ));
        assert_eq!(
            reducer.runtime_snapshot().planning_handoff,
            Some(task_a.clone())
        );

        reducer
            .apply_correlated_turn_stream_event(
                turn_a_correlation,
                TurnStreamEvent::ThreadPrepared {
                    thread_id: "thread-1".to_string(),
                    title: "Core stream".to_string(),
                    cwd: "/workspace".to_string(),
                    runtime_envelope: Box::default(),
                },
            )
            .expect("thread preparation should be correlated");
        reducer
            .apply_correlated_turn_stream_event(
                turn_a_correlation,
                TurnStreamEvent::TurnStarted {
                    turn_id: "turn-a".to_string(),
                    runtime_request: Box::default(),
                },
            )
            .expect("turn A should start");
        reducer
            .apply_correlated_turn_stream_event(
                turn_a_correlation,
                TurnStreamEvent::TurnTerminal {
                    receipt:
                        crate::domain::turn_terminal::ConversationTurnTerminalReceipt::completed(
                            "thread-1",
                            "turn-a",
                            Vec::new(),
                        )
                        .with_application_delivery(
                            crate::domain::turn_terminal::ConversationTurnApplicationDelivery::Confirmed,
                        ),
                    execution_snapshot_capture: None,
                },
            )
            .expect("turn A terminal should settle");

        let mut post_turn = PostTurnRequest {
            context: PostTurnContext {
                thread_id: "thread-1".to_string(),
                planning_workspace_directory: "/workspace".to_string(),
                latest_user_message: None,
                latest_main_reply: None,
                previous_handoff_task: Some(task_b),
                current_runtime_projection: RuntimeProjection::invalid("refresh required"),
                parallel_mode_enabled: true,
                parallel_automation_epoch_id: Some(1),
                planning_settlement_paused: false,
                continuation_paused: false,
                can_queue_next: false,
                stop_keyword: ":stop".to_string(),
                stop_keyword_matched: false,
                no_file_changes_stop_matched: false,
                mode_label: "test".to_string(),
            },
            workspace_directory: "/workspace".to_string(),
            completed_turn_id: "turn-a".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
            continuation_permit: PostTurnContinuationGate::default().capture(),
        };

        assert!(reducer.admit_post_turn_evaluation(&mut post_turn).is_some());
        assert_eq!(post_turn.context.previous_handoff_task, Some(task_a));
    }
}
