/*
 * 학습 주석: conversation_lifecycle.rs는 "어떤 대화 세션을 보고 있는가"를 관리하는 작은 reducer입니다.
 * runtime reducer가 한 turn의 submit/stream/post-turn 흐름을 다룬다면, lifecycle reducer는 새 draft, 기존 session 선택,
 * session snapshot load 완료처럼 대화 컨테이너 자체가 바뀌는 사건을 처리합니다.
 */
use super::{ConversationState, ConversationViewModel};
use crate::domain::conversation::{ConversationRuntimeControlTruth, ConversationSnapshot};
use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone)]
/*
 * 학습 주석: LifecycleEvent는 session browser나 shell controller에서 발생한 고수준 navigation event입니다.
 * 이 reducer는 app-server를 직접 읽지 않고 "어떤 conversation container를 보여야 하는가"만 결정합니다.
 */
pub(super) enum ConversationLifecycleEvent {
    NewDraftOpened {
        // 학습 주석: 새 draft는 아직 thread id가 없으므로 workspace만으로 Ready view model을 재구성합니다.
        workspace_directory: String,
    },
    SessionChosen {
        // 학습 주석: 목록의 summary는 shell chrome에 보관하고, 본문은 effect가 snapshot을 가져온 뒤 채웁니다.
        session: SessionSummary,
    },
    ConversationLoaded {
        // 학습 주석: inbound adapter가 app-server/session read 결과를 reducer가 이해하는 성공/실패 값으로 접습니다.
        result: Result<ConversationSnapshot, String>,
        // 학습 주석: snapshot에 cwd가 비어 있거나 draft fallback이 필요할 때 같은 shell 기준 경로를 유지합니다.
        draft_workspace_directory: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: Effect는 reducer 밖에서 실행할 IO 요청입니다. 이 파일은 어떤 thread를 읽을지만 선언합니다.
pub(super) enum ConversationLifecycleEffect {
    LoadConversation { thread_id: String },
}

#[derive(Debug, Clone)]
// 학습 주석: Lifecycle state는 conversation body와 session chrome을 함께 보관해 두 화면이 같은 선택을 보게 합니다.
pub(super) struct ConversationLifecycleState {
    // 학습 주석: body 영역의 Loading/Ready/Failed 상태입니다.
    pub conversation_state: ConversationState,
    // 학습 주석: 현재 shell이 "선택된 기존 세션"으로 강조할 summary입니다. draft에서는 비워 둡니다.
    pub active_session: Option<SessionSummary>,
    // 학습 주석: stop/interrupt 같은 turn 제어 truth는 snapshot 재구성 시에도 동일한 source of truth를 씁니다.
    pub turn_control_truth: ConversationRuntimeControlTruth,
}

#[derive(Debug, Clone)]
// 학습 주석: Reduction은 순수한 상태 갱신 결과와 adapter가 실행할 effect queue를 한 번에 반환합니다.
pub(super) struct ConversationLifecycleReduction {
    pub state: ConversationLifecycleState,
    pub effects: Vec<ConversationLifecycleEffect>,
}

pub(super) fn reduce_conversation_lifecycle(
    mut state: ConversationLifecycleState,
    event: ConversationLifecycleEvent,
) -> ConversationLifecycleReduction {
    /*
     * 학습 주석: lifecycle reducer도 runtime reducer와 같은 패턴을 씁니다.
     * state 변화는 즉시 계산하지만, 실제 app-server thread read는 LoadConversation effect로 밖에 맡깁니다.
     * 이렇게 하면 session 선택 UI는 즉시 Loading을 표시하고, 네트워크/프로세스 결과는 ConversationLoaded event로 나중에 돌아옵니다.
     */
    let mut effects = Vec::new();

    match event {
        ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory,
        } => {
            // 학습 주석: 새 draft는 session 목록의 선택과 독립적이므로 active_session을 먼저 끊어 shell 강조를 없앱니다.
            state.active_session = None;
            state.conversation_state =
                ConversationState::ready(ConversationViewModel::new_draft_with_truth(
                    workspace_directory,
                    state.turn_control_truth,
                ));
        }
        ConversationLifecycleEvent::SessionChosen { session } => {
            // 학습 주석: session summary는 move되어 state에 들어가므로 effect용 thread id를 먼저 복사합니다.
            let thread_id = session.id.clone();
            state.active_session = Some(session);
            state.conversation_state = ConversationState::Loading;
            effects.push(ConversationLifecycleEffect::LoadConversation { thread_id });
        }
        ConversationLifecycleEvent::ConversationLoaded {
            result,
            draft_workspace_directory,
        } => {
            state.conversation_state = match result {
                Ok(snapshot) => {
                    // 학습 주석: loaded snapshot도 runtime truth를 새로 만들지 않고 shell이 가진 truth를 주입받습니다.
                    ConversationState::ready(ConversationViewModel::from_snapshot_with_truth(
                        snapshot,
                        draft_workspace_directory,
                        state.turn_control_truth,
                    ))
                }
                Err(message) => ConversationState::Failed(message),
            };
        }
    }

    ConversationLifecycleReduction { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosing_session_marks_state_loading_and_emits_load_effect() {
        // 학습 주석: 선택 직후에는 snapshot을 기다리므로 Ready 내용 대신 Loading과 effect 계약을 검증합니다.
        let state = sample_state();
        let session = sample_session("thread-2");

        let reduced = reduce_conversation_lifecycle(
            state,
            ConversationLifecycleEvent::SessionChosen { session },
        );

        assert!(matches!(
            reduced.state.conversation_state,
            ConversationState::Loading
        ));
        assert_eq!(
            reduced
                .state
                .active_session
                .as_ref()
                .map(|session| session.id.as_str()),
            Some("thread-2")
        );
        assert_eq!(
            reduced.effects,
            vec![ConversationLifecycleEffect::LoadConversation {
                thread_id: "thread-2".to_string()
            }]
        );
    }

    #[test]
    fn new_draft_replaces_active_session_and_sets_workspace() {
        // 학습 주석: draft 전환은 기존 session 선택을 해제하고 즉시 입력 가능한 Ready 상태로 돌아가야 합니다.
        let mut state = sample_state();
        state.active_session = Some(sample_session("thread-1"));

        let reduced = reduce_conversation_lifecycle(
            state,
            ConversationLifecycleEvent::NewDraftOpened {
                workspace_directory: "/tmp/new-root".to_string(),
            },
        );

        let ConversationState::Ready(conversation) = reduced.state.conversation_state else {
            panic!("draft should become ready");
        };
        assert!(reduced.state.active_session.is_none());
        assert_eq!(conversation.cwd, "/tmp/new-root");
    }

    fn sample_state() -> ConversationLifecycleState {
        // 학습 주석: 기본 fixture는 이미 Ready인 draft에서 lifecycle event만 바꿔 보는 형태로 둡니다.
        ConversationLifecycleState {
            conversation_state: ConversationState::ready(
                ConversationViewModel::new_draft_with_truth(
                    "/tmp/root".to_string(),
                    ConversationRuntimeControlTruth::default(),
                ),
            ),
            active_session: None,
            turn_control_truth: ConversationRuntimeControlTruth::default(),
        }
    }

    fn sample_session(id: &str) -> SessionSummary {
        // 학습 주석: reducer는 summary의 id/cwd만 직접 의미 있게 쓰지만 전체 struct를 채워 실제 목록 입력과 맞춥니다.
        SessionSummary {
            id: id.to_string(),
            name: Some(id.to_string()),
            preview: "preview".to_string(),
            cwd: "/tmp/root".to_string(),
            source: "codex".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: format!("/tmp/root/{id}.json"),
            git_branch: Some("main".to_string()),
        }
    }
}
