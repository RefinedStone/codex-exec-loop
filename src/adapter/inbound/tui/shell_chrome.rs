use crate::core::app::{
    SessionCatalogLoadMode, SessionCatalogSnapshot, StartupReadySnapshot, StartupSnapshot,
};
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
    ModelSelection,
    ViewSelection,
    LanguageSelection,
    Supersession,
    ParallelPeek,
    Activity,
    Help,
    Reviews,
    Queue,
    DirectionsMaintenance,
    PlanningInit,
    Approval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellOverlayExitMode {
    Exit,
    Suspend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellOverlayTransition {
    pub from: ShellOverlay,
    pub to: ShellOverlay,
    pub exit_mode: ShellOverlayExitMode,
}

impl ShellOverlay {
    pub(crate) fn prompt_input_has_focus(
        self,
        dialog_visible: bool,
        parallel_prompt_input_locked: bool,
    ) -> bool {
        if dialog_visible {
            return false;
        }
        match self {
            Self::Hidden => true,
            Self::Supersession => !parallel_prompt_input_locked,
            _ => false,
        }
    }
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
    // Approval은 비동기 turn event가 여는 우선순위 modal이다. 미저장
    // Directions editor를 잠시 선점한 경우에만 explicit close에서 돌려준다.
    pub approval_return_overlay: Option<ShellOverlay>,
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
            approval_return_overlay: None,
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
    StartupProjected {
        snapshot: StartupSnapshot,
        session_page_size: usize,
    },
    SessionsRequested {
        limit: usize,
    },
    SessionCatalogProjected {
        snapshot: SessionCatalogSnapshot,
        selection_policy: SessionCatalogSelectionPolicy,
    },
    SessionSelectionProjected {
        index: usize,
    },
    StartupOverlayShown,
    SessionsOverlayShown {
        limit: usize,
    },
    ModelSelectionOverlayShown,
    ViewSelectionOverlayShown,
    LanguageSelectionOverlayShown,
    SupersessionOverlayShown,
    ParallelPeekOverlayShown,
    ActivityOverlayShown,
    HelpOverlayShown,
    ReviewsOverlayShown,
    QueueOverlayShown,
    DirectionsMaintenanceOverlayShown,
    PlanningInitOverlayShown,
    ApprovalOverlayShown,
    ApprovalOverlayClosed,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCatalogSelectionPolicy {
    Preserve,
    ResetOnReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellChromeEffect {
    RunStartupChecks,
    LoadSessionCatalog {
        mode: SessionCatalogLoadMode,
        // caller가 준 page size를 사용해 startup preload와 explicit reload의 paging policy를 맞춘다.
        limit: usize,
        // startup preload는 validated workspace를 고정하고, 나머지 trigger는 실행 시점의 visible workspace를 사용한다.
        current_workspace_directory: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ShellChromeReduction {
    pub state: ShellChromeState,
    pub effects: Vec<ShellChromeEffect>,
    pub overlay_transition: Option<ShellOverlayTransition>,
}

pub fn reduce_shell_chrome(
    mut state: ShellChromeState,
    event: ShellChromeEvent,
) -> ShellChromeReduction {
    let previous_overlay = state.shell_overlay;
    let mut effects = Vec::new();
    if state.shell_overlay == ShellOverlay::Approval
        && matches!(
            &event,
            ShellChromeEvent::StartupOverlayShown
                | ShellChromeEvent::SessionsOverlayShown { .. }
                | ShellChromeEvent::ModelSelectionOverlayShown
                | ShellChromeEvent::ViewSelectionOverlayShown
                | ShellChromeEvent::LanguageSelectionOverlayShown
                | ShellChromeEvent::SupersessionOverlayShown
                | ShellChromeEvent::ParallelPeekOverlayShown
                | ShellChromeEvent::ActivityOverlayShown
                | ShellChromeEvent::HelpOverlayShown
                | ShellChromeEvent::ReviewsOverlayShown
                | ShellChromeEvent::QueueOverlayShown
                | ShellChromeEvent::DirectionsMaintenanceOverlayShown
                | ShellChromeEvent::PlanningInitOverlayShown
                | ShellChromeEvent::StartupOverlayToggled
                | ShellChromeEvent::SessionsOverlayToggled { .. }
                | ShellChromeEvent::SupersessionOverlayToggled
                | ShellChromeEvent::OverlayClosed
                | ShellChromeEvent::ExitConfirmationShown
                | ShellChromeEvent::TransientChromeDismissed
        )
    {
        return finish_shell_chrome_reduction(state, effects, previous_overlay);
    }

    match event {
        ShellChromeEvent::StartupCheckRequested => {
            // Loading은 Core의 StartupChanged projection만 설치할 수 있다. 이 intent는 effect만 기술한다.
            effects.push(ShellChromeEffect::RunStartupChecks);
        }
        ShellChromeEvent::StartupProjected {
            snapshot,
            session_page_size,
        } => match snapshot {
            StartupSnapshot::Idle => {
                state.startup_state = StartupState::Idle;
            }
            StartupSnapshot::Loading => {
                state.startup_state = StartupState::Loading;
            }
            StartupSnapshot::Ready(ready) => {
                // ready snapshot을 state 안으로 move하기 전에 session preload gate와 workspace scope에 필요한 값을 빼 둔다.
                let can_continue = ready.can_continue;
                let workspace_path = ready.workspace_path.clone();
                state.startup_state = StartupState::Ready(ready);
                // startup 성공은 ensure intent를 Core에 전달한다. 기존 catalog와 in-flight
                // request를 보고 실제 load를 시작할지는 Core admission이 결정한다.
                if can_continue {
                    effects.push(ShellChromeEffect::LoadSessionCatalog {
                        mode: SessionCatalogLoadMode::EnsureLoaded,
                        limit: session_page_size,
                        current_workspace_directory: Some(workspace_path),
                    });
                }
            }
            StartupSnapshot::Failed { message } => {
                state.startup_state = StartupState::Failed(message);
            }
        },
        ShellChromeEvent::SessionsRequested { limit } => {
            queue_session_catalog_intent_if_startup_ready(
                &state,
                SessionCatalogLoadMode::Refresh,
                limit,
                &mut effects,
            );
        }
        ShellChromeEvent::SessionCatalogProjected {
            snapshot,
            selection_policy,
        } => {
            state.session_state = match snapshot {
                SessionCatalogSnapshot::Idle => SessionState::Idle,
                SessionCatalogSnapshot::Loading => SessionState::Loading,
                SessionCatalogSnapshot::Ready(ready) => {
                    // 새 catalog가 도착하면 browser focus를 첫 visible row로 되돌려 이전 catalog index가 새 목록을 벗어나지 않게 한다.
                    if selection_policy == SessionCatalogSelectionPolicy::ResetOnReady {
                        state.selected_session_index = 0;
                    }
                    SessionState::Ready(*ready.catalog)
                }
                SessionCatalogSnapshot::Failed { message } => SessionState::Failed(message),
            };
        }
        ShellChromeEvent::SessionSelectionProjected { index } => {
            state.selected_session_index = index;
        }
        ShellChromeEvent::StartupOverlayShown => {
            // non-exit overlay를 열면 exit prompt를 닫아 shell chrome의 focus owner를 하나로 유지한다.
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Startup;
        }
        ShellChromeEvent::SessionsOverlayShown { limit } => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Sessions;
            queue_session_catalog_intent_if_startup_ready(
                &state,
                SessionCatalogLoadMode::EnsureLoaded,
                limit,
                &mut effects,
            );
        }
        ShellChromeEvent::ModelSelectionOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::ModelSelection;
        }
        ShellChromeEvent::ViewSelectionOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::ViewSelection;
        }
        ShellChromeEvent::LanguageSelectionOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::LanguageSelection;
        }
        ShellChromeEvent::SupersessionOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Supersession;
        }
        ShellChromeEvent::ParallelPeekOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::ParallelPeek;
        }
        ShellChromeEvent::ActivityOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Activity;
        }
        ShellChromeEvent::HelpOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Help;
        }
        ShellChromeEvent::ReviewsOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            state.shell_overlay = ShellOverlay::Reviews;
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
        ShellChromeEvent::ApprovalOverlayShown => {
            state.exit_confirmation_state = ExitConfirmationState::Hidden;
            if state.shell_overlay == ShellOverlay::DirectionsMaintenance {
                state.approval_return_overlay = Some(state.shell_overlay);
            } else if state.shell_overlay != ShellOverlay::Approval {
                state.approval_return_overlay = None;
            }
            state.shell_overlay = ShellOverlay::Approval;
        }
        ShellChromeEvent::ApprovalOverlayClosed => {
            if state.shell_overlay == ShellOverlay::Approval {
                state.shell_overlay = state
                    .approval_return_overlay
                    .take()
                    .unwrap_or(ShellOverlay::Hidden);
            }
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
                queue_session_catalog_intent_if_startup_ready(
                    &state,
                    SessionCatalogLoadMode::EnsureLoaded,
                    limit,
                    &mut effects,
                );
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
            if state.shell_overlay != ShellOverlay::Approval {
                state.shell_overlay = ShellOverlay::Hidden;
            }
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
            if state.shell_overlay != ShellOverlay::Approval {
                state.shell_overlay = ShellOverlay::Hidden;
            }
        }
    }

    finish_shell_chrome_reduction(state, effects, previous_overlay)
}

fn finish_shell_chrome_reduction(
    state: ShellChromeState,
    effects: Vec<ShellChromeEffect>,
    previous_overlay: ShellOverlay,
) -> ShellChromeReduction {
    let overlay_transition =
        (previous_overlay != state.shell_overlay).then_some(ShellOverlayTransition {
            from: previous_overlay,
            to: state.shell_overlay,
            exit_mode: if state.shell_overlay == ShellOverlay::Approval {
                ShellOverlayExitMode::Suspend
            } else {
                ShellOverlayExitMode::Exit
            },
        });
    ShellChromeReduction {
        state,
        effects,
        overlay_transition,
    }
}

/*
 * Shell chrome은 startup readiness만 확인하고 typed catalog intent를 Core로 전달한다.
 * catalog projection을 보고 initial/reload를 억제하거나 Loading을 선반영하면 adapter가
 * semantic admission authority를 다시 소유하게 되므로 여기서는 SessionState를 읽지 않는다.
 */
fn queue_session_catalog_intent_if_startup_ready(
    state: &ShellChromeState,
    mode: SessionCatalogLoadMode,
    limit: usize,
    effects: &mut Vec<ShellChromeEffect>,
) {
    if state.can_open_session_list() {
        effects.push(ShellChromeEffect::LoadSessionCatalog {
            mode,
            limit,
            current_workspace_directory: None,
        });
    }
}
#[cfg(test)]
mod tests {
    use super::{
        ExitConfirmationState, SessionCatalogSelectionPolicy, SessionState, ShellChromeEffect,
        ShellChromeEvent, ShellChromeState, ShellOverlay, ShellOverlayExitMode,
        ShellOverlayTransition, StartupState, reduce_shell_chrome,
    };
    use crate::core::app::{
        SessionCatalogLoadMode, SessionCatalogReadySnapshot, SessionCatalogSnapshot,
        StartupReadySnapshot, StartupSnapshot,
    };
    use crate::domain::recent_sessions::RecentSessions;
    use crate::domain::session_summary::SessionSummary;
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    #[test]
    fn startup_ready_projection_auto_requests_sessions() {
        // startup 성공 직후에는 validated workspace로 recent session preload를 한 번 걸어 첫 화면 진입 비용을 줄인다.
        let state = ShellChromeState::new();
        let reduced = reduce_shell_chrome(
            state,
            ShellChromeEvent::StartupProjected {
                snapshot: StartupSnapshot::Ready(sample_startup_diagnostics()),
                session_page_size: 10,
            },
        );

        assert!(matches!(
            reduced.state.startup_state,
            StartupState::Ready(_)
        ));
        assert!(matches!(reduced.state.session_state, SessionState::Idle));
        assert_eq!(
            reduced.effects,
            vec![ShellChromeEffect::LoadSessionCatalog {
                mode: SessionCatalogLoadMode::EnsureLoaded,
                limit: 10,
                current_workspace_directory: Some("/tmp/root".to_string()),
            }]
        );
    }

    #[test]
    fn startup_request_waits_for_the_core_loading_projection() {
        let reduced = reduce_shell_chrome(
            ShellChromeState::new(),
            ShellChromeEvent::StartupCheckRequested,
        );

        assert!(matches!(reduced.state.startup_state, StartupState::Idle));
        assert_eq!(reduced.effects, vec![ShellChromeEffect::RunStartupChecks]);

        let projected = reduce_shell_chrome(
            reduced.state,
            ShellChromeEvent::StartupProjected {
                snapshot: StartupSnapshot::Loading,
                session_page_size: 10,
            },
        );
        assert!(matches!(
            projected.state.startup_state,
            StartupState::Loading
        ));
        assert!(projected.effects.is_empty());
    }

    #[test]
    fn startup_idle_and_failure_are_installed_only_from_typed_projections() {
        let mut state = ShellChromeState::new();
        state.startup_state = StartupState::Loading;
        let idle = reduce_shell_chrome(
            state,
            ShellChromeEvent::StartupProjected {
                snapshot: StartupSnapshot::Idle,
                session_page_size: 10,
            },
        );
        assert!(matches!(idle.state.startup_state, StartupState::Idle));

        let failed = reduce_shell_chrome(
            idle.state,
            ShellChromeEvent::StartupProjected {
                snapshot: StartupSnapshot::Failed {
                    message: "missing provider".to_string(),
                },
                session_page_size: 10,
            },
        );
        assert!(matches!(
            failed.state.startup_state,
            StartupState::Failed(ref message) if message == "missing provider"
        ));
        assert!(failed.effects.is_empty());
    }

    #[test]
    fn catalog_projection_policy_resets_only_authoritative_load_results() {
        let catalog = SessionCatalogReadySnapshot {
            catalog: Box::new(
                RecentSessions {
                    items: vec![sample_session("thread-1"), sample_session("thread-2")],
                    warnings: Vec::new(),
                    next_cursor: None,
                }
                .into(),
            ),
            tier_label: "provider-backed-catalog".to_string(),
            item_count: 2,
            warnings: Vec::new(),
        };
        let mut state = ShellChromeState::new();
        state.selected_session_index = 1;

        let preserved = reduce_shell_chrome(
            state,
            ShellChromeEvent::SessionCatalogProjected {
                snapshot: SessionCatalogSnapshot::Ready(catalog.clone()),
                selection_policy: SessionCatalogSelectionPolicy::Preserve,
            },
        );
        assert_eq!(preserved.state.selected_session_index, 1);

        let reset = reduce_shell_chrome(
            preserved.state,
            ShellChromeEvent::SessionCatalogProjected {
                snapshot: SessionCatalogSnapshot::Ready(catalog),
                selection_policy: SessionCatalogSelectionPolicy::ResetOnReady,
            },
        );
        assert_eq!(reset.state.selected_session_index, 0);
    }

    #[test]
    fn non_ready_catalog_projections_preserve_the_browser_selection() {
        let snapshots = [
            SessionCatalogSnapshot::Idle,
            SessionCatalogSnapshot::Loading,
            SessionCatalogSnapshot::Failed {
                message: "catalog unavailable".to_string(),
            },
        ];

        for snapshot in snapshots {
            let mut state = ShellChromeState::new();
            state.selected_session_index = 3;
            let reduced = reduce_shell_chrome(
                state,
                ShellChromeEvent::SessionCatalogProjected {
                    snapshot,
                    selection_policy: SessionCatalogSelectionPolicy::ResetOnReady,
                },
            );
            assert_eq!(reduced.state.selected_session_index, 3);
        }
    }
    #[test]
    fn opening_sessions_overlay_forwards_each_ensure_intent_to_core() {
        // overlay open은 projection을 선반영하거나 자체 coalescing하지 않고 매번 Core에 ensure intent를 전달한다.
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
                mode: SessionCatalogLoadMode::EnsureLoaded,
                limit: 10,
                current_workspace_directory: None
            }]
        );
        assert!(matches!(first.state.session_state, SessionState::Idle));
        assert_eq!(second.effects, first.effects);
        assert!(matches!(second.state.session_state, SessionState::Idle));
    }

    #[test]
    fn opening_sessions_overlay_does_not_admit_from_catalog_projection() {
        let catalog = RecentSessions {
            items: Vec::new(),
            warnings: Vec::new(),
            next_cursor: None,
        }
        .into();
        for session_state in [
            SessionState::Idle,
            SessionState::Loading,
            SessionState::Ready(catalog),
            SessionState::Failed("catalog unavailable".to_string()),
        ] {
            let mut state = ShellChromeState::new();
            state.startup_state = StartupState::Ready(sample_startup_diagnostics());
            let expected_session_state = session_state.clone();
            state.session_state = session_state;

            let reduced =
                reduce_shell_chrome(state, ShellChromeEvent::SessionsOverlayShown { limit: 10 });

            assert_eq!(
                reduced.effects,
                vec![ShellChromeEffect::LoadSessionCatalog {
                    mode: SessionCatalogLoadMode::EnsureLoaded,
                    limit: 10,
                    current_workspace_directory: None,
                }]
            );
            match (expected_session_state, &reduced.state.session_state) {
                (SessionState::Idle, SessionState::Idle)
                | (SessionState::Loading, SessionState::Loading) => {}
                (SessionState::Ready(expected), SessionState::Ready(actual)) => {
                    assert_eq!(actual, &expected);
                }
                (SessionState::Failed(expected), SessionState::Failed(actual)) => {
                    assert_eq!(actual, &expected);
                }
                (expected, actual) => {
                    panic!(
                        "overlay ensure must preserve the catalog projection: expected {expected:?}, got {actual:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn explicit_sessions_request_reloads_after_failure() {
        // explicit request는 실패한 catalog를 복구하려는 사용자 의도이므로 Failed 상태에서도 reload effect를 허용한다.
        let mut state = ShellChromeState::new();
        state.startup_state = StartupState::Ready(sample_startup_diagnostics());
        state.session_state = SessionState::Failed("boom".to_string());
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::SessionsRequested { limit: 10 });

        assert!(matches!(
            reduced.state.session_state,
            SessionState::Failed(ref message) if message == "boom"
        ));
        assert_eq!(
            reduced.effects,
            vec![ShellChromeEffect::LoadSessionCatalog {
                mode: SessionCatalogLoadMode::Refresh,
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
        assert!(matches!(opened.state.session_state, SessionState::Idle));
        assert_eq!(
            opened.effects,
            vec![ShellChromeEffect::LoadSessionCatalog {
                mode: SessionCatalogLoadMode::EnsureLoaded,
                limit: 10,
                current_workspace_directory: None
            }]
        );
        assert_eq!(closed.state.shell_overlay, ShellOverlay::Hidden);
        assert!(closed.effects.is_empty());
    }
    #[test]
    fn explicit_sessions_request_while_loading_still_reaches_core_admission() {
        // Loading은 Core projection일 뿐이다. 동일 target coalescing과 target supersession은
        // Core가 결정할 수 있도록 explicit refresh intent를 그대로 전달한다.
        let mut state = ShellChromeState::new();
        state.startup_state = StartupState::Ready(sample_startup_diagnostics());
        state.session_state = SessionState::Loading;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::SessionsRequested { limit: 10 });

        assert!(matches!(reduced.state.session_state, SessionState::Loading));
        assert_eq!(
            reduced.effects,
            vec![ShellChromeEffect::LoadSessionCatalog {
                mode: SessionCatalogLoadMode::Refresh,
                limit: 10,
                current_workspace_directory: None,
            }]
        );
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
    fn showing_model_selection_overlay_hides_exit_confirmation() {
        // model selection도 app-server IO 없이 shell focus만 가져가는 inspection surface다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::ModelSelectionOverlayShown);

        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert_eq!(reduced.state.shell_overlay, ShellOverlay::ModelSelection);
        assert!(reduced.effects.is_empty());
    }
    #[test]
    fn showing_view_selection_overlay_hides_exit_confirmation() {
        // view selection은 transcript projection만 바꾸는 local inspection surface다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::ViewSelectionOverlayShown);

        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert_eq!(reduced.state.shell_overlay, ShellOverlay::ViewSelection);
        assert!(reduced.effects.is_empty());
    }
    #[test]
    fn showing_language_selection_overlay_hides_exit_confirmation() {
        // language selection도 TUI-local projection만 바꾸므로 shell focus owner 규칙을 따른다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::LanguageSelectionOverlayShown);

        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert_eq!(reduced.state.shell_overlay, ShellOverlay::LanguageSelection);
        assert!(reduced.effects.is_empty());
    }
    #[test]
    fn showing_parallel_peek_overlay_hides_exit_confirmation() {
        // parallel peek도 active agent 대화를 엿보는 shell focus owner이므로 exit prompt와 겹치지 않는다.
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        let reduced = reduce_shell_chrome(state, ShellChromeEvent::ParallelPeekOverlayShown);

        assert_eq!(
            reduced.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );
        assert_eq!(reduced.state.shell_overlay, ShellOverlay::ParallelPeek);
        assert!(reduced.effects.is_empty());
    }
    #[test]
    fn activity_overlay_obeys_exit_and_approval_focus_priority() {
        let mut state = ShellChromeState::new();
        state.exit_confirmation_state = ExitConfirmationState::Visible;
        let activity = reduce_shell_chrome(state, ShellChromeEvent::ActivityOverlayShown);

        assert_eq!(activity.state.shell_overlay, ShellOverlay::Activity);
        assert_eq!(
            activity.state.exit_confirmation_state,
            ExitConfirmationState::Hidden
        );

        let approval = reduce_shell_chrome(activity.state, ShellChromeEvent::ApprovalOverlayShown);
        assert_eq!(approval.state.shell_overlay, ShellOverlay::Approval);

        let blocked = reduce_shell_chrome(approval.state, ShellChromeEvent::ActivityOverlayShown);
        assert_eq!(blocked.state.shell_overlay, ShellOverlay::Approval);
        assert!(blocked.effects.is_empty());
        assert_eq!(blocked.overlay_transition, None);
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
    #[test]
    fn approval_overlay_requires_its_explicit_close_event() {
        let shown = reduce_shell_chrome(
            ShellChromeState::new(),
            ShellChromeEvent::ApprovalOverlayShown,
        );
        assert_eq!(shown.state.shell_overlay, ShellOverlay::Approval);

        let generic_close = reduce_shell_chrome(shown.state, ShellChromeEvent::OverlayClosed);
        assert_eq!(generic_close.state.shell_overlay, ShellOverlay::Approval);
        assert_eq!(generic_close.overlay_transition, None);

        let competing_overlay =
            reduce_shell_chrome(generic_close.state, ShellChromeEvent::HelpOverlayShown);
        assert_eq!(
            competing_overlay.state.shell_overlay,
            ShellOverlay::Approval
        );
        assert_eq!(competing_overlay.overlay_transition, None);

        let closed = reduce_shell_chrome(
            competing_overlay.state,
            ShellChromeEvent::ApprovalOverlayClosed,
        );
        assert_eq!(closed.state.shell_overlay, ShellOverlay::Hidden);
    }

    #[test]
    fn overlay_transition_table_covers_open_replace_and_dismiss_paths() {
        let cases = [
            (
                "open",
                ShellOverlay::Hidden,
                ShellChromeEvent::HelpOverlayShown,
                ShellOverlay::Help,
            ),
            (
                "replace",
                ShellOverlay::Help,
                ShellChromeEvent::ReviewsOverlayShown,
                ShellOverlay::Reviews,
            ),
            (
                "toggle close",
                ShellOverlay::Startup,
                ShellChromeEvent::StartupOverlayToggled,
                ShellOverlay::Hidden,
            ),
            (
                "explicit close",
                ShellOverlay::Reviews,
                ShellChromeEvent::OverlayClosed,
                ShellOverlay::Hidden,
            ),
            (
                "transient dismiss",
                ShellOverlay::Queue,
                ShellChromeEvent::TransientChromeDismissed,
                ShellOverlay::Hidden,
            ),
        ];

        for (label, from, event, to) in cases {
            let mut state = ShellChromeState::new();
            state.shell_overlay = from;

            let reduced = reduce_shell_chrome(state, event);

            assert_eq!(reduced.state.shell_overlay, to, "{label}");
            assert_eq!(
                reduced.overlay_transition,
                Some(ShellOverlayTransition {
                    from,
                    to,
                    exit_mode: ShellOverlayExitMode::Exit,
                }),
                "{label}"
            );
        }
    }

    #[test]
    fn approval_overlay_suspends_departed_overlay_and_restores_directions() {
        let mut state = ShellChromeState::new();
        state.shell_overlay = ShellOverlay::DirectionsMaintenance;

        let shown = reduce_shell_chrome(state, ShellChromeEvent::ApprovalOverlayShown);
        assert_eq!(shown.state.shell_overlay, ShellOverlay::Approval);
        assert_eq!(
            shown.state.approval_return_overlay,
            Some(ShellOverlay::DirectionsMaintenance)
        );
        assert_eq!(
            shown.overlay_transition,
            Some(ShellOverlayTransition {
                from: ShellOverlay::DirectionsMaintenance,
                to: ShellOverlay::Approval,
                exit_mode: ShellOverlayExitMode::Suspend,
            })
        );

        let closed = reduce_shell_chrome(shown.state, ShellChromeEvent::ApprovalOverlayClosed);
        assert_eq!(
            closed.state.shell_overlay,
            ShellOverlay::DirectionsMaintenance
        );
        assert_eq!(closed.state.approval_return_overlay, None);
        assert_eq!(
            closed.overlay_transition,
            Some(ShellOverlayTransition {
                from: ShellOverlay::Approval,
                to: ShellOverlay::DirectionsMaintenance,
                exit_mode: ShellOverlayExitMode::Exit,
            })
        );

        let mut planning_state = ShellChromeState::new();
        planning_state.shell_overlay = ShellOverlay::PlanningInit;
        let planning_approval =
            reduce_shell_chrome(planning_state, ShellChromeEvent::ApprovalOverlayShown);
        assert_eq!(planning_approval.state.approval_return_overlay, None);
        assert_eq!(
            planning_approval.overlay_transition,
            Some(ShellOverlayTransition {
                from: ShellOverlay::PlanningInit,
                to: ShellOverlay::Approval,
                exit_mode: ShellOverlayExitMode::Suspend,
            })
        );

        let planning_closed = reduce_shell_chrome(
            planning_approval.state,
            ShellChromeEvent::ApprovalOverlayClosed,
        );
        assert_eq!(planning_closed.state.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            planning_closed.overlay_transition,
            Some(ShellOverlayTransition {
                from: ShellOverlay::Approval,
                to: ShellOverlay::Hidden,
                exit_mode: ShellOverlayExitMode::Exit,
            })
        );
    }

    #[test]
    fn unchanged_overlay_and_selection_projection_do_not_publish_a_transition() {
        let mut unchanged_state = ShellChromeState::new();
        unchanged_state.shell_overlay = ShellOverlay::Help;
        let unchanged = reduce_shell_chrome(unchanged_state, ShellChromeEvent::HelpOverlayShown);
        assert_eq!(unchanged.overlay_transition, None);

        let mut selection_state = ShellChromeState::new();
        selection_state.shell_overlay = ShellOverlay::Sessions;
        let selection = reduce_shell_chrome(
            selection_state,
            ShellChromeEvent::SessionSelectionProjected { index: 1 },
        );
        assert_eq!(selection.state.shell_overlay, ShellOverlay::Sessions);
        assert_eq!(selection.overlay_transition, None);
    }
    #[test]
    fn prompt_focus_policy_covers_dialogs_overlays_and_supersession_loading() {
        assert!(ShellOverlay::Hidden.prompt_input_has_focus(false, false));
        assert!(!ShellOverlay::Hidden.prompt_input_has_focus(true, false));
        assert!(!ShellOverlay::Queue.prompt_input_has_focus(false, false));
        assert!(ShellOverlay::Supersession.prompt_input_has_focus(false, false));
        assert!(!ShellOverlay::Supersession.prompt_input_has_focus(false, true));
        assert!(!ShellOverlay::Supersession.prompt_input_has_focus(true, false));
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
