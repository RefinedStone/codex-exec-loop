/*
 * 학습 주석: directions overlay controller는 shell key input을 directions maintenance state machine에
 * 연결한다. application service가 만든 summary와 `DirectionsMaintenanceOverlayUiState`가 화면 상태를
 * 보관하고, 이 파일은 사용자의 키 입력을 "editor 열기", "detail doc 생성 확인", "status message 표시"
 * 같은 app-level action으로 바꾸는 inbound adapter 역할을 한다.
 */
use super::*;

impl NativeTuiApp {
    /*
     * 학습 주석: shell_controller는 DirectionsMaintenance overlay가 열려 있을 때 모든 key event를
     * 이 함수로 넘긴다. 반환값 true는 key가 directions overlay context에서 소비됐다는 뜻이며,
     * manual editor step에서도 draft editor handler까지 위임한 뒤 shell 전역 shortcut으로 흘리지 않는다.
     */
    pub(crate) fn handle_directions_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        match self.directions_maintenance_overlay_ui_state.step() {
            DirectionsMaintenanceOverlayStep::Overview => match key.code {
                /*
                 * 학습 주석: Overview의 Enter는 가장 흔한 복구 작업인 queue-idle prompt editor로 바로 들어간다.
                 * prompt는 directions maintenance의 supporting file 중 하나라 manual editor flow를 재사용한다.
                 */
                KeyCode::Enter if key.modifiers.is_empty() => self.open_queue_idle_prompt_editor(),
                /*
                 * 학습 주석: detail doc 생성은 DB direction authority가 parse 가능한 상태에서만 허용한다.
                 * parse error가 남아 있으면 생성할 대상과 파일 경로 판단 자체가 불안정하므로 status line으로
                 * 먼저 authority 수정을 요구한다.
                 */
                KeyCode::Char('d') if key.modifiers.is_empty() => {
                    if self
                        .directions_maintenance_overlay_ui_state
                        .summary()
                        .and_then(|summary| summary.parse_error.as_deref())
                        .is_some()
                    {
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "fix DB direction authority errors before generating detail docs"
                                        .to_string(),
                            },
                        );
                    } else if self
                        .directions_maintenance_overlay_ui_state
                        .actionable_detail_doc_directions()
                        .is_empty()
                    {
                        /*
                         * 학습 주석: actionable list가 비어 있으면 service summary상 모든 direction이 이미
                         * ready 상태다. selection step을 열어 빈 목록을 보여 주지 않고 현재 상태를 설명한다.
                         */
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "every direction already has a healthy detail doc mapping"
                                        .to_string(),
                            },
                        );
                    } else {
                        self.directions_maintenance_overlay_ui_state
                            .open_detail_doc_selection();
                    }
                }
                /*
                 * 학습 주석: `p`는 queue-idle prompt 편집 shortcut이다. prompt도 direction authority를
                 * 기준으로 생성/검증되므로 parse error가 있으면 editor를 열지 않고 같은 recovery channel인
                 * status_text로 막는다.
                 */
                KeyCode::Char('p') if key.modifiers.is_empty() => {
                    if self
                        .directions_maintenance_overlay_ui_state
                        .summary()
                        .and_then(|summary| summary.parse_error.as_deref())
                        .is_some()
                    {
                        self.dispatch_conversation_input(
                            ConversationInputEvent::StatusMessageShown {
                                status_text:
                                    "fix DB direction authority errors before editing queue-idle prompt"
                                        .to_string(),
                            },
                        );
                    } else {
                        self.open_queue_idle_prompt_editor();
                    }
                }
                /*
                 * 학습 주석: reload는 overlay state를 service의 최신 workspace summary로 교체한다.
                 * `present_directions_maintenance_overview`가 summary load, overlay visibility, status dispatch를
                 * 함께 처리하므로 controller는 여기서 동일한 entrypoint를 재사용한다.
                 */
                KeyCode::Char('r') if key.modifiers.is_empty() => self
                    .present_directions_maintenance_overview(
                        "reloaded directions maintenance".to_string(),
                        true,
                    ),
                _ => {}
            },
            DirectionsMaintenanceOverlayStep::DetailDocSelection => match key.code {
                // 학습 주석: selection step의 back/left는 pending 생성 없이 overview로 돌아가는 탐색 동작이다.
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .return_to_overview(),
                // 학습 주석: 위/아래 이동은 actionable detail-doc 목록 안에서만 clamp된다.
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_missing_detail_doc_selection(-1),
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_missing_detail_doc_selection(1),
                /*
                 * 학습 주석: Enter는 곧바로 파일 생성 service를 호출하지 않고 confirm step을 연다.
                 * UI state가 현재 direction id/title을 snapshot으로 잡아 이후 Enter on Yes가 같은 대상을 실행한다.
                 */
                KeyCode::Enter if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .open_detail_doc_confirm(),
                _ => {}
            },
            DirectionsMaintenanceOverlayStep::DetailDocConfirm => match key.code {
                // 학습 주석: confirm에서 back/left는 선택 목록으로 돌아가 대상 direction을 다시 고르게 한다.
                KeyCode::Backspace | KeyCode::Left if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .open_detail_doc_selection(),
                /*
                 * 학습 주석: confirm choice는 Yes/No 두 칸짜리 선택 상태다. 숫자 1/2와 j/k를 함께 받아
                 * keyboard-only 사용자가 renderer의 옵션 순서를 그대로 조작할 수 있게 한다.
                 */
                KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_detail_doc_confirm_choice(-1),
                KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_detail_doc_confirm_choice(1),
                KeyCode::Char('1') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_detail_doc_confirm_choice(-1),
                KeyCode::Char('2') if key.modifiers.is_empty() => self
                    .directions_maintenance_overlay_ui_state
                    .move_detail_doc_confirm_choice(1),
                KeyCode::Enter if key.modifiers.is_empty() => {
                    match self
                        .directions_maintenance_overlay_ui_state
                        .detail_doc_confirm_choice()
                    {
                        DetailDocConfirmChoice::Yes => {
                            /*
                             * 학습 주석: service/editor 호출에는 title이 아니라 direction id만 넘긴다.
                             * pending snapshot이 없으면 confirm state가 불완전한 것이므로 아무 작업도 시작하지 않는다.
                             */
                            let direction_id = self
                                .directions_maintenance_overlay_ui_state
                                .pending_detail_doc_creation()
                                .map(|pending| pending.direction_id().to_string());
                            if let Some(direction_id) = direction_id {
                                self.open_directions_detail_doc_editor(&direction_id);
                            }
                        }
                        DetailDocConfirmChoice::No => {
                            /*
                             * 학습 주석: No는 service를 호출하지 않는 명시적 취소다. overview로 돌아가고,
                             * status line에 directions 파일이 바뀌지 않았음을 남겨 operator가 결과를 확인하게 한다.
                             */
                            self.directions_maintenance_overlay_ui_state
                                .return_to_overview();
                            self.dispatch_conversation_input(
                                ConversationInputEvent::StatusMessageShown {
                                    status_text:
                                        "detail doc creation skipped; directions remain unchanged"
                                            .to_string(),
                                },
                            );
                        }
                    }
                }
                _ => {}
            },
            DirectionsMaintenanceOverlayStep::ManualEditor => {
                /*
                 * 학습 주석: manual editor step은 directions overlay 안에 draft editor를 중첩한 상태다.
                 * 먼저 닫기 확인 키를 처리해 dirty/invalid draft 위험을 보존하고, 일반 편집 키는 공통
                 * draft editor handler에 save/promote 함수를 주입해 처리한다.
                 */
                if self.handle_directions_manual_editor_close_confirmation_key(key) {
                    return true;
                }
                self.handle_draft_editor_key(
                    key,
                    Self::save_directions_manual_editor,
                    Self::promote_directions_manual_editor,
                );
            }
        }

        true
    }
}
