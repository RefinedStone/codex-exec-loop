use ratatui::text::Line;

use crate::adapter::inbound::tui::app::conversation_model::ProgressiveActivityItemKind;
use crate::domain::conversation_runtime_envelope::ConversationRuntimeObservedValue;

use super::super::ConversationViewModel;

const ACTIVITY_RAIL_PREFIX: &str = "activity: ";
const ACTIVITY_RAIL_SEPARATOR: &str = " | ";
const MAX_MODEL_FACT_CELLS: usize = 32;
const MAX_MODEL_COMPACT_FACT_CELLS: usize = 16;
const MAX_TASK_FACT_CELLS: usize = 32;
const MAX_TASK_COMPACT_FACT_CELLS: usize = 16;
const MAX_FACT_SCAN_CHARACTERS: usize = 128;

pub(super) fn build_activity_rail_notice_line(
    conversation: &ConversationViewModel,
    max_detail_cells: usize,
) -> Option<String> {
    let activity = &conversation.progressive_activity;
    let mut facts: Vec<(String, Option<String>)> = Vec::new();

    if let Some(terminal_state) = conversation.activity_rail_terminal_state {
        let fitted = fit_activity_facts(
            vec![(
                format!("terminal:{}", terminal_state.label()),
                Some(format!("term:{}", terminal_state.compact_label())),
            )],
            max_detail_cells,
        );
        return fitted.or_else(|| {
            let compact = terminal_state.compact_label().to_string();
            (display_width(&compact) <= max_detail_cells).then_some(compact)
        });
    }
    if let Some(kind) = activity.active_item_kind() {
        let label = progressive_item_kind_label(kind);
        facts.push((format!("active:{label}"), Some(label.to_string())));
    }
    if activity.plan_total_count() > 0 {
        let retained_total = activity
            .plan_total_count()
            .saturating_sub(activity.plan_omitted_count());
        let mut plan = format!(
            "plan:{}/{}",
            activity.plan_completed_count(),
            retained_total
        );
        if activity.plan_omitted_count() > 0 {
            plan.push_str(&format!(" +{} hidden", activity.plan_omitted_count()));
        }
        let compact_plan = if activity.plan_omitted_count() > 0 {
            format!(
                "plan:{}/{}+{}?",
                activity.plan_completed_count(),
                retained_total,
                activity.plan_omitted_count()
            )
        } else {
            format!(
                "plan:{}/{}",
                activity.plan_completed_count(),
                retained_total
            )
        };
        facts.push((plan, Some(compact_plan)));
    }
    if activity.diff_addition_count() > 0
        || activity.diff_deletion_count() > 0
        || activity.diff_hunk_count() > 0
    {
        facts.push((
            format!(
                "diff:+{} -{} h{}",
                activity.diff_addition_count(),
                activity.diff_deletion_count(),
                activity.diff_hunk_count()
            ),
            Some(format!(
                "diff:+{}/-{}",
                activity.diff_addition_count(),
                activity.diff_deletion_count()
            )),
        ));
    }
    if activity.command_line_count() > 0 {
        facts.push((
            format!("cmd:{} lines", activity.command_line_count()),
            Some(format!("cmd:{}", activity.command_line_count())),
        ));
    }
    if activity.patch_count() > 0 {
        facts.push((
            format!("patch:{} files", activity.patch_count()),
            Some(format!("patch:{}", activity.patch_count())),
        ));
    }
    if activity.mcp_update_count() > 0 {
        facts.push((
            format!("mcp:{} updates", activity.mcp_update_count()),
            Some(format!("mcp:{}", activity.mcp_update_count())),
        ));
    }
    if let Some(basis_points) = activity.context_pressure_basis_points() {
        facts.push((
            format!("ctx:{}.{:02}%", basis_points / 100, basis_points % 100),
            None,
        ));
    }
    if activity.bounded_history() {
        facts.push(("history:bounded".to_string(), None));
    }

    if conversation.has_running_turn() {
        if let Some(model_fact) = effective_model_fact(conversation) {
            facts.push(model_fact);
        }
        if let Some(task_fact) = current_task_fact(conversation) {
            facts.push(task_fact);
        }
        if !activity.has_primary_fact()
            && let Some(lane_fact) = coarse_lane_fact(conversation)
        {
            facts.push(lane_fact);
        }
    }

    fit_activity_facts(facts, max_detail_cells)
}

fn effective_model_fact(conversation: &ConversationViewModel) -> Option<(String, Option<String>)> {
    let model = &conversation.runtime_envelope.as_ref()?.applied.model;
    match model {
        ConversationRuntimeObservedValue::Observed(model) => {
            let full = bounded_fact_value(model, MAX_MODEL_FACT_CELLS)?;
            let compact = bounded_fact_value(model, MAX_MODEL_COMPACT_FACT_CELLS)
                .map(|model| format!("model:{model}"));
            Some((format!("model:{full}"), compact))
        }
        ConversationRuntimeObservedValue::Defaulted(_) => None,
        ConversationRuntimeObservedValue::Null => Some(("model:null".to_string(), None)),
        ConversationRuntimeObservedValue::Malformed(_) => Some(("model:invalid".to_string(), None)),
        ConversationRuntimeObservedValue::UnavailableOnStableResponse
        | ConversationRuntimeObservedValue::UnavailableAfterObservationGap => {
            Some(("model:unavailable".to_string(), None))
        }
        ConversationRuntimeObservedValue::Missing => None,
    }
}

fn current_task_fact(conversation: &ConversationViewModel) -> Option<(String, Option<String>)> {
    let task = conversation.last_planning_task_handoff()?;
    let full = bounded_fact_value(&task.task_title, MAX_TASK_FACT_CELLS)
        .or_else(|| bounded_fact_value(&task.task_id, MAX_TASK_FACT_CELLS))?;
    let compact = bounded_fact_value(&task.task_id, MAX_TASK_COMPACT_FACT_CELLS)
        .map(|task_id| format!("task:{task_id}"))
        .or_else(|| Some("task:active".to_string()));
    Some((format!("task:{full}"), compact))
}

fn coarse_lane_fact(conversation: &ConversationViewModel) -> Option<(String, Option<String>)> {
    conversation.active_turn_id()?;
    let command_count = conversation.turn_activity.activity_command_count(true);
    let file_change_count = conversation.turn_activity.activity_file_change_count(true);
    let has_summary = !matches!(
        conversation.turn_activity.activity_summary(true),
        "idle" | "none"
    );
    if command_count == 0 && file_change_count == 0 && !has_summary {
        return None;
    }
    if command_count == 0 && file_change_count == 0 {
        return Some(("lane:tool".to_string(), None));
    }
    Some((
        format!("lane:cmd{command_count}/files{file_change_count}"),
        Some(format!(
            "lane:{}",
            command_count.saturating_add(file_change_count)
        )),
    ))
}

fn fit_activity_facts(
    facts: Vec<(String, Option<String>)>,
    max_detail_cells: usize,
) -> Option<String> {
    let mut line = ACTIVITY_RAIL_PREFIX.to_string();
    let mut included = 0usize;
    for (full_fact, compact_fact) in facts {
        let separator_cells = if included == 0 {
            0
        } else {
            ACTIVITY_RAIL_SEPARATOR.len()
        };
        let fits = |fact: &str| {
            display_width(&line)
                .saturating_add(separator_cells)
                .saturating_add(display_width(fact))
                <= max_detail_cells
        };
        let fact = if fits(&full_fact) {
            full_fact
        } else if let Some(compact_fact) = compact_fact.filter(|fact| fits(fact)) {
            compact_fact
        } else {
            break;
        };
        if included > 0 {
            line.push_str(ACTIVITY_RAIL_SEPARATOR);
        }
        line.push_str(&fact);
        included += 1;
    }
    (included > 0).then_some(line)
}

fn bounded_fact_value(value: &str, max_cells: usize) -> Option<String> {
    if max_cells == 0 {
        return None;
    }
    let mut output = String::new();
    let mut output_cells = 0usize;
    let mut previous_was_space = true;
    let mut truncated = false;

    for (index, character) in value.chars().enumerate() {
        if index == MAX_FACT_SCAN_CHARACTERS {
            truncated = true;
            break;
        }
        let character = if character.is_whitespace() {
            ' '
        } else if character == '|'
            || character.is_control()
            || display_width(&character.to_string()) == 0
        {
            '?'
        } else {
            character
        };
        if character == ' ' && previous_was_space {
            continue;
        }
        let character_cells = display_width(&character.to_string());
        if output_cells.saturating_add(character_cells) > max_cells {
            truncated = true;
            break;
        }
        output.push(character);
        output_cells = output_cells.saturating_add(character_cells);
        previous_was_space = character == ' ';
    }

    while output.ends_with(' ') {
        output.pop();
    }
    if output.is_empty() {
        return None;
    }
    if truncated {
        while !output.is_empty() && display_width(&output).saturating_add(3) > max_cells {
            output.pop();
        }
        let marker_cells = max_cells.saturating_sub(display_width(&output)).min(3);
        output.extend(std::iter::repeat_n('.', marker_cells));
    }
    Some(output)
}

fn display_width(value: &str) -> usize {
    Line::from(value).width()
}

const fn progressive_item_kind_label(kind: ProgressiveActivityItemKind) -> &'static str {
    match kind {
        ProgressiveActivityItemKind::UserMessage => "user",
        ProgressiveActivityItemKind::HookPrompt => "hook",
        ProgressiveActivityItemKind::AgentMessage => "agent",
        ProgressiveActivityItemKind::Plan => "plan",
        ProgressiveActivityItemKind::Reasoning => "reasoning",
        ProgressiveActivityItemKind::CommandExecution => "command",
        ProgressiveActivityItemKind::FileChange => "patch",
        ProgressiveActivityItemKind::McpToolCall => "mcp",
        ProgressiveActivityItemKind::DynamicToolCall => "tool",
        ProgressiveActivityItemKind::CollaborationAgentToolCall => "collaboration",
        ProgressiveActivityItemKind::SubAgentActivity => "sub-agent",
        ProgressiveActivityItemKind::WebSearch => "web-search",
        ProgressiveActivityItemKind::ImageView => "image-view",
        ProgressiveActivityItemKind::Sleep => "sleep",
        ProgressiveActivityItemKind::ImageGeneration => "image-generation",
        ProgressiveActivityItemKind::ReviewMode => "review",
        ProgressiveActivityItemKind::ContextCompaction => "compaction",
        ProgressiveActivityItemKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_fact_value, build_activity_rail_notice_line, display_width};
    use crate::adapter::inbound::tui::app::conversation_model::{
        ActivityRailTerminalState, ConversationViewModel, ProgressiveActivityState,
    };
    use crate::application::service::planning::PlanningTaskHandoff;
    use crate::core::app::{
        ActiveTurnPhase, ActiveTurnSnapshot, CorePromptOrigin, TurnSubmissionCorrelation,
    };
    use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};
    use crate::domain::conversation_item_lifecycle::{
        ConversationItemKind, ConversationItemLifecycleConsistency,
        ConversationItemLifecycleObservation, ConversationItemLifecyclePhase,
        ConversationItemLifecycleSource, ConversationItemOutcome,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityKind,
        ConversationProgressiveActivityObservation, ConversationProgressiveActivityPayload,
        ConversationProgressiveActivityProjection, ConversationProgressiveFileChange,
        ConversationProgressiveFileChangeKind, ConversationProgressivePlanStep,
        ConversationProgressivePlanStepStatus, ConversationProgressiveTokenUsage,
        ConversationProgressiveTokenUsageBreakdown,
    };
    use crate::domain::conversation_runtime_envelope::{
        ConversationRuntimeConfigurationObservation, ConversationRuntimeConfigurationRequest,
        ConversationRuntimeEnvelope, ConversationRuntimeLaunchEnvironment,
        ConversationRuntimeMalformedValue, ConversationRuntimeModelReroute,
        ConversationRuntimeModelRerouteReason, ConversationRuntimeObservedValue,
        ConversationRuntimeRequestedValue,
    };
    use std::time::Instant;

    const SECRET: &str = "ultra-secret-payload";

    fn set_active_turn(
        conversation: &mut ConversationViewModel,
        phase: ActiveTurnPhase,
        turn_id: Option<&str>,
        generation: u64,
    ) {
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.active_turn = Some(ActiveTurnSnapshot {
            correlation: TurnSubmissionCorrelation::new(generation),
            phase,
            workspace_directory: conversation.cwd.clone(),
            turn_id: turn_id.map(str::to_string),
            prompt_origin: CorePromptOrigin::Manual,
            started_at: Instant::now(),
        });
        conversation.apply_runtime_snapshot(snapshot);
    }

    #[test]
    fn empty_conversation_produces_no_activity_rail() {
        let conversation = ConversationViewModel::new_draft("/tmp/root".to_string());

        assert_eq!(build_activity_rail_notice_line(&conversation, 160), None);
    }

    #[test]
    fn terminal_fact_compacts_before_it_is_dropped() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.activity_rail_terminal_state =
            Some(ActivityRailTerminalState::RecoveryPending);
        assert_eq!(
            build_activity_rail_notice_line(&conversation, 24).as_deref(),
            Some("activity: term:recover")
        );

        conversation.activity_rail_terminal_state = Some(ActivityRailTerminalState::RuntimeFailed);
        assert_eq!(
            build_activity_rail_notice_line(&conversation, 32).as_deref(),
            Some("activity: term:runtime-fail")
        );
        assert_eq!(
            build_activity_rail_notice_line(&conversation, 12).as_deref(),
            Some("runtime-fail")
        );
    }

    #[test]
    fn wide_rail_orders_progress_model_task_and_coarse_lane() {
        let conversation = populated_conversation(false);
        let notice =
            build_activity_rail_notice_line(&conversation, 240).expect("wide activity rail notice");

        assert_eq!(
            notice,
            "activity: active:command | plan:2/3 +2 hidden | diff:+1 -1 h1 | cmd:2 lines | patch:3 files | mcp:1 updates | ctx:75.00% | model:applied-model | task:Ship typed priority rail"
        );
        assert!(!notice.contains("requested-model"));
        assert!(!notice.contains("lane:"));
        assert!(!notice.contains(SECRET));
    }

    #[test]
    fn narrow_rail_drops_lower_priority_facts_whole() {
        let conversation = populated_conversation(true);

        assert_eq!(
            build_activity_rail_notice_line(&conversation, 37).as_deref(),
            Some("activity: active:command")
        );
        assert_eq!(
            build_activity_rail_notice_line(&conversation, 55).as_deref(),
            Some("activity: active:command | plan:2/3 +2 hidden")
        );
        assert_eq!(
            build_activity_rail_notice_line(&conversation, 20).as_deref(),
            Some("activity: command")
        );
        assert_eq!(build_activity_rail_notice_line(&conversation, 12), None);
    }

    #[test]
    fn context_precedes_model_task_and_coarse_lane() {
        let mut conversation = running_conversation();
        conversation.progressive_activity = context_only_activity();
        conversation.runtime_envelope = Some(runtime_envelope("requested", "applied"));
        conversation.replace_planning_handoff_for_test(Some(planning_task()));
        conversation.turn_activity.current_turn_command_count = 1;
        conversation.turn_activity.current_turn_last_summary = Some("running command".to_string());

        let notice = build_activity_rail_notice_line(&conversation, 160)
            .expect("context activity rail notice");

        assert_eq!(
            notice,
            "activity: ctx:75.00% | model:applied | task:Ship typed priority rail | lane:cmd1/files0"
        );
    }

    #[test]
    fn applied_model_reroute_replaces_requested_model_without_fallback() {
        let mut conversation = running_conversation();
        let mut envelope = runtime_envelope("requested-model", "applied-model");
        envelope.apply_model_reroute(ConversationRuntimeModelReroute {
            from_model: "applied-model".to_string(),
            to_model: "rerouted-model".to_string(),
            reason: ConversationRuntimeModelRerouteReason::HighRiskCyberActivity,
        });
        conversation.runtime_envelope = Some(envelope);

        let notice = build_activity_rail_notice_line(&conversation, 80)
            .expect("effective model rail notice");

        assert!(notice.contains("model:rerouted-model"));
        assert!(!notice.contains("requested-model"));
        assert!(!notice.contains("applied-model"));
    }

    #[test]
    fn unavailable_effective_model_is_explicit_and_missing_is_omitted() {
        let mut conversation = running_conversation();
        let mut envelope = runtime_envelope("requested-model", "applied-model");
        envelope.applied.model = ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
        conversation.runtime_envelope = Some(envelope);
        assert_eq!(
            build_activity_rail_notice_line(&conversation, 80).as_deref(),
            Some("activity: model:unavailable")
        );

        conversation
            .runtime_envelope
            .as_mut()
            .expect("runtime envelope")
            .applied
            .model = ConversationRuntimeObservedValue::Missing;
        assert_eq!(build_activity_rail_notice_line(&conversation, 80), None);
    }

    #[test]
    fn defaulted_effective_model_is_benign_but_invalid_model_remains_visible() {
        let mut conversation = running_conversation();
        let mut envelope = runtime_envelope("requested-model", "applied-model");
        envelope.applied.model = ConversationRuntimeObservedValue::Defaulted("gpt-default".into());
        conversation.runtime_envelope = Some(envelope);
        assert_eq!(build_activity_rail_notice_line(&conversation, 80), None);

        conversation
            .runtime_envelope
            .as_mut()
            .expect("runtime envelope")
            .applied
            .model = ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::ExpectedString,
        );
        assert_eq!(
            build_activity_rail_notice_line(&conversation, 80).as_deref(),
            Some("activity: model:invalid")
        );
    }

    #[test]
    fn task_handoff_requires_a_correlated_submission_and_running_turn() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.replace_planning_handoff_for_test(Some(planning_task()));
        assert_eq!(build_activity_rail_notice_line(&conversation, 80), None);

        conversation.record_thread_prepared(
            "thread-1".to_string(),
            "Activity".to_string(),
            "/tmp/root".to_string(),
        );
        set_active_turn(
            &mut conversation,
            ActiveTurnPhase::Running,
            Some("turn-1"),
            1,
        );
        conversation.record_turn_started("turn-1".to_string());
        conversation.replace_planning_handoff_for_test(None);
        assert_eq!(conversation.last_planning_task_handoff(), None);
        assert_eq!(build_activity_rail_notice_line(&conversation, 80), None);

        conversation.fail_turn(Some("turn-1"), "recovered turn ended".to_string());
        conversation.replace_planning_handoff_for_test(Some(planning_task()));
        set_active_turn(
            &mut conversation,
            ActiveTurnPhase::Running,
            Some("turn-2"),
            2,
        );
        let mut runtime_snapshot = conversation.runtime_snapshot().clone();
        runtime_snapshot
            .active_turn
            .as_mut()
            .expect("running turn fixture")
            .prompt_origin = CorePromptOrigin::ManualIntake;
        conversation.apply_runtime_snapshot(runtime_snapshot);
        conversation.record_turn_started("turn-2".to_string());
        let notice = build_activity_rail_notice_line(&conversation, 80)
            .expect("correlated running task rail notice");
        assert!(notice.contains("task:Ship typed priority rail"));
    }

    #[test]
    fn submitting_turn_does_not_replay_the_previous_coarse_lane() {
        let mut conversation = running_conversation();
        conversation.turn_activity.current_turn_command_count = 2;
        conversation.turn_activity.current_turn_file_change_count = 1;
        conversation.turn_activity.current_turn_last_summary = Some("previous turn".to_string());
        conversation.fail_turn(Some("turn-1"), "failed".to_string());
        let workspace_directory = conversation.cwd.clone();
        conversation.record_submitted_prompt(
            ConversationMessage::new(ConversationMessageKind::User, "next prompt", None, None),
            workspace_directory,
            true,
        );
        set_active_turn(&mut conversation, ActiveTurnPhase::Submitting, None, 2);

        assert_eq!(build_activity_rail_notice_line(&conversation, 80), None);
    }

    #[test]
    fn dynamic_fact_values_are_control_safe_and_cell_bounded() {
        let value = format!("한글|\u{1b}[31m\u{202e}{}", "x".repeat(512));
        let bounded = bounded_fact_value(&value, 16).expect("bounded value");

        assert!(!bounded.contains('|'));
        assert!(!bounded.contains('\u{1b}'));
        assert!(!bounded.contains('\u{202e}'));
        assert!(bounded.contains('?'));
        assert!(bounded.ends_with("..."));
        assert!(display_width(&bounded) <= 16, "{bounded:?}");
    }

    #[test]
    fn dynamic_facts_cannot_inject_a_rail_separator() {
        let mut conversation = running_conversation();
        conversation.runtime_envelope =
            Some(runtime_envelope("requested", "model | terminal:failed"));
        let mut task = planning_task();
        task.task_title = "ship | terminal:failed".to_string();
        conversation.replace_planning_handoff_for_test(Some(task));

        let notice =
            build_activity_rail_notice_line(&conversation, 160).expect("sanitized dynamic facts");

        assert!(notice.contains("model:model ? terminal:failed"));
        assert!(notice.contains("task:ship ? terminal:failed"));
        assert!(!notice.contains(" | terminal:failed"));
    }

    #[test]
    fn task_title_scan_is_bounded_and_falls_back_to_the_task_id() {
        let mut conversation = running_conversation();
        let mut task = planning_task();
        task.task_title = format!("{}{SECRET}", " ".repeat(4_096));
        conversation.replace_planning_handoff_for_test(Some(task));

        let notice = build_activity_rail_notice_line(&conversation, 80)
            .expect("bounded task fallback rail notice");

        assert!(notice.contains("task:task-priority-rail"));
        assert!(!notice.contains(SECRET));
    }

    #[test]
    fn bounded_history_never_exposes_retained_payload_text() {
        let conversation = populated_conversation(true);
        let notice = build_activity_rail_notice_line(&conversation, 280)
            .expect("bounded activity rail notice");

        let context_index = notice.find("ctx:").expect("context fact");
        let history_index = notice.find("history:bounded").expect("history fact");
        let model_index = notice.find("model:").expect("model fact");
        assert!(context_index < history_index);
        assert!(history_index < model_index);
        assert!(!notice.contains(SECRET));
    }

    fn populated_conversation(bounded_history: bool) -> ConversationViewModel {
        let mut conversation = running_conversation();
        conversation.progressive_activity = populated_activity(bounded_history);
        conversation.runtime_envelope = Some(runtime_envelope("requested-model", "applied-model"));
        conversation.replace_planning_handoff_for_test(Some(planning_task()));
        conversation.turn_activity.current_turn_command_count = 1;
        conversation.turn_activity.current_turn_file_change_count = 2;
        conversation.turn_activity.current_turn_last_summary =
            Some("raw coarse detail".to_string());
        conversation
    }

    fn running_conversation() -> ConversationViewModel {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.record_thread_prepared(
            "thread-1".to_string(),
            "Activity".to_string(),
            "/tmp/root".to_string(),
        );
        set_active_turn(
            &mut conversation,
            ActiveTurnPhase::Running,
            Some("turn-1"),
            1,
        );
        conversation.record_turn_started("turn-1".to_string());
        conversation
    }

    fn runtime_envelope(requested_model: &str, applied_model: &str) -> ConversationRuntimeEnvelope {
        let request = ConversationRuntimeConfigurationRequest {
            model: ConversationRuntimeRequestedValue::Value(requested_model.to_string()),
            ..ConversationRuntimeConfigurationRequest::default()
        };
        ConversationRuntimeEnvelope::prepared(
            request,
            ConversationRuntimeConfigurationObservation {
                model: ConversationRuntimeObservedValue::Observed(applied_model.to_string()),
                ..ConversationRuntimeConfigurationObservation::default()
            },
            ConversationRuntimeLaunchEnvironment::unknown(),
            ConversationRuntimeObservedValue::Missing,
        )
    }

    fn planning_task() -> PlanningTaskHandoff {
        PlanningTaskHandoff {
            task_id: "task-priority-rail".to_string(),
            task_title: "Ship typed priority rail".to_string(),
            direction_id: "direction-tui".to_string(),
            combined_priority: 90,
            updated_at: "2026-07-13T00:00:00Z".to_string(),
            status_label: "ready".to_string(),
        }
    }

    fn populated_activity(bounded_history: bool) -> ProgressiveActivityState {
        let command_tail = format!("{SECRET}\nsecond line\n");
        let patch = ConversationProgressiveFileChange {
            path: "src/private.rs".to_string(),
            diff: SECRET.to_string(),
            kind: ConversationProgressiveFileChangeKind::Update {
                move_path_present: false,
            },
        };
        let patch_source_bytes = patch.path.len().saturating_add(patch.diff.len()) as u64;
        let explanation = SECRET.to_string();
        let plan_steps = vec![
            plan_step(ConversationProgressivePlanStepStatus::Completed),
            plan_step(ConversationProgressivePlanStepStatus::Completed),
            plan_step(ConversationProgressivePlanStepStatus::InProgress),
        ];
        let plan_source_bytes = explanation
            .len()
            .saturating_add(plan_steps.iter().map(|step| step.text.len()).sum::<usize>())
            as u64;
        let observations = vec![
            observation(
                0,
                Some("command-1"),
                ConversationProgressiveActivityKind::CommandOutput,
                ConversationProgressiveActivityPayload::CommandOutput {
                    tail: command_tail.clone(),
                    chunk_count: 1,
                    source_bytes: command_tail.len() as u64,
                    newline_count: 2,
                    ends_with_newline: true,
                    truncated_bytes: 0,
                },
            ),
            observation(
                1,
                Some("patch-1"),
                ConversationProgressiveActivityKind::FileChangePatch,
                ConversationProgressiveActivityPayload::FileChangePatch {
                    changes: vec![patch],
                    omitted_change_count: 2,
                    source_bytes: patch_source_bytes,
                    truncated_bytes: 0,
                },
            ),
            observation(
                2,
                None,
                ConversationProgressiveActivityKind::TurnPlan,
                ConversationProgressiveActivityPayload::TurnPlan {
                    explanation: Some(explanation),
                    steps: plan_steps,
                    omitted_step_count: 2,
                    source_bytes: plan_source_bytes,
                    truncated_bytes: 0,
                },
            ),
            observation(
                3,
                Some("mcp-1"),
                ConversationProgressiveActivityKind::McpProgress,
                ConversationProgressiveActivityPayload::McpProgress {
                    message: SECRET.to_string(),
                    update_count: 1,
                    source_bytes: SECRET.len() as u64,
                    truncated_bytes: 0,
                },
            ),
            observation(
                4,
                None,
                ConversationProgressiveActivityKind::TurnDiff,
                ConversationProgressiveActivityPayload::TurnDiff {
                    detail: SECRET.to_string(),
                    source_bytes: SECRET.len() as u64,
                    line_count: 3,
                    addition_count: 1,
                    deletion_count: 1,
                    hunk_count: 1,
                    truncated_bytes: 0,
                },
            ),
            observation(
                5,
                None,
                ConversationProgressiveActivityKind::TokenUsage,
                ConversationProgressiveActivityPayload::TokenUsage {
                    usage: ConversationProgressiveTokenUsage {
                        last: token_breakdown(75),
                        total: token_breakdown(75),
                        model_context_window: Some(100),
                    },
                },
            ),
        ];

        let mut projection = ConversationProgressiveActivityProjection::default();
        for observation in observations {
            projection
                .apply_batch_correlated(
                    Some("thread-1"),
                    Some("turn-1"),
                    ConversationProgressiveActivityBatch::single(observation)
                        .expect("valid activity batch"),
                )
                .expect("apply activity batch");
        }
        let snapshot = projection.snapshot();
        let mut state = ProgressiveActivityState::default();
        for (item_id, kind) in [
            ("command-1", ConversationItemKind::CommandExecution),
            ("patch-1", ConversationItemKind::FileChange),
            ("mcp-1", ConversationItemKind::McpToolCall),
        ] {
            state.observe_item_lifecycle(
                &ConversationItemLifecycleObservation {
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                    item_id: item_id.to_string(),
                    kind,
                    phase: ConversationItemLifecyclePhase::Started,
                    source: ConversationItemLifecycleSource::Live,
                    observed_at_ms: None,
                    outcome: ConversationItemOutcome::InProgress,
                    summary: SECRET.to_string(),
                    command_actions: Default::default(),
                },
                Some(ConversationItemLifecycleConsistency::Accepted),
            );
        }
        state.apply_projection_update(
            snapshot.as_ref(),
            Some(0),
            Some(5),
            u64::from(bounded_history),
            0,
            0,
            0,
        );
        state
    }

    fn context_only_activity() -> ProgressiveActivityState {
        let observation = observation(
            0,
            None,
            ConversationProgressiveActivityKind::TokenUsage,
            ConversationProgressiveActivityPayload::TokenUsage {
                usage: ConversationProgressiveTokenUsage {
                    last: token_breakdown(75),
                    total: token_breakdown(75),
                    model_context_window: Some(100),
                },
            },
        );
        let batch = ConversationProgressiveActivityBatch::single(observation)
            .expect("context activity should be valid");
        let mut projection = ConversationProgressiveActivityProjection::default();
        projection
            .apply_batch_correlated(Some("thread-1"), Some("turn-1"), batch)
            .expect("context activity should project");
        let snapshot = projection.snapshot();
        let mut state = ProgressiveActivityState::default();
        state.apply_projection_update(snapshot.as_ref(), Some(0), Some(0), 0, 0, 0, 0);
        state
    }

    fn observation(
        sequence: u64,
        item_id: Option<&str>,
        kind: ConversationProgressiveActivityKind,
        payload: ConversationProgressiveActivityPayload,
    ) -> ConversationProgressiveActivityObservation {
        ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            item_id: item_id.map(str::to_string),
            kind,
            payload,
        }
    }

    fn plan_step(status: ConversationProgressivePlanStepStatus) -> ConversationProgressivePlanStep {
        ConversationProgressivePlanStep {
            status,
            text: SECRET.to_string(),
        }
    }

    const fn token_breakdown(total_tokens: u64) -> ConversationProgressiveTokenUsageBreakdown {
        ConversationProgressiveTokenUsageBreakdown {
            cached_input_tokens: 0,
            input_tokens: total_tokens,
            output_tokens: 0,
            reasoning_output_tokens: 0,
            total_tokens,
        }
    }
}
