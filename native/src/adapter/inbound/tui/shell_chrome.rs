use crate::domain::recent_sessions::RecentSessions;
use crate::domain::startup_diagnostics::StartupDiagnostics;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellOverlay {
    Hidden,
    Startup,
    Sessions,
    FollowupTemplates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitConfirmationState {
    Hidden,
    Visible,
}

#[derive(Debug, Clone)]
pub enum StartupState {
    Idle,
    Loading,
    Ready(StartupDiagnostics),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum SessionState {
    Idle,
    Loading,
    Ready(RecentSessions),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct ShellChromeState {
    pub shell_overlay: ShellOverlay,
    pub exit_confirmation_state: ExitConfirmationState,
    pub startup_state: StartupState,
    pub session_state: SessionState,
    pub selected_session_index: usize,
}

impl ShellChromeState {
    pub fn new() -> Self {
        Self {
            shell_overlay: ShellOverlay::Hidden,
            exit_confirmation_state: ExitConfirmationState::Hidden,
            startup_state: StartupState::Idle,
            session_state: SessionState::Idle,
            selected_session_index: 0,
        }
    }

    pub fn can_open_session_list(&self) -> bool {
        matches!(
            &self.startup_state,
            StartupState::Ready(diagnostics) if diagnostics.can_continue()
        )
    }
}

#[derive(Debug, Clone)]
pub enum ShellChromeEvent {
    StartupCheckRequested,
    StartupLoaded {
        result: Result<StartupDiagnostics, String>,
        session_page_size: usize,
    },
    SessionsRequested {
        limit: usize,
    },
    SessionsLoaded(Result<RecentSessions, String>),
    StartupOverlayShown,
    SessionsOverlayShown {
        limit: usize,
    },
    FollowupTemplatesOverlayShown,
    StartupOverlayToggled,
    SessionsOverlayToggled {
        limit: usize,
    },
    FollowupTemplatesOverlayToggled,
    OverlayClosed,
    ExitConfirmationShown,
    ExitConfirmationHidden,
    SessionSelectionMoved {
        delta: isize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellChromeEffect {
    RunStartupChecks,
    LoadRecentSessions { limit: usize },
}

#[derive(Debug, Clone)]
pub struct ShellChromeReduction {
    pub state: ShellChromeState,
    pub effects: Vec<ShellChromeEffect>,
}

pub fn reduce_shell_chrome(
    mut state: ShellChromeState,
    event: ShellChromeEvent,
) -> ShellChromeReduction {
    let mut effects = Vec::new();

    match event {
        ShellChromeEvent::StartupCheckRequested => {
            state.startup_state = StartupState::Loading;
            effects.push(ShellChromeEffect::RunStartupChecks);
        }
        ShellChromeEvent::StartupLoaded {
            result,
            session_page_size,
        } => match result {
            Ok(diagnostics) => {
                let can_continue = diagnostics.can_continue();
                state.startup_state = StartupState::Ready(diagnostics);
                if can_continue && matches!(state.session_state, SessionState::Idle) {
                    state.session_state = SessionState::Loading;
                    effects.push(ShellChromeEffect::LoadRecentSessions {
                        limit: session_page_size,
                    });
                }
            }
            Err(message) => {
                state.startup_state = StartupState::Failed(message);
            }
        },
        ShellChromeEvent::SessionsRequested { limit } => {
            queue_session_load_if_allowed(&mut state, limit, &mut effects);
        }
        ShellChromeEvent::SessionsLoaded(result) => {
            state.session_state = match result {
                Ok(recent_sessions) => {
                    state.selected_session_index = 0;
                    SessionState::Ready(recent_sessions)
                }
                Err(message) => SessionState::Failed(message),
            };
        }
        ShellChromeEvent::StartupOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Startup;
        }
        ShellChromeEvent::SessionsOverlayShown { limit } => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Sessions;
            queue_session_load_if_allowed(&mut state, limit, &mut effects);
        }
        ShellChromeEvent::FollowupTemplatesOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::FollowupTemplates;
        }
        ShellChromeEvent::StartupOverlayToggled => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = if state.shell_overlay == ShellOverlay::Startup {
                ShellOverlay::Hidden
            } else {
                ShellOverlay::Startup
            };
        }
        ShellChromeEvent::SessionsOverlayToggled { limit } => {
            if state.shell_overlay == ShellOverlay::Sessions {
                state.shell_overlay = ShellOverlay::Hidden;
            } else {
                state.exit_confirmation_state = ExitConfirmationState::Hidden;
                state.shell_overlay = ShellOverlay::Sessions;
                queue_session_load_if_allowed(&mut state, limit, &mut effects);
            }
        }
        ShellChromeEvent::FollowupTemplatesOverlayToggled => {
            if state.shell_overlay == ShellOverlay::FollowupTemplates {
                state.shell_overlay = ShellOverlay::Hidden;
            } else {
                state.exit_confirmation_state = ExitConfirmationState::Hidden;
                state.shell_overlay = ShellOverlay::FollowupTemplates;
            }
        }
        ShellChromeEvent::OverlayClosed => {
            state.shell_overlay = ShellOverlay::Hidden;
        }
        ShellChromeEvent::ExitConfirmationShown => {
            state.exit_confirmation_state = ExitConfirmationState::Visible;
        }
        ShellChromeEvent::ExitConfirmationHidden => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
        }
        ShellChromeEvent::SessionSelectionMoved { delta } => {
            let SessionState::Ready(recent_sessions) = &state.session_state else {
                return ShellChromeReduction { state, effects };
            };

            if recent_sessions.items.is_empty() {
                state.selected_session_index = 0;
            } else {
                let max_index = recent_sessions.items.len().saturating_sub(1) as isize;
                let current_index = state.selected_session_index as isize;
                let next_index = (current_index + delta).clamp(0, max_index);
                state.selected_session_index = next_index as usize;
            }
        }
    }

    ShellChromeReduction { state, effects }
}

fn queue_session_load_if_allowed(
    state: &mut ShellChromeState,
    limit: usize,
    effects: &mut Vec<ShellChromeEffect>,
) {
    if state.can_open_session_list() && matches!(state.session_state, SessionState::Idle) {
        state.session_state = SessionState::Loading;
        effects.push(ShellChromeEffect::LoadRecentSessions { limit });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExitConfirmationState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
        ShellOverlay, StartupState, reduce_shell_chrome,
    };
    use crate::domain::recent_sessions::RecentSessions;
    use crate::domain::session_summary::SessionSummary;
    use crate::domain::startup_diagnostics::StartupDiagnostics;

    #[test]
    fn startup_loaded_auto_requests_sessions_when_ready() {
        let state = ShellChromeState::new();

        let reduced = reduce_shell_chrome(
            state,
            ShellChromeEvent::StartupLoaded {
                result: Ok(sample_startup_diagnostics()),
                session_page_size: 10,
            },
        );

        assert!(matches!(
            reduced.state.startup_state,
            StartupState::Ready(_)
        ));
        assert!(matches!(reduced.state.session_state, SessionState::Loading));
        assert_eq!(
            reduced.effects,
            vec![ShellChromeEffect::LoadRecentSessions { limit: 10 }]
        );
    }

    #[test]
    fn opening_sessions_overlay_requests_load_only_once() {
        let mut state = ShellChromeState::new();
        state.startup_state = StartupState::Ready(sample_startup_diagnostics());

        let first =
            reduce_shell_chrome(state, ShellChromeEvent::SessionsOverlayShown { limit: 10 });
        let second = reduce_shell_chrome(
            first.state.clone(),
            ShellChromeEvent::SessionsOverlayShown { limit: 10 },
        );

        assert_eq!(first.state.shell_overlay, ShellOverlay::Sessions);
        assert_eq!(
            first.effects,
            vec![ShellChromeEffect::LoadRecentSessions { limit: 10 }]
        );
        assert!(second.effects.is_empty());
    }

    #[test]
    fn moving_selection_clamps_to_available_bounds() {
        let mut state = ShellChromeState::new();
        state.session_state = SessionState::Ready(RecentSessions {
            items: vec![sample_session("thread-1"), sample_session("thread-2")],
            warnings: Vec::new(),
            next_cursor: None,
        });
        state.selected_session_index = 1;

        let reduced =
            reduce_shell_chrome(state, ShellChromeEvent::SessionSelectionMoved { delta: 5 });

        assert_eq!(reduced.state.selected_session_index, 1);
    }

    #[test]
    fn toggle_followup_templates_hides_exit_confirmation() {
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;

        let reduced = reduce_shell_chrome(state, ShellChromeEvent::FollowupTemplatesOverlayToggled);

        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert_eq!(reduced.state.shell_overlay, ShellOverlay::FollowupTemplates);
    }

    fn sample_startup_diagnostics() -> StartupDiagnostics {
        StartupDiagnostics {
            cwd: "/tmp/root".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "/opt/homebrew/bin/codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/root".to_string(),
            workspace_detail: "git repo: /tmp/root".to_string(),
            initialize_ok: true,
            initialize_detail: "darwin / unix / codex".to_string(),
            account_ok: true,
            account_detail: "logged in".to_string(),
            warnings: Vec::new(),
            schema_snapshot: "native/schema/codex_app_server_protocol.v2.schemas.json".to_string(),
        }
    }

    fn sample_session(id: &str) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            name: Some(id.to_string()),
            preview: "preview".to_string(),
            cwd: "/tmp/root".to_string(),
            source: "codex".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: format!("/tmp/root/{id}.json"),
            git_branch: Some("main".to_string()),
        }
    }
}
