use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};

use super::{
    ConversationInputEvent, ConversationState, NativeTuiApp, ParallelDispatchSubmitContext,
    PromptOrigin,
};

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
                    self.dispatch_next_parallel_queue_head().unwrap_or_else(|| {
                        "parallel mode: on / no actionable queue head to dispatch".to_string()
                    })
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

    fn dispatch_next_parallel_queue_head(&mut self) -> Option<String> {
        let workspace_directory = self.planning_workspace_directory();
        let planning_snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
        let handoff = self
            .planning
            .runtime
            .build_builtin_next_task_handoff(&planning_snapshot)?;
        let task_title = handoff.task.task_title.clone();
        let Some(auto_turn_progress) = self.reserve_parallel_auto_turn_status() else {
            return Some(format!(
                "parallel mode: on / queue head not dispatched / auto-turn budget exhausted / task: {task_title}"
            ));
        };
        let origin = PromptOrigin::ParallelDispatch(Box::new(ParallelDispatchSubmitContext {
            transcript_text: handoff.transcript_text,
            handoff_task: handoff.task,
        }));
        if self.conversation_has_running_turn() {
            return Some(format!(
                "parallel mode: on / queue head not dispatched / active turn still running / task: {task_title}"
            ));
        }
        if !self.shell_action_availability().allows_actions() {
            return Some(format!(
                "parallel mode: on / queue head not dispatched / {}",
                self.submission_blocked_status(origin)
            ));
        }
        if self.submit_prompt(handoff.prompt, origin) {
            Some(format!(
                "parallel mode: on / dispatched queue head / turn {auto_turn_progress} / task: {task_title}"
            ))
        } else {
            Some(format!(
                "parallel mode: on / queue head not dispatched / task: {task_title}"
            ))
        }
    }

    fn reserve_parallel_auto_turn_status(&self) -> Option<String> {
        let ConversationState::Ready(conversation) = &self.conversation_state else {
            return None;
        };
        conversation.auto_follow_state.can_queue_next().then(|| {
            format!(
                "{}/{}",
                conversation.auto_follow_state.next_auto_turn_index(),
                conversation.auto_follow_state.max_auto_turns_label()
            )
        })
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
