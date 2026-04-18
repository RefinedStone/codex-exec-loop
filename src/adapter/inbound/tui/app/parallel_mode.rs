use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};

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
        if let Some(snapshot) = self.parallel_mode_supervisor_snapshot.as_ref() {
            if snapshot.workspace_path == workspace_directory {
                return snapshot.clone();
            }
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
                    format!(
                        "parallel mode: blocked / readiness: {} / inspect the prepare surface before retrying",
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
                        "parallel mode command: unsupported argument `{value}` / supported: on, off"
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
            _ => {}
        }

        true
    }
}
