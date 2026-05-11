/*
 * Conversation lifecycle reducer는 "현재 shell이 어떤 conversation container를
 * 보여 주는가"를 관리한다. runtime reducer가 한 turn의 submit/stream/post-turn
 * 흐름을 다룬다면, 이 reducer는 새 draft, 기존 session 선택, snapshot load 완료처럼
 * conversation 자체가 교체되는 사건만 처리한다.
 */
use super::{ConversationState, ConversationViewModel};
use crate::core::app::ConversationSnapshot as CoreConversationSnapshot;
use crate::domain::conversation::ConversationRuntimeControlTruth;
#[cfg(test)]
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone)]
/*
 * LifecycleEvent는 session browser나 shell controller에서 들어오는 navigation
 * intent다. 이 layer는 app-server를 직접 읽지 않고 어떤 body state와 어떤 IO effect가
 * 필요한지만 결정한다.
 */
pub(super) enum ConversationLifecycleEvent {
    NewDraftOpened {
        // 새 draft는 아직 thread id가 없으므로 workspace만으로 Ready view model을 만든다.
        workspace_directory: String,
    },
    SessionChosen {
        // Summary는 shell chrome에 즉시 보관하고, body는 snapshot load effect 뒤에 채운다.
        session: SessionSummary,
    },
    CoreConversationSnapshotApplied {
        // Loading/Ready/Failed lifecycle authority는 core AppSnapshot에서 내려온다.
        snapshot: CoreConversationSnapshot,
        // Snapshot cwd가 비어 있거나 fallback이 필요할 때 shell 기준 workspace를 유지한다.
        draft_workspace_directory: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Effect는 reducer 밖에서 실행할 IO 요청이다. 이 파일은 어떤 thread를 읽을지만 선언한다.
pub(super) enum ConversationLifecycleEffect {
    LoadConversation { thread_id: String },
}

#[derive(Debug, Clone)]
// Lifecycle state는 conversation body와 session chrome이 같은 선택을 보도록 묶는다.
pub(super) struct ConversationLifecycleState {
    // Body 영역의 Loading/Ready/Failed state다.
    pub conversation_state: ConversationState,
    // Shell이 선택된 기존 session으로 강조할 summary다. Draft에서는 비워 둔다.
    pub active_session: Option<SessionSummary>,
    // Stop/interrupt 같은 turn-control truth는 snapshot 재구성 뒤에도 같은 source를 쓴다.
    pub turn_control_truth: ConversationRuntimeControlTruth,
}

#[derive(Debug, Clone)]
// Reduction은 순수 상태 갱신 결과와 adapter가 실행할 effect queue를 함께 반환한다.
pub(super) struct ConversationLifecycleReduction {
    pub state: ConversationLifecycleState,
    pub effects: Vec<ConversationLifecycleEffect>,
}

pub(super) fn reduce_conversation_lifecycle(
    mut state: ConversationLifecycleState,
    event: ConversationLifecycleEvent,
) -> ConversationLifecycleReduction {
    /*
     * Lifecycle도 runtime reducer와 같은 effect split을 쓴다. State 변화는 여기서 즉시
     * 계산하지만, 실제 app-server thread read는 LoadConversation effect로 밖에 맡긴다.
     * 그래서 session 선택 UI는 core가 ConversationChanged(Loading)을 emit한 뒤
     * loading body를 표시하고, process 결과도 core snapshot event로 되돌아온다.
     */
    let mut effects = Vec::new();

    match event {
        ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory,
        } => {
            // Draft 전환은 session 목록 선택과 독립적이므로 shell highlight부터 끊는다.
            state.active_session = None;
            state.conversation_state =
                ConversationState::ready(ConversationViewModel::new_draft_with_truth(
                    workspace_directory,
                    state.turn_control_truth,
                ));
        }
        ConversationLifecycleEvent::SessionChosen { session } => {
            // Summary는 state로 move되므로 effect용 thread id를 먼저 복사한다. Body lifecycle은
            // 이어서 발생하는 core ConversationChanged(Loading) snapshot이 정한다.
            let thread_id = session.id.clone();
            state.active_session = Some(session);
            effects.push(ConversationLifecycleEffect::LoadConversation { thread_id });
        }
        ConversationLifecycleEvent::CoreConversationSnapshotApplied {
            snapshot,
            draft_workspace_directory,
        } => {
            state.conversation_state = match snapshot {
                CoreConversationSnapshot::Idle => state.conversation_state,
                CoreConversationSnapshot::Loading => ConversationState::Loading,
                CoreConversationSnapshot::Ready(ready) => {
                    // Loaded snapshot도 shell이 가진 turn-control truth를 주입받아 runtime 제어를 공유한다.
                    ConversationState::ready(ConversationViewModel::from_snapshot_with_truth(
                        *ready.conversation,
                        draft_workspace_directory,
                        state.turn_control_truth,
                    ))
                }
                CoreConversationSnapshot::Failed { message } => ConversationState::Failed(message),
            };
        }
    }

    ConversationLifecycleReduction { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosing_session_preserves_body_until_core_loading_snapshot_and_emits_load_effect() {
        // 선택 intent는 shell chrome만 바꾸고, Loading body는 core snapshot event가 적용한다.
        let state = sample_state();
        let session = sample_session("thread-2");

        let reduced = reduce_conversation_lifecycle(
            state,
            ConversationLifecycleEvent::SessionChosen { session },
        );

        assert!(matches!(
            reduced.state.conversation_state,
            ConversationState::Ready(_)
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
    fn core_loading_snapshot_marks_state_loading() {
        let reduced = reduce_conversation_lifecycle(
            sample_state(),
            ConversationLifecycleEvent::CoreConversationSnapshotApplied {
                snapshot: CoreConversationSnapshot::Loading,
                draft_workspace_directory: "/tmp/root".to_string(),
            },
        );

        assert!(matches!(
            reduced.state.conversation_state,
            ConversationState::Loading
        ));
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn core_failed_snapshot_marks_state_failed() {
        let reduced = reduce_conversation_lifecycle(
            sample_state(),
            ConversationLifecycleEvent::CoreConversationSnapshotApplied {
                snapshot: CoreConversationSnapshot::Failed {
                    message: "thread unavailable".to_string(),
                },
                draft_workspace_directory: "/tmp/root".to_string(),
            },
        );

        assert!(matches!(
            reduced.state.conversation_state,
            ConversationState::Failed(message) if message == "thread unavailable"
        ));
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn core_ready_snapshot_builds_ready_view_model() {
        let reduced = reduce_conversation_lifecycle(
            sample_state(),
            ConversationLifecycleEvent::CoreConversationSnapshotApplied {
                snapshot: CoreConversationSnapshot::Ready(Box::new(
                    crate::core::app::ConversationReadySnapshot::from(
                        sample_conversation_snapshot("thread-3"),
                    ),
                )),
                draft_workspace_directory: "/tmp/root".to_string(),
            },
        );

        let ConversationState::Ready(conversation) = reduced.state.conversation_state else {
            panic!("core ready snapshot should create ready conversation");
        };
        assert_eq!(conversation.thread_id, "thread-3");
        assert_eq!(conversation.status_text, "thread loaded");
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn new_draft_replaces_active_session_and_sets_workspace() {
        // Draft 전환은 기존 session 선택을 해제하고 즉시 입력 가능한 Ready 상태로 돌아간다.
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
        // 기본 fixture는 이미 Ready인 draft에서 lifecycle event만 바꿔 보는 형태다.
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

    fn sample_conversation_snapshot(thread_id: &str) -> ConversationSnapshot {
        ConversationSnapshot {
            thread_id: thread_id.to_string(),
            title: thread_id.to_string(),
            cwd: "/tmp/root".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
        }
    }

    fn sample_session(id: &str) -> SessionSummary {
        // Reducer는 summary의 id/cwd만 직접 의미 있게 쓰지만 실제 목록 입력 shape를 유지한다.
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
