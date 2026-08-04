use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::Rect;

use super::ShellFrontendMode;
use super::shell_rendering::draw_projected;
use super::shell_runtime::ShellRuntime;

/// Owns one alternate-screen terminal and commits app UI state only after a
/// complete frame draw. Unlike the retired inline adapter, this boundary never
/// writes transcript rows outside Ratatui's frame or maintains a host cursor.
pub(super) struct FullscreenTerminalAdapter<B: Backend> {
    terminal: Terminal<B>,
}

impl<B: Backend> FullscreenTerminalAdapter<B> {
    pub(super) fn new(terminal: Terminal<B>) -> Self {
        Self { terminal }
    }

    pub(super) fn draw_fullscreen_transaction(
        &mut self,
        runtime: &mut ShellRuntime,
    ) -> Result<bool, B::Error> {
        self.terminal.autoresize()?;
        let size_before = self.terminal.size()?;
        let resize_epoch = runtime.terminal_resize_epoch();
        let sample = runtime.capture_fullscreen_projection_sample();
        let projection =
            runtime.capture_fullscreen_conversation_frame_projection(size_before.width, &sample);
        let model = runtime.capture_fullscreen_shell_frame_model(
            ShellFrontendMode::Fullscreen,
            Rect::new(0, 0, size_before.width, size_before.height),
            projection,
        );
        let mut model = Some(model);
        let mut receipt = None;

        self.terminal.draw(|frame| {
            receipt = Some(draw_projected(
                frame,
                ShellFrontendMode::Fullscreen,
                model
                    .take()
                    .expect("fullscreen frame model must be consumed exactly once"),
            ));
        })?;

        let stable_geometry =
            self.terminal.size()? == size_before && runtime.terminal_resize_epoch() == resize_epoch;
        if !stable_geometry {
            runtime.clear_queue_receipt_undo_hit_area();
            runtime.clear_transcript_card_hit_areas();
            runtime.request_resize_redraw_retry();
            return Ok(false);
        }

        let Some(receipt) = receipt else {
            runtime.request_delivery_redraw();
            return Ok(false);
        };
        if !runtime.commit_fullscreen_frame_render_receipt(receipt) {
            runtime.clear_queue_receipt_undo_hit_area();
            runtime.clear_transcript_card_hit_areas();
            runtime.request_delivery_redraw();
            return Ok(false);
        }

        runtime.record_successful_frame_delivery();
        Ok(true)
    }

    #[cfg(test)]
    pub(super) fn terminal(&self) -> &Terminal<B> {
        &self.terminal
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::time::Instant;

    use ratatui::backend::TestBackend;

    use super::*;
    use crate::adapter::inbound::tui::app::{ConversationState, test_helpers::test_native_tui_app};

    #[test]
    fn stable_fullscreen_draw_commits_one_app_viewport_transaction() {
        let terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        let mut adapter = FullscreenTerminalAdapter::new(terminal);
        let mut runtime = ShellRuntime::new(test_native_tui_app());

        assert!(
            adapter
                .draw_fullscreen_transaction(&mut runtime)
                .expect("fullscreen draw")
        );
        let buffer = adapter.terminal().backend().buffer();
        assert!(
            buffer
                .content
                .iter()
                .any(|cell| cell.symbol().contains("Task") || cell.symbol().contains("A"))
        );
    }

    #[test]
    fn transcript_projection_cache_reuses_scroll_frames_and_invalidates_inputs() {
        let mut runtime = ShellRuntime::new(test_native_tui_app());
        let ConversationState::Ready(conversation) =
            &mut runtime.app_mut().conversation.lifecycle.conversation_state
        else {
            panic!("cache fixture should contain a ready conversation");
        };
        for index in 0..64 {
            assert!(conversation.finalize_agent_message(
                format!("cache-agent-{index}"),
                Some("commentary".to_string()),
                format!("cache row {index} with enough content to exercise wrapping"),
            ));
        }

        let sample = runtime.capture_fullscreen_projection_sample();
        let first = runtime.capture_fullscreen_conversation_frame_projection(120, &sample);
        let second = runtime.capture_fullscreen_conversation_frame_projection(120, &sample);
        assert!(Rc::ptr_eq(
            &first.transcript_document,
            &second.transcript_document
        ));
        assert_eq!(runtime.transcript_projection_rebuild_count(), 1);

        let resized = runtime.capture_fullscreen_conversation_frame_projection(100, &sample);
        assert!(!Rc::ptr_eq(
            &second.transcript_document,
            &resized.transcript_document
        ));
        assert_eq!(runtime.transcript_projection_rebuild_count(), 2);

        let ConversationState::Ready(conversation) =
            &mut runtime.app_mut().conversation.lifecycle.conversation_state
        else {
            panic!("cache fixture should remain ready");
        };
        assert!(conversation.finalize_agent_message(
            "cache-agent-final".to_string(),
            Some("commentary".to_string()),
            "new canonical transcript revision".to_string(),
        ));
        let changed = runtime.capture_fullscreen_conversation_frame_projection(100, &sample);
        assert!(!Rc::ptr_eq(
            &resized.transcript_document,
            &changed.transcript_document
        ));
        assert_eq!(runtime.transcript_projection_rebuild_count(), 3);
    }

    #[test]
    #[ignore = "manual long-session CPU and memory profile"]
    fn long_session_scroll_profile() {
        let message_count = std::env::var("AKRA_PROFILE_LONG_SESSION_MESSAGES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(2_048);
        let frame_count = std::env::var("AKRA_PROFILE_LONG_SESSION_FRAMES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(60);
        let terminal = Terminal::new(TestBackend::new(120, 40)).expect("test terminal");
        let mut adapter = FullscreenTerminalAdapter::new(terminal);
        let mut runtime = ShellRuntime::new(test_native_tui_app());
        let ConversationState::Ready(conversation) =
            &mut runtime.app_mut().conversation.lifecycle.conversation_state
        else {
            panic!("profile fixture should contain a ready conversation");
        };
        for index in 0..message_count {
            assert!(conversation.finalize_agent_message(
                format!("profile-agent-{index}"),
                Some("commentary".to_string()),
                format!(
                    "long session response {index}: the renderer must preserve markdown **emphasis** and Korean text 긴 세션.\n\
                     second line contains enough words to wrap at the profile width and expose repeated layout work.\n\
                     third line records tool-like paths src/adapter/inbound/tui/app/shell_rendering.rs:{index}.\n\
                     fourth line closes the deterministic performance fixture."
                ),
            ));
        }

        let warmup_started = Instant::now();
        assert!(
            adapter
                .draw_fullscreen_transaction(&mut runtime)
                .expect("warmup frame")
        );
        let warmup_elapsed = warmup_started.elapsed();

        let profile_started = Instant::now();
        for _ in 0..frame_count {
            runtime
                .app_mut()
                .shell
                .transcript_viewport_ui_state
                .scroll_up(3);
            assert!(
                adapter
                    .draw_fullscreen_transaction(&mut runtime)
                    .expect("profile scroll frame")
            );
        }
        let profile_elapsed = profile_started.elapsed();
        let projection_rebuilds = runtime.transcript_projection_rebuild_count();
        assert_eq!(
            projection_rebuilds, 1,
            "scroll-only frames must reuse one width-bound transcript projection"
        );
        println!(
            "AKRA_LONG_SESSION_PROFILE messages={message_count} frames={frame_count} projection_rebuilds={projection_rebuilds} warmup_ms={:.3} scroll_total_ms={:.3} scroll_mean_ms={:.3}",
            warmup_elapsed.as_secs_f64() * 1_000.0,
            profile_elapsed.as_secs_f64() * 1_000.0,
            profile_elapsed.as_secs_f64() * 1_000.0 / frame_count.max(1) as f64,
        );
    }
}
