use super::*;

#[derive(Debug, Clone)]
pub(super) enum ConversationLifecycleEvent {
    NewDraftOpened {
        workspace_directory: String,
        template_load_result: FollowupTemplateCatalogLoadResult,
    },
    SessionChosen {
        session: SessionSummary,
    },
    ConversationLoaded {
        result: Result<ConversationSnapshot, String>,
        template_load_result: Option<FollowupTemplateCatalogLoadResult>,
        draft_workspace_directory: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConversationLifecycleEffect {
    LoadConversation { thread_id: String },
}

#[derive(Debug, Clone)]
pub(super) struct ConversationLifecycleState {
    pub conversation_state: ConversationState,
    pub active_session: Option<SessionSummary>,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationLifecycleReduction {
    pub state: ConversationLifecycleState,
    pub effects: Vec<ConversationLifecycleEffect>,
}

pub(super) fn reduce_conversation_lifecycle(
    mut state: ConversationLifecycleState,
    event: ConversationLifecycleEvent,
) -> ConversationLifecycleReduction {
    let mut effects = Vec::new();

    match event {
        ConversationLifecycleEvent::NewDraftOpened {
            workspace_directory,
            template_load_result,
        } => {
            state.active_session = None;
            state.conversation_state = ConversationState::ready(ConversationViewModel::new_draft(
                workspace_directory,
                template_load_result,
            ));
        }
        ConversationLifecycleEvent::SessionChosen { session } => {
            let thread_id = session.id.clone();
            state.active_session = Some(session);
            state.conversation_state = ConversationState::Loading;
            effects.push(ConversationLifecycleEffect::LoadConversation { thread_id });
        }
        ConversationLifecycleEvent::ConversationLoaded {
            result,
            template_load_result,
            draft_workspace_directory,
        } => {
            state.conversation_state = match result {
                Ok(snapshot) => match template_load_result {
                    Some(template_load_result) => {
                        ConversationState::ready(ConversationViewModel::from_snapshot(
                            snapshot,
                            template_load_result,
                            draft_workspace_directory,
                        ))
                    }
                    None => ConversationState::Failed(
                        "loaded snapshot missing follow-up template data".to_string(),
                    ),
                },
                Err(message) => ConversationState::Failed(message),
            };
        }
    }

    ConversationLifecycleReduction { state, effects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::followup_template::{
        FollowupTemplateCatalog, FollowupTemplateDefinition, FollowupTemplateSource,
    };

    #[test]
    fn choosing_session_marks_state_loading_and_emits_load_effect() {
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
        let mut state = sample_state();
        state.active_session = Some(sample_session("thread-1"));

        let reduced = reduce_conversation_lifecycle(
            state,
            ConversationLifecycleEvent::NewDraftOpened {
                workspace_directory: "/tmp/new-root".to_string(),
                template_load_result: sample_template_load_result(),
            },
        );

        let ConversationState::Ready(conversation) = reduced.state.conversation_state else {
            panic!("draft should become ready");
        };
        assert!(reduced.state.active_session.is_none());
        assert_eq!(conversation.cwd, "/tmp/new-root");
    }

    fn sample_state() -> ConversationLifecycleState {
        ConversationLifecycleState {
            conversation_state: ConversationState::ready(ConversationViewModel::new_draft(
                "/tmp/root".to_string(),
                sample_template_load_result(),
            )),
            active_session: None,
        }
    }

    fn sample_template_load_result() -> FollowupTemplateCatalogLoadResult {
        FollowupTemplateCatalogLoadResult {
            catalog: FollowupTemplateCatalog {
                items: vec![FollowupTemplateDefinition {
                    id: "builtin-next-task".to_string(),
                    label: "builtin next-task".to_string(),
                    body: "follow up".to_string(),
                    source: FollowupTemplateSource::Builtin,
                }],
            },
            warnings: Vec::new(),
        }
    }

    fn sample_session(id: &str) -> SessionSummary {
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
