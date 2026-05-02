/*
 * 학습 주석: conversation_runtime.rs는 TUI 대화 화면의 "순수 상태 전이"를 담당합니다.
 * 이 파일은 app-server를 직접 호출하지 않고, ConversationRuntimeEvent를 받아 ConversationViewModel을 갱신한 뒤
 * ConversationRuntimeEffect를 반환합니다. 실제 I/O는 turn_submission_runtime.rs가 effect를 실행합니다.
 *
 * 이 구조를 reducer/effect로 나눈 이유:
 * - reducer는 테스트하기 쉽고, 입력 event와 이전 state만으로 다음 state/effect를 결정합니다.
 * - app-server stream, post-turn planning evaluation 같은 부수효과는 UI 상태 전이와 분리되어 race를 줄입니다.
 * - auto follow-up은 turn 종료 후 다시 SubmitPrompt로 돌아오는 순환 구조라, 상태 기록과 effect 발행을 명확히 나눠야 합니다.
 */
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::PromptOrigin;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::conversation_model::{
    AutoFollowupSkipReason, ConversationViewModel, PlanningRepairState,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::adapter::inbound::tui::conversation_text::{
    approval_review_manual_client_action_notice, attachment_runtime_notice,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningTaskHandoff};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone)]
// 학습 주석: `enum`은 가능한 상태나 명령을 정해진 선택지로 제한해 패턴 매칭으로 안전하게 처리하게 해줍니다.
pub(super) enum ConversationRuntimeEvent {
    /*
     * 학습 주석: RuntimeEvent는 "이미 일어난 일"을 reducer에 전달하는 입력입니다.
     * SubmitPrompt는 사용자가 보낸 prompt나 auto-follow prompt를 상태에 반영하고,
     * StreamUpdated는 app-server notification을 transcript/status로 줄이며,
     * PostTurnEvaluated는 planning runtime이 다음 자동 prompt를 만들지 판단한 결과를 다시 상태로 넣습니다.
     */
    SubmitPrompt {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        prompt: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        transcript_text: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        origin: PromptOrigin,
    },
    StreamUpdated(ConversationStreamEvent),
    StreamExecutionObserved {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        notice: String,
    },
    PostTurnEvaluated {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        evaluation: Box<ConversationPostTurnEvaluation>,
    },
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `enum`은 가능한 상태나 명령을 정해진 선택지로 제한해 패턴 매칭으로 안전하게 처리하게 해줍니다.
pub(super) enum ConversationRuntimeEffect {
    /*
     * 학습 주석: RuntimeEffect는 reducer가 직접 실행하지 않는 외부 작업 요청입니다.
     * StartStream은 app-server turn stream을 시작하고, EvaluateAutoFollowup은 turn 종료 후 planning snapshot/reconciliation을 수행하며,
     * QueueAutoPrompt는 그 결과 생성된 내부 prompt를 다시 submit 흐름으로 넣습니다.
     */
    StartStream {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_directory: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        thread_id: Option<String>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        prompt: String,
    },
    EvaluateAutoFollowup {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_directory: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        queued_from_turn_id: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        changed_planning_file_paths: Vec<String>,
    },
    QueueAutoPrompt {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        prompt: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        queued_from_turn_id: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        mode_label: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        transcript_text: String,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        handoff_task: Option<PlanningTaskHandoff>,
    },
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub(super) struct ConversationPostTurnEvaluation {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub planning_runtime_snapshot: PlanningRuntimeSnapshot,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub planning_repair_state: Option<PlanningRepairState>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub runtime_notices: Vec<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub action: ConversationPostTurnAction,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub(super) struct QueuedAutoPrompt {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub prompt: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub queued_from_turn_id: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub mode_label: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub transcript_text: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub handoff_task: Option<PlanningTaskHandoff>,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone)]
// 학습 주석: `enum`은 가능한 상태나 명령을 정해진 선택지로 제한해 패턴 매칭으로 안전하게 처리하게 해줍니다.
pub(super) enum ConversationPostTurnAction {
    QueueAutoPrompt(Box<QueuedAutoPrompt>),
    SkipAutoFollowup { reason: AutoFollowupSkipReason },
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub(super) struct ConversationRuntimeReduction {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub state: ConversationViewModel,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub effects: Vec<ConversationRuntimeEffect>,
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn reduce_conversation_runtime(
    mut state: ConversationViewModel,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    event: ConversationRuntimeEvent,
) -> ConversationRuntimeReduction {
    /*
     * 학습 주석: reduce_conversation_runtime은 TUI conversation runtime의 중심 함수입니다.
     * 핵심 패턴은 "state를 먼저 안전하게 갱신하고, 필요한 외부 작업을 effects에 쌓아서 호출자가 실행하게 한다"입니다.
     * 이 덕분에 stream thread에서 도착한 event와 keyboard event가 섞여도, 한 번에 하나의 event만 모델을 바꿉니다.
     */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let mut effects = Vec::new();

    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match event {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            transcript_text,
            origin,
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        } => {
            /*
             * 학습 주석: SubmitPrompt 분기는 manual prompt와 auto-follow prompt를 같은 turn stream 시작 규칙으로 통합합니다.
             * manual prompt는 repair/auto-follow state를 초기화하고, auto-follow prompt는 어떤 queue head에서 이어졌는지 기록해
             * 같은 queue head가 반복 제출되는 것을 post-turn policy가 막을 수 있게 합니다.
             */
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let prompt = prompt.trim().to_string();
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if prompt.is_empty() || !state.can_accept_runtime_prompt() {
                // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                return ConversationRuntimeReduction { state, effects };
            }
            // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
            if matches!(origin, PromptOrigin::Manual) && !state.can_accept_manual_prompt() {
                // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                return ConversationRuntimeReduction { state, effects };
            }

            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let thread_id = state.has_active_thread().then(|| state.thread_id.clone());
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let workspace_directory = state.planning_workspace_directory().to_string();
            // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
            match &origin {
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                PromptOrigin::Manual => {
                    state.planning_repair_state = None;
                    state.auto_follow_state.reset_for_manual_turn();
                    state.clear_auto_followup_skip();
                    state.clear_last_planning_task_handoff();
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                PromptOrigin::AutoFollow(context) => {
                    state.record_auto_followup_submission(
                        &context.queued_from_turn_id,
                        context.handoff_task.as_ref(),
                    );
                }
            }
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let auto_follow_progress = format!(
                "{}/{}",
                state
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .auto_follow_state
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .active_turn_index()
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .unwrap_or_else(|| state.auto_follow_state.next_auto_turn_index()),
                state.auto_follow_state.max_auto_turns_label()
            );
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let transcript_message = match &origin {
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                PromptOrigin::AutoFollow(context) => {
                    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                    let mut message = ConversationMessage::new(
                        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                        ConversationMessageKind::User,
                        context.transcript_text.clone(),
                        None,
                        None,
                    )
                    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                    .with_display_label("Auto Follow-up");
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if let Some(detail) = context.debug_detail.as_deref() {
                        message = message.with_debug_detail(detail);
                    }
                    message
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                _ => ConversationMessage::new(
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    ConversationMessageKind::User,
                    transcript_text,
                    None,
                    None,
                ),
            };
            state.record_submitted_prompt(
                transcript_message,
                workspace_directory.clone(),
                matches!(origin, PromptOrigin::Manual),
            );
            state.status_text = match origin {
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                PromptOrigin::Manual => "starting turn".to_string(),
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                PromptOrigin::AutoFollow(context) => format!(
                    "auto follow-up submitted / turn {auto_follow_progress} / mode: {}",
                    context.mode_label
                ),
            };
            effects.push(ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
            });
        }
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ConversationRuntimeEvent::StreamUpdated(event) => match event {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationStreamEvent::AttachmentObserved { profile } => {
                state.extend_runtime_notices([attachment_runtime_notice(profile)]);
            }
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ConversationStreamEvent::ThreadPrepared {
                thread_id,
                title,
                cwd,
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            } => {
                state.record_thread_prepared(thread_id, title, cwd);
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationStreamEvent::TurnStarted { turn_id } => {
                state.record_turn_started(turn_id);
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationStreamEvent::StatusUpdated { text } => {
                state.status_text = text;
            }
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ConversationStreamEvent::AgentMessageDelta {
                item_id,
                phase,
                delta,
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            } => {
                state.push_live_agent_delta(item_id, phase, delta);
            }
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ConversationStreamEvent::AgentMessageCompleted {
                item_id,
                phase,
                text,
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            } => {
                state.complete_live_agent_message(item_id, phase, text);
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationStreamEvent::ToolActivity { activity } => {
                state.turn_activity.register_tool_activity(&activity);
                state.buffer_tool_message(activity.text);
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationStreamEvent::ApprovalReviewUpdated { review } => {
                // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                if let Some(notice) = approval_review_manual_client_action_notice(
                    &review,
                    state.turn_control_truth().approval,
                ) {
                    state.extend_runtime_notices([notice]);
                }
                state.update_approval_review(review);
            }
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ConversationStreamEvent::TurnCompleted {
                turn_id,
                changed_planning_file_paths,
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            } => {
                // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                let workspace_directory = state.finish_turn(&turn_id, &changed_planning_file_paths);
                state.begin_auto_followup_evaluation();
                effects.push(ConversationRuntimeEffect::EvaluateAutoFollowup {
                    workspace_directory,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    queued_from_turn_id: turn_id,
                    changed_planning_file_paths,
                });
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            ConversationStreamEvent::Failed { message } => {
                state.fail_turn(message);
            }
        },
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ConversationRuntimeEvent::StreamExecutionObserved { notice } => {
            state.extend_runtime_notices([notice]);
        }
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        ConversationRuntimeEvent::PostTurnEvaluated { evaluation } => {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let evaluation = *evaluation;
            state.replace_planning_runtime_snapshot(evaluation.planning_runtime_snapshot);
            state.planning_repair_state = evaluation.planning_repair_state;
            state.extend_runtime_notices(evaluation.runtime_notices);
            // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
            match evaluation.action {
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                ConversationPostTurnAction::QueueAutoPrompt(queued_prompt) => {
                    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
                    let QueuedAutoPrompt {
                        prompt,
                        queued_from_turn_id,
                        mode_label,
                        transcript_text,
                        handoff_task,
                    } = *queued_prompt;
                    state.clear_auto_followup_skip();
                    state.record_auto_followup_queue(&queued_from_turn_id);
                    state.status_text =
                        format!("turn completed / queued auto follow-up with mode {mode_label}");
                    state.append_status_message(state.status_text.clone());
                    effects.push(ConversationRuntimeEffect::QueueAutoPrompt {
                        prompt,
                        queued_from_turn_id,
                        mode_label,
                        transcript_text,
                        handoff_task,
                    });
                }
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
                ConversationPostTurnAction::SkipAutoFollowup { reason } => {
                    state.record_auto_followup_skip(reason);
                    state.status_text = reason.runtime_status(&state.auto_follow_state);
                    state.append_status_message(state.status_text.clone());
                }
            }
        }
    }

    ConversationRuntimeReduction { state, effects }
}
