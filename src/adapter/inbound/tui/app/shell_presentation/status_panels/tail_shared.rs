use ratatui::text::Line;

use crate::adapter::inbound::tui::app::conversation_model::{
    ProgressiveActivityItemKind, ProgressiveActivityState,
};

use super::super::{
    ConversationViewModel, INLINE_TAIL_THREAD_LABEL_LIMIT, INLINE_TAIL_WARNING_DETAIL_LIMIT,
    NativeTuiApp, compact_inline_detail, format_conversation_lines,
};

const PROGRESSIVE_ACTIVITY_PREFIX: &str = "activity: ";
const PROGRESSIVE_ACTIVITY_SEPARATOR: &str = " | ";

/*
 * tail_shared는 inline tail이 쓰는 "짧은 상태 문장"의 정책 모듈이다.
 * renderer마다 직접 ConversationViewModel을 뒤지면 thread label, auto-follow 상태, operator notice의
 * 우선순위와 축약 규칙이 달라지기 쉽다. 그래서 이 파일이 공통 copy를 만들고, tail_copy는 배치와 스타일에만 집중한다.
 */
pub(super) fn current_live_agent_lines(
    conversation: &ConversationViewModel,
) -> Option<Vec<Line<'static>>> {
    /*
     * live_agent_message는 현재 streaming 중인 agent 답변의 최신 조각이다.
     * None이면 tail/footer가 별도 live block을 그릴 필요가 없고, Some이면 일반 transcript formatter를
     * 재사용해 live preview와 저장된 대화가 같은 markdown/text 규칙을 따르게 한다.
     */
    let message = conversation.live_agent_message.as_ref()?;
    Some(format_conversation_lines(std::slice::from_ref(message)))
}

pub(super) fn parallel_mode_summary_line(app: &NativeTuiApp) -> Option<String> {
    /*
     * parallel mode summary는 readiness, mode toggle, pool, roster, distributor queue를 한 줄로 압축한다.
     * supersession overlay와 footer가 모두 이 문장을 읽으므로, app-wide snapshot 조합을 이곳에 둔다.
     */
    match app.parallel_mode_readiness_snapshot() {
        Some(snapshot) => {
            let supervisor_snapshot = app.parallel_mode_supervisor_snapshot();
            Some(format!(
                "parallel: {}  |  mode: {}  |  pool: {}  |  agents: {}  |  queue: {}",
                snapshot.readiness_label(),
                if app.parallel_mode_enabled() {
                    "parallel"
                } else {
                    "normal"
                },
                supervisor_snapshot.pool.compact_summary(),
                supervisor_snapshot.roster.compact_summary(),
                supervisor_snapshot.distributor.compact_summary(),
            ))
        }
        /*
         * mode는 켜졌지만 readiness snapshot이 아직 없으면 background reconcile 전이다.
         * 이 상태를 "off"로 보이면 사용자가 toggle이 먹지 않았다고 오해하므로 preparing copy를 별도로 둔다.
         */
        None if app.parallel_mode_enabled() => {
            Some(
                "parallel: preparing  |  mode: parallel  |  pool: pending reconcile  |  agents: 0 active  |  queue: pending".to_string(),
            )
        }
        /*
         * snapshot도 없고 mode도 꺼져 있으면 parallel subsystem은 의도적으로 inactive다.
         * 이 상태는 operator가 조치할 정보가 없으므로 inline tail에서는 숨긴다.
         */
        None => None,
    }
}

pub(super) fn parallel_mode_alert_line(app: &NativeTuiApp) -> Option<String> {
    /*
     * readiness snapshot의 top_alert는 missing worktree, dirty integration branch 같은 즉시 조치 항목이다.
     * summary line과 분리해 tail/footer가 경고를 한 줄 더 강조할 수 있게 한다.
     */
    app.parallel_mode_readiness_snapshot()
        .and_then(|snapshot| snapshot.top_alert)
        .map(|alert| format!("parallel alert: {alert}"))
}

pub(super) fn build_operator_notice_line(
    github_review_recent_changes_summary: Option<&str>,
    conversation: &ConversationViewModel,
    max_detail_len: usize,
    max_progressive_notice_len: usize,
) -> Option<String> {
    /*
     * operator notice는 제한된 tail/footer 공간에서 "지금 사람이 봐야 할 것"을 하나만 고른다.
     * pending approval은 operator action이 필요한 control boundary라 모든 activity보다 먼저 보인다.
     * 그 다음 progressive activity, GitHub review 변화, 기존 tool activity, auto-follow 결과 순으로
     * transient copy를 고른다.
     */
    if conversation.pending_approval_request.is_some() {
        return Some(if conversation.pending_approval_decision().is_some() {
            "approval: decision submitted".to_string()
        } else {
            "approval: decision required".to_string()
        });
    }

    let progressive_activity_line = build_progressive_activity_notice_line(
        &conversation.progressive_activity,
        max_progressive_notice_len,
    );
    if conversation.progressive_activity.has_primary_fact()
        && let Some(activity_line) = progressive_activity_line.as_ref()
    {
        return Some(activity_line.clone());
    }

    if let Some(github_review_summary) = github_review_recent_changes_summary {
        return Some(format!(
            "gh update: {}",
            compact_inline_detail(github_review_summary, max_detail_len)
        ));
    }

    /*
     * turn_activity는 현재 turn이 running인지 여부에 따라 "이번 turn"과 "마지막 turn"의 의미가 달라진다.
     * helper에 turn_running을 넘겨 footer copy가 active stream과 post-turn summary를 같은 방식으로 축약한다.
     */
    let turn_running = conversation.has_running_turn();
    let activity_scope = conversation
        .turn_activity
        .activity_scope_label(turn_running);
    let activity_summary = conversation.turn_activity.activity_summary(turn_running);
    let activity_command_count = conversation
        .turn_activity
        .activity_command_count(turn_running);
    let activity_file_change_count = conversation
        .turn_activity
        .activity_file_change_count(turn_running);
    let has_tool_activity = (activity_summary != "idle" && activity_summary != "none")
        || activity_command_count > 0
        || activity_file_change_count > 0;
    if turn_running && has_tool_activity {
        /*
         * 실행 중 tool activity는 live feedback이므로 auto-follow나 approval보다 먼저 보여 준다.
         * approval summary가 있으면 같은 line 끝에 붙여 사용자가 승인 대기와 tool activity를 동시에 볼 수 있게 한다.
         */
        let mut notice_line = format!(
            "tool activity: {}  |  {activity_scope} commands: {}  |  {activity_scope} file changes: {}",
            compact_inline_detail(activity_summary, max_detail_len),
            activity_command_count,
            activity_file_change_count,
        );
        if let Some(approval_summary) = conversation.approval_summary().as_deref() {
            notice_line.push_str(&format!(
                "  |  approval: {}",
                compact_inline_detail(approval_summary, max_detail_len)
            ));
        }
        return Some(notice_line);
    }

    // Context pressure and bounded-history markers are passive diagnostics. They
    // remain visible when no live tool summary exists, but never replace active work.
    if let Some(activity_line) = progressive_activity_line {
        return Some(activity_line);
    }

    if let Some(activity) = conversation.last_auto_follow_activity.as_ref() {
        /*
         * auto-follow 결과는 turn 종료 직후 operator가 다음 자동 동작이 왜 이어졌거나 멈췄는지 보는 copy다.
         * running tool activity가 없을 때만 보여 주어 현재 실행 상황을 가리지 않는다.
         */
        return Some(format!(
            "auto: {}  |  detail: {}",
            activity.summary,
            compact_inline_detail(&activity.detail, max_detail_len)
        ));
    }

    if has_tool_activity {
        /*
         * turn이 끝난 뒤에도 command/file-change count는 마지막 활동 요약으로 의미가 있다.
         * live 상태는 아니지만 approval과 함께 operator notice로 남겨 최근 변경 맥락을 보존한다.
         */
        let mut notice_line = format!(
            "tool activity: {}  |  {activity_scope} commands: {}  |  {activity_scope} file changes: {}",
            compact_inline_detail(activity_summary, max_detail_len),
            activity_command_count,
            activity_file_change_count,
        );
        if let Some(approval_summary) = conversation.approval_summary().as_deref() {
            notice_line.push_str(&format!(
                "  |  approval: {}",
                compact_inline_detail(approval_summary, max_detail_len)
            ));
        }
        return Some(notice_line);
    }

    /*
     * 아무 실행/auto-follow/review notice가 없을 때 approval 상태만 남긴다.
     * approval은 중요하지만 stale하게 오래 남을 수 있어 더 높은 우선순위의 활동에는 자리를 양보한다.
     */
    conversation.approval_summary().map(|approval_summary| {
        format!(
            "approval: {}",
            compact_inline_detail(&approval_summary, max_detail_len)
        )
    })
}

fn build_progressive_activity_notice_line(
    activity: &ProgressiveActivityState,
    max_detail_len: usize,
) -> Option<String> {
    /*
     * The rail only reads bounded counters and enums. Every fact is ASCII and
     * indivisible: when the width budget is exhausted, lower-priority facts are
     * dropped instead of truncating a label or exposing retained payload text.
     */
    let mut facts: Vec<(String, Option<String>)> = Vec::new();
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
    if let Some(kind) = activity.active_item_kind() {
        let label = progressive_item_kind_label(kind);
        facts.push((format!("active:{label}"), Some(label.to_string())));
    }
    if activity.mcp_update_count() > 0 {
        facts.push((
            format!("mcp:{} updates", activity.mcp_update_count()),
            Some(format!("mcp:{}", activity.mcp_update_count())),
        ));
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
    if let Some(basis_points) = activity.context_pressure_basis_points() {
        facts.push((
            format!("ctx:{}.{:02}%", basis_points / 100, basis_points % 100),
            None,
        ));
    }
    if activity.bounded_history() {
        facts.push(("history:bounded".to_string(), None));
    }

    let mut line = PROGRESSIVE_ACTIVITY_PREFIX.to_string();
    let mut included = 0usize;
    for (full_fact, compact_fact) in facts {
        let separator_len = if included == 0 {
            0
        } else {
            PROGRESSIVE_ACTIVITY_SEPARATOR.len()
        };
        let fits = |fact: &str| {
            line.len()
                .saturating_add(separator_len)
                .saturating_add(fact.len())
                <= max_detail_len
        };
        let fact = if fits(&full_fact) {
            full_fact
        } else if let Some(compact_fact) = compact_fact.filter(|fact| fits(fact)) {
            compact_fact
        } else {
            break;
        };
        if included > 0 {
            line.push_str(PROGRESSIVE_ACTIVITY_SEPARATOR);
        }
        line.push_str(&fact);
        included += 1;
    }

    (included > 0).then_some(line)
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

pub(super) fn compact_inline_summary_label(summary: &str) -> String {
    /*
     * runtime warning/notices copy는 원문 그대로 두면 inline tail의 좁은 폭을 빨리 넘긴다.
     * 의미를 유지하는 짧은 약어로 먼저 바꾼 뒤 공통 truncation helper에 넘긴다.
     */
    compact_inline_detail(
        &summary
            .replace("runtime warning:", "rt warn:")
            .replace("runtime warnings", "rt warns")
            .replace("warning:", "warn:")
            .replace("warnings:", "warn:")
            .replace("runtime notices", "notices")
            .replace("runtime:", "notice:"),
        INLINE_TAIL_WARNING_DETAIL_LIMIT,
    )
}

pub(super) fn compact_auto_follow_status_summary(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> String {
    /*
     * auto-follow prompt/footer copy는 queue-driven 상태와 internal pause 상태를 구분해야 한다.
     * pause flag가 있으면 activity label보다 우선해 "paused/internal"을 보여 주고, 아니면 queue 상태를 붙인다.
     */
    let summary = if conversation
        .auto_follow_state
        .post_turn_continuation_paused()
    {
        "paused/internal".to_string()
    } else {
        format!("queue/{}", conversation.auto_follow_state.activity_label())
    };
    compact_inline_detail(&summary, max_detail_len)
}

pub(super) fn inline_thread_label(conversation: &ConversationViewModel) -> String {
    /*
     * active thread가 없는 draft는 session title이 아직 의미 있는 anchor가 아니다.
     * "new draft"를 고정 copy로 쓰고, 기존 thread만 title을 폭 제한에 맞게 축약한다.
     */
    if !conversation.has_active_thread() {
        return "new draft".to_string();
    }

    compact_inline_detail(&conversation.title, INLINE_TAIL_THREAD_LABEL_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
    };
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

    const SECRET: &str = "ultra-secret-payload";

    #[test]
    fn empty_progressive_activity_produces_no_notice() {
        assert_eq!(
            build_progressive_activity_notice_line(&ProgressiveActivityState::default(), 160),
            None
        );
    }

    #[test]
    fn pending_approval_wins_over_review_and_progressive_activity() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.progressive_activity = populated_activity(false);
        conversation.pending_approval_request = Some(ConversationApprovalRequest {
            approval_id: "approval-1".to_string(),
            server_request_id: "request-1".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: SECRET.to_string(),
            details: vec![SECRET.to_string()],
        });

        let notice = build_operator_notice_line(Some("review changed"), &conversation, 160, 160)
            .expect("pending approval notice");

        assert_eq!(notice, "approval: decision required");
        assert!(!notice.contains("activity:"));
        assert!(!notice.contains(SECRET));
    }

    #[test]
    fn wide_progressive_activity_keeps_ordered_multi_fact_summary() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.progressive_activity = populated_activity(false);
        let notice = build_operator_notice_line(Some("review changed"), &conversation, 200, 200)
            .expect("wide activity notice");

        assert_eq!(
            notice,
            "activity: cmd:2 lines | patch:3 files | plan:2/3 +2 hidden | active:command | mcp:1 updates | diff:+1 -1 h1 | ctx:75.00%"
        );
    }

    #[test]
    fn narrow_progressive_activity_drops_lower_priority_whole_facts() {
        let notice = build_progressive_activity_notice_line(&populated_activity(true), 37)
            .expect("narrow activity notice");

        assert_eq!(notice, "activity: cmd:2 lines | patch:3 files");
        assert_eq!(notice.len(), 37);
        assert!(!notice.contains("plan:"));
        assert!(!notice.contains("..."));

        assert_eq!(
            build_progressive_activity_notice_line(&populated_activity(true), 55).as_deref(),
            Some("activity: cmd:2 lines | patch:3 files | plan:2/3+2?")
        );
        assert!(
            !build_progressive_activity_notice_line(&populated_activity(true), 55)
                .expect("compact plan notice")
                .contains("plan:2/5")
        );

        assert_eq!(
            build_progressive_activity_notice_line(&populated_activity(true), 20).as_deref(),
            Some("activity: cmd:2")
        );
        assert_eq!(
            build_progressive_activity_notice_line(&populated_activity(true), 12),
            None
        );
    }

    #[test]
    fn bounded_history_is_explicit_without_retained_secret_text() {
        let notice = build_progressive_activity_notice_line(&populated_activity(true), 240)
            .expect("bounded activity notice");

        assert!(notice.ends_with("history:bounded"));
        assert!(!notice.contains(SECRET));
        assert!(notice.is_ascii());
    }

    #[test]
    fn passive_context_does_not_hide_running_tool_activity() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.record_thread_prepared(
            "thread-1".to_string(),
            "Activity".to_string(),
            "/tmp/root".to_string(),
        );
        conversation.record_turn_started("turn-1".to_string());
        conversation.progressive_activity = context_only_activity();
        conversation.turn_activity.current_turn_command_count = 1;
        conversation.turn_activity.current_turn_last_summary = Some("running command".to_string());

        let notice = build_operator_notice_line(None, &conversation, 160, 160)
            .expect("running tool activity notice");

        assert!(notice.starts_with("tool activity: running command"));
        assert!(!notice.contains("ctx:"));
    }

    #[test]
    fn responsive_rail_budget_does_not_expand_existing_notice_detail_limit() {
        let conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        let notice =
            build_operator_notice_line(Some(&"review detail ".repeat(20)), &conversation, 40, 72)
                .expect("GitHub notice");

        assert!(notice.len() <= 72, "{notice:?}");
        assert!(notice.starts_with("gh update: "));
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
        state.observe_item_lifecycle(
            &ConversationItemLifecycleObservation {
                thread_id: "thread-1".to_string(),
                turn_id: "turn-1".to_string(),
                item_id: "command-1".to_string(),
                kind: ConversationItemKind::CommandExecution,
                phase: ConversationItemLifecyclePhase::Started,
                source: ConversationItemLifecycleSource::Live,
                observed_at_ms: None,
                outcome: ConversationItemOutcome::InProgress,
                summary: SECRET.to_string(),
            },
            Some(ConversationItemLifecycleConsistency::Accepted),
        );
        for (item_id, kind) in [
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
