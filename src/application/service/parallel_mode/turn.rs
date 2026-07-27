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

#[derive(Debug, Clone, Copy)]
struct ParallelTurnStreamCompletionEvidence<'a> {
    saw_turn_started: bool,
    saw_failed_before_turn_started: bool,
    saw_failed_event: bool,
    terminal_failure_observed: bool,
    terminal_failure_detail: Option<&'a str>,
    terminal_failure_persisted: bool,
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
    terminal_failure_detail: Option<String>,
    terminal_failure_persisted: bool,
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
            .field("terminal_failure_detail", &self.terminal_failure_detail)
            .field(
                "terminal_failure_persisted",
                &self.terminal_failure_persisted,
            )
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
            terminal_failure_detail: None,
            terminal_failure_persisted: false,
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
            ConversationStreamEvent::TurnTerminal { .. } | ConversationStreamEvent::Failed { .. }
        );
        let terminal_failure = matches!(event, ConversationStreamEvent::Failed { .. })
            || matches!(
                event,
                ConversationStreamEvent::TurnTerminal { receipt }
                    if !receipt.is_completed_and_confirmed()
            );
        if terminal_failure {
            self.saw_failed_event = true;
            self.terminal_failure_detail = Some(stream_terminal_failure_detail(event));
            if !self.saw_turn_started {
                self.saw_failed_before_turn_started = true;
            }
        }

        let mut runtime_notice = outcome.runtime_notice;
        let mut invalidate_supervisor_snapshot = outcome.invalidate_supervisor_snapshot;
        if terminal_failure && self.saw_turn_started {
            let failure_detail = self
                .terminal_failure_detail
                .as_deref()
                .expect("terminal failure detail should accompany a terminal failure");
            let (failure_outcome, persisted) = self.turn_service.record_running_turn_failure_inner(
                &self.workspace_directory,
                self.expected_lease.as_ref(),
                failure_detail,
            );
            runtime_notice = failure_outcome.runtime_notice;
            invalidate_supervisor_snapshot |= failure_outcome.invalidate_supervisor_snapshot;
            self.terminal_failure_persisted |= persisted;
        }

        ParallelTurnStreamLifecycleEventOutcome {
            runtime_notice,
            invalidate_supervisor_snapshot,
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
            ParallelTurnStreamCompletionEvidence {
                saw_turn_started: self.saw_turn_started,
                saw_failed_before_turn_started: self.saw_failed_before_turn_started,
                saw_failed_event: self.saw_failed_event,
                terminal_failure_observed,
                terminal_failure_detail: self.terminal_failure_detail.as_deref(),
                terminal_failure_persisted: self.terminal_failure_persisted,
            },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelOfficialCompletionDurableProof {
    CommitReadyRecord,
    CommitReadyAwaitingAutomation,
    DistributorQueueRecord,
}

impl ParallelOfficialCompletionDurableProof {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::CommitReadyRecord => "commit_ready_record",
            Self::CommitReadyAwaitingAutomation => "commit_ready_awaiting_automation",
            Self::DistributorQueueRecord => "distributor_queue_record",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelOfficialCompletionFinalizeFailureStage {
    CommitReadyPersistence,
    DistributorEnqueue,
}

impl ParallelOfficialCompletionFinalizeFailureStage {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::CommitReadyPersistence => "commit_ready_persistence",
            Self::DistributorEnqueue => "distributor_enqueue",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParallelOfficialCompletionFinalizeOutcome {
    Durable {
        notices: Vec<String>,
        proof: ParallelOfficialCompletionDurableProof,
        expected_lease: Box<ParallelModeSlotLeaseSnapshot>,
    },
    Failed {
        notices: Vec<String>,
        stage: ParallelOfficialCompletionFinalizeFailureStage,
    },
}

enum ParallelOfficialCompletionSuccessPreparation {
    Durable {
        notices: Vec<String>,
        proof: ParallelOfficialCompletionDurableProof,
        expected_lease: Box<ParallelModeSlotLeaseSnapshot>,
        should_run_delivery_tick: bool,
    },
    Failed {
        notices: Vec<String>,
        stage: ParallelOfficialCompletionFinalizeFailureStage,
    },
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
        if let ConversationStreamEvent::ThreadPrepared { thread_id, cwd, .. } = event {
            if cwd != workspace_directory {
                return ParallelTurnStreamEventOutcome {
                    runtime_notice: Some(
                        "slot lease thread-prepared cwd did not match the expected worktree"
                            .to_string(),
                    ),
                    invalidate_supervisor_snapshot: false,
                    turn_started_observed: false,
                };
            }
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
            ParallelTurnStreamCompletionEvidence {
                saw_turn_started,
                saw_failed_before_turn_started,
                saw_failed_event,
                terminal_failure_observed,
                terminal_failure_detail: None,
                terminal_failure_persisted: false,
            },
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
            ParallelTurnStreamCompletionEvidence {
                saw_turn_started,
                saw_failed_before_turn_started,
                saw_failed_event,
                terminal_failure_observed,
                terminal_failure_detail: None,
                terminal_failure_persisted: false,
            },
        )
    }

    pub(crate) fn settle_unexpected_worker_panic_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
    ) -> Vec<String> {
        let mut notices = Vec::new();
        /*
         * An outer worker panic loses the local stream evidence, and the isolated
         * app-server worker may still be unwinding. Releasing the worktree here
         * could discard or race user-visible changes. Fence the exact lease
         * generation, preserve it as Running, then persist a failed session detail
         * so supervisor recovery sees a terminal state instead of a live zombie.
         */
        match self
            .parallel_mode_service
            .mark_workspace_slot_running_for_lease(expected_lease)
        {
            Ok(Some(_)) => {}
            Ok(None) => notices.push(
                "parallel worker panic settlement skipped because the captured slot lease no longer exists"
                    .to_string(),
            ),
            Err(error) => notices.push(format!(
                "parallel worker panic could not fence the captured slot lease: {error}"
            )),
        }

        let (failure, _) = self.record_running_turn_failure_inner(
            &expected_lease.worktree_path,
            Some(expected_lease),
            "parallel worker exited unexpectedly before durable stream settlement",
        );
        if let Some(notice) = failure.runtime_notice {
            notices.push(notice);
        }
        notices
    }

    fn finalize_stream_completion_inner(
        &self,
        workspace_directory: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
        evidence: ParallelTurnStreamCompletionEvidence<'_>,
    ) -> ParallelTurnStreamCompletionOutcome {
        let ParallelTurnStreamCompletionEvidence {
            saw_turn_started,
            saw_failed_before_turn_started,
            saw_failed_event,
            terminal_failure_observed,
            terminal_failure_detail,
            terminal_failure_persisted,
        } = evidence;
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
        if saw_turn_started && (saw_failed_event || terminal_failure_observed) {
            if terminal_failure_persisted {
                return ParallelTurnStreamCompletionOutcome {
                    runtime_notice: None,
                    invalidate_supervisor_snapshot: false,
                };
            }
            let failure_detail = terminal_failure_detail.unwrap_or(
                "conversation stream ended without a confirmed successful terminal after TurnStarted",
            );
            return self
                .record_running_turn_failure_inner(
                    workspace_directory,
                    expected_lease,
                    failure_detail,
                )
                .0;
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

    fn record_running_turn_failure_inner(
        &self,
        workspace_directory: &str,
        expected_lease: Option<&ParallelModeSlotLeaseSnapshot>,
        failure_detail: &str,
    ) -> (ParallelTurnStreamCompletionOutcome, bool) {
        let transition = match expected_lease {
            Some(expected) => self
                .parallel_mode_service
                .mark_workspace_official_completion_failed_for_lease(expected, failure_detail),
            None => self
                .parallel_mode_service
                .mark_workspace_official_completion_failed(workspace_directory, failure_detail),
        };
        match transition {
            Ok(Some(detail)) => (
                ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(format!(
                        "running slot recorded failed after non-success terminal / session: {} / {}",
                        detail.session_key, failure_detail
                    )),
                    invalidate_supervisor_snapshot: true,
                },
                true,
            ),
            Ok(None) => (
                ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(
                        "running slot failure was not recorded because the expected lease generation no longer matched"
                            .to_string(),
                    ),
                    invalidate_supervisor_snapshot: true,
                },
                false,
            ),
            Err(error) => (
                ParallelTurnStreamCompletionOutcome {
                    runtime_notice: Some(format!(
                        "running slot failure could not be recorded after non-success terminal: {error}"
                    )),
                    invalidate_supervisor_snapshot: true,
                },
                false,
            ),
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
            .mark_official_completion_failed_for_lease_with_notice(expected_lease, failure_detail);
    }

    pub(crate) fn mark_official_completion_failed_for_lease_with_notice(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        failure_detail: &str,
    ) -> Option<String> {
        match self
            .parallel_mode_service
            .mark_workspace_official_completion_failed_for_lease(expected_lease, failure_detail)
        {
            Ok(Some(_)) => None,
            Ok(None) => Some(
                "official completion failure was not recorded because the captured slot lease generation is stale"
                    .to_string(),
            ),
            Err(error) => Some(format!(
                "official completion failure state could not be recorded: {error}"
            )),
        }
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
            Ok(Some(_)) => None,
            Ok(None) => Some(
                "official completion refreshing was not recorded because the captured slot lease generation is stale"
                    .to_string(),
            ),
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
    ) -> ParallelOfficialCompletionFinalizeOutcome {
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
    ) -> ParallelOfficialCompletionFinalizeOutcome {
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
    ) -> ParallelOfficialCompletionFinalizeOutcome {
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
    ) -> ParallelOfficialCompletionFinalizeOutcome {
        match self.prepare_official_completion_success_inner(
            workspace_directory,
            expected_lease,
            authority_refresh_outcome,
            automation_epoch,
        ) {
            ParallelOfficialCompletionSuccessPreparation::Durable {
                mut notices,
                proof,
                expected_lease,
                should_run_delivery_tick,
            } => {
                if should_run_delivery_tick {
                    notices.extend(self.run_official_completion_delivery_tick_inner(
                        workspace_directory,
                        automation_epoch,
                    ));
                }
                ParallelOfficialCompletionFinalizeOutcome::Durable {
                    notices,
                    proof,
                    expected_lease,
                }
            }
            ParallelOfficialCompletionSuccessPreparation::Failed { notices, stage } => {
                ParallelOfficialCompletionFinalizeOutcome::Failed { notices, stage }
            }
        }
    }

    pub(crate) fn mark_official_completion_success_for_post_turn_for_lease(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        authority_refresh_outcome: &str,
    ) -> ParallelOfficialCompletionFinalizeOutcome {
        match self
            .parallel_mode_service
            .mark_workspace_commit_ready_for_lease(expected_lease, authority_refresh_outcome)
        {
            Ok(Some(persistence)) => ParallelOfficialCompletionFinalizeOutcome::Durable {
                notices: persistence.notices,
                proof: ParallelOfficialCompletionDurableProof::CommitReadyRecord,
                expected_lease: Box::new(persistence.lease),
            },
            Ok(None) => ParallelOfficialCompletionFinalizeOutcome::Failed {
                notices: vec![
                    "commit-ready state was not recorded because the captured slot lease generation is stale"
                        .to_string(),
                ],
                stage: ParallelOfficialCompletionFinalizeFailureStage::CommitReadyPersistence,
            },
            Err(error) => ParallelOfficialCompletionFinalizeOutcome::Failed {
                notices: vec![format!(
                    "commit-ready state could not be recorded after official refresh: {error}"
                )],
                stage: ParallelOfficialCompletionFinalizeFailureStage::CommitReadyPersistence,
            },
        }
    }

    pub(crate) fn run_official_completion_delivery_tick_for_post_turn(
        &self,
        expected_lease: &ParallelModeSlotLeaseSnapshot,
        planning_workspace_directory: &str,
        epoch_id: u64,
        continuation_permit: &PostTurnContinuationPermit,
    ) -> ParallelOfficialCompletionFinalizeOutcome {
        let Some(permit) = self
            .automation_permit(planning_workspace_directory, epoch_id)
            .map(|permit| permit.with_continuation_permit(continuation_permit.clone()))
        else {
            return ParallelOfficialCompletionFinalizeOutcome::Durable {
                notices: vec![
                    "parallel result remains commit-ready because no guarded automation epoch is available"
                        .to_string(),
                ],
                proof: ParallelOfficialCompletionDurableProof::CommitReadyAwaitingAutomation,
                expected_lease: Box::new(expected_lease.clone()),
            };
        };
        let mut notices = match self
            .parallel_mode_service
            .enqueue_workspace_commit_ready_result_for_lease_guarded(expected_lease, &permit)
        {
            Ok(Some(item)) => vec![format!(
                "commit-ready result entered the distributor queue / agent: {} / task: {} / state: {}",
                item.source_agent,
                item.task_title,
                item.queue_state.label()
            )],
            Ok(None) if !permit.is_active() => {
                return ParallelOfficialCompletionFinalizeOutcome::Durable {
                    notices: vec![
                        "parallel result remains commit-ready because its automation epoch or post-turn continuation was closed"
                            .to_string(),
                    ],
                    proof: ParallelOfficialCompletionDurableProof::CommitReadyAwaitingAutomation,
                    expected_lease: Box::new(expected_lease.clone()),
                };
            }
            Ok(None) => {
                return ParallelOfficialCompletionFinalizeOutcome::Failed {
                    notices: vec![
                        "distributor enqueue did not persist a queue record after official refresh because the current running lease or commit-ready session no longer matched"
                            .to_string(),
                    ],
                    stage: ParallelOfficialCompletionFinalizeFailureStage::DistributorEnqueue,
                };
            }
            Err(error) => {
                return ParallelOfficialCompletionFinalizeOutcome::Failed {
                    notices: vec![
                        format!("distributor enqueue failed after official refresh: {error}"),
                        "the durable commit-ready result remains available for distributor enqueue recovery"
                            .to_string(),
                    ],
                    stage: ParallelOfficialCompletionFinalizeFailureStage::DistributorEnqueue,
                };
            }
        };
        if !permit.is_active() {
            notices.push(
                "parallel result remains queued because its automation epoch was closed"
                    .to_string(),
            );
            return ParallelOfficialCompletionFinalizeOutcome::Durable {
                notices,
                proof: ParallelOfficialCompletionDurableProof::DistributorQueueRecord,
                expected_lease: Box::new(expected_lease.clone()),
            };
        }
        match self.parallel_mode_service.run_orchestrator_tick_guarded(
            &expected_lease.worktree_path,
            ParallelModeOrchestratorTrigger::PlanningRefreshCompleted,
            &permit,
        ) {
            Ok(tick_result) => notices.extend(tick_result.notices),
            Err(error) => notices.push(format!(
                "orchestrator tick failed after official refresh: {error}"
            )),
        }
        ParallelOfficialCompletionFinalizeOutcome::Durable {
            notices,
            proof: ParallelOfficialCompletionDurableProof::DistributorQueueRecord,
            expected_lease: Box::new(expected_lease.clone()),
        }
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
        // commit_ready is the recovery source for a missing distributor queue record.
        // Enqueue must not run unless this durable state was actually persisted.
        let commit_ready = match expected_lease {
            Some(expected) => self
                .parallel_mode_service
                .mark_workspace_commit_ready_for_lease(expected, authority_refresh_outcome),
            None => self
                .parallel_mode_service
                .mark_workspace_commit_ready(workspace_directory, authority_refresh_outcome),
        };
        let commit_ready = match commit_ready {
            Ok(Some(persistence)) => persistence,
            Ok(None) => {
                notices.push(
                    "commit-ready state was not recorded after official refresh because the current running lease no longer matched"
                        .to_string(),
                );
                return ParallelOfficialCompletionSuccessPreparation::Failed {
                    notices,
                    stage: ParallelOfficialCompletionFinalizeFailureStage::CommitReadyPersistence,
                };
            }
            Err(error) => {
                notices.push(format!(
                    "commit-ready state could not be recorded after official refresh: {error}"
                ));
                return ParallelOfficialCompletionSuccessPreparation::Failed {
                    notices,
                    stage: ParallelOfficialCompletionFinalizeFailureStage::CommitReadyPersistence,
                };
            }
        };
        notices.extend(commit_ready.notices);
        let durable_lease = Box::new(commit_ready.lease);
        if !automation_is_active() {
            notices.push(
                "parallel result was durably marked commit-ready and awaits a new automation epoch"
                    .to_string(),
            );
            return ParallelOfficialCompletionSuccessPreparation::Durable {
                notices,
                proof: ParallelOfficialCompletionDurableProof::CommitReadyAwaitingAutomation,
                expected_lease: durable_lease,
                should_run_delivery_tick: false,
            };
        }
        let enqueue = self
            .parallel_mode_service
            .enqueue_workspace_commit_ready_result_for_lease(&durable_lease);
        match enqueue {
            Ok(Some(item)) => notices.push(format!(
                "commit-ready result entered the distributor queue / agent: {} / task: {} / state: {}",
                item.source_agent,
                item.task_title,
                item.queue_state.label()
            )),
            Ok(None) => {
                notices.push(
                    "distributor enqueue did not persist a queue record after official refresh because the current running lease or commit-ready session no longer matched"
                        .to_string(),
                );
                return ParallelOfficialCompletionSuccessPreparation::Failed {
                    notices,
                    stage: ParallelOfficialCompletionFinalizeFailureStage::DistributorEnqueue,
                };
            }
            Err(error) => {
                /*
                Without an enqueue record there is no queue head for the
                orchestrator to process, so stop here and preserve the enqueue
                error as the actionable notice.
                */
                notices.push(format!(
                    "distributor enqueue failed after official refresh: {error}"
                ));
                notices.push(
                    "the durable commit-ready result remains available for distributor enqueue recovery"
                        .to_string(),
                );
                return ParallelOfficialCompletionSuccessPreparation::Failed {
                    notices,
                    stage: ParallelOfficialCompletionFinalizeFailureStage::DistributorEnqueue,
                };
            }
        }
        if !automation_is_active() {
            notices.push(
                "parallel result remains queued because its automation epoch was closed"
                    .to_string(),
            );
            return ParallelOfficialCompletionSuccessPreparation::Durable {
                notices,
                proof: ParallelOfficialCompletionDurableProof::DistributorQueueRecord,
                expected_lease: durable_lease,
                should_run_delivery_tick: false,
            };
        }
        ParallelOfficialCompletionSuccessPreparation::Durable {
            notices,
            proof: ParallelOfficialCompletionDurableProof::DistributorQueueRecord,
            expected_lease: durable_lease,
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

fn stream_terminal_failure_detail(event: &ConversationStreamEvent) -> String {
    match event {
        ConversationStreamEvent::Failed { message } => {
            format!("conversation stream failed: {}", message.trim())
        }
        ConversationStreamEvent::TurnTerminal { receipt } => format!(
            "conversation turn ended without confirmed completion: {}",
            receipt.status_error_summary()
        ),
        _ => "conversation stream ended without confirmed completion".to_string(),
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
    A confirmed completed terminal without TurnStarted is an event-ordering anomaly,
    but the terminal receipt still proves the worker executed. Promote the slot to Running so
    official completion can capture the result instead of leaving a Leased slot
    orphaned.
    */
    !saw_turn_started && !saw_failed_event && !terminal_failure_observed
}
#[cfg(test)]
mod tests {
    use super::{
        ParallelModeTurnService, ParallelOfficialCompletionDurableProof,
        ParallelOfficialCompletionFinalizeFailureStage, ParallelOfficialCompletionFinalizeOutcome,
        ParallelTurnSlotLeaseHandoff, should_mark_cleanup_pending_after_success,
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
        ParallelModeAutomationGuard, ParallelModeService, agent_session_detail_record_path,
        derive_default_pool_root, write_slot_lease,
    };
    use crate::domain::parallel_mode::{
        ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot, ParallelModeCapabilityState,
        ParallelModeSlotLeaseRequest,
    };
    use crate::domain::turn_terminal::{
        ConversationTurnApplicationDelivery, ConversationTurnApplicationDeliveryFailure,
        ConversationTurnError, ConversationTurnTerminalOutcome, ConversationTurnTerminalReceipt,
        ConversationTurnTerminalUncertainty,
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

        let outcome = turn_service.finalize_official_completion_success_for_epoch(
            &lease.worktree_path,
            &workspace.root,
            7,
            "official ledger refresh succeeded",
        );
        let notices = match outcome {
            ParallelOfficialCompletionFinalizeOutcome::Durable { notices, proof, .. } => {
                assert_eq!(
                    proof,
                    ParallelOfficialCompletionDurableProof::CommitReadyAwaitingAutomation
                );
                notices
            }
            ParallelOfficialCompletionFinalizeOutcome::Failed { notices, stage } => {
                panic!(
                    "closed epoch should preserve durable commit-ready state, not fail at {stage:?}: {notices:?}"
                )
            }
        };

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
    fn commit_ready_authority_proof_survives_runtime_mirror_failure() {
        let workspace = TempGitWorkspace::new("parallel-commit-ready-mirror-failure");
        let parallel_service = test_parallel_mode_service();
        parallel_service
            .reset_pool_on_parallel_initial_setup_report(&workspace.root)
            .expect("pool should initialize");
        let lease = parallel_service
            .acquire_slot_lease(
                &workspace.root,
                ParallelModeSlotLeaseRequest::from_task_identity(
                    "task-mirror-failure",
                    "Preserve authority proof",
                ),
            )
            .expect("slot should lease");
        parallel_service
            .mark_workspace_slot_running(&lease.worktree_path)
            .expect("slot should become running");
        parallel_service
            .mark_workspace_official_completion_refreshing(&lease.worktree_path)
            .expect("official refresh state should persist");

        let repo_root = fs::canonicalize(&workspace.root).expect("repo root should canonicalize");
        let detail_path = agent_session_detail_record_path(
            &derive_default_pool_root(&repo_root),
            &lease.session_key(),
        );
        fs::remove_file(&detail_path).expect("existing session mirror should be removable");
        fs::create_dir(&detail_path).expect("directory collision should fail mirror replacement");

        let guard = ParallelModeAutomationGuard::default();
        guard.activate(workspace.root.clone(), 8);
        let turn_service = ParallelModeTurnService::new(parallel_service.clone())
            .with_automation_guard(guard.clone());
        guard.cancel(&workspace.root);
        let outcome = turn_service.finalize_official_completion_success_for_epoch(
            &lease.worktree_path,
            &workspace.root,
            8,
            "official ledger refresh succeeded",
        );

        let notices = match outcome {
            ParallelOfficialCompletionFinalizeOutcome::Durable { notices, proof, .. } => {
                assert_eq!(
                    proof,
                    ParallelOfficialCompletionDurableProof::CommitReadyAwaitingAutomation
                );
                notices
            }
            ParallelOfficialCompletionFinalizeOutcome::Failed { notices, stage } => {
                panic!("authority proof must survive mirror failure at {stage:?}: {notices:?}")
            }
        };
        assert!(notices.iter().any(|notice| {
            notice.contains(
                "commit-ready authority proof was persisted, but its runtime mirror could not be updated",
            )
        }));
        let snapshot = parallel_service.build_passive_supervisor_snapshot(&workspace.root, None);
        assert_eq!(
            snapshot
                .detail
                .session
                .as_ref()
                .map(|detail| detail.state_label.as_str()),
            Some("commit_ready")
        );
    }

    #[test]
    fn post_turn_delivery_rejects_a_reused_slot_generation() {
        let workspace = TempGitWorkspace::new("parallel-post-turn-generation-race");
        let parallel_service = test_parallel_mode_service();
        parallel_service
            .reset_pool_on_parallel_initial_setup_report(&workspace.root)
            .expect("pool should initialize");
        let acquired_lease = parallel_service
            .acquire_slot_lease(
                &workspace.root,
                ParallelModeSlotLeaseRequest::from_task_identity(
                    "task-old-generation",
                    "Old post-turn continuation",
                ),
            )
            .expect("old slot generation should lease");
        let old_lease = parallel_service
            .mark_workspace_slot_running(&acquired_lease.worktree_path)
            .expect("old slot generation should become running")
            .expect("old slot generation should still match");
        parallel_service
            .mark_workspace_official_completion_refreshing(&old_lease.worktree_path)
            .expect("old official refresh state should persist");

        let guard = ParallelModeAutomationGuard::default();
        guard.activate(workspace.root.clone(), 11);
        let turn_service =
            ParallelModeTurnService::new(parallel_service.clone()).with_automation_guard(guard);
        let expected_old_lease = match turn_service
            .mark_official_completion_success_for_post_turn_for_lease(
                &old_lease,
                "old official ledger refresh succeeded",
            ) {
            ParallelOfficialCompletionFinalizeOutcome::Durable {
                proof,
                expected_lease,
                ..
            } => {
                assert_eq!(
                    proof,
                    ParallelOfficialCompletionDurableProof::CommitReadyRecord
                );
                assert!(expected_lease.same_generation_as(&old_lease));
                expected_lease
            }
            ParallelOfficialCompletionFinalizeOutcome::Failed { notices, stage } => {
                panic!("old commit-ready proof should persist at {stage:?}: {notices:?}")
            }
        };

        let authority = SqlitePlanningAuthorityAdapter::new();
        let repo_root = fs::canonicalize(&workspace.root).expect("repo root should canonicalize");
        let pool_root = derive_default_pool_root(&repo_root);
        let mut replacement = (*expected_old_lease).clone();
        replacement.task_id = "task-replacement-generation".to_string();
        replacement.task_title = "Replacement post-turn continuation".to_string();
        replacement.agent_id = "agent-replacement-generation".to_string();
        replacement.lease_generation = Some("f".repeat(64));
        replacement.leased_at = "2026-07-12T00:00:00Z".to_string();
        replacement.running_started_at = Some("2026-07-12T00:00:01Z".to_string());
        write_slot_lease(
            &authority,
            &GitParallelModeRuntimeAdapter::new(),
            &workspace.root,
            &pool_root,
            &replacement,
        )
        .expect("replacement slot generation should persist");
        assert!(!replacement.same_generation_as(&expected_old_lease));
        parallel_service
            .mark_workspace_official_completion_refreshing(&replacement.worktree_path)
            .expect("replacement official refresh state should persist")
            .expect("replacement running lease should resolve for official refresh");
        parallel_service
            .mark_workspace_commit_ready(
                &replacement.worktree_path,
                "replacement official ledger refresh succeeded",
            )
            .expect("replacement commit-ready state should persist")
            .expect("replacement running lease should match");

        let continuation_gate = crate::domain::planning::PostTurnContinuationGate::default();
        let outcome = turn_service.run_official_completion_delivery_tick_for_post_turn(
            &expected_old_lease,
            &workspace.root,
            11,
            &continuation_gate.capture(),
        );
        match outcome {
            ParallelOfficialCompletionFinalizeOutcome::Failed { notices, stage } => {
                assert_eq!(
                    stage,
                    ParallelOfficialCompletionFinalizeFailureStage::DistributorEnqueue
                );
                assert!(notices.iter().any(|notice| {
                    notice
                        .contains("current running lease or commit-ready session no longer matched")
                }));
            }
            ParallelOfficialCompletionFinalizeOutcome::Durable { notices, proof, .. } => {
                panic!("stale post-turn continuation must not produce {proof:?}: {notices:?}")
            }
        }
        let projections = SqlitePlanningAuthorityAdapter::load_runtime_projections(&workspace.root)
            .expect("authority projections should remain readable");
        assert!(projections.distributor_queue_records.is_empty());
        let replacement_detail = projections
            .session_details
            .iter()
            .find(|detail| detail.session_key == replacement.session_key())
            .expect("replacement session detail should remain present");
        assert_eq!(replacement_detail.state_label, "commit_ready");
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
    fn stream_lifecycle_stops_on_any_terminal_but_only_accepts_confirmed_completion() {
        let service = ParallelModeTurnService::new(test_parallel_mode_service());
        let confirmed =
            ConversationTurnTerminalReceipt::completed("thread-1", "turn-completed", Vec::new())
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        let interrupted = ConversationTurnTerminalReceipt::new(
            "thread-1",
            "turn-interrupted",
            ConversationTurnTerminalOutcome::Interrupted,
        )
        .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed);
        let unconfirmed =
            ConversationTurnTerminalReceipt::completed("thread-1", "turn-unconfirmed", Vec::new())
                .with_application_delivery(ConversationTurnApplicationDelivery::Unconfirmed(
                    ConversationTurnApplicationDeliveryFailure::Full,
                ));

        let mut successful_lifecycle = service.stream_lifecycle("/tmp/no-slot-success");
        let successful = successful_lifecycle
            .observe_event(&ConversationStreamEvent::TurnTerminal { receipt: confirmed });
        assert!(successful.should_stop_stream_forwarding);
        assert!(!successful_lifecycle.saw_failed_event);

        for (workspace, receipt) in [
            ("/tmp/no-slot-interrupted", interrupted),
            ("/tmp/no-slot-unconfirmed", unconfirmed),
        ] {
            let mut lifecycle = service.stream_lifecycle(workspace);
            let outcome =
                lifecycle.observe_event(&ConversationStreamEvent::TurnTerminal { receipt });
            assert!(outcome.should_stop_stream_forwarding);
            assert!(lifecycle.saw_failed_event);
            assert!(lifecycle.saw_failed_before_turn_started);
        }
    }

    #[test]
    fn running_leases_persist_failed_detail_for_every_non_success_terminal_receipt() {
        let receipts = [
            (
                "interrupted",
                ConversationTurnTerminalReceipt::new(
                    "thread-interrupted",
                    "turn-interrupted",
                    ConversationTurnTerminalOutcome::Interrupted,
                )
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
                "interrupted",
            ),
            (
                "failed",
                ConversationTurnTerminalReceipt::new(
                    "thread-failed",
                    "turn-failed",
                    ConversationTurnTerminalOutcome::Failed {
                        error: ConversationTurnError::new("worker failed", None::<&str>, None),
                    },
                )
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
                "failed: worker failed",
            ),
            (
                "unknown",
                ConversationTurnTerminalReceipt::new(
                    "thread-unknown",
                    "turn-unknown",
                    ConversationTurnTerminalOutcome::Unknown {
                        reason: ConversationTurnTerminalUncertainty::protocol_inconsistency(
                            "terminal proof was inconsistent",
                        ),
                        observed_error: None,
                    },
                )
                .with_application_delivery(ConversationTurnApplicationDelivery::Confirmed),
                "unknown: terminal proof was inconsistent",
            ),
            (
                "unconfirmed",
                ConversationTurnTerminalReceipt::completed(
                    "thread-unconfirmed",
                    "turn-unconfirmed",
                    Vec::new(),
                )
                .with_application_delivery(
                    ConversationTurnApplicationDelivery::Unconfirmed(
                        ConversationTurnApplicationDeliveryFailure::DeadlineExceeded,
                    ),
                ),
                "application delivery unconfirmed",
            ),
        ];

        for (label, receipt, expected_failure_detail) in receipts {
            let workspace = TempGitWorkspace::new(&format!("parallel-running-terminal-{label}"));
            let parallel_service = test_parallel_mode_service();
            parallel_service
                .reset_pool_on_parallel_initial_setup_report(&workspace.root)
                .expect("pool should initialize for terminal failure case");
            let lease = parallel_service
                .acquire_slot_lease(
                    &workspace.root,
                    ParallelModeSlotLeaseRequest::from_task_identity(
                        format!("task-terminal-{label}"),
                        format!("Terminal {label}"),
                    ),
                )
                .expect("slot lease should be acquired");
            let turn_service = ParallelModeTurnService::new(parallel_service.clone());
            let mut lifecycle = turn_service.stream_lifecycle_for_lease(lease.clone());

            let thread_prepared =
                lifecycle.observe_event(&ConversationStreamEvent::ThreadPrepared {
                    thread_id: receipt.thread_id.clone(),
                    title: format!("Terminal {label}"),
                    cwd: lease.worktree_path.clone(),
                    runtime_envelope: Box::default(),
                });
            assert!(thread_prepared.invalidate_supervisor_snapshot);
            let turn_started = lifecycle.observe_event(&ConversationStreamEvent::TurnStarted {
                turn_id: receipt.turn_id.clone(),
                runtime_request: Box::default(),
            });
            assert!(turn_started.invalidate_supervisor_snapshot);
            assert!(lifecycle.saw_turn_started);

            let terminal =
                lifecycle.observe_event(&ConversationStreamEvent::TurnTerminal { receipt });
            assert!(terminal.should_stop_stream_forwarding);
            assert!(terminal.invalidate_supervisor_snapshot);
            assert!(terminal.runtime_notice.as_deref().is_some_and(|notice| {
                notice.contains("running slot recorded failed after non-success terminal")
                    && notice.contains(expected_failure_detail)
            }));
            let completion = lifecycle.finalize_after_stream_completion(false);
            assert!(completion.runtime_notice.is_none());
            assert!(!completion.invalidate_supervisor_snapshot);

            let supervisor =
                parallel_service.build_passive_supervisor_snapshot(&workspace.root, None);
            let detail = supervisor
                .detail
                .session
                .as_ref()
                .expect("failed session detail should remain visible");
            assert_eq!(detail.session_key, lease.session_key());
            assert_eq!(detail.state_label, "failed");
            assert_eq!(detail.completion_state_label, "failed");
            assert!(
                detail
                    .authority_refresh_outcome
                    .contains(expected_failure_detail)
            );
            let roster_entry = supervisor
                .roster
                .entries
                .iter()
                .find(|entry| entry.slot_id == lease.slot_id)
                .expect("failed running lease should remain in the roster");
            assert_eq!(roster_entry.state_label, "failed");
        }
    }

    #[test]
    fn producer_return_failure_after_turn_started_persists_generic_failed_detail() {
        let workspace = TempGitWorkspace::new("parallel-running-producer-return-failure");
        let parallel_service = test_parallel_mode_service();
        parallel_service
            .reset_pool_on_parallel_initial_setup_report(&workspace.root)
            .expect("pool should initialize for producer return failure");
        let lease = parallel_service
            .acquire_slot_lease(
                &workspace.root,
                ParallelModeSlotLeaseRequest::from_task_identity(
                    "task-producer-return-failure",
                    "Producer return failure",
                ),
            )
            .expect("slot lease should be acquired");
        let turn_service = ParallelModeTurnService::new(parallel_service.clone());
        let mut lifecycle = turn_service.stream_lifecycle_for_lease(lease.clone());
        lifecycle.observe_event(&ConversationStreamEvent::TurnStarted {
            turn_id: "turn-producer-return-failure".to_string(),
            runtime_request: Box::default(),
        });

        let completion = lifecycle.finalize_after_stream_completion(true);
        assert!(completion.invalidate_supervisor_snapshot);
        assert!(completion.runtime_notice.as_deref().is_some_and(|notice| {
            notice.contains("running slot recorded failed after non-success terminal")
                && notice.contains(
                    "conversation stream ended without a confirmed successful terminal after TurnStarted",
                )
        }));

        let supervisor = parallel_service.build_passive_supervisor_snapshot(&workspace.root, None);
        let detail = supervisor
            .detail
            .session
            .as_ref()
            .expect("producer return failure detail should persist");
        assert_eq!(detail.session_key, lease.session_key());
        assert_eq!(detail.state_label, "failed");
        assert_eq!(detail.completion_state_label, "failed");
        assert!(detail.authority_refresh_outcome.contains(
            "conversation stream ended without a confirmed successful terminal after TurnStarted"
        ));
    }
    #[test]
    fn stream_lifecycle_keeps_retrying_event_nonterminal() {
        let service = ParallelModeTurnService::new(test_parallel_mode_service());
        let mut lifecycle = service.stream_lifecycle("/tmp/no-slot-retry");

        let outcome = lifecycle.observe_event(&ConversationStreamEvent::TurnRetrying {
            thread_id: "thread-1".to_string(),
            turn_id: "turn-1".to_string(),
            error: ConversationTurnError::new("retrying", None::<&str>, None),
        });

        assert!(!outcome.should_stop_stream_forwarding);
        assert!(!lifecycle.saw_failed_event);
    }
    #[test]
    fn turn_started_without_slot_lease_keeps_snapshot_steady() {
        let workspace = TempGitWorkspace::new("parallel-turn-no-lease");
        let service = ParallelModeTurnService::new(test_parallel_mode_service());
        let outcome = service.sync_stream_event(
            &workspace.root,
            &ConversationStreamEvent::TurnStarted {
                turn_id: "turn-1".to_string(),
                runtime_request: Box::default(),
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
                runtime_envelope: Box::default(),
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
        let outcome = service
            .finalize_official_completion_success(&workspace, "official ledger refresh succeeded");
        let notices = match outcome {
            ParallelOfficialCompletionFinalizeOutcome::Failed { notices, stage } => {
                assert_eq!(
                    stage,
                    ParallelOfficialCompletionFinalizeFailureStage::CommitReadyPersistence
                );
                notices
            }
            ParallelOfficialCompletionFinalizeOutcome::Durable { notices, proof, .. } => {
                panic!("invalid workspace must not produce durable proof {proof:?}: {notices:?}")
            }
        };

        assert!(notices.iter().any(|notice| {
            notice.contains("commit-ready state could not be recorded after official refresh")
        }));
        assert!(
            notices
                .iter()
                .all(|notice| !notice.contains("distributor enqueue")),
            "enqueue must not run without durable commit-ready proof: {notices:?}"
        );
        let _ = fs::remove_dir_all(workspace);
    }
}
