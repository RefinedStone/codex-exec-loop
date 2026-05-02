// 학습 주석: planning controller 하위 모듈의 공통 event, overlay step, key code, NativeTuiApp 타입을
// 가져옵니다. 이 파일은 key router라 여러 UI state와 app effect entrypoint를 함께 다룹니다.
use super::*;

impl NativeTuiApp {
    pub(crate) fn handle_planning_init_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        // 학습 주석: planning init overlay는 작은 wizard입니다. 같은 키라도 현재 step에 따라
        // queue overlay 이동, mode 선택, detail 선택, editor 입력으로 의미가 달라지므로 step을 먼저 나눕니다.
        match self.planning_init_overlay_ui_state.step() {
            PlanningInitOverlayStep::ExistingWorkspace => match key.code {
                // 학습 주석: 기존 planning workspace가 이미 있으면 Enter는 새 init 대신 queue overlay로 이동합니다.
                KeyCode::Enter if key.modifiers.is_empty() => {
                    self.close_shell_overlay();
                    self.show_queue_overlay();
                }
                // 학습 주석: Q도 queue로 이동하는 빠른 경로입니다. Shift는 uppercase 입력을 위해 허용합니다.
                KeyCode::Char('q') | KeyCode::Char('Q')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.close_shell_overlay();
                    self.show_queue_overlay();
                }
                // 학습 주석: 기존 workspace에서도 direction catalog 관리는 별도 overlay로 열 수 있어야 합니다.
                KeyCode::Char('d') | KeyCode::Char('D')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.close_shell_overlay();
                    self.show_directions_maintenance_overlay();
                }
                // 학습 주석: 나머지 키는 existing workspace 안내 화면에서 소비하지만 상태는 바꾸지 않습니다.
                _ => {}
            },
            PlanningInitOverlayStep::ModeSelection => match key.code {
                // 학습 주석: 위/k는 simple/detail mode selection cursor를 위로 이동합니다.
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_mode_selection(-1)
                }
                // 학습 주석: 아래/j는 mode selection cursor를 아래로 이동합니다.
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_mode_selection(1)
                }
                // 학습 주석: A는 simple mode를 직접 선택합니다. 이 선택은 아직 draft 생성이 아니라 UI cursor 변경입니다.
                KeyCode::Char('a') | KeyCode::Char('A')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .select_mode(PlanningInitModeSelection::Simple)
                }
                // 학습 주석: B는 detail mode를 직접 선택합니다.
                KeyCode::Char('b') | KeyCode::Char('B')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .select_mode(PlanningInitModeSelection::Detail)
                }
                // 학습 주석: Enter는 선택된 mode를 실행합니다. simple은 바로 draft staging으로 가고,
                // detail은 manual/LLM-assisted detail 선택 단계로 한 번 더 들어갑니다.
                KeyCode::Enter if key.modifiers.is_empty() => {
                    match self.planning_init_overlay_ui_state.selected_mode() {
                        PlanningInitModeSelection::Simple => {
                            self.stage_simple_mode_planning_init_draft()
                        }
                        PlanningInitModeSelection::Detail => {
                            self.planning_init_overlay_ui_state.open_detail_selection()
                        }
                    }
                }
                // 학습 주석: 알 수 없는 키는 mode selection 화면 안에서 조용히 소비합니다.
                _ => {}
            },
            PlanningInitOverlayStep::DetailSelection => match key.code {
                // 학습 주석: Backspace/Left는 detail selection에서 mode selection으로 돌아가는 breadcrumb 역할입니다.
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .planning_init_overlay_ui_state
                    .return_from_detail_selection(),
                // 학습 주석: 위/k는 manual/LLM-assisted detail cursor를 위로 이동합니다.
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                    .planning_init_overlay_ui_state
                    .move_detail_selection(-1),
                // 학습 주석: 아래/j는 detail cursor를 아래로 이동합니다.
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                    self.planning_init_overlay_ui_state.move_detail_selection(1)
                }
                // 학습 주석: A는 manual detail authoring을 선택합니다.
                KeyCode::Char('a') | KeyCode::Char('A')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .select_detail(PlanningInitDetailSelection::Manual)
                }
                // 학습 주석: B는 LLM-assisted detail을 선택하지만, 현재는 아직 실행 path가 막혀 있습니다.
                KeyCode::Char('b') | KeyCode::Char('B')
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    self.planning_init_overlay_ui_state
                        .select_detail(PlanningInitDetailSelection::LlmAssisted)
                }
                // 학습 주석: Enter는 detail selection을 실행합니다. manual은 editor를 열고, LLM-assisted는
                // unsupported status를 conversation input 상태로 표시합니다.
                KeyCode::Enter if key.modifiers.is_empty() => {
                    match self.planning_init_overlay_ui_state.selected_detail() {
                        PlanningInitDetailSelection::Manual => self.open_planning_manual_editor(),
                        PlanningInitDetailSelection::LlmAssisted => {
                            // 학습 주석: 미지원 경로도 overlay를 닫지 않고 status_text만 갱신해 사용자가
                            // manual path로 다시 선택할 수 있게 합니다.
                            self.dispatch_conversation_input(
                                ConversationInputEvent::StatusMessageShown {
                                    status_text:
                                        "planning llm-assisted detail mode is not supported yet"
                                            .to_string(),
                                },
                            );
                        }
                    }
                }
                // 학습 주석: 나머지 키는 detail selection 안에서 소비합니다.
                _ => {}
            },
            PlanningInitOverlayStep::SimpleReview => {
                // 학습 주석: SimpleReview는 생성된 simple draft를 승격하거나, detail/manual editor로 확장하거나,
                // auto-follow limit을 편집하는 확인 단계입니다.
                match key.code {
                    // 학습 주석: Enter는 기본 action인 simple draft promote입니다.
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        self.promote_simple_mode_planning_draft()
                    }
                    // 학습 주석: D는 simple review에서 detail selection으로 다시 열어 advanced draft authoring을 선택하게 합니다.
                    KeyCode::Char('d') | KeyCode::Char('D')
                        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        self.planning_init_overlay_ui_state.open_detail_selection();
                        // 학습 주석: step 전환과 함께 status_text를 갱신해 footer/header가 현재 선택 맥락을 설명합니다.
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "planning detail authoring: choose how the advanced draft should open"
                                        .to_string(),
                            },
                        );
                    }
                    // 학습 주석: Ctrl+L은 simple review 안의 max auto turns inline editor를 시작합니다.
                    KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                        self.start_max_auto_turns_edit()
                    }
                    // 학습 주석: Ctrl+E는 simple draft를 manual editor로 열어 사용자가 생성된 내용을 수정하게 합니다.
                    KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => {
                        self.open_simple_mode_planning_editor()
                    }
                    // 학습 주석: Ctrl+P는 Enter와 같은 promote action을 명시적으로 실행하는 shortcut입니다.
                    KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                        self.promote_simple_mode_planning_draft()
                    }
                    // 학습 주석: 다른 키는 SimpleReview 화면에서 별도 effect 없이 소비합니다.
                    _ => {}
                }
            }
            PlanningInitOverlayStep::ManualEditor => {
                // 학습 주석: manual editor는 닫기 확인 상태가 있을 수 있으므로, 일반 editor 입력보다
                // close-confirmation key handling이 먼저 키를 가져가야 합니다.
                if self.handle_planning_manual_editor_close_confirmation_key(key) {
                    return true;
                }
                // 학습 주석: 그 외 key는 공통 draft editor handler로 넘깁니다. save/promote callback만
                // planning init manual editor용 함수로 주입해 editor core를 재사용합니다.
                self.handle_draft_editor_key(
                    key,
                    Self::save_planning_manual_editor,
                    Self::promote_planning_manual_editor,
                );
            }
        }

        true
    }
}
