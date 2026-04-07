use ratatui::widgets::ListState;

#[derive(Debug, Default)]
pub(super) struct FollowupOverlayUiState {
    pub preview_scroll: u16,
    pub list_state: ListState,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum FollowupOverlayUiEvent {
    OverlayShown,
    TemplateChanged,
    ContentReset,
    PreviewScrolled { delta: i32 },
}

pub(super) fn reduce_followup_overlay_ui(
    mut state: FollowupOverlayUiState,
    event: FollowupOverlayUiEvent,
) -> FollowupOverlayUiState {
    match event {
        FollowupOverlayUiEvent::OverlayShown | FollowupOverlayUiEvent::ContentReset => {
            state.preview_scroll = 0;
            state.list_state = ListState::default();
        }
        FollowupOverlayUiEvent::TemplateChanged => {
            state.preview_scroll = 0;
        }
        FollowupOverlayUiEvent::PreviewScrolled { delta } => {
            let amount = delta.unsigned_abs().min(u16::MAX as u32) as u16;
            if delta.is_negative() {
                state.preview_scroll = state.preview_scroll.saturating_sub(amount);
            } else {
                state.preview_scroll = state.preview_scroll.saturating_add(amount);
            }
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_changed_resets_preview_scroll() {
        let state = FollowupOverlayUiState {
            preview_scroll: 12,
            ..Default::default()
        };

        let reduced = reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::TemplateChanged);

        assert_eq!(reduced.preview_scroll, 0);
    }

    #[test]
    fn preview_scrolled_saturates_at_zero() {
        let state = FollowupOverlayUiState {
            preview_scroll: 3,
            ..Default::default()
        };

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::PreviewScrolled { delta: -12 },
        );

        assert_eq!(reduced.preview_scroll, 0);
    }

    #[test]
    fn preview_scrolled_moves_forward() {
        let state = FollowupOverlayUiState::default();

        let reduced =
            reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::PreviewScrolled { delta: 6 });

        assert_eq!(reduced.preview_scroll, 6);
    }

    #[test]
    fn overlay_shown_resets_list_state() {
        let mut state = FollowupOverlayUiState::default();
        state.list_state.select(Some(3));

        let reduced = reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::OverlayShown);

        assert_eq!(reduced.list_state.selected(), None);
    }
}
