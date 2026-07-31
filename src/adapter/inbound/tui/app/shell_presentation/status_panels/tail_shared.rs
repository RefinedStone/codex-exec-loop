use ratatui::text::Line;

use crate::adapter::inbound::tui::supersession_mud::parallel_mode_progress_summary;

use super::super::{
    AutoFollowSnapshotPresentation, ConversationLiveTranscriptScreenModel, ConversationScreenModel,
    ConversationViewModel, compact_inline_detail, format_conversation_lines,
};
use super::activity_rail::build_activity_rail_notice_line;

/*
 * tail_shared는 inline tail이 쓰는 "짧은 상태 문장"의 정책 모듈이다.
 * renderer마다 직접 ConversationViewModel을 뒤지면 thread label, auto-follow 상태, operator notice의
 * 우선순위와 축약 규칙이 달라지기 쉽다. 그래서 이 파일이 공통 copy를 만들고, tail_copy는 배치와 스타일에만 집중한다.
 */
pub(super) fn current_live_agent_lines(
    live_transcript: &ConversationLiveTranscriptScreenModel<'_>,
) -> Option<Vec<Line<'static>>> {
    /*
     * Host scrollback으로 아직 넘기지 않은 완료 메시지와 현재 streaming item을 순서대로 그린다.
     * 동일 formatter를 재사용해 임시 viewport와 저장된 transcript의 markdown 규칙을 맞춘다.
     */
    let mut lines = Vec::new();
    if let Some(messages) = live_transcript.handoff_messages {
        lines.extend(format_conversation_lines(messages));
    }
    if let Some(message) = live_transcript.live_agent_message {
        lines.extend(format_conversation_lines(std::slice::from_ref(message)));
    }
    (!lines.is_empty()).then_some(lines)
}

pub(super) fn parallel_mode_summary_line(
    screen_model: &ConversationScreenModel<'_>,
) -> Option<String> {
    /*
     * 기본 tail은 사용자가 지금 기다려야 하는 단계만 보여 준다. pool 내부 ID, agent 수, distributor
     * 구현 용어는 supersession board/event stream에서 계속 확인할 수 있지만, 일상 화면에서는 작업 흐름과
     * 사용 가능한 capacity가 먼저 보여야 dispatch 지연을 idle로 오해하지 않는다.
     */
    match screen_model.parallel_mode_readiness.as_ref() {
        Some(_) if screen_model.parallel_mode_enabled => {
            let progress = parallel_mode_progress_summary(
                &screen_model.parallel_mode_supervisor,
                screen_model.planning_runtime_projection.queue_projection(),
                screen_model.parallel_mode_control_effect_in_flight,
            );
            Some(format!("Parallel  {}", progress.compact_line()))
        }
        Some(_) => None,
        /*
         * mode는 켜졌지만 readiness snapshot이 아직 없으면 background reconcile 전이다.
         * 이 상태를 "off"로 보이면 사용자가 toggle이 먹지 않았다고 오해하므로 preparing copy를 별도로 둔다.
         */
        None if screen_model.parallel_mode_enabled => {
            Some("Parallel  ◐ preparing workspace".to_string())
        }
        /*
         * snapshot도 없고 mode도 꺼져 있으면 parallel subsystem은 의도적으로 inactive다.
         * 이 상태는 operator가 조치할 정보가 없으므로 inline tail에서는 숨긴다.
         */
        None => None,
    }
}

pub(super) fn parallel_mode_alert_line(
    screen_model: &ConversationScreenModel<'_>,
) -> Option<String> {
    /*
     * readiness snapshot의 top_alert는 missing worktree, dirty integration branch 같은 즉시 조치 항목이다.
     * summary line과 분리해 tail/footer가 경고를 한 줄 더 강조할 수 있게 한다.
     */
    screen_model
        .parallel_mode_readiness
        .as_ref()
        .and_then(|snapshot| snapshot.top_alert.as_deref())
        .map(|alert| format!("parallel alert: {alert}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperatorNoticeKind {
    RequiredAction,
    TerminalActivity,
    Activity,
    Detail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OperatorNotice {
    pub(super) text: String,
    pub(super) kind: OperatorNoticeKind,
}

impl OperatorNotice {
    fn new(kind: OperatorNoticeKind, text: String) -> Self {
        Self { text, kind }
    }
}

pub(super) fn build_operator_notice(
    github_review_recent_changes_summary: Option<&str>,
    conversation: &ConversationViewModel,
    max_detail_len: usize,
    max_progressive_notice_len: usize,
) -> Option<OperatorNotice> {
    /*
     * operator notice는 제한된 tail/footer 공간에서 "지금 사람이 봐야 할 것"을 하나만 고른다.
     * pending approval은 operator action이 필요한 control boundary라 모든 activity보다 먼저 보인다.
     * 그 다음 progressive activity, GitHub review 변화, 기존 tool activity, auto-follow 결과 순으로
     * transient copy를 고른다.
     */
    if conversation.pending_approval_request().is_some() {
        return Some(OperatorNotice::new(
            OperatorNoticeKind::RequiredAction,
            if conversation.pending_approval_decision().is_some() {
                "approval: decision submitted".to_string()
            } else {
                "approval: decision required".to_string()
            },
        ));
    }

    let activity_rail_line =
        build_activity_rail_notice_line(conversation, max_progressive_notice_len);
    if conversation.activity_rail_terminal_state.is_some() {
        return activity_rail_line
            .map(|line| OperatorNotice::new(OperatorNoticeKind::TerminalActivity, line));
    }
    if conversation.progressive_activity.has_primary_fact() || conversation.has_running_turn() {
        return activity_rail_line
            .map(|line| OperatorNotice::new(OperatorNoticeKind::Activity, line));
    }
    if !conversation.can_accept_runtime_prompt() {
        // A Core-accepted submission is already the current operator focus even
        // before TurnStarted supplies an id. Do not fill that gap with stale
        // activity or a lower-priority review from the previous turn.
        return None;
    }

    if let Some(github_review_summary) = github_review_recent_changes_summary {
        return Some(OperatorNotice::new(
            OperatorNoticeKind::Detail,
            format!(
                "gh update: {}",
                compact_inline_detail(github_review_summary, max_detail_len)
            ),
        ));
    }

    // Live activity is owned exclusively by the typed rail above. The legacy
    // summary remains only as a post-turn recap.
    let activity_scope = conversation.turn_activity.activity_scope_label(false);
    let activity_summary = conversation.turn_activity.activity_summary(false);
    let activity_command_count = conversation.turn_activity.activity_command_count(false);
    let activity_file_change_count = conversation.turn_activity.activity_file_change_count(false);
    let has_tool_activity = (activity_summary != "idle" && activity_summary != "none")
        || activity_command_count > 0
        || activity_file_change_count > 0;

    // Context pressure and bounded-history markers are passive diagnostics. They
    // remain visible when no live tool summary exists, but never replace active work.
    if let Some(activity_line) = activity_rail_line {
        return Some(OperatorNotice::new(
            OperatorNoticeKind::Activity,
            activity_line,
        ));
    }

    if let Some(activity) = conversation.last_auto_follow_activity.as_ref() {
        /*
         * auto-follow 결과는 turn 종료 직후 operator가 다음 자동 동작이 왜 이어졌거나 멈췄는지 보는 copy다.
         * running tool activity가 없을 때만 보여 주어 현재 실행 상황을 가리지 않는다.
         */
        return Some(OperatorNotice::new(
            OperatorNoticeKind::Detail,
            format!(
                "auto: {}  |  detail: {}",
                activity.summary,
                compact_inline_detail(&activity.detail, max_detail_len)
            ),
        ));
    }

    if has_tool_activity {
        /*
         * turn이 끝난 뒤에도 command/file-change count는 마지막 활동 요약으로 의미가 있다.
         * live 상태는 아니지만 approval과 함께 operator notice로 남겨 최근 변경 맥락을 보존한다.
         */
        let mut parts = vec![format!(
            "tool activity: {}",
            compact_inline_detail(activity_summary, max_detail_len)
        )];
        if activity_command_count > 0 {
            parts.push(format!(
                "{activity_scope} commands: {activity_command_count}"
            ));
        }
        if activity_file_change_count > 0 {
            parts.push(format!(
                "{activity_scope} file changes: {activity_file_change_count}"
            ));
        }
        if let Some(approval_summary) = conversation.approval_summary().as_deref() {
            parts.push(format!(
                "approval: {}",
                compact_inline_detail(approval_summary, max_detail_len)
            ));
        }
        return Some(OperatorNotice::new(
            OperatorNoticeKind::Detail,
            parts.join("  |  "),
        ));
    }

    /*
     * 아무 실행/auto-follow/review notice가 없을 때 approval 상태만 남긴다.
     * approval은 중요하지만 stale하게 오래 남을 수 있어 더 높은 우선순위의 활동에는 자리를 양보한다.
     */
    conversation.approval_summary().map(|approval_summary| {
        OperatorNotice::new(
            OperatorNoticeKind::Detail,
            format!(
                "approval: {}",
                compact_inline_detail(&approval_summary, max_detail_len)
            ),
        )
    })
}

pub(super) fn compact_auto_follow_status_summary(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> String {
    /*
     * auto-follow prompt/footer copy는 queue-driven 상태와 internal pause 상태를 구분해야 한다.
     * pause flag가 있으면 activity label보다 우선해 "paused/internal"을 보여 주고, 아니면 queue 상태를 붙인다.
     */
    let summary = if conversation.auto_follow_state().continuation_paused {
        "paused/internal".to_string()
    } else {
        format!(
            "queue/{}",
            conversation.auto_follow_state().activity_label()
        )
    };
    compact_inline_detail(&summary, max_detail_len)
}

#[cfg(test)]
mod tests {
    use super::{OperatorNotice, OperatorNoticeKind, build_operator_notice};
    use crate::adapter::inbound::tui::app::conversation_model::{
        ActivityRailTerminalState, ConversationViewModel,
    };
    use crate::core::app::{
        ActiveTurnPhase, ActiveTurnSnapshot, ApprovalAuthorityPhase, ApprovalAuthoritySnapshot,
        CorePromptOrigin, TurnSubmissionCorrelation,
    };
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind, ConversationMessage,
        ConversationMessageKind,
    };
    use std::time::Instant;

    const SECRET: &str = "ultra-secret-payload";

    fn set_active_turn(
        conversation: &mut ConversationViewModel,
        phase: ActiveTurnPhase,
        turn_id: Option<&str>,
    ) {
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.active_turn = Some(ActiveTurnSnapshot {
            correlation: TurnSubmissionCorrelation::new(1),
            phase,
            workspace_directory: conversation.cwd.clone(),
            turn_id: turn_id.map(str::to_string),
            prompt_origin: CorePromptOrigin::Manual,
            started_at: Instant::now(),
        });
        conversation.apply_runtime_snapshot(snapshot);
    }

    fn set_pending_approval(
        conversation: &mut ConversationViewModel,
        request: ConversationApprovalRequest,
    ) {
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.approval = Some(ApprovalAuthoritySnapshot {
            request,
            decision: None,
            phase: ApprovalAuthorityPhase::Pending,
        });
        conversation.apply_runtime_snapshot(snapshot);
    }

    #[test]
    fn pending_approval_wins_over_terminal_activity_and_review() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.activity_rail_terminal_state = Some(ActivityRailTerminalState::Failed);
        set_pending_approval(
            &mut conversation,
            ConversationApprovalRequest {
                approval_id: "approval-1".to_string(),
                server_request_id: "request-1".to_string(),
                method: "item/commandExecution/requestApproval".to_string(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: SECRET.to_string(),
                details: vec![SECRET.to_string()],
            },
        );

        let notice = build_operator_notice(Some("review changed"), &conversation, 160, 160)
            .expect("pending approval notice");

        assert_eq!(
            notice,
            OperatorNotice {
                text: "approval: decision required".to_string(),
                kind: OperatorNoticeKind::RequiredAction,
            }
        );
        assert!(!notice.text.contains("terminal:"));
        assert!(!notice.text.contains(SECRET));
    }

    #[test]
    fn typed_terminal_activity_wins_over_review() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.activity_rail_terminal_state =
            Some(ActivityRailTerminalState::RecoveryPending);

        let notice = build_operator_notice(Some("review changed"), &conversation, 160, 160)
            .expect("terminal activity notice");

        assert_eq!(
            notice,
            OperatorNotice {
                text: "activity: terminal:recovery-pending".to_string(),
                kind: OperatorNoticeKind::TerminalActivity,
            }
        );
    }

    #[test]
    fn typed_terminal_compacts_before_lower_priority_review_when_narrow() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.activity_rail_terminal_state = Some(ActivityRailTerminalState::RuntimeFailed);

        assert_eq!(
            build_operator_notice(Some("review changed"), &conversation, 160, 12),
            Some(OperatorNotice {
                text: "runtime-fail".to_string(),
                kind: OperatorNoticeKind::TerminalActivity,
            })
        );
    }

    #[test]
    fn submitting_turn_never_replays_legacy_activity_or_lower_priority_review() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.record_thread_prepared(
            "thread-1".to_string(),
            "Activity".to_string(),
            "/tmp/root".to_string(),
        );
        set_active_turn(&mut conversation, ActiveTurnPhase::Running, Some("turn-1"));
        conversation.record_turn_started("turn-1".to_string());
        conversation.turn_activity.current_turn_command_count = 1;
        conversation.turn_activity.current_turn_last_summary = Some(format!("{SECRET}\u{1b}[31m"));
        conversation.fail_turn(Some("turn-1"), "failed".to_string());
        let workspace_directory = conversation.cwd.clone();
        conversation.record_submitted_prompt(
            ConversationMessage::new(ConversationMessageKind::User, "next prompt", None, None),
            workspace_directory,
            true,
        );
        set_active_turn(&mut conversation, ActiveTurnPhase::Submitting, None);

        assert_eq!(
            build_operator_notice(Some("review changed"), &conversation, 160, 160),
            None
        );
    }

    #[test]
    fn responsive_rail_budget_does_not_expand_existing_notice_detail_limit() {
        let conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        let notice =
            build_operator_notice(Some(&"review detail ".repeat(20)), &conversation, 40, 72)
                .expect("GitHub notice");

        assert!(notice.text.len() <= 72, "{notice:?}");
        assert!(notice.text.starts_with("gh update: "));
        assert_eq!(notice.kind, OperatorNoticeKind::Detail);
    }

    #[test]
    fn passive_activity_is_typed_only_when_it_wins_notice_selection() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.progressive_activity.apply_projection_update(
            &Default::default(),
            None,
            None,
            1,
            0,
            0,
            0,
        );

        let passive = build_operator_notice(None, &conversation, 160, 160)
            .expect("bounded-history notice should remain visible");
        assert_eq!(passive.text, "activity: history:bounded");
        assert_eq!(passive.kind, OperatorNoticeKind::Activity);

        let review = build_operator_notice(Some("review changed"), &conversation, 160, 160)
            .expect("GitHub review should precede passive activity");
        assert_eq!(review.text, "gh update: review changed");
        assert_eq!(review.kind, OperatorNoticeKind::Detail);
    }
}
