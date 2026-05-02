use super::{AutoFollowState, ConversationViewModel};

/*
 * Followup controls는 overlay의 임시 입력 상태가 아니라 ConversationViewModel 안의
 * 실제 follow-up 정책을 바꾸는 reducer다. app_runtime은 controller에서 올라온 이벤트를
 * 이 함수에 넣고, 반환된 effect를 다시 followup_overlay_ui reducer로 보내 화면 buffer를
 * 동기화한다. 이 분리 덕분에 "값을 편집하는 UI"와 "auto-follow 실행 정책"이 서로 섞이지 않는다.
 */
#[derive(Debug, Clone)]
pub(super) enum FollowupControlEvent {
    /*
     * workspace 변경은 conversation draft의 cwd와 follow-up 상태가 같은 기준 디렉터리를 보도록 맞춘다.
     * conversation/controller가 새 workspace를 받으면 이 이벤트로 follow-up 쪽 상태까지 따라오게 한다.
     */
    DraftWorkspaceSynced { workspace_directory: String },
    /*
     * AutoFollowPaused는 실행 중인 내부 continuation을 중단하라는 operator intent다.
     * 완료 turn 수는 유지해야 하므로 AutoFollowState의 phase를 통째로 reset하지 않는다.
     */
    AutoFollowPaused,
    /*
     * MaxAutoTurnsUpdated는 `:turns` editor가 확정한 raw 문자열을 실제 정책 값으로 반영한다.
     * 입력 검증은 AutoFollowState의 canonical parser를 사용해 UI와 runtime copy가 같은 규칙을 쓴다.
     */
    MaxAutoTurnsUpdated { value: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FollowupControlEffect {
    /*
     * OverlayUi는 conversation state 변경을 overlay 입력 상태에도 다시 반영하라는 신호다.
     * 예를 들어 draft workspace가 바뀌면 overlay의 표시 값도 새 conversation context로 reset된다.
     */
    OverlayUi,
    /*
     * MaxAutoTurnsEditor는 정책 변경이 성공했을 때 editor buffer를 canonical label로 닫고 맞춘다.
     * `infinite`, 숫자 trim/normalization 결과를 화면에 다시 돌려주는 단일 경로다.
     */
    MaxAutoTurnsEditor { value: String },
}

#[derive(Debug, Clone)]
pub(super) struct FollowupControlReduction {
    /*
     * state는 reducer가 갱신한 conversation model이다. caller인 NativeTuiApp은 이 값을 다시
     * ConversationState::Ready에 넣어 runtime, footer, prompt composer가 같은 값을 보게 한다.
     */
    pub state: ConversationViewModel,
    /*
     * effects는 conversation model 밖의 UI state를 맞추기 위한 후속 작업이다.
     * reducer가 직접 overlay state를 만지지 않아 app_runtime이 두 reducer 사이의 연결 지점으로 남는다.
     */
    pub effects: Vec<FollowupControlEffect>,
}

pub(super) fn reduce_followup_controls(
    mut state: ConversationViewModel,
    event: FollowupControlEvent,
) -> FollowupControlReduction {
    /*
     * 이 reducer는 순수하게 새 conversation state와 후속 effect 목록을 만든다.
     * 외부 I/O나 terminal drawing은 하지 않으므로 tests가 state transition만 검증할 수 있다.
     */
    let mut effects = Vec::new();

    match event {
        FollowupControlEvent::DraftWorkspaceSynced {
            workspace_directory,
        } => {
            /*
             * sync_draft_workspace는 cwd 변경뿐 아니라 draft workspace 기준 status와 skip state를 함께 정리한다.
             * 실제 변화가 있었을 때만 overlay reset effect를 내보내 불필요한 UI buffer 갱신을 피한다.
             */
            if state.sync_draft_workspace(workspace_directory) {
                effects.push(FollowupControlEffect::OverlayUi);
            }
        }
        FollowupControlEvent::AutoFollowPaused => {
            /*
             * pause_post_turn_continuation은 다음 자동 turn 제출을 막는 operator flag를 세운다.
             * record_internal_continuation_paused는 tail/footer가 "사용자가 멈췄다"는 이유를 표시하게 하며,
             * running phase 자체는 유지해 turn budget accounting이 중간에 사라지지 않게 한다.
             */
            state.pause_post_turn_continuation();
            state.record_internal_continuation_paused();
            state.status_text = "internal continuation paused".to_string();
        }
        FollowupControlEvent::MaxAutoTurnsUpdated { value } => {
            /*
             * raw editor buffer는 숫자, 공백, infinite 같은 표현이 섞일 수 있다.
             * AutoFollowState가 canonical parser를 소유하게 해서 runtime limit 판단과 UI 저장 검증이 분리되지 않게 한다.
             */
            let Some(value) = AutoFollowState::normalize_max_auto_turns_candidate(&value) else {
                state.status_text = "auto follow-up max turns must be a whole number greater than 0 or the word infinite".to_string();
                return FollowupControlReduction { state, effects };
            };

            /*
             * limit 변경은 사용자의 새 의사 표현이므로 이전 auto-follow skip reason을 지운다.
             * 그렇지 않으면 footer가 새 설정 뒤에도 오래된 "skipped" 상태를 계속 보여 줄 수 있다.
             */
            state.auto_follow_state.set_max_auto_turns(value);
            state.clear_auto_followup_skip();
            state.status_text = format!(
                "auto follow-up max turns {}",
                state.auto_follow_state.max_auto_turns_label()
            );
            effects.push(FollowupControlEffect::MaxAutoTurnsEditor {
                value: state.auto_follow_state.max_auto_turns_label(),
            });
        }
    }

    FollowupControlReduction { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::{
        AutoFollowupSkipReason, DEFAULT_AUTO_FOLLOW_MAX_TURNS,
    };

    #[test]
    fn draft_workspace_sync_updates_blank_draft_and_emits_ui_sync() {
        /*
         * 새 draft가 workspace를 바꾸면 conversation cwd와 overlay buffer가 함께 따라가야 한다.
         * OverlayUi effect가 없으면 app_runtime이 followup_overlay_ui의 content reset을 호출하지 않는다.
         */
        let draft = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_followup_controls(
            draft,
            FollowupControlEvent::DraftWorkspaceSynced {
                workspace_directory: "/tmp/alt".to_string(),
            },
        );

        assert_eq!(reduced.state.cwd, "/tmp/alt");
        assert!(reduced.state.status_text.contains("draft workspace synced"));
        assert_eq!(reduced.effects, vec![FollowupControlEffect::OverlayUi]);
    }

    #[test]
    fn draft_workspace_sync_clears_skip_state() {
        /*
         * workspace 기준이 바뀌면 이전 workspace에서 계산된 skip reason은 더 이상 신뢰할 수 없다.
         * reducer가 sync_draft_workspace를 통해 stale auto-follow activity를 제거하는지 확인한다.
         */
        let mut draft = ConversationViewModel::new_draft("/tmp/root".to_string());
        draft.record_auto_followup_skip(AutoFollowupSkipReason::NoAgentReply);

        let reduced = reduce_followup_controls(
            draft,
            FollowupControlEvent::DraftWorkspaceSynced {
                workspace_directory: "/tmp/alt".to_string(),
            },
        );

        assert!(reduced.state.last_auto_followup_activity.is_none());
    }

    #[test]
    fn updating_max_auto_turns_clears_skip_and_emits_editor_sync() {
        /*
         * turn budget 변경은 auto-follow를 다시 시도하려는 operator action이다.
         * 정책 값, stale skip reason 제거, editor canonical value sync가 한 reducer 결과에 같이 담긴다.
         */
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.record_auto_followup_skip(AutoFollowupSkipReason::NoAgentReply);

        let reduced = reduce_followup_controls(
            state,
            FollowupControlEvent::MaxAutoTurnsUpdated {
                value: "5".to_string(),
            },
        );

        assert_eq!(reduced.state.auto_follow_state.max_auto_turns_value(), 5);
        assert!(reduced.state.last_auto_followup_activity.is_none());
        assert_eq!(
            reduced.effects,
            vec![FollowupControlEffect::MaxAutoTurnsEditor {
                value: "5".to_string()
            }]
        );
    }

    #[test]
    fn invalid_max_auto_turns_keeps_existing_limit() {
        /*
         * invalid raw input must not partially update conversation policy or close/sync the editor.
         * status_text is enough feedback; empty effects keeps the user in the editing context.
         */
        let state = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_followup_controls(
            state,
            FollowupControlEvent::MaxAutoTurnsUpdated {
                value: "0".to_string(),
            },
        );

        assert_eq!(
            reduced.state.auto_follow_state.max_auto_turns_value(),
            DEFAULT_AUTO_FOLLOW_MAX_TURNS
        );
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn pausing_internal_continuation_keeps_running_phase_for_turn_budget() {
        /*
         * auto-follow pause는 현재 internal continuation을 멈추는 조작이지 이미 제출된 turn을
         * 완료 처리하는 조작이 아니다. running phase가 유지되어야 completed_auto_turns가 부풀지 않는다.
         */
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.auto_follow_state.mark_auto_turn_submitted();

        let reduced = reduce_followup_controls(state, FollowupControlEvent::AutoFollowPaused);

        assert!(reduced.state.auto_follow_state.has_live_activity());
        assert!(
            reduced
                .state
                .auto_follow_state
                .post_turn_continuation_paused()
        );
        assert_eq!(reduced.state.auto_follow_state.completed_auto_turns, 0);
    }
}
