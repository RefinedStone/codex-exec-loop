// 학습 주석: task intake popup은 application runtime이 만든 task proposal을 사람이 확인하는 얇은
// presentation layer입니다. 그래서 이 파일은 service를 직접 호출하지 않고, controller가 이미 갱신해 둔
// `NativeTuiApp.task_intake_overlay_ui_state`와 공통 TUI 타입만 읽습니다.
use super::super::super::{AkraTheme, Line, NativeTuiApp, Span, TaskIntakeOverlayStep};
// 학습 주석: `TaskIntakeOverlayView`는 popup renderer가 소비하는 DTO입니다. 이 builder가 문자열과
// style을 모두 결정해 두면 실제 drawing code는 layout만 책임질 수 있습니다.
use super::TaskIntakeOverlayView;

// 학습 주석: 이 함수는 task-intake modal의 projection boundary입니다. shell_controller는 prompt 입력,
// preview 생성, commit 결과를 state에 기록하고, 이 함수는 그 상태를 header/prompt/preview/status/key
// line 묶음으로 변환해 popup renderer와 inline inspection renderer가 같은 copy를 보게 합니다.
pub(crate) fn build_task_intake_overlay_view(app: &NativeTuiApp) -> TaskIntakeOverlayView {
    // 학습 주석: overlay state는 입력 buffer, preview proposal, commit result, error, current step을 함께
    // 보관합니다. 여기서는 immutable borrow만 사용해 rendering 중 application state mutation이 섞이지 않게 합니다.
    let state = &app.task_intake_overlay_ui_state;
    // 학습 주석: header는 이 modal이 planning runtime queue에 넣을 "ready task" 초안을 다룬다는 컨텍스트를
    // 고정 copy로 제공합니다. 실제 task 내용은 아래 prompt/preview 영역에서 state 기반으로 달라집니다.
    let header_lines = vec![
        // 학습 주석: title_line은 popup chrome의 제목 스타일을 통일하고, subtitle은 이 modal이 runtime
        // planning 경로에 속한다는 위치 정보를 붙입니다.
        AkraTheme::title_line("Task Intake", " / runtime planning"),
        // 학습 주석: 이 안내 문구는 사용자가 지금 작성하는 값이 자유 대화가 아니라 accepted planning queue로
        // 들어갈 단일 task draft의 원천 prompt라는 점을 분명히 합니다.
        Line::from("Draft one ready task for the accepted planning queue."),
    ];
    // 학습 주석: prompt_lines는 raw prompt buffer를 그대로 보여 주는 입력 echo입니다. 빈 buffer에도
    // "prompt:" label을 남겨 두어 modal이 열린 직후와 Ctrl+u clear 직후의 화면 구조가 흔들리지 않게 합니다.
    let prompt_lines = if state.prompt_buffer().trim().is_empty() {
        vec![Line::from("prompt: ")]
    } else {
        state
            // 학습 주석: prompt_buffer는 `:task` 인자나 modal typing으로 만들어진 원문입니다. preview 요청은
            // trim된 값을 쓰지만, 화면 echo는 사용자가 작성한 줄 구성을 보존합니다.
            .prompt_buffer()
            // 학습 주석: 여러 줄 prompt를 각 Line으로 나누면 renderer가 별도 wrapping 로직 없이 paragraph처럼
            // 쌓을 수 있고, 각 줄 앞에 같은 label을 붙여 prompt 영역임을 유지합니다.
            .lines()
            // 학습 주석: `Line::from(format!(...))` 단계에서 domain prompt를 ratatui text 타입으로 바꿉니다.
            // mapping을 여기서 끝내면 drawing layer는 `Line`만 다룹니다.
            .map(|line| Line::from(format!("prompt: {line}")))
            // 학습 주석: collect는 iterator projection을 renderer가 반복해서 읽을 수 있는 owned Vec으로 고정합니다.
            // view DTO가 app state borrow에 매달리지 않도록 하는 작은 ownership 경계입니다.
            .collect()
    };
    // 학습 주석: preview_lines는 prepare_task_intake 성공 전후를 가르는 핵심 영역입니다. proposal이 있으면
    // service가 만든 task preview를 보여 주고, 없으면 Enter로 preview를 생성해야 한다는 빈 상태 copy를 보여 줍니다.
    let preview_lines = state
        // 학습 주석: proposal은 runtime service가 prompt, workspace, queue context를 검증해 만든 draft입니다.
        // None이면 아직 preview 단계에 도달하지 않았거나 error 후 다시 입력 중인 상태입니다.
        .proposal()
        // 학습 주석: proposal이 있을 때만 domain preview copy를 TUI Line으로 바꿉니다. 이 branch는
        // `TaskIntakeOverlayStep::Preview`에서 사용자가 commit 여부를 판단하는 화면 근거가 됩니다.
        .map(|proposal| {
            // 학습 주석: `preview_lines`는 service가 이미 "title, direction, priority..." 같은 task 요약으로 만든
            // 문자열들입니다. presentation은 순서와 내용을 재해석하지 않고 ratatui Line으로만 옮깁니다.
            let mut lines = proposal
                .preview_lines
                // 학습 주석: iter/clone은 proposal을 그대로 state에 남긴 채, view DTO가 독립적으로 소유할 Line을
                // 만들기 위한 변환입니다. commit 단계는 같은 proposal 값을 다시 읽어야 합니다.
                .iter()
                .map(|line| Line::from(line.clone()))
                .collect::<Vec<_>>();
            // 학습 주석: task_id는 preview body의 일부가 아니라 commit 대상 식별자입니다. 별도 줄로 붙여 주면
            // 사용자가 Y를 누를 때 어떤 draft id가 planning ledger/queue에 들어갈지 확인할 수 있습니다.
            lines.push(Line::from(format!(
                "task_id: {}",
                proposal.draft.task.id.trim()
            )));
            lines
        })
        // 학습 주석: preview가 없는 상태의 copy는 action hint 역할을 합니다. key line에도 Enter가 있지만,
        // preview 영역 자체가 비어 보이지 않도록 여기서 placeholder를 둡니다.
        .unwrap_or_else(|| vec![Line::from("Preview appears after Enter.")]);

    // 학습 주석: status_lines는 error, accepted result, current step 중 하나만 보여 줍니다. 이 우선순위는
    // 사용자가 방금 한 action의 결과를 먼저 보게 하려는 UI 계약입니다.
    let mut status_lines = Vec::new();
    // 학습 주석: error가 있으면 preview/commit 실패 원인을 danger style로 최상단 표시합니다. controller가
    // validation 또는 runtime error를 state에 기록하면 이 branch가 즉시 renderer copy로 전파합니다.
    if let Some(error) = state.error() {
        status_lines.push(Line::from(vec![
            // 학습 주석: label만 danger style을 적용해 상태의 심각도를 빠르게 읽게 하고, 실제 error text는
            // 원문을 보존해 service/debugging 문맥을 잃지 않게 합니다.
            Span::styled("error: ", AkraTheme::danger()),
            Span::raw(error.to_string()),
        ]));
    } else if let Some(result) = state.commit_result() {
        // 학습 주석: commit_result는 accepted task id와 planning revision을 함께 표시합니다. revision까지
        // 보여 주면 사용자는 queue/ledger mutation이 실제로 반영된 시점을 확인할 수 있습니다.
        status_lines.push(Line::from(format!(
            "accepted: {} / revision {}",
            result.committed_task_id, result.committed_planning_revision
        )));
    } else {
        // 학습 주석: error와 commit result가 없을 때는 current step을 상태 copy로 바꿉니다. Prompt는 아직
        // raw input editing이고, Preview는 service가 만든 proposal을 commit할 준비가 된 상태입니다.
        status_lines.push(Line::from(match state.step() {
            TaskIntakeOverlayStep::Prompt => "status: editing prompt",
            TaskIntakeOverlayStep::Preview => "status: preview ready",
        }));
    }

    // 학습 주석: 반환 DTO는 popup renderer와 inline inspection renderer 사이의 공통 계약입니다. 모든 line
    // group을 여기서 완성해 두면 두 renderer가 상태 해석을 중복하지 않습니다.
    TaskIntakeOverlayView {
        header_lines,
        prompt_lines,
        preview_lines,
        status_lines,
        // 학습 주석: key_lines는 current step과 함께 바뀝니다. Prompt에서는 입력 편집 명령을, Preview에서는
        // commit/edit/cancel 명령을 노출해 controller의 key handling state machine과 같은 경계를 유지합니다.
        key_lines: build_task_intake_key_lines(state.step()),
    }
}

// 학습 주석: key line builder는 modal state machine을 사용자가 누를 수 있는 action copy로 변환합니다.
// rendering view 생성 함수에서 분리해 둔 이유는 step별 단축키 계약을 작고 독립적으로 읽게 하기 위해서입니다.
fn build_task_intake_key_lines(step: TaskIntakeOverlayStep) -> Vec<Line<'static>> {
    // 학습 주석: `TaskIntakeOverlayStep`은 Prompt와 Preview 두 단계만 갖습니다. 각 branch의 문구는
    // shell_controller.handle_task_intake_overlay_key가 실제로 처리하는 키와 맞아야 합니다.
    match step {
        // 학습 주석: Prompt 단계는 아직 proposal이 없으므로 Enter는 service preview 생성으로, Ctrl+u는
        // prompt buffer clear로, Esc는 modal cancel로 이어집니다.
        TaskIntakeOverlayStep::Prompt => {
            vec![AkraTheme::key_line(
                "Enter preview  |  Ctrl+u clear  |  Esc cancel",
            )]
        }
        // 학습 주석: Preview 단계는 proposal이 준비된 상태입니다. Y는 commit_task_intake로 ledger/queue를
        // 갱신하고, E는 prompt 편집으로 돌아가며, N/Esc는 proposal을 버리고 닫는 선택지입니다.
        TaskIntakeOverlayStep::Preview => {
            vec![AkraTheme::key_line("Y commit  |  E edit  |  N/Esc cancel")]
        }
    }
}
