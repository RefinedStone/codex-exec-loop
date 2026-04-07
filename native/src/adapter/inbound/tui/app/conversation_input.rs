use super::*;

#[derive(Debug, Clone)]
pub(super) enum ConversationInputEvent {
    CharacterTyped { character: char },
    NewlineInserted,
    BackspacePressed,
    StatusMessageShown { status_text: String },
}

#[derive(Debug, Clone)]
pub(super) struct ConversationInputReduction {
    pub state: ConversationViewModel,
}

pub(super) fn reduce_conversation_input(
    mut state: ConversationViewModel,
    event: ConversationInputEvent,
) -> ConversationInputReduction {
    match event {
        ConversationInputEvent::CharacterTyped { character } => {
            state.input_buffer.push(character);
        }
        ConversationInputEvent::NewlineInserted => {
            state.input_buffer.push('\n');
        }
        ConversationInputEvent::BackspacePressed => {
            state.input_buffer.pop();
        }
        ConversationInputEvent::StatusMessageShown { status_text } => {
            state.status_text = status_text;
        }
    }

    ConversationInputReduction { state }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::followup_template::{FollowupTemplateCatalog, FollowupTemplateSource};

    #[test]
    fn character_typed_appends_to_input_buffer() {
        let state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::CharacterTyped { character: 'a' },
        );

        assert_eq!(reduced.state.input_buffer, "a");
    }

    #[test]
    fn backspace_pressed_removes_last_character() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );
        state.input_buffer = "draft".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::BackspacePressed);

        assert_eq!(reduced.state.input_buffer, "draf");
    }

    #[test]
    fn newline_inserted_adds_line_break() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );
        state.input_buffer = "draft".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::NewlineInserted);

        assert_eq!(reduced.state.input_buffer, "draft\n");
    }

    #[test]
    fn status_message_shown_replaces_status_text() {
        let state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::StatusMessageShown {
                status_text: "turn still running".to_string(),
            },
        );

        assert_eq!(reduced.state.status_text, "turn still running");
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
}
