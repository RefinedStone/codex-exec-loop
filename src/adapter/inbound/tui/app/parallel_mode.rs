use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};

#[path = "parallel_mode/dispatch_worker.rs"]
mod dispatch_worker;

use self::dispatch_worker::{ParallelDispatchWorkerRequest, spawn_parallel_dispatch_worker};
use super::turn_submission_runtime::parallel_mode_slot_lease_request;
use super::{ConversationInputEvent, NativeTuiApp};

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
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
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
