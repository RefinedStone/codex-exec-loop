// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::time::{Duration, Instant};

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{
    BackgroundMessage, ConversationLifecycleEvent, ConversationRuntimeEvent, ConversationState,
    FollowupOverlayUiEvent, NativeTuiApp, SESSION_PAGE_SIZE, ShellChromeEvent,
};

// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub(super) struct ShellRuntime {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    app: NativeTuiApp,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    should_quit: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    frame_scheduler: TuiFrameScheduler,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    last_live_activity_pulse: Option<u64>,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl ShellRuntime {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn new(app: NativeTuiApp) -> Self {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let now = Instant::now();
        Self {
            app,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            should_quit: false,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            frame_scheduler: TuiFrameScheduler::new(now),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            last_live_activity_pulse: None,
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn app_mut(&mut self) -> &mut NativeTuiApp {
        &mut self.app
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[cfg(test)]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn app(&self) -> &NativeTuiApp {
        &self.app
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn should_quit(&self) -> bool {
        self.should_quit
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[cfg(test)]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn take_redraw_request(&mut self) -> bool {
        self.take_due_draw_request(Instant::now())
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn take_due_draw_request(&mut self, now: Instant) -> bool {
        self.frame_scheduler.take_due(now)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn next_event_poll_timeout(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        now: Instant,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        default_timeout: Duration,
    ) -> Duration {
        self.frame_scheduler.next_poll_timeout(now, default_timeout)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn request_redraw_at(&mut self, now: Instant) {
        self.frame_scheduler.request_immediate(now);
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn poll_background_messages(&mut self) {
        self.poll_background_messages_at(Instant::now());
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn poll_background_messages_at(&mut self, now: Instant) {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut redraw_requested = false;

        // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
        while let Ok(message) = self.app.rx.try_recv() {
            redraw_requested = true;
            // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
            match message {
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                BackgroundMessage::StartupLoaded(result) => {
                    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                    let workspace_directory = match &result {
                        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                        Ok(diagnostics) => Some(diagnostics.workspace_path.clone()),
                        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                        Err(_) => None,
                    };
                    self.app
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .dispatch_shell_chrome(ShellChromeEvent::StartupLoaded {
                            result,
                            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                            session_page_size: SESSION_PAGE_SIZE,
                        });
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if let Some(workspace_directory) = workspace_directory {
                        self.app.sync_draft_shell_workspace(&workspace_directory);
                    }
                    self.app.resolve_startup_submit_queue();
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                BackgroundMessage::SessionsLoaded(result) => {
                    self.app
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .dispatch_shell_chrome(ShellChromeEvent::SessionsLoaded(result));
                    self.app.session_overlay_ui_state.reset();
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                BackgroundMessage::ConversationLoaded(result) => {
                    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                    let loaded_successfully = result.is_ok();
                    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                    let draft_workspace_directory = self.app.current_workspace_directory();
                    self.app.reset_planner_worker_panel_state();
                    self.app.dispatch_conversation_lifecycle(
                        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                        ConversationLifecycleEvent::ConversationLoaded {
                            result,
                            draft_workspace_directory,
                        },
                    );
                    self.app
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .refresh_ready_conversation_planning_runtime_snapshot();
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if loaded_successfully {
                        self.app.surface_resumed_session_planning_context();
                    }
                    self.app
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .dispatch_followup_overlay_ui(FollowupOverlayUiEvent::ContentReset {
                            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                            max_auto_turns: self.app.current_max_auto_turns_label(),
                        });
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                BackgroundMessage::ConversationStream(event) => {
                    self.app.dispatch_conversation_runtime(
                        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                        ConversationRuntimeEvent::StreamUpdated(event),
                    );
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                BackgroundMessage::ConversationRuntimeNotice(notice) => {
                    self.app.dispatch_conversation_runtime(
                        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                        ConversationRuntimeEvent::StreamExecutionObserved { notice },
                    );
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                BackgroundMessage::InvalidateParallelModeSupervisorSnapshot => {
                    self.app.invalidate_parallel_mode_supervisor_snapshot();
                }
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                BackgroundMessage::PostTurnEvaluated {
                    thread_id,
                    queued_from_turn_id,
                    evaluation,
                    planner_worker_panel_state,
                    // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                } => {
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if !self
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .app
                        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                        .should_apply_post_turn_evaluation(&thread_id, &queued_from_turn_id)
                    {
                        // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
                        continue;
                    }
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if let ConversationState::Ready(conversation) = &mut self.app.conversation_state
                    {
                        conversation.record_post_turn_evaluation_applied(&queued_from_turn_id);
                    }
                    self.app.planner_worker_panel_state = planner_worker_panel_state;
                    self.app.invalidate_parallel_mode_supervisor_snapshot();
                    self.app.dispatch_conversation_runtime(
                        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                        ConversationRuntimeEvent::PostTurnEvaluated { evaluation },
                    );
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                BackgroundMessage::GithubReviewPollLoaded(result) => {
                    self.app.record_github_review_poll_result(now, result)
                }
            }
        }

        redraw_requested |= self.app.maybe_start_github_review_poll(now);
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let live_activity_pulse = self.app.live_activity_pulse(now);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if live_activity_pulse != self.last_live_activity_pulse {
            redraw_requested = true;
        }
        self.last_live_activity_pulse = live_activity_pulse;
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if redraw_requested {
            self.request_redraw_at(now);
        } else if live_activity_pulse.is_some() {
            self.frame_scheduler
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .request_delayed(now, Duration::from_millis(250));
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn handle_terminal_event(&mut self, event: Event) {
        self.handle_terminal_event_at(event, Instant::now());
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn handle_terminal_event_at(&mut self, event: Event, now: Instant) {
        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match event {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Event::Key(key) => {
                // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                if key.kind != KeyEventKind::Press {
                    // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                    return;
                }

                self.handle_key_press(key, now);
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Event::Resize(_, _) => self.request_redraw_at(now),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Event::FocusGained => self.frame_scheduler.set_focused(true, now),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            Event::FocusLost => self.frame_scheduler.set_focused(false, now),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            _ => {}
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn handle_key_press(&mut self, key: KeyEvent, now: Instant) {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let Some(confirmed_exit) = self.app.handle_exit_confirmation_key(key) {
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if !confirmed_exit {
                self.request_redraw_at(now);
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if confirmed_exit {
                self.should_quit = true;
            }
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('q') {
            self.should_quit = true;
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if self.app.handle_shell_overlay_key(key) {
            self.request_redraw_at(now);
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if self.app.is_inline_command_palette_active() {
            // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
            match key.code {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                KeyCode::Esc
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if key.modifiers.is_empty() && self.app.dismiss_inline_command_palette() =>
                {
                    self.request_redraw_at(now);
                    // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                    return;
                }
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                KeyCode::Up
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if key.modifiers.is_empty()
                        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                        && self.app.move_inline_command_palette_selection(-1) =>
                {
                    self.request_redraw_at(now);
                    // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                    return;
                }
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                KeyCode::Down
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if key.modifiers.is_empty()
                        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                        && self.app.move_inline_command_palette_selection(1) =>
                {
                    self.request_redraw_at(now);
                    // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                    return;
                }
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                KeyCode::Enter
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if key.modifiers.is_empty()
                        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                        && self.app.accept_inline_command_palette_selection() =>
                {
                    self.request_redraw_at(now);
                    // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                    return;
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                _ => {}
            }
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            self.app.handle_ctrl_c();
            self.request_redraw_at(now);
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match key.code {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_startup_overlay()
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.toggle_session_overlay()
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => self
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .app
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .dispatch_shell_chrome(ShellChromeEvent::StartupCheckRequested),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Char('t') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.open_new_conversation_shell()
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.insert_input_newline()
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.clear_prompt_input()
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                self.app.delete_previous_input_word()
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Backspace => self.app.pop_input_character(),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            KeyCode::Enter => self.app.start_turn_submission(),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            KeyCode::Char(character)
                // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.app.push_input_character(character)
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            _ => return,
        }

        self.request_redraw_at(now);
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
struct TuiFrameScheduler {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    focused: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    next_deadline: Option<Instant>,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl TuiFrameScheduler {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn new(now: Instant) -> Self {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut scheduler = Self {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            focused: true,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            next_deadline: None,
        };
        scheduler.request_immediate(now);
        scheduler
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn request_immediate(&mut self, now: Instant) {
        self.coalesce_deadline(now);
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn request_delayed(&mut self, now: Instant, delay: Duration) {
        self.coalesce_deadline(now + delay);
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn take_due(&mut self, now: Instant) -> bool {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !self.focused {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return false;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(deadline) = self.next_deadline else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return false;
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if deadline > now {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return false;
        }

        self.next_deadline = None;
        true
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn next_poll_timeout(&self, now: Instant, default_timeout: Duration) -> Duration {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !self.focused {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return default_timeout;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(deadline) = self.next_deadline else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return default_timeout;
        };
        default_timeout.min(deadline.saturating_duration_since(now))
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn set_focused(&mut self, focused: bool, now: Instant) {
        self.focused = focused;
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if focused {
            self.request_immediate(now);
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn coalesce_deadline(&mut self, deadline: Instant) {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if self
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .next_deadline
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .is_none_or(|existing_deadline| deadline < existing_deadline)
        {
            self.next_deadline = Some(deadline);
        }
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(test)]
// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[path = "shell_runtime/tests.rs"]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod tests;
