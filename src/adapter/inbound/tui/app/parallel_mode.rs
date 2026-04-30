use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::{
    PlanningOfficialCompletionRefreshRequest, PlanningServices, PlanningTaskHandoff,
};
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};

use super::turn_submission_runtime::parallel_mode_slot_lease_request;
use super::{BackgroundMessage, ConversationInputEvent, NativeTuiApp};

impl NativeTuiApp {
    pub(crate) fn parallel_mode_enabled(&self) -> bool {
        self.parallel_mode_enabled
    }

    pub(crate) fn parallel_mode_readiness_snapshot(
        &self,
    ) -> Option<&ParallelModeReadinessSnapshot> {
        self.parallel_mode_readiness_snapshot.as_ref()
    }

    pub(crate) fn parallel_mode_service(&self) -> &ParallelModeService {
        &self.parallel_mode_service
    }

    pub(crate) fn parallel_mode_supervisor_snapshot(&self) -> ParallelModeSupervisorSnapshot {
        let workspace_directory = self.current_workspace_directory();
        if let Some(snapshot) = self.parallel_mode_supervisor_snapshot.as_ref()
            && snapshot.workspace_path == workspace_directory
        {
            return snapshot.clone();
        }

        self.parallel_mode_service().build_supervisor_snapshot(
            &workspace_directory,
            self.parallel_mode_enabled(),
            self.parallel_mode_readiness_snapshot(),
        )
    }

    pub(super) fn invalidate_parallel_mode_supervisor_snapshot(&mut self) {
        self.parallel_mode_supervisor_snapshot = None;
    }

    pub(crate) fn refresh_parallel_mode_readiness_snapshot(
        &mut self,
    ) -> ParallelModeReadinessSnapshot {
        let workspace_directory = self.current_workspace_directory();
        let planning_snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        let snapshot = self
            .parallel_mode_service()
            .inspect_readiness(&workspace_directory, &planning_snapshot);
        self.parallel_mode_readiness_snapshot = Some(snapshot.clone());
        snapshot
    }

    fn sync_parallel_mode_supervisor_snapshot(
        &mut self,
        execute_pool_actions: bool,
    ) -> ParallelModeSupervisorSnapshot {
        let snapshot = if execute_pool_actions {
            self.parallel_mode_service().reconcile_supervisor_snapshot(
                &self.current_workspace_directory(),
                self.parallel_mode_enabled(),
                self.parallel_mode_readiness_snapshot(),
            )
        } else {
            self.parallel_mode_service().build_supervisor_snapshot(
                &self.current_workspace_directory(),
                self.parallel_mode_enabled(),
                self.parallel_mode_readiness_snapshot(),
            )
        };
        self.parallel_mode_supervisor_snapshot = Some(snapshot.clone());
        snapshot
    }

    pub(super) fn show_supersession_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SupersessionOverlayShown);
    }

    pub(super) fn toggle_supersession_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SupersessionOverlayToggled);
    }

    pub(super) fn inspect_parallel_mode_shell(&mut self) {
        self.refresh_parallel_mode_readiness_snapshot();
        self.sync_parallel_mode_supervisor_snapshot(false);
        self.show_supersession_overlay();
    }

    pub(super) fn handle_parallel_shell_command(&mut self, argument: Option<&str>) {
        match argument {
            Some(value) if value.eq_ignore_ascii_case("off") => {
                self.parallel_mode_enabled = false;
                self.sync_parallel_mode_supervisor_snapshot(false);
                if self.shell_overlay == ShellOverlay::Supersession {
                    self.close_shell_overlay();
                }
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "parallel mode: off / shell returned to normal mode".to_string(),
                });
            }
            Some(value) if value.eq_ignore_ascii_case("on") => {
                let snapshot = self.refresh_parallel_mode_readiness_snapshot();
                let status_text = if snapshot.allows_parallel_mode() {
                    self.parallel_mode_enabled = true;
                    self.sync_parallel_mode_supervisor_snapshot(true);
                    format!(
                        "parallel mode: on / readiness: {} / control tower opened",
                        snapshot.readiness_label()
                    )
                } else {
                    self.parallel_mode_enabled = false;
                    self.sync_parallel_mode_supervisor_snapshot(false);
                    let cause = snapshot
                        .top_alert
                        .as_deref()
                        .unwrap_or("inspect the readiness panel before retrying");
                    format!(
                        "parallel mode: blocked / readiness: {} / {cause}",
                        snapshot.readiness_label()
                    )
                };
                self.show_supersession_overlay();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            Some(value) if value.eq_ignore_ascii_case("dispatch") => {
                let snapshot = self.refresh_parallel_mode_readiness_snapshot();
                let status_text = if !self.parallel_mode_enabled {
                    self.sync_parallel_mode_supervisor_snapshot(false);
                    "parallel mode: off / use `:parallel on` before dispatching queue head"
                        .to_string()
                } else if snapshot.allows_parallel_mode() {
                    self.sync_parallel_mode_supervisor_snapshot(true);
                    self.dispatch_parallel_queue_pool()
                } else {
                    self.parallel_mode_enabled = false;
                    self.sync_parallel_mode_supervisor_snapshot(false);
                    let cause = snapshot
                        .top_alert
                        .as_deref()
                        .unwrap_or("inspect the readiness panel before retrying");
                    format!(
                        "parallel mode: blocked / readiness: {} / {cause}",
                        snapshot.readiness_label()
                    )
                };
                self.show_supersession_overlay();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            Some(value) => {
                self.inspect_parallel_mode_shell();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "parallel mode command: unsupported argument `{value}` / supported: on, off, dispatch"
                    ),
                });
            }
            None => {
                let snapshot = self.refresh_parallel_mode_readiness_snapshot();
                self.sync_parallel_mode_supervisor_snapshot(false);
                self.show_supersession_overlay();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "parallel mode: {} / readiness: {}",
                        if self.parallel_mode_enabled {
                            "on"
                        } else {
                            "off"
                        },
                        snapshot.readiness_label()
                    ),
                });
            }
        }
    }

    fn dispatch_parallel_queue_pool(&mut self) -> String {
        let workspace_directory = self.planning_workspace_directory();
        let planning_snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );

        let dispatch_plan = match self.parallel_mode_service().build_dispatch_plan(
            &workspace_directory,
            &planning_snapshot,
            usize::MAX,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                return format!("parallel mode: on / dispatch blocked / {error}");
            }
        };
        if dispatch_plan.idle_slot_count == 0 {
            return "parallel mode: on / no idle slot is available for dispatch".to_string();
        }
        if dispatch_plan.candidates.is_empty() {
            return if dispatch_plan.excluded_task_ids.is_empty() {
                "parallel mode: on / no actionable queue task to dispatch".to_string()
            } else {
                format!(
                    "parallel mode: on / no undispatched queue task available / excluded: {}",
                    dispatch_plan.excluded_task_ids.join(", ")
                )
            };
        }

        let mut launched_titles = Vec::new();
        let mut blocked_details = Vec::new();
        for task in dispatch_plan.candidates {
            let handoff = self.planning.runtime.build_task_handoff(&task);
            let lease_request = parallel_mode_slot_lease_request(&handoff.task);
            match self
                .parallel_mode_service()
                .acquire_slot_lease(&workspace_directory, lease_request)
            {
                Ok(lease) => {
                    let worker_request = ParallelDispatchWorkerRequest {
                        planning_workspace_directory: workspace_directory.clone(),
                        worktree_directory: lease.worktree_path.clone(),
                        prompt: handoff.prompt,
                        handoff_task: handoff.task.clone(),
                    };
                    spawn_parallel_dispatch_worker(
                        worker_request,
                        self.parallel_agent_worker_port.clone(),
                        self.parallel_mode_turn_service(),
                        self.planning.clone(),
                        self.tx.clone(),
                    );
                    launched_titles.push(handoff.task.task_title);
                }
                Err(error) => blocked_details.push(format!("{}: {error}", handoff.task.task_id)),
            }
        }
        self.invalidate_parallel_mode_supervisor_snapshot();

        let launched_count = launched_titles.len();
        if launched_count == 0 {
            return format!(
                "parallel mode: on / dispatch blocked before worker launch / {}",
                blocked_details.join(" | ")
            );
        }

        let mut status = format!(
            "parallel mode: on / dispatched {launched_count} worker(s) / tasks: {}",
            launched_titles.join(" | ")
        );
        if !blocked_details.is_empty() {
            status.push_str(&format!(" / blocked: {}", blocked_details.join(" | ")));
        }
        status
    }

    pub(super) fn handle_supersession_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::Supersession {
            return false;
        }

        match key.code {
            KeyCode::Char('r') if key.modifiers.is_empty() => {
                let snapshot = self.refresh_parallel_mode_readiness_snapshot();
                self.sync_parallel_mode_supervisor_snapshot(self.parallel_mode_enabled());
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "parallel readiness refreshed / state: {}",
                        snapshot.readiness_label()
                    ),
                });
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                self.close_shell_overlay();
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                self.handle_parallel_shell_command(Some("off"));
            }
            _ => return false,
        }

        true
    }
}

#[derive(Debug, Clone)]
struct ParallelDispatchWorkerRequest {
    planning_workspace_directory: String,
    worktree_directory: String,
    prompt: String,
    handoff_task: PlanningTaskHandoff,
}

#[derive(Debug, Clone, Default)]
struct ParallelDispatchWorkerStreamState {
    saw_turn_started: bool,
    saw_failed_before_turn_started: bool,
    saw_failed_event: bool,
    turn_completed: Option<ParallelDispatchTurnCompleted>,
    latest_main_reply: Option<String>,
}

#[derive(Debug, Clone)]
struct ParallelDispatchTurnCompleted {
    turn_id: String,
    changed_planning_file_paths: Vec<String>,
}

fn spawn_parallel_dispatch_worker(
    request: ParallelDispatchWorkerRequest,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
    turn_service: ParallelModeTurnService,
    planning: PlanningServices,
    outer_tx: std::sync::mpsc::Sender<BackgroundMessage>,
) {
    thread::spawn(move || {
        let notices = run_parallel_dispatch_worker(request, worker_port, turn_service, planning);
        for notice in notices {
            let _ = outer_tx.send(BackgroundMessage::ConversationRuntimeNotice(notice));
        }
        let _ = outer_tx.send(BackgroundMessage::InvalidateParallelModeSupervisorSnapshot);
    });
}

fn run_parallel_dispatch_worker(
    request: ParallelDispatchWorkerRequest,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
    turn_service: ParallelModeTurnService,
    planning: PlanningServices,
) -> Vec<String> {
    let (event_tx, event_rx) = mpsc::channel();
    let service_request = request.clone();
    let service_thread = thread::spawn(move || {
        worker_port.run_isolated_new_thread_stream(
            &service_request.worktree_directory,
            &service_request.prompt,
            event_tx,
        )
    });

    let mut notices = Vec::new();
    let mut stream_state = ParallelDispatchWorkerStreamState::default();
    while let Ok(event) = event_rx.recv() {
        sync_parallel_dispatch_worker_event(&turn_service, &request, &event, &mut stream_state)
            .into_iter()
            .for_each(|notice| notices.push(notice));
        if matches!(
            event,
            ConversationStreamEvent::TurnCompleted { .. } | ConversationStreamEvent::Failed { .. }
        ) {
            break;
        }
    }

    match service_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            if !stream_state.saw_failed_event {
                stream_state.saw_failed_event = true;
                if !stream_state.saw_turn_started {
                    stream_state.saw_failed_before_turn_started = true;
                }
            }
            notices.push(format!(
                "parallel worker stream returned an error / task: {} / {error}",
                request.handoff_task.task_title
            ));
        }
        Err(_) => {
            if !stream_state.saw_failed_event {
                stream_state.saw_failed_event = true;
                if !stream_state.saw_turn_started {
                    stream_state.saw_failed_before_turn_started = true;
                }
            }
            notices.push(format!(
                "parallel worker stream panicked / task: {}",
                request.handoff_task.task_title
            ));
        }
    }

    if !stream_state.saw_failed_event && stream_state.turn_completed.is_none() {
        stream_state.saw_failed_event = true;
        if !stream_state.saw_turn_started {
            stream_state.saw_failed_before_turn_started = true;
        }
        notices.push(format!(
            "parallel worker stream ended without a completed turn / task: {}",
            request.handoff_task.task_title
        ));
    }

    let completion = turn_service.finalize_stream_completion(
        &request.worktree_directory,
        stream_state.saw_turn_started,
        stream_state.saw_failed_before_turn_started,
        stream_state.saw_failed_event,
        stream_state.saw_failed_event && stream_state.turn_completed.is_none(),
    );
    if let Some(notice) = completion.runtime_notice {
        notices.push(notice);
    }

    if stream_state.saw_failed_event {
        turn_service.mark_official_completion_failed(
            &request.worktree_directory,
            "parallel worker stream failed before official completion refresh",
        );
        return notices;
    }

    let Some(turn_completed) = stream_state.turn_completed else {
        turn_service.mark_official_completion_failed(
            &request.worktree_directory,
            "parallel worker stream ended without a completed turn",
        );
        return notices;
    };

    notices.extend(run_parallel_dispatch_official_completion(
        &request,
        &turn_service,
        &planning,
        &turn_completed,
        stream_state.latest_main_reply.as_deref(),
    ));
    notices
}

fn sync_parallel_dispatch_worker_event(
    turn_service: &ParallelModeTurnService,
    request: &ParallelDispatchWorkerRequest,
    event: &ConversationStreamEvent,
    stream_state: &mut ParallelDispatchWorkerStreamState,
) -> Vec<String> {
    let mut notices = Vec::new();
    let outcome = turn_service.sync_stream_event(&request.worktree_directory, event);
    stream_state.saw_turn_started |= outcome.turn_started_observed;
    if let Some(notice) = outcome.runtime_notice {
        notices.push(notice);
    }

    match event {
        ConversationStreamEvent::AgentMessageCompleted { text, .. } => {
            let text = text.trim();
            if !text.is_empty() {
                stream_state.latest_main_reply = Some(text.to_string());
            }
        }
        ConversationStreamEvent::TurnCompleted {
            turn_id,
            changed_planning_file_paths,
        } => {
            stream_state.turn_completed = Some(ParallelDispatchTurnCompleted {
                turn_id: turn_id.clone(),
                changed_planning_file_paths: changed_planning_file_paths.clone(),
            });
        }
        ConversationStreamEvent::Failed { .. } => {
            stream_state.saw_failed_event = true;
            if !stream_state.saw_turn_started {
                stream_state.saw_failed_before_turn_started = true;
            }
        }
        _ => {}
    }

    notices
}

fn run_parallel_dispatch_official_completion(
    request: &ParallelDispatchWorkerRequest,
    turn_service: &ParallelModeTurnService,
    planning: &PlanningServices,
    turn_completed: &ParallelDispatchTurnCompleted,
    latest_main_reply: Option<&str>,
) -> Vec<String> {
    let mut notices = Vec::new();
    let refresh_order = match turn_service
        .reserve_official_completion_refresh_order(&request.worktree_directory)
    {
        Ok(Some(order)) => order,
        Ok(None) => {
            return vec![format!(
                "parallel worker completion skipped official refresh because no running slot lease was found / task: {}",
                request.handoff_task.task_title
            )];
        }
        Err(error) => {
            turn_service.mark_official_completion_failed(&request.worktree_directory, &error);
            return vec![format!(
                "parallel worker completion could not reserve official refresh order / task: {} / {error}",
                request.handoff_task.task_title
            )];
        }
    };
    let latest_main_reply = latest_main_reply
        .filter(|reply| !reply.trim().is_empty())
        .unwrap_or("parallel worker completed without a final text response");
    let validation_summary =
        parallel_dispatch_validation_summary(&turn_completed.changed_planning_file_paths);
    let completion_report = match turn_service.begin_official_completion(
        &request.worktree_directory,
        &turn_completed.turn_id,
        Some(refresh_order),
        Some(latest_main_reply),
        Some(&validation_summary),
    ) {
        Ok(Some(report)) => report,
        Ok(None) => {
            return vec![format!(
                "parallel worker completion had no running slot to report / task: {}",
                request.handoff_task.task_title
            )];
        }
        Err(error) => {
            turn_service.mark_official_completion_failed(&request.worktree_directory, &error);
            return vec![format!(
                "parallel worker completion capture failed / task: {} / {error}",
                request.handoff_task.task_title
            )];
        }
    };

    if let Some(notice) =
        turn_service.mark_official_completion_refreshing(&request.worktree_directory)
    {
        notices.push(notice);
    }
    let worker_request = PlanningOfficialCompletionRefreshRequest {
        workspace_directory: &request.planning_workspace_directory,
        latest_user_message: None,
        latest_main_reply,
        previous_handoff_task: Some(&request.handoff_task),
        contract: &completion_report,
    };
    let worker_outcome = planning
        .worker
        .refresh_queue_from_official_completion(worker_request);
    let outcome = match worker_outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let detail = format!("parallel official completion refresh failed: {error}");
            turn_service.mark_official_completion_failed(&request.worktree_directory, &detail);
            return vec![detail];
        }
    };

    if outcome.repair_request.is_some() || outcome.runtime_snapshot.blocks_auto_followup() {
        let detail = outcome
            .runtime_snapshot
            .preview_detail()
            .unwrap_or("parallel official completion refresh requires planning repair")
            .to_string();
        turn_service.mark_official_completion_failed(&request.worktree_directory, &detail);
        notices.push(format!(
            "parallel official completion refresh blocked / task: {} / {detail}",
            request.handoff_task.task_title
        ));
        return notices;
    }

    if !matches!(
        outcome.runtime_snapshot.workspace_status(),
        crate::application::service::planning::PlanningRuntimeWorkspaceStatus::ReadyNoTask
            | crate::application::service::planning::PlanningRuntimeWorkspaceStatus::ReadyWithTask
    ) {
        let detail = "parallel official completion refresh left planning unavailable";
        turn_service.mark_official_completion_failed(&request.worktree_directory, detail);
        notices.push(format!(
            "parallel official completion refresh blocked / task: {} / {detail}",
            request.handoff_task.task_title
        ));
        return notices;
    }

    let authority_refresh_outcome = outcome
        .worker_summary
        .as_deref()
        .map(|summary| format!("official ledger refresh succeeded: {summary}"))
        .unwrap_or_else(|| "official ledger refresh succeeded".to_string());
    notices.extend(turn_service.finalize_official_completion_success(
        &request.worktree_directory,
        &authority_refresh_outcome,
    ));
    notices
}

fn parallel_dispatch_validation_summary(changed_planning_file_paths: &[String]) -> String {
    if changed_planning_file_paths.is_empty() {
        return "parallel worker completed without planning file changes".to_string();
    }

    format!(
        "parallel worker completed with planning file changes: {}",
        changed_planning_file_paths.join(", ")
    )
}
