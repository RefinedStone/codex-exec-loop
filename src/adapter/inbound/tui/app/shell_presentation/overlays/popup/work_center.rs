use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::super::super::super::{
    AkraTheme, ProgressiveActivityCardKind, ProgressiveActivityCardOutcome, WorkCenterSection,
    language::WorkCenterLocalizedCopy,
};
use super::super::super::ConversationScreenModel;
use super::super::super::terminal_text::truncate_end_to_cells;
use super::WorkCenterOverlayView;
use super::supersession::{
    global_operations_blocker, operations_board_state_label, parallel_projection_has_desync,
};

struct WorkCenterItem {
    section: WorkCenterSection,
    label: &'static str,
    state: &'static str,
    summary: String,
    detail: String,
    destination: &'static str,
}

pub(crate) fn build_work_center_overlay_view(
    screen_model: &ConversationScreenModel<'_>,
    selected_section: WorkCenterSection,
    content_width: u16,
) -> WorkCenterOverlayView {
    let copy = screen_model.tui_language.work_center_copy();
    let items = [
        task_item(screen_model, &copy),
        agents_item(screen_model),
        terminal_item(screen_model, &copy),
        approval_item(screen_model, &copy),
        delivery_item(screen_model),
    ];
    let line_width = usize::from(content_width.max(20)).saturating_sub(1);
    let selected_item = items
        .iter()
        .find(|item| item.section == selected_section)
        .expect("work center selection must resolve to one of five stable rows");
    let summary = items
        .iter()
        .map(|item| format!("{} {}", item.label.to_ascii_lowercase(), item.state))
        .collect::<Vec<_>>()
        .join(" · ");

    WorkCenterOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Work Center", ""),
            Line::styled(
                truncate_end_to_cells(copy.subtitle, line_width),
                AkraTheme::subtle(),
            ),
        ],
        summary_lines: vec![Line::from(truncate_end_to_cells(&summary, line_width))],
        item_lines: items
            .iter()
            .map(|item| work_center_item_line(item, selected_section, line_width))
            .collect(),
        detail_lines: vec![Line::styled(
            truncate_end_to_cells(
                &format!(
                    "{} · {} · {} → {} · {}",
                    copy.selected_prefix,
                    selected_item.label,
                    selected_item.state,
                    selected_item.destination,
                    selected_item.detail
                ),
                line_width,
            ),
            AkraTheme::muted(),
        )],
        key_lines: vec![
            AkraTheme::key_line(truncate_end_to_cells(copy.keys_navigation, line_width)),
            AkraTheme::key_line(truncate_end_to_cells(copy.keys_direct, line_width)),
        ],
    }
}

fn task_item(
    screen_model: &ConversationScreenModel<'_>,
    copy: &WorkCenterLocalizedCopy,
) -> WorkCenterItem {
    let Some(conversation) = screen_model.ready_conversation() else {
        return WorkCenterItem {
            section: WorkCenterSection::Task,
            label: "TASK",
            state: "UNKNOWN",
            summary: copy.no_task.to_string(),
            detail: format!(
                "{} · workspace {}",
                copy.unknown, screen_model.workspace_directory
            ),
            destination: "Activity",
        };
    };
    let state = if conversation.pending_approval_request().is_some() {
        "WAITING"
    } else if conversation.has_post_turn_settlement_in_flight() {
        "VERIFYING"
    } else if conversation.has_running_turn() {
        "RUNNING"
    } else {
        "IDLE"
    };
    let title = if conversation.title.trim().is_empty() {
        "Untitled task"
    } else {
        conversation.title.as_str()
    };
    let active_turn = conversation.active_turn_id().unwrap_or(copy.no_active_turn);
    WorkCenterItem {
        section: WorkCenterSection::Task,
        label: "TASK",
        state,
        summary: compact_inline(&format!("{title} · {}", conversation.status_text)),
        detail: compact_inline(&format!(
            "turn {active_turn} · thread {}",
            conversation.thread_id
        )),
        destination: "Activity",
    }
}

fn agents_item(screen_model: &ConversationScreenModel<'_>) -> WorkCenterItem {
    let snapshot = &screen_model.parallel_mode_supervisor;
    let active = snapshot.roster.active_count();
    let desync = parallel_projection_has_desync(snapshot);
    let blocker = global_operations_blocker(screen_model, snapshot);
    let state = if desync {
        "DESYNC"
    } else if blocker.is_some()
        || snapshot.pool.blocked_slots
            + snapshot.pool.missing_slots
            + snapshot.pool.unavailable_slots
            > 0
    {
        "BLOCKED"
    } else if !screen_model.parallel_mode_enabled {
        "OFF"
    } else if screen_model.parallel_mode_control_effect_in_flight {
        "PENDING"
    } else if active > 0 {
        "RUNNING"
    } else {
        "READY"
    };
    let current = snapshot.roster.entries.first().map_or_else(
        || snapshot.roster.empty_state.clone(),
        |entry| {
            compact_inline(&format!(
                "{} · {} · {}",
                entry.agent_id, entry.task_title, entry.latest_summary
            ))
        },
    );
    let detail = if desync {
        "Pool and roster identities disagree · inspect Parallel Operations".to_string()
    } else if let Some(blocker) = blocker {
        compact_inline(&blocker)
    } else {
        compact_inline(&format!(
            "{} · {}",
            operations_board_state_label(
                screen_model.parallel_mode_enabled,
                screen_model.parallel_mode_control_effect_in_flight,
                snapshot
            ),
            snapshot.pool.reconcile_status
        ))
    };
    WorkCenterItem {
        section: WorkCenterSection::Agents,
        label: "AGENTS",
        state,
        summary: format!(
            "{active}/{} active · {current}",
            snapshot.pool.configured_size
        ),
        detail,
        destination: "Parallel Peek",
    }
}

fn terminal_item(
    screen_model: &ConversationScreenModel<'_>,
    copy: &WorkCenterLocalizedCopy,
) -> WorkCenterItem {
    let Some(conversation) = screen_model.ready_conversation() else {
        return WorkCenterItem {
            section: WorkCenterSection::Terminal,
            label: "TERMINAL",
            state: "UNKNOWN",
            summary: copy.no_task.to_string(),
            detail: copy.unknown.to_string(),
            destination: "Terminal Activity",
        };
    };
    let active = conversation.progressive_activity.active_terminal_count();
    let latest = conversation
        .progressive_activity_detail
        .cards()
        .into_iter()
        .rfind(|card| card.key.kind == ProgressiveActivityCardKind::Terminal);
    let state = if active > 0 {
        "ACTIVE"
    } else if latest.as_ref().is_some_and(|card| {
        matches!(
            card.outcome,
            ProgressiveActivityCardOutcome::Failed
                | ProgressiveActivityCardOutcome::Declined
                | ProgressiveActivityCardOutcome::Interrupted
        )
    }) {
        "ATTENTION"
    } else {
        "IDLE"
    };
    let (summary, detail) = latest.map_or_else(
        || (copy.no_terminal.to_string(), copy.no_terminal.to_string()),
        |card| {
            (
                compact_inline(&format!(
                    "{active} active · {} · {}",
                    card.outcome.label(),
                    card.summary
                )),
                compact_inline(&format!(
                    "{} · {} · {}",
                    card.title,
                    card.outcome.label(),
                    card.fact
                )),
            )
        },
    );
    WorkCenterItem {
        section: WorkCenterSection::Terminal,
        label: "TERMINAL",
        state,
        summary,
        detail,
        destination: "Terminal Activity",
    }
}

fn approval_item(
    screen_model: &ConversationScreenModel<'_>,
    copy: &WorkCenterLocalizedCopy,
) -> WorkCenterItem {
    let Some(conversation) = screen_model.ready_conversation() else {
        return WorkCenterItem {
            section: WorkCenterSection::Approval,
            label: "APPROVAL",
            state: "UNKNOWN",
            summary: copy.no_task.to_string(),
            detail: screen_model.github_review_polling_status_label.clone(),
            destination: "Review Center",
        };
    };
    if let Some(request) = conversation.pending_approval_request() {
        let detail = request.details.first().map_or_else(
            || request.method.clone(),
            |detail| format!("{} · {detail}", request.method),
        );
        return WorkCenterItem {
            section: WorkCenterSection::Approval,
            label: "APPROVAL",
            state: "WAITING",
            summary: compact_inline(&request.summary),
            detail: compact_inline(&detail),
            destination: "Runtime Approval",
        };
    }
    if let Some(summary) = conversation.approval_summary() {
        return WorkCenterItem {
            section: WorkCenterSection::Approval,
            label: "APPROVAL",
            state: "REVIEW",
            summary: compact_inline(&summary),
            detail: screen_model.github_review_polling_status_label.clone(),
            destination: "Review Center",
        };
    }
    WorkCenterItem {
        section: WorkCenterSection::Approval,
        label: "APPROVAL",
        state: "NONE",
        summary: copy.no_approval.to_string(),
        detail: compact_inline(&screen_model.github_review_polling_status_label),
        destination: "Review Center",
    }
}

fn delivery_item(screen_model: &ConversationScreenModel<'_>) -> WorkCenterItem {
    let snapshot = &screen_model.parallel_mode_supervisor;
    let distributor = &snapshot.distributor;
    let desync = parallel_projection_has_desync(snapshot);
    let blocker = global_operations_blocker(screen_model, snapshot);
    let active_item = distributor
        .queue_items
        .iter()
        .find(|item| item.queue_state.is_active());
    let state = if desync {
        "DESYNC"
    } else if blocker.is_some() {
        "BLOCKED"
    } else if active_item.is_some() {
        "WORKING"
    } else if !screen_model.parallel_mode_enabled && distributor.queue_items.is_empty() {
        "OFF"
    } else {
        "IDLE"
    };
    let summary = active_item.map_or_else(
        || {
            compact_inline(&format!(
                "{} · depth {}",
                distributor.head_summary,
                distributor.queue_depth()
            ))
        },
        |item| {
            compact_inline(&format!(
                "{} · {} · {}",
                item.queue_state.label(),
                item.task_title,
                distributor.head_summary
            ))
        },
    );
    let detail = if desync {
        "Delivery authority cannot be correlated with the current lane projection".to_string()
    } else if let Some(blocker) = blocker {
        compact_inline(&blocker)
    } else {
        compact_inline(&format!(
            "barrier {} · head {} · {}",
            distributor.orchestrator_status.barrier_state,
            distributor.orchestrator_status.queue_head,
            distributor.note
        ))
    };
    WorkCenterItem {
        section: WorkCenterSection::Delivery,
        label: "DELIVERY",
        state,
        summary,
        detail,
        destination: "Parallel Operations",
    }
}

fn work_center_item_line(
    item: &WorkCenterItem,
    selected_section: WorkCenterSection,
    line_width: usize,
) -> Line<'static> {
    let marker = if item.section == selected_section {
        AkraTheme::selected_marker()
    } else {
        AkraTheme::idle_marker()
    };
    let prefix = format!("{marker}{:<9} {:<9} ", item.label, item.state);
    let summary_width = line_width.saturating_sub(prefix.chars().count());
    let summary = truncate_end_to_cells(&item.summary, summary_width);
    if item.section == selected_section {
        return Line::styled(format!("{prefix}{summary}"), AkraTheme::selected());
    }
    Line::from(vec![
        Span::raw(format!("{marker}{:<9} ", item.label)),
        Span::styled(format!("{:<9} ", item.state), work_state_style(item.state)),
        Span::raw(summary),
    ])
}

fn work_state_style(state: &str) -> Style {
    match state {
        "DESYNC" | "BLOCKED" | "ATTENTION" => AkraTheme::danger(),
        "WAITING" | "VERIFYING" | "PENDING" | "REVIEW" => AkraTheme::warning(),
        "RUNNING" | "ACTIVE" | "WORKING" | "READY" => AkraTheme::brand(),
        "UNKNOWN" | "OFF" | "NONE" => AkraTheme::subtle(),
        _ => AkraTheme::muted(),
    }
}

fn compact_inline(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::adapter::inbound::tui::app::{ConversationState, NativeTuiApp};
    use crate::core::app::{
        ActiveTurnPhase, ActiveTurnSnapshot, CorePromptOrigin, TurnSubmissionCorrelation,
    };
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterEntry, ParallelModePoolBoardSnapshot, ParallelModePoolSlotSnapshot,
        ParallelModePoolSlotState,
    };

    fn screen_text(app: &NativeTuiApp, selected: WorkCenterSection) -> String {
        let screen = ConversationScreenModel::from_app(app);
        normalized_view_text(build_work_center_overlay_view(&screen, selected, 120))
    }

    fn normalized_view_text(view: WorkCenterOverlayView) -> String {
        view.header_lines
            .into_iter()
            .chain(view.summary_lines)
            .chain(view.item_lines)
            .chain(view.detail_lines)
            .chain(view.key_lines)
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn idle_and_running_task_states_follow_the_conversation_authority() {
        let mut app = test_native_tui_app();
        assert!(screen_text(&app, WorkCenterSection::Task).contains("TASK IDLE"));

        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should have a ready conversation");
        };
        let mut runtime = conversation.runtime_snapshot().clone();
        runtime.active_turn = Some(ActiveTurnSnapshot {
            correlation: TurnSubmissionCorrelation::new(1),
            phase: ActiveTurnPhase::Running,
            workspace_directory: conversation.cwd.clone(),
            turn_id: Some("turn-work-center".to_string()),
            prompt_origin: CorePromptOrigin::Manual,
            started_at: std::time::Instant::now(),
        });
        conversation.apply_runtime_snapshot(runtime);
        conversation.record_turn_started("turn-work-center".to_string());

        assert!(screen_text(&app, WorkCenterSection::Task).contains("TASK RUNNING"));
    }

    #[test]
    fn blocked_delivery_reason_is_shared_with_the_parallel_operations_projection() {
        let app = test_native_tui_app();
        let mut screen = ConversationScreenModel::from_app(&app);
        screen
            .parallel_mode_supervisor
            .distributor
            .orchestrator_status
            .blocked_reason = Some("branch protection waiting".to_string());

        let text = normalized_view_text(build_work_center_overlay_view(
            &screen,
            WorkCenterSection::Delivery,
            120,
        ));

        assert!(text.contains("AGENTS BLOCKED"), "{text}");
        assert!(text.contains("DELIVERY BLOCKED"), "{text}");
        assert!(text.contains("branch protection waiting"), "{text}");
    }

    #[test]
    fn pool_roster_disagreement_is_explicitly_desync_instead_of_invented_progress() {
        let app = test_native_tui_app();
        let mut screen = ConversationScreenModel::from_app(&app);
        screen.parallel_mode_supervisor.pool = ParallelModePoolBoardSnapshot::new(
            1,
            "pool",
            "reconciled",
            vec![ParallelModePoolSlotSnapshot::new(
                "slot-1",
                ParallelModePoolSlotState::Leased,
                "agent/slot-1",
                "worktree-1",
                "agent-1",
            )],
        );
        screen.parallel_mode_supervisor.roster.entries.clear();

        let text = normalized_view_text(build_work_center_overlay_view(
            &screen,
            WorkCenterSection::Agents,
            120,
        ));

        assert!(text.contains("AGENTS DESYNC"), "{text}");
        assert!(text.contains("DELIVERY DESYNC"), "{text}");
        assert!(
            text.contains("Pool and roster identities disagree"),
            "{text}"
        );
    }

    #[test]
    fn orphan_roster_entry_is_explicitly_desync_after_pool_loading_finishes() {
        let app = test_native_tui_app();
        let mut screen = ConversationScreenModel::from_app(&app);
        screen.parallel_mode_supervisor.pool =
            ParallelModePoolBoardSnapshot::new(1, "pool", "reconciled", Vec::new());
        screen.parallel_mode_supervisor.roster.entries = vec![ParallelModeAgentRosterEntry::new(
            "agent-orphan",
            "Orphaned work",
            "slot-missing",
            "codex/orphan",
            "running",
            "01m 00s",
            "slot projection missing",
        )];

        let text = normalized_view_text(build_work_center_overlay_view(
            &screen,
            WorkCenterSection::Agents,
            120,
        ));

        assert!(text.contains("AGENTS DESYNC"), "{text}");
        assert!(text.contains("DELIVERY DESYNC"), "{text}");
    }

    #[test]
    fn every_dynamic_work_center_line_is_bounded_at_supported_widths() {
        let app = test_native_tui_app();
        let screen = ConversationScreenModel::from_app(&app);

        for width in [80, 120, 160] {
            let view = build_work_center_overlay_view(&screen, WorkCenterSection::Delivery, width);
            for line in view
                .header_lines
                .iter()
                .chain(&view.summary_lines)
                .chain(&view.item_lines)
                .chain(&view.detail_lines)
                .chain(&view.key_lines)
            {
                assert!(line.width() <= usize::from(width), "{width}: {line}");
            }
        }
    }
}
