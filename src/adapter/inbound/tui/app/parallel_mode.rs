use crossterm::event::{self, KeyCode, KeyModifiers};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::parallel_mode::ParallelModeService;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeDistributorSnapshot,
    ParallelModePoolBoardSnapshot, ParallelModeReadinessSnapshot,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};

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

        pending_parallel_mode_supervisor_snapshot(
            &workspace_directory,
            self.parallel_mode_enabled(),
            self.parallel_mode_readiness_snapshot(),
        )
    }

    pub(super) fn invalidate_parallel_mode_supervisor_snapshot(&mut self) {
        // Worker dispatch changes leases asynchronously. Clearing this cache is
        // the UI-side signal that the next overlay render should show a cheap
        // pending board instead of recalculating pool state on the input path.
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
            Some(value) => {
                // Unsupported arguments still open the control tower. That makes
                // the supported commands and current readiness visible next to
                // the error copy.
                self.inspect_parallel_mode_shell();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "parallel mode command: unsupported argument `{value}` / supported: :parallel, :parallel off"
                    ),
                });
            }
            None => {
                // Bare `:parallel` is the only enable entrypoint. Open the
                // control tower first, then let readiness/reconcile/dispatch run
                // off the terminal event loop so prompt typing stays responsive.
                let workspace_directory = self.planning_workspace_directory();
                self.parallel_mode_enabled = true;
                self.parallel_mode_readiness_snapshot = None;
                self.parallel_mode_supervisor_snapshot = Some(
                    pending_parallel_mode_supervisor_snapshot(&workspace_directory, true, None),
                );
                self.show_supersession_overlay();
                self.spawn_parallel_mode_enter_worker(workspace_directory);
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "parallel mode: preparing / readiness, pool reconcile, and first dispatch are running".to_string(),
                });
            }
        }
    }

    fn spawn_parallel_mode_enter_worker(&self, workspace_directory: String) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let parallel_agent_worker_port = self.parallel_agent_worker_port.clone();
        let parallel_mode_turn_service = self.parallel_mode_turn_service();
        let planning = self.planning.clone();
        let tx = self.tx.clone();

        thread::spawn(move || {
            let planning_snapshot = planning
                .runtime
                .load_runtime_snapshot_or_invalid(&workspace_directory);
            let readiness_snapshot =
                parallel_mode_service.inspect_readiness(&workspace_directory, &planning_snapshot);

            let (supervisor_snapshot, status_text) = if readiness_snapshot.allows_parallel_mode() {
                let dispatch_status = dispatch_parallel_queue_pool(
                    &workspace_directory,
                    &planning_snapshot,
                    &parallel_mode_service,
                    parallel_agent_worker_port,
                    parallel_mode_turn_service,
                    planning,
                    tx.clone(),
                );
                let supervisor_snapshot = parallel_mode_service.build_supervisor_snapshot(
                    &workspace_directory,
                    true,
                    Some(&readiness_snapshot),
                );
                let mut status_text = format!(
                    "parallel mode: on / readiness: {} / control tower ready",
                    readiness_snapshot.readiness_label()
                );
                if !dispatch_status.trim().is_empty() {
                    status_text.push_str(" / ");
                    status_text.push_str(&dispatch_status);
                }
                (supervisor_snapshot, status_text)
            } else {
                let supervisor_snapshot = parallel_mode_service.build_supervisor_snapshot(
                    &workspace_directory,
                    false,
                    Some(&readiness_snapshot),
                );
                let cause = readiness_snapshot
                    .top_alert
                    .as_deref()
                    .unwrap_or("inspect the readiness panel before retrying");
                let status_text = format!(
                    "parallel mode: blocked / readiness: {} / {cause}",
                    readiness_snapshot.readiness_label()
                );
                (supervisor_snapshot, status_text)
            };

            let _ = tx.send(BackgroundMessage::ParallelModeEntered {
                workspace_directory,
                readiness_snapshot,
                supervisor_snapshot: Box::new(supervisor_snapshot),
                status_text,
            });
        });
    }

    pub(super) fn apply_parallel_mode_entered(
        &mut self,
        workspace_directory: &str,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
        status_text: String,
    ) {
        // A delayed enter result should not reopen parallel mode after the user
        // has already switched it off or moved to another workspace.
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
        {
            return;
        }

        self.parallel_mode_enabled = readiness_snapshot.allows_parallel_mode();
        self.parallel_mode_readiness_snapshot = Some(readiness_snapshot);
        self.parallel_mode_supervisor_snapshot = Some(supervisor_snapshot);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            workspace_directory,
        );
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }
}

fn dispatch_parallel_queue_pool(
    workspace_directory: &str,
    planning_snapshot: &PlanningRuntimeSnapshot,
    parallel_mode_service: &ParallelModeService,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
    parallel_mode_turn_service: ParallelModeTurnService,
    planning: PlanningServices,
    tx: Sender<BackgroundMessage>,
) -> String {
    /*
     * Dispatch is the handoff bridge from planning queue to parallel worker.
     * The service chooses candidates and leases slots; the TUI assembles the
     * sub-session prompt, starts a background worker, and reports a compact
     * launch summary back through conversation status.
     */

    let dispatch_plan = match parallel_mode_service.build_dispatch_plan(
        workspace_directory,
        planning_snapshot,
        // The UI command dispatches the entire currently actionable queue;
        // the service still limits work by idle slots and candidate rules.
        usize::MAX,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            return format!("auto dispatch blocked / {error}");
        }
    };
    // Distinguish infrastructure capacity from queue availability so the
    // operator can decide whether to wait for slots or change planning tasks.
    if dispatch_plan.idle_slot_count == 0 {
        return "no idle slot is available for auto dispatch".to_string();
    }
    if dispatch_plan.candidates.is_empty() {
        return if dispatch_plan.excluded_task_ids.is_empty() {
            "no actionable queue task to auto dispatch".to_string()
        } else {
            format!(
                "no undispatched queue task available for auto dispatch / excluded: {}",
                dispatch_plan.excluded_task_ids.join(", ")
            )
        };
    }

    let mut launched_titles = Vec::new();
    let mut blocked_details = Vec::new();
    for task in dispatch_plan.candidates {
        // Handoff creation belongs to planning runtime because it knows how
        // to turn a queue task into sub-session prompt text and task identity.
        let handoff = planning.runtime.build_sub_session_task_handoff(&task);
        let lease_request = parallel_mode_slot_lease_request(&handoff.task);
        match parallel_mode_service.acquire_slot_lease(workspace_directory, lease_request) {
            Ok(lease) => {
                // After the lease is acquired, the worker owns app-server
                // turn execution in the slot worktree. The TUI keeps only
                // status copy and receives later updates over its channel.
                let worker_request = ParallelDispatchWorkerRequest {
                    planning_workspace_directory: workspace_directory.to_string(),
                    worktree_directory: lease.worktree_path.clone(),
                    prompt: handoff.prompt,
                    handoff_task: handoff.task.clone(),
                };
                spawn_parallel_dispatch_worker(
                    worker_request,
                    parallel_agent_worker_port.clone(),
                    parallel_mode_turn_service.clone(),
                    planning.clone(),
                    tx.clone(),
                );
                launched_titles.push(handoff.task.task_title);
            }
            Err(error) => blocked_details.push(format!("{}: {error}", handoff.task.task_id)),
        }
    }
    let launched_count = launched_titles.len();
    if launched_count == 0 {
        return format!(
            "auto dispatch blocked before worker launch / {}",
            blocked_details.join(" | ")
        );
    }

    let mut status = format!(
        "auto dispatched {launched_count} worker(s) / tasks: {}",
        launched_titles.join(" | ")
    );
    if !blocked_details.is_empty() {
        status.push_str(&format!(" / blocked: {}", blocked_details.join(" | ")));
    }
    status
}

fn pending_parallel_mode_supervisor_snapshot(
    workspace_directory: &str,
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot),
        workspace_directory,
        ParallelModePoolBoardSnapshot::new(
            0,
            "pending reconcile",
            "background reconcile pending",
            Vec::new(),
        ),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), "agent roster pending"),
        ParallelModeSupervisorDetailSnapshot::new(None, "supervisor detail pending"),
        ParallelModeDistributorSnapshot::new(
            Vec::new(),
            Vec::new(),
            "pending",
            "background distributor inspection pending",
        ),
        Some("parallel preparation is running in the background".to_string()),
    )
}

impl NativeTuiApp {
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
