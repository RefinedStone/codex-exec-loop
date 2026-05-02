// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[path = "turn_submission_runtime/post_turn_execution.rs"]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod post_turn_execution;
// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[path = "turn_submission_runtime/stream_execution.rs"]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod stream_execution;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::planning::{
    BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT, PlanningTaskHandoff,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::parallel_mode::ParallelModeSlotLeaseRequest;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use post_turn_execution::PostTurnEvaluationRequest;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use stream_execution::PreparedTurnStreamRequest;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::planner_debug_preview::build_debug_preview_lines;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{
    AutoFollowupSubmitContext, ConversationInputEvent, ConversationRuntimeEffect,
    ConversationRuntimeEvent, ConversationState, InlineShellCommandInput, NativeTuiApp,
    PromptOrigin, ShellActionAvailability, ShellChromeEvent,
};

// 학습 주석: `const`는 컴파일 시점에 값이 고정되는 이름으로, 런타임에 바뀌지 않는 설정값을 표현합니다.
const AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES: usize = 32;

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl NativeTuiApp {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn parallel_mode_turn_service(&self) -> ParallelModeTurnService {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeTurnService::new(self.parallel_mode_service.clone())
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn start_turn_submission(&mut self) {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let inline_command = match &self.conversation_state {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationState::Ready(conversation) => {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                InlineShellCommandInput::parse(&conversation.input_buffer)
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            _ => None,
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let Some(command) = inline_command {
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if self.should_queue_task_intake_command(&command) {
                self.queue_task_intake_command_until_idle(command);
                // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                return;
            }
            self.execute_inline_shell_command_input(command);
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let operator_prompt = match &self.conversation_state {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationState::Ready(conversation) if conversation.can_accept_manual_prompt() => {
                conversation.input_buffer.clone()
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            _ => return,
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if operator_prompt.trim().is_empty() {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        self.submit_manual_prompt_from_text(operator_prompt);
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn execute_conversation_runtime_effect(
        &mut self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        effect: ConversationRuntimeEffect,
    ) {
        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match effect {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            } => self.execute_start_stream(self.build_turn_stream_request(
                workspace_directory,
                thread_id,
                prompt,
            )),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ConversationRuntimeEffect::EvaluateAutoFollowup {
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            } => self.execute_post_turn_evaluation(PostTurnEvaluationRequest {
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
            }),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ConversationRuntimeEffect::QueueAutoPrompt {
                prompt,
                queued_from_turn_id,
                mode_label,
                transcript_text,
                handoff_task,
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            } => {
                // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                let debug_detail = self.build_auto_follow_transcript_debug_detail(&transcript_text);
                // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                let _ = self.submit_prompt(
                    prompt,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    PromptOrigin::AutoFollow(Box::new(AutoFollowupSubmitContext {
                        queued_from_turn_id,
                        mode_label,
                        transcript_text,
                        debug_detail,
                        handoff_task,
                    })),
                );
            }
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn build_turn_stream_request(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_directory: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        thread_id: Option<String>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        prompt: String,
    ) -> PreparedTurnStreamRequest {
        PreparedTurnStreamRequest {
            workspace_directory,
            thread_id,
            prompt,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            slot_lease_request: self.build_parallel_mode_slot_lease_request(),
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn build_parallel_mode_slot_lease_request(&self) -> Option<ParallelModeSlotLeaseRequest> {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !self.parallel_mode_enabled() {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return None;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let ConversationState::Ready(conversation) = &self.conversation_state else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return None;
        };
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let handoff_task = conversation.last_planning_task_handoff()?;

        Some(parallel_mode_slot_lease_request(handoff_task))
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn resolve_startup_submit_queue(&mut self) {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let (startup_submit_armed, operator_prompt) = match &self.conversation_state {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationState::Ready(conversation) => (
                conversation.startup_submit_armed,
                conversation.input_buffer.clone(),
            ),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !startup_submit_armed {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match self.shell_action_availability() {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ShellActionAvailability::Ready if operator_prompt.trim().is_empty() => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    status_text: None,
                });
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ShellActionAvailability::Ready => {
                self.submit_manual_prompt_from_text(operator_prompt);
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ShellActionAvailability::Pending => {}
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    status_text: Some(format!(
                        "{}; queued prompt kept in buffer",
                        self.submission_blocked_status(PromptOrigin::Manual)
                    )),
                });
            }
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn submit_manual_prompt_from_text(&mut self, operator_prompt: String) {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let transcript_text = operator_prompt.trim().to_string();
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if transcript_text.is_empty() {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !self.ensure_manual_planning_workspace(&transcript_text) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let prompt = match &self.conversation_state {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationState::Ready(conversation) => self
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .planning
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .runtime
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .build_manual_prompt(&transcript_text, &conversation.planning_runtime_snapshot),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationState::Loading | ConversationState::Failed(_) => None,
        };
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(prompt) = prompt else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return;
        };

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = self.submit_prompt_with_transcript(prompt, transcript_text, PromptOrigin::Manual);
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) -> bool {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let transcript_text = match &prompt_origin {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            PromptOrigin::Manual => prompt.trim().to_string(),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            PromptOrigin::AutoFollow(context) => context.transcript_text.clone(),
        };
        self.submit_prompt_with_transcript(prompt, transcript_text, prompt_origin)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub(super) fn submit_prompt_with_transcript(
        &mut self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        prompt: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        transcript_text: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        prompt_origin: PromptOrigin,
    ) -> bool {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if matches!(prompt_origin, PromptOrigin::Manual)
            && matches!(
                self.shell_action_availability(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ShellActionAvailability::Pending
            )
        {
            self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                status_text: "prompt queued until startup checks finish".to_string(),
            });
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return false;
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !self.shell_action_availability().allows_actions() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                status_text: self.submission_blocked_status(prompt_origin),
            });
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return false;
        }

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            transcript_text,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            origin: prompt_origin,
        })
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn ensure_manual_planning_workspace(&mut self, manual_prompt: &str) -> bool {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let workspace_directory = self.planning_workspace_directory();
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if snapshot.workspace_present() {
            self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                &workspace_directory,
            );
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return true;
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if manual_prompt.trim().is_empty() {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return false;
        }

        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match self
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .planning
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .workspace
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .stage_simple_mode_draft(&workspace_directory)
        {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok(stage_result) => {
                // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                let draft_name = stage_result.draft_name.clone();
                // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
                match self
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .planning
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .workspace
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .promote_staged_draft(&workspace_directory, &draft_name)
                {
                    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                    Ok(result) => {
                        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
                            &workspace_directory,
                        );
                        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                        if result.promoted_file_count > 0 {
                            true
                        } else {
                            self.planning_init_overlay_ui_state
                                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                                .open_simple_review(stage_result);
                            self.planning_draft_editor_ui_state.reset();
                            self.dispatch_shell_chrome(ShellChromeEvent::PlanningInitOverlayShown);
                            self.dispatch_conversation_input(
                                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                                ConversationInputEvent::StatusMessageShown {
                                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                                    status_text: format!(
                                        "planning bootstrap promote blocked / draft: {} / validation needs attention",
                                        result.draft_name
                                    ),
                                },
                            );
                            false
                        }
                    }
                    // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                    Err(error) => {
                        self.dispatch_conversation_input(
                            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                            ConversationInputEvent::StatusMessageShown {
                                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                                status_text: format!("planning bootstrap promote failed: {error}"),
                            },
                        );
                        false
                    }
                }
            }
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Err(error) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    status_text: format!("planning bootstrap failed: {error}"),
                });
                false
            }
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn build_auto_follow_transcript_debug_detail(&self, transcript_text: &str) -> Option<String> {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !self.planner_shows_debug_details()
            // 학습 주석: 클로저는 이름 없는 작은 함수로, 주변 값을 캡처해 iterator나 콜백에 전달할 때 자주 사용합니다.
            || transcript_text != BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT
        {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return None;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let planner = &self.planner_worker_panel_state;
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let operation_label = planner.last_operation_label.as_deref().unwrap_or("unknown");
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let prompt = planner.last_prompt.as_deref();
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let response = planner.last_response.as_deref();
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let summary = planner.last_summary.as_deref();

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if prompt.is_none() && response.is_none() && summary.is_none() {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return None;
        }

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut lines = vec![format!(
            "planner temp session: {operation_label} / {}",
            planner.status.label()
        )];
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let Some(summary) = summary.filter(|summary: &&str| !summary.trim().is_empty()) {
            lines.push(format!("planner summary: {summary}"));
        }
        append_debug_detail_preview_block(&mut lines, "planner prompt:", prompt);
        append_debug_detail_preview_block(&mut lines, "planner response:", response);

        Some(lines.join("\n"))
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn append_debug_detail_preview_block(lines: &mut Vec<String>, label: &str, block: Option<&str>) {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let Some(block) = block.filter(|block| !block.trim().is_empty()) else {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return;
    };

    lines.push(label.to_string());
    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for line in build_debug_preview_lines(block, AUTO_FOLLOW_TRANSCRIPT_DEBUG_MAX_BLOCK_LINES) {
        lines.push(format!("  {line}"));
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn parallel_mode_slot_lease_request(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    handoff_task: &PlanningTaskHandoff,
) -> ParallelModeSlotLeaseRequest {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let task_id = handoff_task.task_id.trim();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let task_title = handoff_task.task_title.trim();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let common_slug = sanitize_parallel_mode_identifier(task_id)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .or_else(|| sanitize_parallel_mode_identifier(task_title));
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let task_slug = common_slug.clone().unwrap_or_else(|| "task".to_string());
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let agent_slug = common_slug.unwrap_or_else(|| "agent".to_string());

    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    ParallelModeSlotLeaseRequest::new(
        task_id,
        task_title,
        format!("agent-{agent_slug}"),
        task_slug,
    )
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn sanitize_parallel_mode_identifier(input: &str) -> Option<String> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut slug = String::new();
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut previous_was_dash = false;

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    for character in input.chars() {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_was_dash = false;
            // 학습 주석: `break`와 `continue`는 반복문의 진행을 직접 제어할 때 사용합니다.
            continue;
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !previous_was_dash && !slug.is_empty() {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    // 학습 주석: 반복문은 컬렉션이나 조건을 기준으로 같은 처리를 여러 번 수행할 때 사용합니다.
    while slug.ends_with('-') {
        slug.pop();
    }

    (!slug.is_empty()).then_some(slug)
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(test)]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod tests {
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use super::*;

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn parallel_mode_slot_lease_request_uses_handoff_task_identity() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let request = parallel_mode_slot_lease_request(&PlanningTaskHandoff {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            task_id: "task-supersession-runtime".to_string(),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            task_title: "Wire runtime into slot lease lifecycle".to_string(),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            direction_id: "supersession-git-worktree-pool".to_string(),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            combined_priority: 96,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            updated_at: "2026-04-17T05:20:00Z".to_string(),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            status_label: "ready".to_string(),
        });

        assert_eq!(request.task_id, "task-supersession-runtime");
        assert_eq!(request.task_title, "Wire runtime into slot lease lifecycle");
        assert_eq!(request.agent_id, "agent-task-supersession-runtime");
        assert_eq!(request.task_slug, "task-supersession-runtime");
    }
}
