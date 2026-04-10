use super::*;

#[derive(Debug, Clone)]
pub(super) enum ConversationInputEvent {
    CharacterTyped { character: char },
    NewlineInserted,
    BackspacePressed,
    PreviousWordDeleted,
    InputCleared,
    StartupSubmitArmed { status_text: String },
    StartupSubmitDisarmed { status_text: Option<String> },
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
            clear_startup_submit_after_input_change(&mut state);
            state.input_buffer.push(character);
        }
        ConversationInputEvent::NewlineInserted => {
            clear_startup_submit_after_input_change(&mut state);
            state.input_buffer.push('\n');
        }
        ConversationInputEvent::BackspacePressed => {
            clear_startup_submit_after_input_change(&mut state);
            state.input_buffer.pop();
        }
        ConversationInputEvent::PreviousWordDeleted => {
            clear_startup_submit_after_input_change(&mut state);
            delete_previous_word(&mut state.input_buffer);
        }
        ConversationInputEvent::InputCleared => {
            clear_startup_submit_after_input_change(&mut state);
            state.input_buffer.clear();
        }
        ConversationInputEvent::StartupSubmitArmed { status_text } => {
            state.arm_startup_submit();
            state.status_text = status_text;
        }
        ConversationInputEvent::StartupSubmitDisarmed { status_text } => {
            if state.clear_startup_submit() {
                if let Some(status_text) = status_text {
                    state.status_text = status_text;
                }
            }
        }
        ConversationInputEvent::StatusMessageShown { status_text } => {
            state.status_text = status_text;
        }
    }

    ConversationInputReduction { state }
}

fn delete_previous_word(buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }

    let trimmed = buffer.trim_end_matches(|character: char| character.is_whitespace());
    let word_start = trimmed
        .rfind(|character: char| character.is_whitespace())
        .map(|index| index + 1)
        .unwrap_or(0);
    buffer.truncate(word_start);
}

fn clear_startup_submit_after_input_change(state: &mut ConversationViewModel) {
    if state.clear_startup_submit() {
        state.status_text = "queued startup send canceled after input changed".to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::followup_template::{
        FollowupTemplateCatalog, FollowupTemplateDefinition, FollowupTemplateSource,
    };

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
    fn previous_word_deleted_removes_last_word() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );
        state.input_buffer = "ship this next".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "ship this ");
    }

    #[test]
    fn previous_word_deleted_trims_trailing_space_before_removing_last_word() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );
        state.input_buffer = "ship this   ".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "ship ");
    }

    #[test]
    fn previous_word_deleted_respects_newline_boundaries() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );
        state.input_buffer = "first line\nsecond".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "first line\n");
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

    #[test]
    fn startup_submit_armed_sets_queue_status() {
        let state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            },
        );

        assert!(reduced.state.startup_submit_armed);
        assert_eq!(
            reduced.state.status_text,
            "prompt queued until startup checks finish"
        );
    }

    #[test]
    fn input_change_cancels_armed_startup_submit() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );
        state.arm_startup_submit();

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::CharacterTyped { character: 'a' },
        );

        assert!(!reduced.state.startup_submit_armed);
        assert_eq!(
            reduced.state.status_text,
            "queued startup send canceled after input changed"
        );
        assert_eq!(reduced.state.input_buffer, "a");
    }

    #[test]
    fn input_cleared_empties_buffer() {
        let mut state = ConversationViewModel::new_draft(
            "/tmp/root".to_string(),
            sample_template_load_result(),
        );
        state.input_buffer = ":diag".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::InputCleared);

        assert!(reduced.state.input_buffer.is_empty());
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
