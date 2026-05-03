use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};

/*
 * parallel_mode.rs is the TUI adapter for the supersession control tower. The
 * application service owns pool/readiness/lease rules; this file decides when
 * shell commands should refresh snapshots, show overlay chrome, publish status
 * copy, and spawn background workers for dispatchable planning queue tasks.
 */
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
        // Supervisor snapshots are expensive enough to cache, but they are only
        // valid for the workspace that produced them. A cwd change must rebuild
        // from the service rather than showing stale slot/roster state.
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
        // Worker dispatch changes leases asynchronously. Clearing this cache is
        // the UI-side signal that the next overlay render should ask the service
        // for fresh pool/roster/distributor state.
        self.parallel_mode_supervisor_snapshot = None;
    }

    pub(crate) fn refresh_parallel_mode_readiness_snapshot(
        &mut self,
    ) -> ParallelModeReadinessSnapshot {
        // Readiness depends on both repository/runtime checks and planning
        // workspace state. Reload planning first so queue-idle and authority
        // issues are reflected before enabling or dispatching parallel mode.
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
        // `build` is a read-only projection for inspection. `reconcile` may
        // create/repair pool worktrees and cleanup reusable slots, so callers opt
        // into it only when the user explicitly enables/refreshes active control.
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
        // Plain inspection is intentionally non-mutating for the pool: refresh
        // readiness and projection, then open the overlay without provisioning or
        // cleaning worktrees.
        self.refresh_parallel_mode_readiness_snapshot();
        self.sync_parallel_mode_supervisor_snapshot(false);
        self.show_supersession_overlay();
    }

    pub(super) fn handle_parallel_shell_command(&mut self, argument: Option<&str>) {
        /*
         * `:parallel` commands are operator controls, not prompt text. Each
         * branch updates the same conversation status line so the inline shell,
         * footer, and popup all report the most recent control action.
         */
        match argument {
            Some(value) if value.eq_ignore_ascii_case("off") => {
                // Turning off parallel mode is local UI state. Keep the snapshot
                // read-only and close the control tower so normal shell focus
                // resumes immediately.
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
                // Enabling is gated by a fresh readiness snapshot. When allowed,
                // reconcile the pool because active supervision expects slot
                // worktrees to exist and stale cleanup candidates to be handled.
                let snapshot = self.refresh_parallel_mode_readiness_snapshot();
                let status_text = if snapshot.allows_parallel_mode() {
                    self.parallel_mode_enabled = true;
                    self.sync_parallel_mode_supervisor_snapshot(true);
                    format!(
                        "parallel mode: on / readiness: {} / control tower opened",
                        snapshot.readiness_label()
                    )
                } else {
                    // A blocked readiness state turns the flag back off so later
                    // dispatch cannot proceed on a stale successful state.
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
                // Dispatch uses the same readiness gate as enable. The worker
                // launch path leases slots first, so the UI only calls it after
                // the service says pool/repo state can support parallel work.
                let snapshot = self.refresh_parallel_mode_readiness_snapshot();
                let status_text = if !self.parallel_mode_enabled {
                    self.sync_parallel_mode_supervisor_snapshot(false);
                    "parallel mode: off / use `:parallel on` before dispatching queue head"
                        .to_string()
                } else if snapshot.allows_parallel_mode() {
                    self.sync_parallel_mode_supervisor_snapshot(true);
                    self.dispatch_parallel_queue_pool()
                } else {
                    // If readiness regressed while parallel mode was on, fail
                    // closed and require the operator to inspect before retrying.
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
                // Unsupported arguments still open the control tower. That makes
                // the supported commands and current readiness visible next to
                // the error copy.
                self.inspect_parallel_mode_shell();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "parallel mode command: unsupported argument `{value}` / supported: on, off, dispatch"
                    ),
                });
            }
            None => {
                // Bare `:parallel` is an inspect command. It refreshes readiness
                // and opens the overlay without changing enabled/off state.
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
        /*
         * Dispatch is the handoff bridge from planning queue to parallel worker.
         * The service chooses candidates and leases slots; the TUI assembles the
         * sub-session prompt, starts a background worker, and reports a compact
         * launch summary back through conversation status.
         */
        let workspace_directory = self.planning_workspace_directory();
        let planning_snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        // Keep the ready conversation's embedded planning snapshot aligned with
        // the workspace used to build the dispatch plan so post-turn copy does
        // not lag behind the worker launch.
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );

        let dispatch_plan = match self.parallel_mode_service().build_dispatch_plan(
            &workspace_directory,
            &planning_snapshot,
            // The UI command dispatches the entire currently actionable queue;
            // the service still limits work by idle slots and candidate rules.
            usize::MAX,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                return format!("parallel mode: on / dispatch blocked / {error}");
            }
        };
        // Distinguish infrastructure capacity from queue availability so the
        // operator can decide whether to wait for slots or change planning tasks.
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
            // Handoff creation belongs to planning runtime because it knows how
            // to turn a queue task into sub-session prompt text and task identity.
            let handoff = self.planning.runtime.build_sub_session_task_handoff(&task);
            let lease_request = parallel_mode_slot_lease_request(&handoff.task);
            match self
                .parallel_mode_service()
                .acquire_slot_lease(&workspace_directory, lease_request)
            {
                Ok(lease) => {
                    // After the lease is acquired, the worker owns app-server
                    // turn execution in the slot worktree. The TUI keeps only
                    // status copy and receives later updates over its channel.
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
        // Return false outside the overlay so the normal shell keymap can handle
        // the event. Supersession shortcuts are scoped to the control tower.
        if self.shell_overlay != ShellOverlay::Supersession {
            return false;
        }
        match key.code {
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                // Ctrl+R is the operator's explicit "re-read the world" command:
                // readiness is refreshed and supervisor projection is synced
                // using the current enabled state.
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
                // Ctrl+O hides the tower without changing mode. Active workers
                // continue and can be inspected later.
                self.close_shell_overlay();
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                // Ctrl+P is the emergency local off-switch and reuses the same
                // command path as `:parallel off` so status copy stays identical.
                self.handle_parallel_shell_command(Some("off"));
            }
            _ => return false,
        }

        true
    }
}
