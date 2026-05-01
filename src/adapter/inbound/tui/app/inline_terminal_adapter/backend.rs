use std::ops::Range;

use ratatui::backend::{Backend, ClearType};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};

pub(crate) trait InlineResizeBackend: Backend {
    fn set_resize_append_lines_suppressed(&mut self, suppressed: bool);
}

pub(crate) struct InlineTerminalBackend<B> {
    inner: B,
    suppress_resize_append_lines: bool,
    tracked_cursor_position: Option<Position>,
}

impl<B> InlineTerminalBackend<B> {
    pub(crate) fn new(inner: B) -> Self {
        Self {
            inner,
            suppress_resize_append_lines: false,
            tracked_cursor_position: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn inner(&self) -> &B {
        &self.inner
    }

    #[cfg(test)]
    pub(crate) fn inner_mut(&mut self) -> &mut B {
        &mut self.inner
    }
}

impl<B: Backend> InlineResizeBackend for InlineTerminalBackend<B> {
    fn set_resize_append_lines_suppressed(&mut self, suppressed: bool) {
        self.suppress_resize_append_lines = suppressed;
    }
}

impl<B: Backend> Backend for InlineTerminalBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let size = self.inner.size()?;
        self.inner
            .draw(content.filter(move |(x, y, _)| *x < size.width && *y < size.height))
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        if self.suppress_resize_append_lines {
            return Ok(());
        }
        self.inner.append_lines(n)?;
        self.track_cursor_after_append_lines(n);
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        if let Some(position) = self.tracked_cursor_position {
            return Ok(position);
        }
        let position = self.inner.get_cursor_position()?;
        self.tracked_cursor_position = Some(position);
        Ok(position)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let position = position.into();
        self.inner.set_cursor_position(position)?;
        self.tracked_cursor_position = Some(position);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }

    fn scroll_region_up(&mut self, region: Range<u16>, line_count: u16) -> Result<(), Self::Error> {
        self.inner.scroll_region_up(region, line_count)
    }

    fn scroll_region_down(
        &mut self,
        region: Range<u16>,
        line_count: u16,
    ) -> Result<(), Self::Error> {
        self.inner.scroll_region_down(region, line_count)
    }
}

impl<B: Backend> InlineTerminalBackend<B> {
    fn track_cursor_after_append_lines(&mut self, line_count: u16) {
        if line_count == 0 {
            return;
        }
        let Some(mut position) = self.tracked_cursor_position else {
            return;
        };
        if let Ok(size) = self.inner.size() {
            if size.height == 0 {
                return;
            }
            position.x = 0;
            position.y = position
                .y
                .saturating_add(line_count)
                .min(size.height.saturating_sub(1));
            self.tracked_cursor_position = Some(position);
        }
    }
}

impl<B: std::fmt::Display> std::fmt::Display for InlineTerminalBackend<B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(formatter)
    }
}
