use anyhow::Result;

use super::ALT_SCREEN_ENV_VAR;
use super::ratatui_frontend::run as run_ratatui_frontend;
use super::shell_runtime::ShellRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellFrontendMode {
    InlineMainBuffer,
    AlternateScreen,
}

impl ShellFrontendMode {
    pub(super) fn from_environment() -> Self {
        Self::from_env_value(std::env::var(ALT_SCREEN_ENV_VAR).ok().as_deref())
    }

    fn from_env_value(value: Option<&str>) -> Self {
        if value.is_some_and(env_flag_is_truthy) {
            Self::AlternateScreen
        } else {
            Self::InlineMainBuffer
        }
    }

    pub(super) fn uses_alternate_screen(self) -> bool {
        matches!(self, Self::AlternateScreen)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ShellFrontend {
    mode: ShellFrontendMode,
}

impl ShellFrontend {
    pub(super) fn from_environment() -> Self {
        Self::new(ShellFrontendMode::from_environment())
    }

    pub(super) fn new(mode: ShellFrontendMode) -> Self {
        Self { mode }
    }

    pub(super) fn mode(self) -> ShellFrontendMode {
        self.mode
    }

    pub(super) fn run(self, runtime: ShellRuntime) -> Result<()> {
        run_ratatui_frontend(runtime, self)
    }
}

fn env_flag_is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::{ShellFrontend, ShellFrontendMode};

    #[test]
    fn shell_frontend_mode_defaults_to_inline_main_buffer() {
        assert_eq!(
            ShellFrontendMode::from_env_value(None),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("0")),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("no")),
            ShellFrontendMode::InlineMainBuffer
        );
    }

    #[test]
    fn shell_frontend_mode_accepts_truthy_alt_screen_flag() {
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("1")),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontendMode::from_env_value(Some(" true ")),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("ON")),
            ShellFrontendMode::AlternateScreen
        );
    }

    #[test]
    fn shell_frontend_mode_ignores_unrecognized_flag_values() {
        assert_eq!(
            ShellFrontendMode::from_env_value(Some("maybe")),
            ShellFrontendMode::InlineMainBuffer
        );
    }

    #[test]
    fn shell_frontend_wraps_the_explicit_frontend_mode() {
        assert_eq!(
            ShellFrontend::new(ShellFrontendMode::AlternateScreen).mode(),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontend::new(ShellFrontendMode::InlineMainBuffer).mode(),
            ShellFrontendMode::InlineMainBuffer
        );
    }

    #[test]
    fn shell_frontend_mode_reports_alternate_screen_usage() {
        assert!(ShellFrontendMode::AlternateScreen.uses_alternate_screen());
        assert!(!ShellFrontendMode::InlineMainBuffer.uses_alternate_screen());
    }
}
