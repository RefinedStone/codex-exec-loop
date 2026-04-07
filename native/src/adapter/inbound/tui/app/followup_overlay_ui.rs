#[derive(Debug, Clone, Copy, Default)]
pub(super) struct FollowupOverlayUiState {
    pub preview_scroll: u16,
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
        FollowupOverlayUiEvent::OverlayShown
        | FollowupOverlayUiEvent::TemplateChanged
        | FollowupOverlayUiEvent::ContentReset => {
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
        let state = FollowupOverlayUiState { preview_scroll: 12 };

        let reduced = reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::TemplateChanged);

        assert_eq!(reduced.preview_scroll, 0);
    }

    #[test]
    fn preview_scrolled_saturates_at_zero() {
        let state = FollowupOverlayUiState { preview_scroll: 3 };

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::PreviewScrolled { delta: -12 },
        );

        assert_eq!(reduced.preview_scroll, 0);
    }

    #[test]
    fn preview_scrolled_moves_forward() {
        let state = FollowupOverlayUiState { preview_scroll: 0 };

        let reduced =
            reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::PreviewScrolled { delta: 6 });

        assert_eq!(reduced.preview_scroll, 6);
    }
}
