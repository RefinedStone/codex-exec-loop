//! TUI 가 Gateway 의 커널에 붙는 클라이언트이다 (plan 18 K6-2 의 첫 조각, K6-2a).
//!
//! 다른 프로세스(Gateway)의 커널이 저장소를 쥐고 있으면 TUI 는 잠금을 잡지 않고 이 클라이언트로 연다. 커널의
//! 값(세션 목록, 일이 남은 thread, 자동 후속 설정)을 `akra/kernel/*` 로 읽고 그 이벤트를 따라간다. 이 조각은 읽기만
//! 한다: 대화 열기, 프롬프트, 명령 보내기는 다음 조각이다. 그래서 core 입력은 시작 점검과 세션 목록 읽기만 답한다.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::adapter::inbound::kernel_protocol::{
    AutoFollowSettings, SessionsState, SessionsStatus, ThreadActivity,
};
use crate::adapter::outbound::gateway_kernel_client::{
    KernelValueUpdate as KernelClientUpdate, spawn_kernel_value_watcher,
};
use crate::core::app::{
    AppCommand, AppEvent, AppSnapshot, CoreDispatchOutcome, CoreInput, ParallelModeProjection,
    PlanningParallelProjection, RevisionedPlanningParallelProjection, SessionCatalogReadySnapshot,
    SessionCatalogSnapshot, StartupAttachmentSnapshot, StartupCheckCorrelation,
    StartupDiagnosticSnapshot, StartupReadySnapshot, StartupSnapshot, ThreadActivitySummary,
};
use crate::core::native_client_port::{
    NativeClientDispatchOutcome, NativeClientEvent, NativeClientPort,
    ParallelModeControlPlaneEpochSnapshot, ParallelModeControlPlanePresentationProjection,
};
use crate::domain::conversation::{ConversationApprovalRequest, ConversationApprovalRequestKind};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogTier};
use crate::domain::session_summary::SessionSummary;

/// 커널이 기록하는 이 클라이언트의 이름이다.
pub(crate) const TUI_KERNEL_CLIENT_NAME: &str = "akra-tui";
pub(crate) struct GatewayKernelClientRuntime {
    workspace_directory: String,
    snapshot: AppSnapshot,
    startup_generation: u64,
    updates: Receiver<KernelClientUpdate>,
}

impl GatewayKernelClientRuntime {
    /// Gateway 에 붙어 값을 읽는 스레드를 띄운다. 스레드는 연결이 끊기면 다시 붙고, 이 값이 사라지면 끝난다.
    pub(crate) fn attach(workspace_directory: String) -> Self {
        let (sender, updates) = mpsc::channel();
        spawn_kernel_value_watcher(workspace_directory.clone(), TUI_KERNEL_CLIENT_NAME, sender);
        Self::with_updates(workspace_directory, updates)
    }

    fn with_updates(workspace_directory: String, updates: Receiver<KernelClientUpdate>) -> Self {
        let mut snapshot = AppSnapshot::initial();
        snapshot.session_catalog = SessionCatalogSnapshot::Loading;
        Self {
            workspace_directory,
            snapshot,
            startup_generation: 0,
            updates,
        }
    }

    fn outcome(&self, events: Vec<AppEvent>) -> NativeClientDispatchOutcome {
        NativeClientDispatchOutcome::Core(CoreDispatchOutcome {
            events,
            effects: Vec::new(),
            snapshot: Arc::new(self.snapshot.clone()),
        })
    }

    fn apply(&mut self, update: KernelClientUpdate) -> Vec<AppEvent> {
        self.snapshot.revision += 1;
        match update {
            KernelClientUpdate::Value(value) => self.apply_value(value),
            KernelClientUpdate::Disconnected(reason) => {
                self.snapshot.session_catalog = SessionCatalogSnapshot::Failed {
                    message: format!(
                        "lost the connection to the Akra Gateway kernel ({reason}); reconnecting"
                    ),
                };
                vec![
                    AppEvent::SessionCatalogChanged(self.snapshot.session_catalog.clone()),
                    AppEvent::ThreadActivityChanged(Vec::new()),
                ]
            }
        }
    }

    fn apply_value(&mut self, value: Value) -> Vec<AppEvent> {
        match value.get("type").and_then(Value::as_str) {
            Some("sessions") => match serde_json::from_value::<SessionsState>(value) {
                Ok(state) => {
                    self.snapshot.session_catalog = session_catalog(state);
                    vec![AppEvent::SessionCatalogChanged(
                        self.snapshot.session_catalog.clone(),
                    )]
                }
                Err(error) => unreadable("sessions", &error),
            },
            Some("auto_follow") => match serde_json::from_value::<AutoFollowSettings>(value) {
                Ok(settings) => {
                    let auto_follow = &mut self.snapshot.conversation_runtime.auto_follow;
                    auto_follow.max_auto_turns = settings.max_turns;
                    auto_follow.continuation_paused = settings.paused;
                    vec![AppEvent::ConversationRuntimeAuthorityChanged(Box::new(
                        self.snapshot.conversation_runtime.clone(),
                    ))]
                }
                Err(error) => unreadable("auto_follow", &error),
            },
            Some("thread_activity") => {
                match serde_json::from_value::<Vec<ThreadActivity>>(value["threads"].clone()) {
                    Ok(threads) => vec![AppEvent::ThreadActivityChanged(
                        threads.into_iter().map(thread_activity).collect(),
                    )],
                    Err(error) => unreadable("thread_activity", &error),
                }
            }
            _ => Vec::new(),
        }
    }

    fn startup_ready(&mut self, workspace_directory: String) -> Vec<AppEvent> {
        self.startup_generation += 1;
        let ready = |detail: &str| StartupDiagnosticSnapshot {
            ok: true,
            detail: detail.to_string(),
        };
        let snapshot = StartupSnapshot::Ready(Box::new(StartupReadySnapshot {
            cwd: workspace_directory.clone(),
            workspace_path: self.workspace_directory.clone(),
            can_continue: true,
            codex_binary: ready("the Akra Gateway runs codex"),
            workspace: ready(&self.workspace_directory),
            app_server_initialize: ready("attached to the Akra Gateway kernel"),
            account: ready("checked by the Akra Gateway"),
            attachment: StartupAttachmentSnapshot {
                mode_label: "gateway kernel client".to_string(),
                recovery_anchor_label: "Akra Gateway".to_string(),
            },
            // A warning would mark the runtime degraded. The attachment label names this client instead.
            warnings: Vec::new(),
            schema_snapshot: String::new(),
        }));
        self.snapshot.startup = snapshot.clone();
        vec![AppEvent::StartupChanged {
            correlation: StartupCheckCorrelation {
                generation: self.startup_generation,
                workspace_directory,
            },
            snapshot,
        }]
    }
}

fn unreadable(value: &str, error: &serde_json::Error) -> Vec<AppEvent> {
    tracing::warn!(%error, value, "the kernel client could not read a kernel value");
    Vec::new()
}

fn session_catalog(state: SessionsState) -> SessionCatalogSnapshot {
    match state.status {
        SessionsStatus::Idle => SessionCatalogSnapshot::Idle,
        SessionsStatus::Loading => SessionCatalogSnapshot::Loading,
        SessionsStatus::Failed => SessionCatalogSnapshot::Failed {
            message: state
                .message
                .unwrap_or_else(|| "the kernel could not read the sessions".to_string()),
        },
        SessionsStatus::Unavailable => SessionCatalogSnapshot::Failed {
            message: "the kernel has no session list for this workspace".to_string(),
        },
        SessionsStatus::Ready => {
            let items = state
                .sessions
                .into_iter()
                .map(|entry| SessionSummary {
                    id: entry.thread_id,
                    name: Some(entry.title),
                    preview: String::new(),
                    cwd: entry.workspace,
                    source: "akra-kernel".to_string(),
                    model_provider: String::new(),
                    updated_at_epoch: entry.updated_at_epoch,
                    status_type: String::new(),
                    path: String::new(),
                    git_branch: None,
                })
                .collect();
            SessionCatalogSnapshot::Ready(SessionCatalogReadySnapshot::from_catalog(
                SessionCatalog::ready(
                    SessionCatalogTier::ProviderBackedCatalog,
                    RecentSessions {
                        items,
                        warnings: Vec::new(),
                        next_cursor: None,
                    },
                ),
            ))
        }
    }
}

fn thread_activity(activity: ThreadActivity) -> ThreadActivitySummary {
    ThreadActivitySummary {
        thread_id: activity.thread_id,
        running: activity.running,
        active_turn_id: activity.active_turn_id,
        // The protocol leaves out the method and kind. The session list shows only that an approval waits.
        approval: activity
            .approval
            .map(|request| ConversationApprovalRequest {
                approval_id: request.approval_id,
                server_request_id: request.server_request_id,
                method: String::new(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: request.summary,
                details: request.details,
            }),
        auto_follow_active: activity.auto_follow_active,
    }
}

impl NativeClientPort for GatewayKernelClientRuntime {
    fn dispatch_client_event(&mut self, event: NativeClientEvent) -> NativeClientDispatchOutcome {
        let events = match event {
            NativeClientEvent::Core(input) => match *input {
                CoreInput::Command(AppCommand::RunStartupChecks {
                    workspace_directory,
                }) => self.startup_ready(workspace_directory),
                CoreInput::Command(AppCommand::LoadSessionCatalog(_)) => {
                    vec![AppEvent::SessionCatalogChanged(
                        self.snapshot.session_catalog.clone(),
                    )]
                }
                // The kernel owns every other decision; this read-only client does not send it yet.
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        self.outcome(events)
    }

    #[cfg(test)]
    fn poll_pending_client_event(&mut self) -> Option<NativeClientDispatchOutcome> {
        let update = self.updates.try_recv().ok()?;
        let events = self.apply(update);
        Some(self.outcome(events))
    }

    fn run_host_cycle(
        &mut self,
        _now: Instant,
        wait: Duration,
        max_inputs: usize,
    ) -> Vec<NativeClientDispatchOutcome> {
        let mut outcomes = Vec::new();
        let mut next = if wait.is_zero() {
            self.updates.try_recv().ok()
        } else {
            self.updates.recv_timeout(wait).ok()
        };
        let mut applied = 0;
        while let Some(update) = next {
            let events = self.apply(update);
            if !events.is_empty() {
                outcomes.push(self.outcome(events));
            }
            applied += 1;
            next = if applied < max_inputs {
                self.updates.try_recv().ok()
            } else {
                None
            };
        }
        outcomes
    }

    fn snapshot(&self) -> AppSnapshot {
        self.snapshot.clone()
    }

    fn revisioned_planning_parallel_projection(&self) -> RevisionedPlanningParallelProjection {
        RevisionedPlanningParallelProjection {
            revision: 0,
            planning_parallel: PlanningParallelProjection::initial(),
        }
    }

    fn parallel_mode_projection(&self) -> ParallelModeProjection {
        ParallelModeProjection::default()
    }

    fn parallel_mode_enabled(&self) -> bool {
        false
    }

    fn parallel_control_plane_projection(&self) -> ParallelModeControlPlanePresentationProjection {
        ParallelModeControlPlanePresentationProjection {
            mode_enabled: false,
            control_effect_in_flight: false,
            last_dispatch_withheld_reason: None,
            global_runtime_notices: Vec::new(),
        }
    }

    fn parallel_epoch_snapshot(&self) -> ParallelModeControlPlaneEpochSnapshot {
        ParallelModeControlPlaneEpochSnapshot {
            workspace_directory: None,
            current_epoch_id: None,
        }
    }

    fn is_gateway_kernel_client(&self) -> bool {
        true
    }

    #[cfg(test)]
    fn current_parallel_epoch_id_for_workspace(&self, _workspace_directory: &str) -> Option<u64> {
        None
    }

    #[cfg(test)]
    fn dispatch_parallel_completion_for_test(
        &mut self,
        _event: crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent,
    ) -> NativeClientDispatchOutcome {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn begin_test_turn_submission(&mut self) -> crate::core::app::TurnSubmissionCorrelation {
        unreachable!("the Gateway kernel client does not submit turns")
    }

    #[cfg(test)]
    fn force_parallel_mode_for_test(&self, _workspace_directory: &str, _enabled: bool) {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn force_parallel_epoch_for_test(&self, _workspace_directory: &str, _epoch_id: u64) {
        unreachable!("the Gateway kernel client has no parallel control plane")
    }

    #[cfg(test)]
    fn parallel_automation_epoch_is_active_for_test(
        &self,
        _workspace_directory: &str,
        _epoch_id: u64,
    ) -> bool {
        false
    }

    #[cfg(test)]
    fn recv_parallel_completion_for_test(
        &self,
        _timeout: Duration,
    ) -> crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneBackgroundEvent{
        unreachable!("the Gateway kernel client has no parallel control plane")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::*;

    /// 값을 보내는 쪽과 함께 클라이언트를 만든다. TUI 시험이 Gateway 없이 이 클라이언트를 쓴다.
    pub(crate) fn kernel_client_with_values(
        workspace_directory: &str,
        values: Vec<Value>,
    ) -> GatewayKernelClientRuntime {
        let (sender, updates) = mpsc::channel();
        for value in values {
            sender.send(KernelClientUpdate::Value(value)).unwrap();
        }
        drop(sender);
        GatewayKernelClientRuntime::with_updates(workspace_directory.to_string(), updates)
    }

    fn events(outcomes: Vec<NativeClientDispatchOutcome>) -> Vec<AppEvent> {
        outcomes
            .into_iter()
            .flat_map(|outcome| match outcome {
                NativeClientDispatchOutcome::Core(core) => core.events,
                _ => panic!("the kernel client only answers with core outcomes"),
            })
            .collect()
    }

    #[test]
    fn kernel_values_become_the_session_list_thread_activity_and_auto_follow_settings() {
        let mut client = kernel_client_with_values(
            "/repo",
            vec![
                json!({"type":"sessions","status":"ready","sessions":[
                    {"thread_id":"t-1","title":"Fix CI","workspace":"/repo","updated_at_epoch":7}
                ]}),
                json!({"type":"auto_follow","max_turns":3,"paused":true}),
                json!({"type":"thread_activity","threads":[
                    {"thread_id":"t-1","running":true,"active_turn_id":"turn-1","auto_follow_active":false,
                     "approval":{"approval_id":"a-1","server_request_id":"7","summary":"run tests","details":[]}}
                ]}),
                json!({"type":"planning"}),
            ],
        );

        let events = events(client.run_host_cycle(Instant::now(), Duration::ZERO, 16));

        assert_eq!(
            events.len(),
            3,
            "a value the client does not show changes nothing"
        );
        let snapshot = client.snapshot();
        let SessionCatalogSnapshot::Ready(ready) = &snapshot.session_catalog else {
            panic!(
                "expected a ready session list, got {:?}",
                snapshot.session_catalog
            );
        };
        let SessionCatalog::Ready {
            recent_sessions, ..
        } = ready.catalog.as_ref()
        else {
            panic!("expected a ready catalog");
        };
        assert_eq!(recent_sessions.items[0].id, "t-1");
        assert_eq!(recent_sessions.items[0].title(), "Fix CI");
        assert_eq!(snapshot.conversation_runtime.auto_follow.max_auto_turns, 3);
        assert!(
            snapshot
                .conversation_runtime
                .auto_follow
                .continuation_paused
        );
        let Some(AppEvent::ThreadActivityChanged(activity)) = events.last() else {
            panic!("expected thread activity, got {events:?}");
        };
        assert_eq!(activity[0].active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(
            activity[0]
                .approval
                .as_ref()
                .map(|request| request.summary.as_str()),
            Some("run tests")
        );
    }

    #[test]
    fn startup_is_ready_without_a_local_app_server_and_a_lost_connection_shows_in_the_session_list()
    {
        let (sender, updates) = mpsc::channel();
        let mut client = GatewayKernelClientRuntime::with_updates("/repo".to_string(), updates);

        let NativeClientDispatchOutcome::Core(started) = client.dispatch_client_event(
            NativeClientEvent::core(CoreInput::Command(AppCommand::RunStartupChecks {
                workspace_directory: "/repo/sub".to_string(),
            })),
        ) else {
            panic!("expected a core outcome");
        };
        assert!(matches!(
            started.events.as_slice(),
            [AppEvent::StartupChanged { snapshot: StartupSnapshot::Ready(ready), .. }]
                if ready.can_continue && ready.attachment.mode_label == "gateway kernel client"
        ));

        sender
            .send(KernelClientUpdate::Disconnected("closed".to_string()))
            .unwrap();
        client.run_host_cycle(Instant::now(), Duration::ZERO, 16);
        assert!(matches!(
            client.snapshot().session_catalog,
            SessionCatalogSnapshot::Failed { message } if message.contains("reconnecting")
        ));
        assert!(client.is_gateway_kernel_client());
    }
}
