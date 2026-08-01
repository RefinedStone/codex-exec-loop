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
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;

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
}
