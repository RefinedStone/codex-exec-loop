/*
 * Task intake command routing is application policy, not terminal UI routing.
 * The TUI adapter still parses shell syntax and owns prompt buffers, but the
 * decision to defer task intake until a planning-safe point belongs here so all
 * inbound surfaces can share the same "do not mutate planning while a turn is
 * running" rule.
 */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningTaskIntakeCommandRoute {
    /*
     * The inline command is unrelated to task intake. The adapter should route
     * it through its normal command switchboard.
     */
    NotTaskIntake,
    /*
     * Task intake can open immediately because no conversation turn is mutating
     * planning state.
     */
    ExecuteNow,
    /*
     * A turn is running, so task intake must wait for the post-turn safe point.
     * Auto-follow should pause so automation does not race the queued manual
     * planning mutation.
     */
    QueueUntilIdle { pause_auto_follow: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningTaskIntakeCommandRouteRequest {
    pub is_task_intake_command: bool,
    pub turn_running: bool,
}

pub fn route_planning_task_intake_command(
    request: PlanningTaskIntakeCommandRouteRequest,
) -> PlanningTaskIntakeCommandRoute {
    if !request.is_task_intake_command {
        return PlanningTaskIntakeCommandRoute::NotTaskIntake;
    }
    if request.turn_running {
        return PlanningTaskIntakeCommandRoute::QueueUntilIdle {
            pause_auto_follow: true,
        };
    }
    PlanningTaskIntakeCommandRoute::ExecuteNow
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingPlanningTaskIntakeCommandRoute {
    /*
     * The queued command still matches the prompt, but the turn has not reached
     * an idle planning-safe point yet.
     */
    WaitForIdle,
    /*
     * The queued command still matches the prompt and can now be executed.
     */
    Execute,
    /*
     * The operator edited or cleared the command while it was queued. Drop it
     * quietly because the stale command is no longer the operator's visible
     * intent.
     */
    DropStale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingPlanningTaskIntakeCommandRouteRequest {
    pub turn_running: bool,
    pub command_still_buffered: bool,
}

pub fn route_pending_planning_task_intake_command(
    request: PendingPlanningTaskIntakeCommandRouteRequest,
) -> PendingPlanningTaskIntakeCommandRoute {
    if !request.command_still_buffered {
        return PendingPlanningTaskIntakeCommandRoute::DropStale;
    }
    if request.turn_running {
        return PendingPlanningTaskIntakeCommandRoute::WaitForIdle;
    }
    PendingPlanningTaskIntakeCommandRoute::Execute
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_task_command_uses_normal_inline_routing() {
        let route = route_planning_task_intake_command(PlanningTaskIntakeCommandRouteRequest {
            is_task_intake_command: false,
            turn_running: true,
        });

        assert_eq!(route, PlanningTaskIntakeCommandRoute::NotTaskIntake);
    }

    #[test]
    fn task_command_queues_and_pauses_auto_follow_while_turn_runs() {
        let route = route_planning_task_intake_command(PlanningTaskIntakeCommandRouteRequest {
            is_task_intake_command: true,
            turn_running: true,
        });

        assert_eq!(
            route,
            PlanningTaskIntakeCommandRoute::QueueUntilIdle {
                pause_auto_follow: true
            }
        );
    }

    #[test]
    fn task_command_executes_immediately_when_turn_is_idle() {
        let route = route_planning_task_intake_command(PlanningTaskIntakeCommandRouteRequest {
            is_task_intake_command: true,
            turn_running: false,
        });

        assert_eq!(route, PlanningTaskIntakeCommandRoute::ExecuteNow);
    }

    #[test]
    fn queued_task_command_drops_when_operator_edits_prompt() {
        let route = route_pending_planning_task_intake_command(
            PendingPlanningTaskIntakeCommandRouteRequest {
                turn_running: false,
                command_still_buffered: false,
            },
        );

        assert_eq!(route, PendingPlanningTaskIntakeCommandRoute::DropStale);
    }

    #[test]
    fn queued_task_command_waits_while_turn_is_still_running() {
        let route = route_pending_planning_task_intake_command(
            PendingPlanningTaskIntakeCommandRouteRequest {
                turn_running: true,
                command_still_buffered: true,
            },
        );

        assert_eq!(route, PendingPlanningTaskIntakeCommandRoute::WaitForIdle);
    }

    #[test]
    fn queued_task_command_executes_at_idle_safe_point() {
        let route = route_pending_planning_task_intake_command(
            PendingPlanningTaskIntakeCommandRouteRequest {
                turn_running: false,
                command_still_buffered: true,
            },
        );

        assert_eq!(route, PendingPlanningTaskIntakeCommandRoute::Execute);
    }
}
