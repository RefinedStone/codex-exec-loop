use crossterm::event::{KeyCode, KeyEvent};

use super::TUI_SKIN_ENV_VAR;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShellUiSkin {
    Inline,
    Dashboard,
}

impl ShellUiSkin {
    pub(super) fn from_environment() -> Self {
        Self::from_env_value(std::env::var(TUI_SKIN_ENV_VAR).ok().as_deref())
    }

    pub(super) fn from_env_value(value: Option<&str>) -> Self {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) if value.eq_ignore_ascii_case("dashboard") => Self::Dashboard,
            _ => Self::Inline,
        }
    }

    pub(super) fn is_dashboard(self) -> bool {
        matches!(self, Self::Dashboard)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DashboardPanelFocus {
    Primary,
    Secondary,
    Tertiary,
    Quaternary,
    Feed,
    Status,
}

impl DashboardPanelFocus {
    fn next(self) -> Self {
        match self {
            Self::Primary => Self::Secondary,
            Self::Secondary => Self::Tertiary,
            Self::Tertiary => Self::Quaternary,
            Self::Quaternary => Self::Feed,
            Self::Feed => Self::Status,
            Self::Status => Self::Primary,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Primary => Self::Status,
            Self::Secondary => Self::Primary,
            Self::Tertiary => Self::Secondary,
            Self::Quaternary => Self::Tertiary,
            Self::Feed => Self::Quaternary,
            Self::Status => Self::Feed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DashboardUiState {
    focused_panel: DashboardPanelFocus,
    selected_rows: [usize; 6],
}

impl Default for DashboardUiState {
    fn default() -> Self {
        Self {
            focused_panel: DashboardPanelFocus::Primary,
            selected_rows: [0; 6],
        }
    }
}

impl DashboardUiState {
    pub(super) fn focused_panel(&self) -> DashboardPanelFocus {
        self.focused_panel
    }

    pub(super) fn handle_navigation_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.focused_panel = self.focused_panel.next();
                true
            }
            KeyCode::BackTab => {
                self.focused_panel = self.focused_panel.previous();
                true
            }
            KeyCode::Up | KeyCode::Left if key.modifiers.is_empty() => {
                let selected_row = &mut self.selected_rows[self.focused_panel_index()];
                *selected_row = selected_row.saturating_sub(1);
                true
            }
            KeyCode::Down | KeyCode::Right if key.modifiers.is_empty() => {
                let selected_row = &mut self.selected_rows[self.focused_panel_index()];
                *selected_row = selected_row.saturating_add(1);
                true
            }
            _ => false,
        }
    }

    fn focused_panel_index(&self) -> usize {
        match self.focused_panel {
            DashboardPanelFocus::Primary => 0,
            DashboardPanelFocus::Secondary => 1,
            DashboardPanelFocus::Tertiary => 2,
            DashboardPanelFocus::Quaternary => 3,
            DashboardPanelFocus::Feed => 4,
            DashboardPanelFocus::Status => 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    #[test]
    fn shell_ui_skin_uses_dashboard_only_for_explicit_flag() {
        assert_eq!(ShellUiSkin::from_env_value(None), ShellUiSkin::Inline);
        assert_eq!(ShellUiSkin::from_env_value(Some("")), ShellUiSkin::Inline);
        assert_eq!(
            ShellUiSkin::from_env_value(Some("inline")),
            ShellUiSkin::Inline
        );
        assert_eq!(
            ShellUiSkin::from_env_value(Some("unknown")),
            ShellUiSkin::Inline
        );
        assert_eq!(
            ShellUiSkin::from_env_value(Some(" dashboard ")),
            ShellUiSkin::Dashboard
        );
    }

    #[test]
    fn dashboard_navigation_changes_only_dashboard_state() {
        let mut state = DashboardUiState::default();
        assert_eq!(state.focused_panel(), DashboardPanelFocus::Primary);

        assert!(state.handle_navigation_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty(),)));
        assert_eq!(state.focused_panel(), DashboardPanelFocus::Secondary);

        assert!(
            state.handle_navigation_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::empty(),))
        );
        assert_eq!(state.focused_panel(), DashboardPanelFocus::Primary);
    }
}
