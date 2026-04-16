use anyhow::Result;

use super::ratatui_frontend::run as run_ratatui_frontend;
use super::shell_runtime::ShellRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellFrontendMode {
    InlineMainBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct ShellFrontend;

impl ShellFrontend {
    pub(super) fn new() -> Self {
        Self
    }

    #[cfg(test)]
    pub(super) fn mode(self) -> ShellFrontendMode {
        ShellFrontendMode::InlineMainBuffer
    }

    pub(super) fn run(self, runtime: ShellRuntime) -> Result<()> {
        run_ratatui_frontend(runtime)
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellFrontend, ShellFrontendMode};

    #[test]
    fn shell_frontend_is_inline_only() {
        assert_eq!(
            ShellFrontend::new().mode(),
            ShellFrontendMode::InlineMainBuffer
        );
    }
}
