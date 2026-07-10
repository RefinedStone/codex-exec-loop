use crate::application::port::outbound::parallel_mode_runtime_port::ParallelWorkerCommitOutcome;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::parallel_mode::{
    ParallelModeOfficialCompletionReport, ParallelModeOrchestratorTrigger, ParallelModeService,
};
use crate::domain::parallel_mode::{ParallelModeSlotLeaseRequest, ParallelModeSlotLeaseSnapshot};
use crate::domain::planning::{ParallelTurnHandoff, PostTurnContinuationPermit};

pub type ParallelTurnSlotLeaseHandoff = ParallelTurnHandoff;

fn slot_lease_request_from_handoff(
    handoff: &ParallelTurnSlotLeaseHandoff,
) -> ParallelModeSlotLeaseRequest {
    ParallelModeSlotLeaseRequest::from_task_identity(&handoff.task_id, &handoff.task_title)
}
#[derive(Debug, Clone, PartialEq, Eq)]
/*
이 요청 타입은 TUI가 "대화 스트림을 시작한다"는 한 가지 동작을 application 계층으로 넘길 때
필요한 입력을 담는다. 평소에는 현재 workspace와 thread_id를 그대로 사용하지만, 병렬 모드에서는
`slot_lease_handoff`가 함께 들어와 "먼저 빈 슬롯 worktree를 빌린 뒤 그 worktree에서 새 thread를
시작하라"는 의미가 된다. 그래서 이 타입은 대화 런타임과 병렬 슬롯 런타임 사이의 작은 경계
객체다.
*/
pub struct ParallelTurnStreamLaunchRequest {
    pub workspace_directory: String,
    pub thread_id: Option<String>,
    pub prompt: String,
    pub slot_lease_handoff: Option<ParallelTurnSlotLeaseHandoff>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
/*
launch outcome은 실제 스트림 실행에 사용할 요청을 다시 돌려준다. 병렬 슬롯을 빌린 경우
`request.workspace_directory`가 원래 저장소에서 슬롯 worktree로 바뀌고 `thread_id`가 `None`이
된다. 슬롯마다 별도의 Codex thread를 시작해야 주 저장소의 대화와 슬롯 대화가 섞이지 않기
때문이다.

`invalidate_supervisor_snapshot`은 TUI 캐시 무효화 신호다. lease 획득은 감독자 패널의 슬롯 상태를
바꾸므로, 상위 화면은 이 값을 보고 최신 pool/roster/detail을 다시 읽는다.
*/
pub struct ParallelTurnStreamLaunchOutcome {
    pub request: ParallelTurnStreamLaunchRequest,
    pub expected_lease: Option<ParallelModeSlotLeaseSnapshot>,
    pub launch_notice: Option<String>,
    pub invalidate_supervisor_snapshot: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
/*
스트림 이벤트 outcome은 "대화 런타임에서 관측한 이벤트가 슬롯 lease 상태를 바꾸었는가"를 알려
준다. `ThreadPrepared`는 thread id를 lease에 기록하고, `TurnStarted`는 슬롯을 running 상태로
전환한다. 이 구분이 있어야 시작 직전 실패와 실행 중 실패를 다르게 처리할 수 있다.
*/
pub struct ParallelTurnStreamEventOutcome {
    pub runtime_notice: Option<String>,
    pub invalidate_supervisor_snapshot: bool,
    pub turn_started_observed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
/*
completion outcome은 스트림 자체가 끝났을 때 supervisor를 다시 읽어야 하는지와 사용자에게 보여 줄
runtime notice를 담는다. 이 타입은 official completion 이전의 "대화 런타임 종료"만 표현하고,
작업 결과 통합은 뒤의 official completion/distributor 경로가 이어받는다.
*/
pub struct ParallelTurnStreamCompletionOutcome {
    pub runtime_notice: Option<String>,
    pub invalidate_supervisor_snapshot: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelTurnStreamLifecycleEventOutcome {
    pub runtime_notice: Option<String>,
    pub invalidate_supervisor_snapshot: bool,
    pub should_stop_stream_forwarding: bool,
}
#[derive(Clone)]
/*
Stream lifecycle keeps the per-turn evidence needed to reconcile a parallel slot
after the conversation stream closes. Inbound adapters forward stream events and
map outcomes to UI messages; they do not own the lease state flags.
*/
pub struct ParallelTurnStreamLifecycle {
    turn_service: ParallelModeTurnService,
    workspace_directory: String,
    expected_lease: Option<ParallelModeSlotLeaseSnapshot>,
    saw_turn_started: bool,
    saw_failed_before_turn_started: bool,
    saw_failed_event: bool,
}
impl std::fmt::Debug for ParallelTurnStreamLifecycle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParallelTurnStreamLifecycle")
            .field("workspace_directory", &self.workspace_directory)
            .field("saw_turn_started", &self.saw_turn_started)
            .field(
                "saw_failed_before_turn_started",
                &self.saw_failed_before_turn_started,
            )
            .field("saw_failed_event", &self.saw_failed_event)
            .finish()
    }
}
impl ParallelTurnStreamLifecycle {
    fn new(
        turn_service: ParallelModeTurnService,
        workspace_directory: impl Into<String>,
        expected_lease: Option<ParallelModeSlotLeaseSnapshot>,
    ) -> Self {
        Self {
            turn_service,
            workspace_directory: workspace_directory.into(),
            expected_lease,
            saw_turn_started: false,
            saw_failed_before_turn_started: false,
            saw_failed_event: false,
        }
    }

    pub fn observe_event(
        &mut self,
        event: &ConversationStreamEvent,
    ) -> ParallelTurnStreamLifecycleEventOutcome {
        let outcome = self.turn_service.sync_stream_event_inner(
            &self.workspace_directory,
            self.expected_lease.as_ref(),
            event,
        );
        self.saw_turn_started |= outcome.turn_started_observed;

        let should_stop_stream_forwarding = matches!(
            event,
            ConversationStreamEvent::TurnCompleted { .. } | ConversationStreamEvent::Failed { .. }
        );
        if matches!(event, ConversationStreamEvent::Failed { .. }) {
            self.saw_failed_event = true;
            if !self.saw_turn_started {
                self.saw_failed_before_turn_started = true;
            }
        }

        ParallelTurnStreamLifecycleEventOutcome {
            runtime_notice: outcome.runtime_notice,
            invalidate_supervisor_snapshot: outcome.invalidate_supervisor_snapshot,
            should_stop_stream_forwarding,
        }
    }

    pub fn finalize_after_stream_completion(
        &self,
        terminal_failure_observed: bool,
    ) -> ParallelTurnStreamCompletionOutcome {
        self.turn_service.finalize_stream_completion_inner(
            &self.workspace_directory,
            self.expected_lease.as_ref(),
            self.saw_turn_started,
            self.saw_failed_before_turn_started,
            self.saw_failed_event,
            terminal_failure_observed,
        )
    }
}
#[derive(Clone)]
/*
이 서비스는 TUI의 conversation stream lifecycle과 `ParallelModeService`의 slot lease 상태 기계를
이어 주는 얇은 application 서비스다. 대화 런타임은 ThreadPrepared, TurnStarted, terminal failure
같은 스트림 사건을 알고 있고, 병렬 모드 서비스는 lease 파일과 planning authority 상태를 알고
있다. 이 타입은 두 세계가 서로의 세부 구현을 직접 알지 않도록 상태 전이 호출만 번역한다.
*/
pub struct ParallelModeTurnService {
    parallel_mode_service: ParallelModeService,
    automation_guard: Option<super::ParallelModeAutomationGuard>,
}

pub(crate) struct ParallelOfficialCompletionSuccessPreparation {
    pub(crate) notices: Vec<String>,
    pub(crate) should_run_delivery_tick: bool,
}
impl std::fmt::Debug for ParallelModeTurnService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParallelModeTurnService")
            .finish_non_exhaustive()
    }
}
impl ParallelModeTurnService {
    pub fn new(parallel_mode_service: ParallelModeService) -> Self {
        Self {
            parallel_mode_service,
            automation_guard: None,
        }
    }

    pub(crate) fn with_automation_guard(
        mut self,
        automation_guard: super::ParallelModeAutomationGuard,
    ) -> Self {
        self.automation_guard = Some(automation_guard);
        self
    }

    pub(crate) fn automation_epoch_is_active(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
    ) -> bool {
        self.automation_guard
            .as_ref()
            .is_none_or(|guard| guard.is_active(workspace_directory, epoch_id))
    }

    pub(crate) fn automation_permit(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
    ) -> Option<super::ParallelModeAutomationPermit> {
        self.automation_guard
            .as_ref()
            .map(|guard| guard.permit(workspace_directory, epoch_id))
    }

    pub fn stream_lifecycle(
        &self,
        workspace_directory: impl Into<String>,
    ) -> ParallelTurnStreamLifecycle {
        ParallelTurnStreamLifecycle::new(self.clone(), workspace_directory, None)
    }

    pub(crate) fn stream_lifecycle_for_lease(
        &self,
        expected_lease: ParallelModeSlotLeaseSnapshot,
    ) -> ParallelTurnStreamLifecycle {
        ParallelTurnStreamLifecycle::new(
            self.clone(),
            expected_lease.worktree_path.clone(),
            Some(expected_lease),
        )
    }

    /*
    스트림을 실제로 띄우기 전 병렬 슬롯 lease가 필요한지 판단하는 진입점이다.
    `slot_lease_handoff`가 없으면 일반 대화이므로 입력 요청을 그대로 돌려주고, 있으면 application
    service가 domain helper로 lease request를 만든 뒤 `ParallelModeService::acquire_slot_lease`로
    비어 있는 슬롯을 하나 확보한다.

    lease가 성공하면 반환 요청을 슬롯 worktree 기준으로 다시 작성한다. 여기서 `thread_id`를 비우는
    것이 중요하다. 기존 thread를 재사용하면 root workspace의 대화 기록과 슬롯 작업 대화가 연결될
    수 있으므로, 슬롯 작업은 항상 leased worktree의 새 thread로 시작한다.
    */
    pub fn prepare_stream_launch(
        &self,
        request: ParallelTurnStreamLaunchRequest,
    ) -> Result<ParallelTurnStreamLaunchOutcome, String> {
        let Some(slot_lease_request) = request
            .slot_lease_handoff
            .as_ref()
            .map(slot_lease_request_from_handoff)
        else {
            return Ok(ParallelTurnStreamLaunchOutcome {
                request,
                expected_lease: None,
                launch_notice: None,
                invalidate_supervisor_snapshot: false,
            });
        };
        let lease = self
            .parallel_mode_service
            .acquire_slot_lease(&request.workspace_directory, slot_lease_request)?;
        Ok(ParallelTurnStreamLaunchOutcome {
            request: ParallelTurnStreamLaunchRequest {
                workspace_directory: lease.worktree_path.clone(),
                thread_id: None,
                prompt: request.prompt,
                slot_lease_handoff: None,
            },
            expected_lease: Some(lease.clone()),
            launch_notice: Some(format!(
                "slot lease acquired before stream launch / slot: {} / agent: {} / task: {}",
                lease.slot_id, lease.agent_id, lease.task_id
            )),
            invalidate_supervisor_snapshot: true,
        })
    }

    /*
    대화 스트림은 여러 이벤트를 순서대로 방출하지만, 병렬 슬롯 상태에 의미가 있는 이벤트는
    일부뿐이다. `ThreadPrepared`는 Codex app-server가 새 thread id를 확정했다는 뜻이므로 lease에
    thread id를 저장한다. `TurnStarted`는 실제 turn이 시작되었다는 뜻이므로 슬롯을 running으로
    바꾼다.

    이 함수가 `turn_started_observed`를 따로 반환하는 이유는 완료 처리에서 "시작도 못한 lease"와
    "시작한 뒤 실패한 turn"을 구분하기 위해서다. 시작 전 실패는 슬롯을 즉시 release할 수 있지만,
    시작 후 성공은 공식 완료/검증/통합 큐 단계로 이어져야 한다.
    */
    pub fn sync_stream_event(
        &self,
        workspace_directory: &str,
        event: &ConversationStreamEvent,
    ) -> ParallelTurnStreamEventOutcome {
        self.sync_stream_event_inner(workspace_directory, None, event)
    }

    pub(crate) fn sync_stream_event_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        event: &ConversationStreamEvent,
    ) -> ParallelTurnStreamEventOutcome {
        self.sync_stream_event_inner(&expected_lease.worktree_path, Some(expected_lease), event)
    }

    fn sync_stream_event_inner(
        &self,
        workspace_directory: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
        event: &ConversationStreamEvent,
    ) -> ParallelTurnStreamEventOutcome {
        if let ConversationStreamEvent::ThreadPrepared { thread_id, .. } = event {
            /*
            ThreadPrepared는 lease가 아직 Running이 되기 전의 식별자 결합 단계다.
            같은 workspace가 slot worktree가 아니면 service가 Ok(None)을 돌려주므로,
            root conversation 이벤트가 병렬 슬롯 상태를 건드리지 않는다.
            */
            let transition = match expected_lease {
                Some(expected) => self
                    .parallel_mode_service
                    .record_workspace_slot_thread_prepared_for_lease(expected, thread_id),
                None => self
                    .parallel_mode_service
                    .record_workspace_slot_thread_prepared(workspace_directory, thread_id),
            };
            return match transition {
                Ok(Some(_)) => ParallelTurnStreamEventOutcome {
                    runtime_notice: None,
                    invalidate_supervisor_snapshot: true,
                    turn_started_observed: false,
                },
                Ok(None) => ParallelTurnStreamEventOutcome {
                    runtime_notice: None,
                    invalidate_supervisor_snapshot: false,
                    turn_started_observed: false,
                },
                Err(error) => ParallelTurnStreamEventOutcome {
                    runtime_notice: Some(format!(
                        "slot lease thread-prepared transition failed: {error}"
                    )),
                    invalidate_supervisor_snapshot: false,
                    turn_started_observed: false,
                },
            };
        }
        if !matches!(event, ConversationStreamEvent::TurnStarted { .. }) {
            /*
            Streaming delta, tool output, completion 같은 이벤트는 슬롯 lifecycle에
            직접 의미가 없다. 여기서 false outcome으로 접어야 TUI가 불필요하게
            supervisor snapshot을 다시 읽지 않는다.
            */
            return ParallelTurnStreamEventOutcome {
                runtime_notice: None,
                invalidate_supervisor_snapshot: false,
                turn_started_observed: false,
            };
        }
        let transition = match expected_lease {
            Some(expected) => self
                .parallel_mode_service
                .mark_workspace_slot_running_for_lease(expected),
            None => self
                .parallel_mode_service
                .mark_workspace_slot_running(workspace_directory),
        };
        match transition {
            /*
            Ok(None) still returns turn_started_observed=true. The conversation
            runtime did see a turn begin, even if this workspace has no matching
            slot lease; completion cleanup needs that stream-level fact.
            */
            Ok(Some(_)) => ParallelTurnStreamEventOutcome {
                runtime_notice: None,
                invalidate_supervisor_snapshot: true,
                turn_started_observed: true,
            },
            Ok(None) => ParallelTurnStreamEventOutcome {
                runtime_notice: None,
                invalidate_supervisor_snapshot: false,
                turn_started_observed: true,
            },
            Err(error) => ParallelTurnStreamEventOutcome {
                runtime_notice: Some(format!("slot lease running transition failed: {error}")),
                invalidate_supervisor_snapshot: false,
                turn_started_observed: true,
            },
        }
    }

    /*
    스트림 종료 시점에는 단순히 lease를 없애면 안 된다. turn이 시작되기도 전에 실패했다면 슬롯
    worktree에 의미 있는 작업이 없으므로 lease를 release한다. 반대로 turn이 정상적으로 시작되고
    실패 이벤트 없이 끝났다면, 슬롯은 곧바로 reusable이 아니라 "공식 완료를 기다리는 작업 결과"가
    된다. 이후 official completion이 작업 요약/검증 결과를 기록하고 distributor queue에 넘긴다.

    `saw_failed_before_turn_started`, `saw_failed_event`, `terminal_failure_observed`를 나눠 받는
    것은 스트림 이벤트의 실패 시점이 슬롯 정리 정책을 바꾸기 때문이다.
    */
    pub fn finalize_stream_completion(
        &self,
        workspace_directory: &str,
        saw_turn_started: bool,
        saw_failed_before_turn_started: bool,
        saw_failed_event: bool,
        terminal_failure_observed: bool,
    ) -> ParallelTurnStreamCompletionOutcome {
        self.finalize_stream_completion_inner(
            workspace_directory,
            None,
            saw_turn_started,
            saw_failed_before_turn_started,
            saw_failed_event,
            terminal_failure_observed,
        )
    }

    pub(crate) fn finalize_stream_completion_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        saw_turn_started: bool,
        saw_failed_before_turn_started: bool,
        saw_failed_event: bool,
        terminal_failure_observed: bool,
    ) -> ParallelTurnStreamCompletionOutcome {
        self.finalize_stream_completion_inner(
            &expected_lease.worktree_path,
            Some(expected_lease),
            saw_turn_started,
            saw_failed_before_turn_started,
            saw_failed_event,
            terminal_failure_observed,
        )
    }

    fn finalize_stream_completion_inner(
        &self,
        workspace_directory: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
        saw_turn_started: bool,
        saw_failed_before_turn_started: bool,
        saw_failed_event: bool,
        terminal_failure_observed: bool,
    ) -> ParallelTurnStreamCompletionOutcome {
        if should_release_unstarted_slot_lease(
            saw_turn_started,
            saw_failed_before_turn_started,
            terminal_failure_observed,
        ) {
            /*
            Startup failure release is intentionally narrow. It only runs before
            TurnStarted because after that point the slot worktree may contain
            meaningful user-visible changes or failure evidence for inspection.
            */
            let transition = match expected_lease {
                Some(expected) => self
                    .parallel_mode_service
                    .release_workspace_slot_lease_after_failed_start_for_lease(expected),
                None => self
                    .parallel_mode_service
                    .release_workspace_slot_lease_after_failed_start(workspace_directory),
            };
            return match transition {
                Ok(Some(lease)) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(format!(
                        "slot lease released after startup failure / slot: {} / agent: {}",
                        lease.slot_id, lease.agent_id
                    )),
                    invalidate_supervisor_snapshot: true,
                },
                Ok(None) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: None,
                    invalidate_supervisor_snapshot: false,
                },
                Err(error) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(format!(
                        "slot lease release failed after startup failure: {error}"
                    )),
                    invalidate_supervisor_snapshot: false,
                },
            };
        }
        if should_mark_cleanup_pending_after_success(
            saw_turn_started,
            saw_failed_event,
            terminal_failure_observed,
        ) {
            // Successful leased turns now wait for post-turn official completion/distributor
            // orchestration before cleanup and slot return.
            return ParallelTurnStreamCompletionOutcome {
                runtime_notice: None,
                invalidate_supervisor_snapshot: false,
            };
        }

        if should_promote_missing_turn_started_before_success(
            saw_turn_started,
            saw_failed_event,
            terminal_failure_observed,
        ) {
            let transition = match expected_lease {
                Some(expected) => self
                    .parallel_mode_service
                    .mark_workspace_slot_running_for_lease(expected),
                None => self
                    .parallel_mode_service
                    .mark_workspace_slot_running(workspace_directory),
            };
            return match transition {
                Ok(Some(lease)) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(format!(
                        "slot lease running transition inferred from terminal completion / slot: {} / agent: {}",
                        lease.slot_id, lease.agent_id
                    )),
                    invalidate_supervisor_snapshot: true,
                },
                Ok(None) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: None,
                    invalidate_supervisor_snapshot: false,
                },
                Err(error) => ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(format!(
                        "slot lease running transition could not be inferred from terminal completion: {error}"
                    )),
                    invalidate_supervisor_snapshot: false,
                },
            };
        }

        ParallelTurnStreamCompletionOutcome {
            runtime_notice: None,
            invalidate_supervisor_snapshot: false,
        }
    }

    /*
    official completion은 슬롯 agent가 낸 결과를 planning authority의 언어로 다시 정리하는
    단계다. 여기서는 완료 turn id, refresh 순서, 최종 답변, 검증 요약을 `ParallelModeService`에
    넘겨 슬롯 lease를 "공식 완료 진행 중" 상태로 전환한다. 이 단계를 거쳐야 distributor가 어떤
    task 결과를 어떤 순서로 통합할지 안정적으로 판단할 수 있다.
    */
    pub fn begin_official_completion(
        &self,
        workspace_directory: &str,
        completed_turn_id: &str,
        refresh_order: Option<u64>,
        latest_main_reply: Option<&str>,
        validation_summary: Option<&str>,
    ) -> Result<Option<ParallelModeOfficialCompletionReport>, String> {
        self.parallel_mode_service
            .begin_workspace_official_completion(
                workspace_directory,
                completed_turn_id,
                refresh_order,
                latest_main_reply,
                validation_summary,
                None,
            )
    }

    pub(crate) fn begin_official_completion_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        completed_turn_id: &str,
        refresh_order: Option<u64>,
        latest_main_reply: Option<&str>,
        validation_summary: Option<&str>,
    ) -> Result<Option<ParallelModeOfficialCompletionReport>, String> {
        self.parallel_mode_service
            .begin_workspace_official_completion_for_lease(
                expected_lease,
                completed_turn_id,
                refresh_order,
                latest_main_reply,
                validation_summary,
                None,
            )
    }
    pub fn reserve_official_completion_refresh_order(
        &self,
        workspace_directory: &str,
    ) -> Result<Option<u64>, String> {
        self.parallel_mode_service
            .reserve_workspace_official_completion_refresh_order(workspace_directory)
    }

    pub(crate) fn reserve_official_completion_refresh_order_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<Option<u64>, String> {
        self.parallel_mode_service
            .reserve_workspace_official_completion_refresh_order_for_lease(expected_lease)
    }
    pub(crate) fn prepare_host_owned_worker_commit(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Result<ParallelWorkerCommitOutcome, String> {
        self.parallel_mode_service
            .prepare_workspace_worker_commit(expected_lease)
    }
    pub fn mark_official_completion_failed(&self, workspace_directory: &str, failure_detail: &str) {
        let _ = self
            .parallel_mode_service
            .mark_workspace_official_completion_failed(workspace_directory, failure_detail);
    }
    pub(crate) fn mark_official_completion_failed_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        failure_detail: &str,
    ) {
        let _ = self
            .parallel_mode_service
            .mark_workspace_official_completion_failed_for_lease(expected_lease, failure_detail);
    }
    pub fn mark_official_completion_refreshing(&self, workspace_directory: &str) -> Option<String> {
        match self
            .parallel_mode_service
            .mark_workspace_official_completion_refreshing(workspace_directory)
        {
            Ok(_) => None,
            Err(error) => Some(format!(
                "official completion refreshing state could not be recorded: {error}"
            )),
        }
    }

    pub(crate) fn mark_official_completion_refreshing_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Option<String> {
        match self
            .parallel_mode_service
            .mark_workspace_official_completion_refreshing_for_lease(expected_lease)
        {
            Ok(_) => None,
            Err(error) => Some(format!(
                "official completion refreshing state could not be recorded: {error}"
            )),
        }
    }

    /*
    official completion이 성공하면 슬롯 결과는 commit-ready 상태가 되고, distributor queue에
    들어간다. 즉, agent worktree에서 나온 변경을 바로 통합하지 않고 queue record로 한 번
    직렬화한다. 병렬 agent는 여러 개지만 prerelease 통합은 한 줄로 처리해야 충돌과 순서 의존성을
    관리할 수 있기 때문이다.

    마지막 `run_orchestrator_tick`은 방금 enqueue한 결과를 계기로 queue 처리를 한 번 더 진행하게
    한다. 이 덕분에 사용자가 별도 새로고침을 누르지 않아도 공식 완료 직후 통합 오케스트레이션이
    이어질 수 있다.
    */
    pub fn finalize_official_completion_success(
        &self,
        workspace_directory: &str,
        authority_refresh_outcome: &str,
    ) -> Vec<String> {
        self.finalize_official_completion_success_inner(
            workspace_directory,
            None,
            authority_refresh_outcome,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn finalize_official_completion_success_for_epoch(
        &self,
        workspace_directory: &str,
        planning_workspace_directory: &str,
        epoch_id: u64,
        authority_refresh_outcome: &str,
    ) -> Vec<String> {
        self.finalize_official_completion_success_inner(
            workspace_directory,
            None,
            authority_refresh_outcome,
            Some((planning_workspace_directory, epoch_id)),
        )
    }

    pub(crate) fn finalize_official_completion_success_for_epoch_and_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        planning_workspace_directory: &str,
        epoch_id: u64,
        authority_refresh_outcome: &str,
    ) -> Vec<String> {
        self.finalize_official_completion_success_inner(
            &expected_lease.worktree_path,
            Some(expected_lease),
            authority_refresh_outcome,
            Some((planning_workspace_directory, epoch_id)),
        )
    }

    fn finalize_official_completion_success_inner(
        &self,
        workspace_directory: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
        authority_refresh_outcome: &str,
        automation_epoch: Option<(&str, u64)>,
    ) -> Vec<String> {
        let preparation = self.prepare_official_completion_success_inner(
            workspace_directory,
            expected_lease,
            authority_refresh_outcome,
            automation_epoch,
        );
        let mut notices = preparation.notices;
        if preparation.should_run_delivery_tick {
            notices.extend(self.run_official_completion_delivery_tick_inner(
                workspace_directory,
                automation_epoch,
            ));
        }
        notices
    }

    pub(crate) fn mark_official_completion_success_for_post_turn(
        &self,
        workspace_directory: &str,
        authority_refresh_outcome: &str,
    ) -> Vec<String> {
        self.parallel_mode_service
            .mark_workspace_commit_ready(workspace_directory, authority_refresh_outcome)
            .err()
            .map(|error| {
                format!("commit-ready state could not be recorded after official refresh: {error}")
            })
            .into_iter()
            .collect()
    }

    pub(crate) fn run_official_completion_delivery_tick_for_post_turn(
        &self,
        workspace_directory: &str,
        planning_workspace_directory: &str,
        epoch_id: u64,
        continuation_permit: &PostTurnContinuationPermit,
    ) -> Vec<String> {
        let Some(permit) = self
            .automation_permit(planning_workspace_directory, epoch_id)
            .map(|permit| permit.with_continuation_permit(continuation_permit.clone()))
        else {
            return vec![
                "parallel result remains queued because no guarded automation epoch is available"
                    .to_string(),
            ];
        };
        let mut notices = match self
            .parallel_mode_service
            .enqueue_workspace_commit_ready_result_guarded(workspace_directory, &permit)
        {
            Ok(Some(item)) => vec![format!(
                "commit-ready result entered the distributor queue / agent: {} / task: {} / state: {}",
                item.source_agent,
                item.task_title,
                item.queue_state.label()
            )],
            Ok(None) if !permit.is_active() => {
                return vec![
                    "parallel result was not enqueued because its automation epoch was closed"
                        .to_string(),
                ];
            }
            Ok(None) => Vec::new(),
            Err(error) => {
                return vec![format!(
                    "distributor enqueue failed after official refresh: {error}"
                )];
            }
        };
        if !permit.is_active() {
            notices.push(
                "parallel result remains queued because its automation epoch was closed"
                    .to_string(),
            );
            return notices;
        }
        match self.parallel_mode_service.run_orchestrator_tick_guarded(
            workspace_directory,
            ParallelModeOrchestratorTrigger::PlanningRefreshCompleted,
            &permit,
        ) {
            Ok(tick_result) => notices.extend(tick_result.notices),
            Err(error) => notices.push(format!(
                "orchestrator tick failed after official refresh: {error}"
            )),
        }
        notices
    }

    fn prepare_official_completion_success_inner(
        &self,
        workspace_directory: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
        authority_refresh_outcome: &str,
        automation_epoch: Option<(&str, u64)>,
    ) -> ParallelOfficialCompletionSuccessPreparation {
        let mut notices = Vec::new();
        let automation_is_active = || {
            automation_epoch.is_none_or(|(planning_workspace_directory, epoch_id)| {
                self.automation_epoch_is_active(planning_workspace_directory, epoch_id)
            })
        };
        /*
        mark_workspace_commit_ready updates the session ledger before enqueue.
        Even if this write fails, enqueue is still attempted because the queue
        record may be recoverable from the lease/session state and should surface
        its own failure separately.
        */
        let commit_ready = match expected_lease {
            Some(expected) => self
                .parallel_mode_service
                .mark_workspace_commit_ready_for_lease(expected, authority_refresh_outcome),
            None => self
                .parallel_mode_service
                .mark_workspace_commit_ready(workspace_directory, authority_refresh_outcome),
        };
        if let Err(error) = commit_ready {
            notices.push(format!(
                "commit-ready state could not be recorded after official refresh: {error}"
            ));
        }
        if !automation_is_active() {
            notices.push(
                "parallel result was durably marked commit-ready and awaits a new automation epoch"
                    .to_string(),
            );
            return ParallelOfficialCompletionSuccessPreparation {
                notices,
                should_run_delivery_tick: false,
            };
        }
        let enqueue = match expected_lease {
            Some(expected) => self
                .parallel_mode_service
                .enqueue_workspace_commit_ready_result_for_lease(expected),
            None => self
                .parallel_mode_service
                .enqueue_workspace_commit_ready_result(workspace_directory),
        };
        match enqueue {
            Ok(Some(item)) => notices.push(format!(
                "commit-ready result entered the distributor queue / agent: {} / task: {} / state: {}",
                item.source_agent,
                item.task_title,
                item.queue_state.label()
            )),
            Ok(None) => {}
            Err(error) => {
                /*
                Without an enqueue record there is no queue head for the
                orchestrator to process, so stop here and preserve the enqueue
                error as the actionable notice.
                */
                notices.push(format!(
                    "distributor enqueue failed after official refresh: {error}"
                ));
                return ParallelOfficialCompletionSuccessPreparation {
                    notices,
                    should_run_delivery_tick: false,
                };
            }
        }
        if !automation_is_active() {
            notices.push(
                "parallel result remains queued because its automation epoch was closed"
                    .to_string(),
            );
            return ParallelOfficialCompletionSuccessPreparation {
                notices,
                should_run_delivery_tick: false,
            };
        }
        ParallelOfficialCompletionSuccessPreparation {
            notices,
            should_run_delivery_tick: true,
        }
    }

    fn run_official_completion_delivery_tick_inner(
        &self,
        workspace_directory: &str,
        automation_epoch: Option<(&str, u64)>,
    ) -> Vec<String> {
        let mut notices = Vec::new();
        let tick_result = match automation_epoch {
            None => self.parallel_mode_service.run_orchestrator_tick(
                workspace_directory,
                ParallelModeOrchestratorTrigger::PlanningRefreshCompleted,
            ),
            Some((planning_workspace_directory, epoch_id)) => {
                let Some(permit) = self.automation_permit(planning_workspace_directory, epoch_id)
                else {
                    return vec![
                        "parallel result remains queued because no guarded automation epoch is available"
                            .to_string(),
                    ];
                };
                self.parallel_mode_service.run_orchestrator_tick_guarded(
                    workspace_directory,
                    ParallelModeOrchestratorTrigger::PlanningRefreshCompleted,
                    &permit,
                )
            }
        };
        match tick_result {
            Ok(tick_result) => notices.extend(tick_result.notices),
            Err(error) => notices.push(format!(
                "orchestrator tick failed after official refresh: {error}"
            )),
        }
        notices
    }
}
fn should_release_unstarted_slot_lease(
    saw_turn_started: bool,
    saw_failed_before_turn_started: bool,
    terminal_failure_observed: bool,
) -> bool {
    /*
    Release is based on evidence that the agent never began useful work. A direct
    pre-start failure is enough, and a terminal failure with no TurnStarted event
    covers transports that report only the final failure.
    */
    saw_failed_before_turn_started || (!saw_turn_started && terminal_failure_observed)
}
fn should_mark_cleanup_pending_after_success(
    saw_turn_started: bool,
    saw_failed_event: bool,
    terminal_failure_observed: bool,
) -> bool {
    /*
    A successful running turn does not immediately return the slot. It becomes a
    candidate for official completion/distributor handoff only when the stream
    both started and ended without any failure signal.
    */
    saw_turn_started && !saw_failed_event && !terminal_failure_observed
}
fn should_promote_missing_turn_started_before_success(
    saw_turn_started: bool,
    saw_failed_event: bool,
    terminal_failure_observed: bool,
) -> bool {
    /*
    TurnCompleted without TurnStarted is an event-ordering anomaly, but terminal
    success still proves the worker executed. Promote the slot to Running so
    official completion can capture the result instead of leaving a Leased slot
    orphaned.
    */
    !saw_turn_started && !saw_failed_event && !terminal_failure_observed
}
#[cfg(test)]
mod tests {
    use super::{
        ParallelModeTurnService, ParallelTurnSlotLeaseHandoff,
        should_mark_cleanup_pending_after_success,
        should_promote_missing_turn_started_before_success, should_release_unstarted_slot_lease,
        slot_lease_request_from_handoff,
    };
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::application::port::outbound::github_automation_port::{
        GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
        GithubRepositoryVisibility,
    };
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::parallel_mode::{
        ParallelModeAutomationGuard, ParallelModeService,
    };
    use crate::domain::parallel_mode::{
        ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
        ParallelModeSlotLeaseRequest,
    };
    use std::fs;
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    struct TempGitWorkspace {
        root: String,
        origin_root: String,
    }
    impl TempGitWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&root).expect("temp git workspace should be created");
            run_git(&root, &["init"]);
            run_git(&root, &["config", "user.name", "RefinedStone"]);
            run_git(&root, &["config", "user.email", "chem.en.9273@gmail.com"]);
            fs::write(root.join("README.md"), "temp repo\n")
                .expect("temp git workspace seed file should write");
            run_git(&root, &["add", "README.md"]);
            run_git(&root, &["commit", "-m", "Initial commit"]);
            run_git(&root, &["branch", "akra"]);
            run_git(&root, &["branch", "prerelease"]);
            let origin = root.with_extension("origin.git");
            let origin_path = origin
                .to_str()
                .expect("temp origin path should be valid utf-8");
            run_git(&root, &["init", "--bare", "-q", origin_path]);
            run_git(&root, &["remote", "add", "origin", origin_path]);
            run_git(&root, &["push", "-q", "-u", "origin", "prerelease"]);

            Self {
                root: root.display().to_string(),
                origin_root: origin.display().to_string(),
            }
        }
    }
    impl Drop for TempGitWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
            let _ = fs::remove_dir_all(&self.origin_root);
        }
    }
    fn create_temp_directory(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&root).expect("temp directory should be created");
        root.display().to_string()
    }
    fn run_git(repo_root: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .status()
            .expect("git command should launch");
        assert!(
            status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
    }
    #[derive(Debug)]
    struct LocalGithubAutomationPort;
    impl GithubAutomationPort for LocalGithubAutomationPort {
        fn inspect_capabilities(&self, _repo_root: &str) -> GithubAutomationCapabilities {
            let ready = |key| {
                ParallelModeCapabilitySnapshot::new(
                    key,
                    ParallelModeCapabilityState::Ready,
                    "test capability ready",
                    None,
                )
            };
            GithubAutomationCapabilities::new(
                ready(ParallelModeCapabilityKey::PushRemote),
                ready(ParallelModeCapabilityKey::GhBinary),
                ready(ParallelModeCapabilityKey::GhAuth),
            )
        }
        fn repository_identity(&self, _repo_root: &str) -> anyhow::Result<String> {
            Ok("RefinedStone/codex-exec-loop".to_string())
        }
        fn repository_visibility(
            &self,
            _repo_root: &str,
        ) -> anyhow::Result<GithubRepositoryVisibility> {
            Ok(GithubRepositoryVisibility::Private)
        }
        fn repository_identity_for_push_url(
            &self,
            repo_root: &str,
            _push_remote: &str,
            _credential_redacted_push_url: &str,
        ) -> anyhow::Result<String> {
            self.repository_identity(repo_root)
        }
        fn repository_visibility_for_push_url(
            &self,
            repo_root: &str,
            _push_remote: &str,
            _credential_redacted_push_url: &str,
        ) -> anyhow::Result<GithubRepositoryVisibility> {
            self.repository_visibility(repo_root)
        }
        fn credential_redacted_push_url_for_remote(
            &self,
            repo_root: &str,
            push_remote: &str,
        ) -> anyhow::Result<String> {
            let output = Command::new("git")
                .current_dir(repo_root)
                .args(["remote", "get-url", "--push", push_remote])
                .output()?;
            anyhow::ensure!(
                output.status.success(),
                "test push remote URL is unavailable"
            );
            Ok(String::from_utf8(output.stdout)?.trim().to_string())
        }
        fn remote_branch_names_for_prefix_for_delivery_target(
            &self,
            repo_root: &str,
            _push_remote: &str,
            credential_redacted_push_url: &str,
            branch_prefix: &str,
        ) -> anyhow::Result<Vec<String>> {
            let remote_pattern = format!("refs/heads/{branch_prefix}*");
            let output = Command::new("git")
                .current_dir(repo_root)
                .args([
                    "ls-remote",
                    "--heads",
                    credential_redacted_push_url,
                    remote_pattern.as_str(),
                ])
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()?;
            anyhow::ensure!(
                output.status.success(),
                "test remote branch listing failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            let stdout = String::from_utf8(output.stdout)?;
            stdout
                .lines()
                .map(|line| {
                    let (_, remote_ref) = line
                        .split_once(char::is_whitespace)
                        .ok_or_else(|| anyhow::anyhow!("test remote branch row is malformed"))?;
                    remote_ref
                        .trim()
                        .strip_prefix("refs/heads/")
                        .map(str::to_string)
                        .ok_or_else(|| anyhow::anyhow!("test remote branch ref is malformed"))
                })
                .collect()
        }
        fn fetch_branch_to_tracking_ref_for_delivery_target(
            &self,
            repo_root: &str,
            _push_remote: &str,
            credential_redacted_push_url: &str,
            branch_name: &str,
            tracking_ref: &str,
        ) -> anyhow::Result<String> {
            let refspec = format!("+refs/heads/{branch_name}:{tracking_ref}");
            let status = Command::new("git")
                .current_dir(repo_root)
                .args(["fetch", "--quiet", credential_redacted_push_url, &refspec])
                .status()?;
            anyhow::ensure!(status.success(), "test frozen-target fetch failed");
            let output = Command::new("git")
                .current_dir(repo_root)
                .args(["rev-parse", tracking_ref])
                .output()?;
            anyhow::ensure!(
                output.status.success(),
                "test tracking ref could not be resolved"
            );
            Ok(String::from_utf8(output.stdout)?.trim().to_string())
        }
        fn push_branch(
            &self,
            _repo_root: &str,
            _branch_name: &str,
            _force_with_lease: bool,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn ensure_pull_request(
            &self,
            _repo_root: &str,
            base_branch: &str,
            head_branch: &str,
            _title: &str,
            _body: &str,
        ) -> anyhow::Result<GithubAutomationPullRequest> {
            Ok(GithubAutomationPullRequest::new(
                1,
                "https://example.invalid/pr/1",
                "OPEN",
                base_branch,
                head_branch,
                false,
            ))
        }
        fn inspect_pull_request(
            &self,
            _repo_root: &str,
            pr_number: u64,
        ) -> anyhow::Result<GithubAutomationPullRequest> {
            Ok(GithubAutomationPullRequest::new(
                pr_number,
                "https://example.invalid/pr/1",
                "OPEN",
                "prerelease",
                "akra-agent/test",
                false,
            ))
        }
        fn push_integration_branch(
            &self,
            _repo_root: &str,
            _branch_name: &str,
            _expected_old_commit_sha: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        fn close_pull_request(&self, _repo_root: &str, _pr_number: u64) -> anyhow::Result<()> {
            Ok(())
        }
    }
    fn test_parallel_mode_service() -> ParallelModeService {
        ParallelModeService::new(
            Arc::new(SqlitePlanningAuthorityAdapter::new()),
            Arc::new(LocalGithubAutomationPort),
            Arc::new(GitParallelModeRuntimeAdapter::new()),
        )
    }
    #[test]
    fn guarded_turn_service_tracks_active_epoch_and_cancellation() {
        let guard = ParallelModeAutomationGuard::default();
        let service = ParallelModeTurnService::new(test_parallel_mode_service())
            .with_automation_guard(guard.clone());

        assert!(!service.automation_epoch_is_active("/workspace", 7));
        guard.activate("/workspace", 7);
        assert!(service.automation_epoch_is_active("/workspace", 7));
        guard.cancel("/workspace");
        assert!(!service.automation_epoch_is_active("/workspace", 7));
    }
    #[test]
    fn closed_epoch_still_persists_successful_official_refresh_as_commit_ready() {
        let workspace = TempGitWorkspace::new("parallel-closed-epoch-commit-ready");
        let parallel_service = test_parallel_mode_service();
        parallel_service
            .reset_pool_on_parallel_initial_setup_report(&workspace.root)
            .expect("pool should initialize");
        let lease = parallel_service
            .acquire_slot_lease(
                &workspace.root,
                ParallelModeSlotLeaseRequest::from_task_identity(
                    "task-closed-epoch",
                    "Preserve completed result",
                ),
            )
            .expect("slot should lease");
        parallel_service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should become running");
        parallel_service
            .mark_workspace_official_completion_refreshing(&lease.worktree_path)
            .expect("official refresh state should persist");

        let guard = ParallelModeAutomationGuard::default();
        guard.activate(workspace.root.clone(), 7);
        let turn_service = ParallelModeTurnService::new(parallel_service.clone())
            .with_automation_guard(guard.clone());
        guard.cancel(&workspace.root);

        let notices = turn_service.finalize_official_completion_success_for_epoch(
            &lease.worktree_path,
            &workspace.root,
            7,
            "official ledger refresh succeeded",
        );

        assert!(
            notices
                .iter()
                .any(|notice| notice.contains("durably marked commit-ready"))
        );
        let snapshot = parallel_service.build_passive_supervisor_snapshot(&workspace.root, None);
        let detail = snapshot
            .detail
            .session
            .as_ref()
            .expect("commit-ready session detail should remain durable");
        assert_eq!(detail.state_label, "commit_ready");
        assert_eq!(snapshot.distributor.queue_depth(), 0);
    }
    #[test]
    fn startup_failure_requests_unstarted_slot_release() {
        assert!(should_release_unstarted_slot_lease(false, true, true));
    }
    #[test]
    fn running_turn_does_not_request_unstarted_slot_release() {
        assert!(!should_release_unstarted_slot_lease(true, false, true));
    }
    #[test]
    fn successful_running_turn_is_cleanup_candidate() {
        assert!(should_mark_cleanup_pending_after_success(
            true, false, false
        ));
    }
    #[test]
    fn terminal_success_without_turn_started_promotes_running_state() {
        assert!(should_promote_missing_turn_started_before_success(
            false, false, false
        ));
        assert!(!should_promote_missing_turn_started_before_success(
            true, false, false
        ));
        assert!(!should_promote_missing_turn_started_before_success(
            false, true, false
        ));
    }
    #[test]
    fn slot_lease_handoff_maps_to_domain_request_inside_turn_service_boundary() {
        let handoff = ParallelTurnSlotLeaseHandoff::new(
            " task-r1-turn-bridge ",
            "Move slot lease request out of TUI",
        );
        let request = slot_lease_request_from_handoff(&handoff);

        assert_eq!(request.task_id, "task-r1-turn-bridge");
        assert_eq!(request.task_title, "Move slot lease request out of TUI");
        assert_eq!(request.agent_id, "agent-task-r1-turn-bridge");
        assert_eq!(request.task_slug, "task-r1-turn-bridge");
    }
    #[test]
    fn stream_lifecycle_releases_unstarted_slot_after_terminal_failure() {
        let workspace = TempGitWorkspace::new("parallel-stream-lifecycle-release");
        let parallel_service = test_parallel_mode_service();
        parallel_service
            .reset_pool_on_parallel_initial_setup_report(&workspace.root)
            .expect("pool should initialize for stream lifecycle test");
        let lease = parallel_service
            .acquire_slot_lease(
                &workspace.root,
                ParallelModeSlotLeaseRequest::from_task_identity(
                    "task-stream-lifecycle",
                    "Stream lifecycle release",
                ),
            )
            .expect("slot lease should be acquired");
        let turn_service = ParallelModeTurnService::new(parallel_service.clone());
        let mut lifecycle = turn_service.stream_lifecycle(lease.worktree_path);

        let event_outcome = lifecycle.observe_event(&ConversationStreamEvent::Failed {
            message: "startup failed".to_string(),
        });
        assert!(event_outcome.should_stop_stream_forwarding);
        assert!(!event_outcome.invalidate_supervisor_snapshot);
        assert!(event_outcome.runtime_notice.is_none());

        let completion = lifecycle.finalize_after_stream_completion(true);
        assert!(completion.invalidate_supervisor_snapshot);
        assert!(completion.runtime_notice.as_deref().is_some_and(|notice| {
            notice.contains("slot lease released after startup failure")
        }));
        let supervisor = parallel_service.build_supervisor_snapshot(&workspace.root, true, None);
        assert_eq!(supervisor.pool.leased_slots, 0);
    }
    #[test]
    fn turn_started_without_slot_lease_keeps_snapshot_steady() {
        let workspace = TempGitWorkspace::new("parallel-turn-no-lease");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());
        let outcome = service.sync_stream_event(
            &workspace.root,
            &ConversationStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
            },
        );

        assert!(!outcome.invalidate_supervisor_snapshot);
        assert!(outcome.turn_started_observed);
        assert!(outcome.runtime_notice.is_none());
    }
    #[test]
    fn thread_prepared_without_slot_lease_keeps_snapshot_steady() {
        let workspace = TempGitWorkspace::new("parallel-thread-prepared-no-lease");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());
        let outcome = service.sync_stream_event(
            &workspace.root,
            &ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-1".to_string(),
                title: "Temp".to_string(),
                cwd: workspace.root.clone(),
            },
        );

        assert!(!outcome.invalidate_supervisor_snapshot);
        assert!(!outcome.turn_started_observed);
        assert!(outcome.runtime_notice.is_none());
    }
    #[test]
    fn official_completion_refreshing_failure_becomes_runtime_notice() {
        let workspace = create_temp_directory("parallel-turn-refresh-failure");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());
        let notice = service.mark_official_completion_refreshing(&workspace);

        assert!(notice.as_deref().is_some_and(|value| {
            value.contains("official completion refreshing state could not be recorded")
        }));
        let _ = fs::remove_dir_all(workspace);
    }
    #[test]
    fn official_completion_finalize_surfaces_commit_ready_transition_failure() {
        let workspace = create_temp_directory("parallel-turn-commit-ready-failure");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());
        let notices = service
            .finalize_official_completion_success(&workspace, "official ledger refresh succeeded");

        assert!(notices.iter().any(|notice| {
            notice.contains("commit-ready state could not be recorded after official refresh")
        }));
        assert!(notices.iter().any(|notice| {
            notice.contains("distributor enqueue failed after official refresh")
        }));
        let _ = fs::remove_dir_all(workspace);
    }
}
