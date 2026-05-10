use crate::core::app::StartupReadySnapshot;
use crate::domain::recent_sessions::SessionCatalog;

/*
 * shell chrome은 단일 conversation transcript 밖에 있는 TUI 상태를 reducer가 소유하게 하는 경계다.
 * startup diagnostics, session browser data, overlay identity, exit confirmation은 transcript reducer와 다른 수명을 가진다.
 * adapter layer는 key/callback/transition을 ShellChromeEvent로 보내고, reducer가 돌려준 effect만 실행한다.
 * 이렇게 해야 rendering은 state의 pure projection으로 남고, startup/session load 같은 side effect는 reducer 안에서 직접 실행되지 않는다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellOverlay {
    Hidden,
    Startup,
    Sessions,
    Supersession,
    Help,
    Queue,
    DirectionsMaintenance,
    PlanningInit,
    TaskIntake,
}

// exit confirmation은 overlay stack 일부가 아니라 별도 focus guard다. 어떤 overlay event도 이를 닫을 수 있어야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitConfirmationState {
    Hidden,
    Visible,
}

#[derive(Debug, Clone)]
pub enum StartupState {
    Idle,
    Loading,
    Ready(Box<StartupReadySnapshot>),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum SessionState {
    Idle,
    Loading,
    Ready(SessionCatalog),
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct ShellChromeState {
    // shell overlay는 한 번에 하나만 render된다. Hidden은 focus를 transcript 입력으로 돌려보내는 상태다.
    pub shell_overlay: ShellOverlay,
    pub exit_confirmation_state: ExitConfirmationState,
    // startup state는 session loading의 gate다. recent session catalog는 validated workspace가 있어야 의미가 있다.
    pub startup_state: StartupState,
    pub session_state: SessionState,
    // selection은 RecentSessions catalog에만 적용된다. attach-only catalog는 기존 선택 index를 건드리지 않는다.
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
            StartupState::Ready(ready) if ready.can_continue
        )
    }
}

impl Default for ShellChromeState {
    fn default() -> Self {
        Self::new()
    }
}

/*
 * event는 key handling, startup callback, conversation transition에서 들어오는 command-style input이다.
 * reducer가 상태 전이에 필요한 data만 싣고, 실제 IO는 여기서 실행하지 않는다.
 * shell chrome이 effect description만 반환하면 TUI adapter가 effect 실행을 scheduling하고 reducer는 deterministic하게 테스트할 수 있다.
 */
#[derive(Debug, Clone)]
pub enum ShellChromeEvent {
    StartupCheckRequested,
    StartupLoaded {
        result: Result<Box<StartupReadySnapshot>, String>,
        session_page_size: usize,
    },
    SessionsRequested {
        limit: usize,
    },
    SessionsLoaded(Result<SessionCatalog, String>),
    StartupOverlayShown,
    SessionsOverlayShown {
        limit: usize,
    },
    SupersessionOverlayShown,
    HelpOverlayShown,
    QueueOverlayShown,
    DirectionsMaintenanceOverlayShown,
    PlanningInitOverlayShown,
    TaskIntakeOverlayShown,
    StartupOverlayToggled,
    SessionsOverlayToggled {
        limit: usize,
    },
    SupersessionOverlayToggled,
    OverlayClosed,
    ExitConfirmationShown,
    ExitConfirmationHidden,
    // conversation transition은 transient shell chrome을 접어 overlay가 열린 shell context보다 오래 남지 않게 한다.
    TransientChromeDismissed,
    SessionSelectionMoved {
        delta: isize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellChromeEffect {
    RunStartupChecks,
    LoadSessionCatalog {
        // caller가 준 page size를 사용해 startup preload와 explicit reload의 paging policy를 맞춘다.
        limit: usize,
        // startup preload는 방금 validate된 workspace로 catalog를 scope하고, manual load는 None으로 전역 recent list를 요청한다.
        current_workspace_directory: Option<String>,
    },
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
            Ok(ready) => {
                // ready snapshot을 state 안으로 move하기 전에 session preload gate와 workspace scope에 필요한 값을 빼 둔다.
                let can_continue = ready.can_continue;
                let workspace_path = ready.workspace_path.clone();
                state.startup_state = StartupState::Ready(ready);
                // startup 성공은 session browser를 한 번 prime하지만, 이미 load/ready 상태인 catalog를 강제로 refresh하지 않는다.
                if can_continue && matches!(state.session_state, SessionState::Idle) {
                    state.session_state = SessionState::Loading;
                    effects.push(ShellChromeEffect::LoadSessionCatalog {
                        limit: session_page_size,
                        current_workspace_directory: Some(workspace_path),
                    });
                }
            }
            Err(message) => {
                state.startup_state = StartupState::Failed(message);
            }
        },
        ShellChromeEvent::SessionsRequested { limit } => {
            queue_session_reload_if_allowed(&mut state, limit, &mut effects);
        }
        ShellChromeEvent::SessionsLoaded(result) => {
            state.session_state = match result {
                Ok(catalog) => {
                    // 새 catalog가 도착하면 browser focus를 첫 visible row로 되돌려 이전 catalog index가 새 목록을 벗어나지 않게 한다.
                    state.selected_session_index = 0;
                    SessionState::Ready(catalog)
                }
                Err(message) => SessionState::Failed(message),
            };
        }
        ShellChromeEvent::StartupOverlayShown => {
            // non-exit overlay를 열면 exit prompt를 닫아 shell chrome의 focus owner를 하나로 유지한다.
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Startup;
        }
        ShellChromeEvent::SessionsOverlayShown { limit } => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Sessions;
            queue_session_load_if_allowed(&mut state, limit, &mut effects);
        }
        ShellChromeEvent::SupersessionOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Supersession;
        }
        ShellChromeEvent::HelpOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Help;
        }
        ShellChromeEvent::QueueOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Queue;
        }
        ShellChromeEvent::DirectionsMaintenanceOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::DirectionsMaintenance;
        }
        ShellChromeEvent::PlanningInitOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::PlanningInit;
        }
        ShellChromeEvent::TaskIntakeOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::TaskIntake;
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
            // session overlay를 닫는 일은 시각적 상태 전이뿐이고, 여는 경우에만 필요하면 initial catalog load를 요청한다.
            if state.shell_overlay == ShellOverlay::Sessions {
                state.shell_overlay = ShellOverlay::Hidden;
            } else {
                state.exit_confirmation_state = ExitConfirmationState::Hidden;
                state.shell_overlay = ShellOverlay::Sessions;
                queue_session_load_if_allowed(&mut state, limit, &mut effects);
            }
        }
        ShellChromeEvent::SupersessionOverlayToggled => {
            // supersession 세부 state는 다른 reducer/projection에서 render되므로 이 toggle은 shell focus만 바꾼다.
            if state.shell_overlay == ShellOverlay::Supersession {
                state.shell_overlay = ShellOverlay::Hidden;
            } else {
                state.exit_confirmation_state = ExitConfirmationState::Hidden;
                state.shell_overlay = ShellOverlay::Supersession;
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
        ShellChromeEvent::TransientChromeDismissed => {
            // conversation change는 transient chrome만 접고, 이미 cache된 startup/session data는 유지한다.
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Hidden;
        }
        ShellChromeEvent::SessionSelectionMoved { delta } => {
            // navigation은 full recent-session catalog가 있을 때만 적용된다. attach-only catalog는 selectable row가 없다.
            let SessionState::Ready(catalog) = &state.session_state else {
                return ShellChromeReduction { state, effects };
            };
            let Some(recent_sessions) = catalog.recent_sessions() else {
                return ShellChromeReduction { state, effects };
            };
            if recent_sessions.items.is_empty() {
                state.selected_session_index = 0;
            } else {
                // wrap 대신 clamp를 써서 list 끝에서 반복 keypress가 같은 row에 머물게 한다.
                let max_index = recent_sessions.items.len().saturating_sub(1) as isize;
                let current_index = state.selected_session_index as isize;
                let next_index = (current_index + delta).clamp(0, max_index);
                state.selected_session_index = next_index as usize;
            }
        }
    }

    ShellChromeReduction { state, effects }
}

/*
 * overlay-open load는 idempotent하다.
 * session overlay를 여는 동작이 failed catalog를 자동 retry하면, 사용자가 단순히 화면을 열었을 뿐인데 IO가 반복된다.
 * 그래서 Idle일 때만 initial load effect를 queue한다.
 */
fn queue_session_load_if_allowed(
    state: &mut ShellChromeState,
    limit: usize,
    effects: &mut Vec<ShellChromeEffect>,
) {
    if state.can_open_session_list() && matches!(state.session_state, SessionState::Idle) {
        state.session_state = SessionState::Loading;
        effects.push(ShellChromeEffect::LoadSessionCatalog {
            limit,
            current_workspace_directory: None,
        });
    }
}

/*
 * explicit reload는 Ready나 Failed 뒤에 허용하지만, Loading 중에는 중복 effect를 만들지 않는다.
 * 사용자가 명시적으로 session reload를 요청한 경우에는 실패 복구 의도가 있으므로 overlay-open load보다 넓게 허용한다.
 */
fn queue_session_reload_if_allowed(
    state: &mut ShellChromeState,
    limit: usize,
    effects: &mut Vec<ShellChromeEffect>,
) {
    if state.can_open_session_list() && !matches!(state.session_state, SessionState::Loading) {
        state.session_state = SessionState::Loading;
        effects.push(ShellChromeEffect::LoadSessionCatalog {
            limit,
            current_workspace_directory: None,
        });
    }
}
#[cfg(test)]
mod tests {
    use super::{
        ExitConfirmationState, SessionState, ShellChromeEffect, ShellChromeEvent, ShellChromeState,
        ShellOverlay, StartupState, reduce_shell_chrome,
    };
    use crate::core::app::StartupReadySnapshot;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogTier};
    use crate::domain::session_summary::SessionSummary;
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    #[test]
    fn startup_loaded_auto_requests_sessions_when_ready() {
        // startup 성공 직후에는 validated workspace로 recent session preload를 한 번 걸어 첫 화면 진입 비용을 줄인다.
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
            vec![ShellChromeEffect::LoadSessionCatalog {
                limit: 10,
                current_workspace_directory: Some("/tmp/root".to_string()),
            }]
        );
    }
    #[test]
    fn opening_sessions_overlay_requests_load_only_once() {
        // session overlay open은 initial load trigger이지만, 이미 loading으로 전이된 catalog에 중복 effect를 더하지 않는다.
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
            vec![ShellChromeEffect::LoadSessionCatalog {
                limit: 10,
                current_workspace_directory: None
            }]
        );
        assert!(second.effects.is_empty());
    }
    #[test]
    fn explicit_sessions_request_reloads_after_failure() {
        // explicit request는 실패한 catalog를 복구하려는 사용자 의도이므로 Failed 상태에서도 reload effect를 허용한다.
        let mut state = ShellChromeState::new();
        state.startup_state = StartupState::Ready(sample_startup_diagnostics());
        state.session_state = SessionState::Failed("boom".to_string());
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::SessionsRequested { limit: 10 });

        assert!(matches!(reduced.state.session_state, SessionState::Loading));
        assert_eq!(
            reduced.effects,
            vec![ShellChromeEffect::LoadSessionCatalog {
                limit: 10,
                current_workspace_directory: None
            }]
        );
    }
    #[test]
    fn opening_sessions_overlay_while_startup_blocked_does_not_queue_load() {
        // startup diagnostic이 continue를 막으면 overlay는 열 수 있어도 workspace-scoped session load는 queue하지 않는다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        state.startup_state = StartupState::Ready(sample_blocked_startup_diagnostics());
        let reduced =
            reduce_shell_chrome(state, ShellChromeEvent::SessionsOverlayShown { limit: 10 });

        assert_eq!(reduced.state.shell_overlay, ShellOverlay::Sessions);
        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert!(matches!(reduced.state.session_state, SessionState::Idle));
        assert!(reduced.effects.is_empty());
    }
    #[test]
    fn toggling_sessions_overlay_requests_load_only_when_opening() {
        // toggle open은 load를 요청할 수 있지만, toggle close는 시각적 collapse라 IO effect를 만들지 않는다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        state.startup_state = StartupState::Ready(sample_startup_diagnostics());
        let opened = reduce_shell_chrome(
            state,
            ShellChromeEvent::SessionsOverlayToggled { limit: 10 },
        );
        let closed = reduce_shell_chrome(
            opened.state.clone(),
            ShellChromeEvent::SessionsOverlayToggled { limit: 10 },
        );

        assert_eq!(opened.state.shell_overlay, ShellOverlay::Sessions);
        assert_eq!(
            opened.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert!(matches!(opened.state.session_state, SessionState::Loading));
        assert_eq!(
            opened.effects,
            vec![ShellChromeEffect::LoadSessionCatalog {
                limit: 10,
                current_workspace_directory: None
            }]
        );
        assert_eq!(closed.state.shell_overlay, ShellOverlay::Hidden);
        assert!(closed.effects.is_empty());
    }
    #[test]
    fn explicit_sessions_request_while_loading_does_not_duplicate_effect() {
        // Loading 중 explicit reload는 in-flight request와 경쟁하지 않도록 no-op으로 접는다.
        let mut state = ShellChromeState::new();
        state.startup_state = StartupState::Ready(sample_startup_diagnostics());
        state.session_state = SessionState::Loading;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::SessionsRequested { limit: 10 });

        assert!(matches!(reduced.state.session_state, SessionState::Loading));
        assert!(reduced.effects.is_empty());
    }
    #[test]
    fn moving_selection_clamps_to_available_bounds() {
        // session browser navigation은 list edge에서 wrap하지 않고 clamp되어 같은 row에 머문다.
        let mut state = ShellChromeState::new();
        state.session_state = SessionState::Ready(
            RecentSessions {
                items: vec![sample_session("thread-1"), sample_session("thread-2")],
                warnings: Vec::new(),
                next_cursor: None,
            }
            .into(),
        );
        state.selected_session_index = 1;
        let reduced =
            reduce_shell_chrome(state, ShellChromeEvent::SessionSelectionMoved { delta: 5 });

        assert_eq!(reduced.state.selected_session_index, 1);
    }
    #[test]
    fn moving_selection_ignores_attach_only_catalog_without_browser_items() {
        // attach-only catalog는 browser item이 없으므로 selection movement가 기존 index를 바꾸지 않는다.
        let mut state = ShellChromeState::new();
        state.session_state = SessionState::Ready(SessionCatalog::unsupported(
            SessionCatalogTier::AttachOnly,
            "session listing is unsupported for this bridge",
            Vec::new(),
        ));
        state.selected_session_index = 1;
        let reduced =
            reduce_shell_chrome(state, ShellChromeEvent::SessionSelectionMoved { delta: -1 });

        assert_eq!(reduced.state.selected_session_index, 1);
    }
    #[test]
    fn showing_planning_init_overlay_hides_exit_confirmation() {
        // planning init overlay가 focus owner가 되면 exit confirmation은 함께 보이지 않아야 한다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::PlanningInitOverlayShown);

        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert_eq!(reduced.state.shell_overlay, ShellOverlay::PlanningInit);
        assert!(reduced.effects.is_empty());
    }
    #[test]
    fn showing_help_overlay_hides_exit_confirmation() {
        // help overlay도 shell focus를 가져가므로 exit confirmation과 동시에 노출되지 않는다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::HelpOverlayShown);

        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert_eq!(reduced.state.shell_overlay, ShellOverlay::Help);
        assert!(reduced.effects.is_empty());
    }
    #[test]
    fn toggling_supersession_overlay_hides_exit_confirmation() {
        // supersession toggle은 별도 projection을 열더라도 shell chrome의 단일 focus owner 규칙을 따른다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::SupersessionOverlayToggled);

        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert_eq!(reduced.state.shell_overlay, ShellOverlay::Supersession);
        assert!(reduced.effects.is_empty());
    }
    fn sample_startup_diagnostics() -> Box<StartupReadySnapshot> {
        Box::new(StartupReadySnapshot::from_diagnostics(
            sample_startup_diagnostics_source(),
        ))
    }
    fn sample_blocked_startup_diagnostics() -> Box<StartupReadySnapshot> {
        Box::new(StartupReadySnapshot::from_diagnostics(StartupDiagnostics {
            account_ok: false,
            account_detail: "login required".to_string(),
            ..sample_startup_diagnostics_source()
        }))
    }
    fn sample_startup_diagnostics_source() -> StartupDiagnostics {
        StartupDiagnostics {
            cwd: "/tmp/root".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "/opt/homebrew/bin/codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/root".to_string(),
            workspace_detail: "git repo: /tmp/root".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_ok: true,
            initialize_detail: "darwin / unix / codex".to_string(),
            account_ok: true,
            account_detail: "logged in".to_string(),
            warnings: Vec::new(),
            schema_snapshot: StartupDiagnostics::bundled_schema_snapshot_label(),
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
