use ratatui::text::Line;

use super::super::ConversationViewModel;
use super::super::{
    INLINE_TAIL_THREAD_LABEL_LIMIT, INLINE_TAIL_WARNING_DETAIL_LIMIT, NativeTuiApp,
    compact_inline_detail, format_conversation_lines,
};

/*
 * tail_shared는 inline tail과 footer가 같이 쓰는 "짧은 상태 문장"의 정책 모듈이다.
 * renderer마다 직접 ConversationViewModel을 뒤지면 thread label, auto-follow 상태, operator notice의
 * 우선순위와 축약 규칙이 달라지기 쉽다. 그래서 이 파일이 공통 copy를 만들고, tail_copy/footer_copy는
 * 배치와 스타일에만 집중한다.
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

pub(super) fn parallel_mode_summary_line(app: &NativeTuiApp) -> String {
    /*
     * parallel mode summary는 readiness, mode toggle, pool, roster, distributor queue를 한 줄로 압축한다.
     * supersession overlay와 footer가 모두 이 문장을 읽으므로, app-wide snapshot 조합을 이곳에 둔다.
     */
    match app.parallel_mode_readiness_snapshot() {
        Some(snapshot) => {
            let supervisor_snapshot = app.parallel_mode_supervisor_snapshot();
            format!(
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
            )
        }
        /*
         * mode는 켜졌지만 readiness snapshot이 아직 없으면 background reconcile 전이다.
         * 이 상태를 "off"로 보이면 사용자가 toggle이 먹지 않았다고 오해하므로 preparing copy를 별도로 둔다.
         */
        None if app.parallel_mode_enabled() => {
            "parallel: preparing  |  mode: parallel  |  pool: pending reconcile  |  agents: 0 active  |  queue: pending".to_string()
        }
        /*
         * snapshot도 없고 mode도 꺼져 있으면 parallel subsystem은 의도적으로 inactive다.
         * pool/agents/queue를 모두 inactive로 맞춰 readiness failure와 구분한다.
         */
        None => {
            "parallel: off  |  mode: normal  |  pool: inactive  |  agents: inactive  |  queue: inactive".to_string()
        }
    }
}

pub(super) fn parallel_mode_alert_line(app: &NativeTuiApp) -> Option<String> {
    /*
     * readiness snapshot의 top_alert는 missing worktree, dirty integration branch 같은 즉시 조치 항목이다.
     * summary line과 분리해 tail/footer가 경고를 한 줄 더 강조할 수 있게 한다.
     */
    app.parallel_mode_readiness_snapshot()
        .and_then(|snapshot| snapshot.top_alert.as_deref())
        .map(|alert| format!("parallel alert: {alert}"))
}

pub(super) fn build_operator_notice_line(
    github_review_recent_changes_summary: Option<&str>,
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Option<String> {
    /*
     * operator notice는 제한된 tail/footer 공간에서 "지금 사람이 봐야 할 것"을 하나만 고른다.
     * 우선순위는 GitHub review 변화, 실행 중 tool activity, 마지막 auto-follow 결과, 잔여 tool activity,
     * approval 상태 순이다. 이렇게 해야 오래된 approval copy가 새 review나 실행 활동을 가리지 않는다.
     */
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

    if let Some(activity) = conversation.last_auto_followup_activity.as_ref() {
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
