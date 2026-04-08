use anyhow::Result;

use super::ALT_SCREEN_ENV_VAR;
use super::ratatui_frontend::run as run_ratatui_frontend;
use super::shell_runtime::ShellRuntime;

const FRONTEND_ENV_VAR: &str = "CODEX_EXEC_LOOP_FRONTEND";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellFrontendMode {
    InlineMainBuffer,
    AlternateScreen,
}

impl ShellFrontendMode {
    pub(super) fn from_environment() -> Self {
        Self::from_env_values(
            std::env::var(FRONTEND_ENV_VAR).ok().as_deref(),
            std::env::var(ALT_SCREEN_ENV_VAR).ok().as_deref(),
        )
    }

    fn from_env_values(frontend_value: Option<&str>, alt_screen_value: Option<&str>) -> Self {
        parse_explicit_frontend_mode(frontend_value).unwrap_or_else(|| {
            if alt_screen_value.is_some_and(env_flag_is_truthy) {
                Self::AlternateScreen
            } else {
                Self::InlineMainBuffer
            }
        })
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

fn parse_explicit_frontend_mode(value: Option<&str>) -> Option<ShellFrontendMode> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "inline" | "main" | "main-buffer" | "inline-main-buffer" => {
            Some(ShellFrontendMode::InlineMainBuffer)
        }
        "alt" | "alternate" | "alternate-screen" | "fullscreen" => {
            Some(ShellFrontendMode::AlternateScreen)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ShellFrontend, ShellFrontendMode};

    #[test]
    fn shell_frontend_mode_defaults_to_inline_main_buffer() {
        assert_eq!(
            ShellFrontendMode::from_env_values(None, None),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(Some(""), None),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(Some("maybe"), Some("0")),
            ShellFrontendMode::InlineMainBuffer
        );
    }

    #[test]
    fn shell_frontend_mode_accepts_truthy_legacy_alt_screen_flag() {
        assert_eq!(
            ShellFrontendMode::from_env_values(None, Some("1")),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(None, Some(" true ")),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(None, Some("ON")),
            ShellFrontendMode::AlternateScreen
        );
    }

    #[test]
    fn shell_frontend_mode_supports_explicit_frontend_values() {
        assert_eq!(
            ShellFrontendMode::from_env_values(Some("inline"), None),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(Some(" main-buffer "), None),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(Some("alternate"), None),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(Some("FULLSCREEN"), None),
            ShellFrontendMode::AlternateScreen
        );
    }

    #[test]
    fn explicit_frontend_value_overrides_legacy_alt_screen_flag() {
        assert_eq!(
            ShellFrontendMode::from_env_values(Some("inline"), Some("1")),
            ShellFrontendMode::InlineMainBuffer
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(Some("alternate"), Some("0")),
            ShellFrontendMode::AlternateScreen
        );
    }

    #[test]
    fn unrecognized_frontend_value_falls_back_to_legacy_alt_screen_flag() {
        assert_eq!(
            ShellFrontendMode::from_env_values(Some("maybe"), Some("1")),
            ShellFrontendMode::AlternateScreen
        );
        assert_eq!(
            ShellFrontendMode::from_env_values(Some("maybe"), Some("0")),
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
