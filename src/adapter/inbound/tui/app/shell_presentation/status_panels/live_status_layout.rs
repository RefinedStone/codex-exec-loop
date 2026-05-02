// 학습 주석: prompt_composer는 prompt 텍스트가 terminal 폭에서 어떻게 줄바꿈되고 cursor가 어디에 놓이는지 계산합니다. inline tail은
// prompt를 status line 아래에 합쳐 그리므로, 같은 계산 함수를 써야 실제 입력 cursor와 표시 line이 어긋나지 않습니다.
use super::super::prompt_composer::{build_prompt_cursor_offset, wrapped_row_count};
// 학습 주석: 이 파일은 NativeTuiApp 전체 상태를 ShellCorePresentationContext로 축약한 뒤, Line 목록과 cursor offset을 함께 반환하는
// layout 계층입니다. INLINE_TAIL_NOTICE_DETAIL_LIMIT는 tail에 붙는 GitHub review 변경 요약의 길이를 제한합니다.
use super::super::{
    INLINE_TAIL_NOTICE_DETAIL_LIMIT, Line, NativeTuiApp, ShellConversationState,
    ShellCorePresentationContext,
};
// 학습 주석: tail_copy는 실제 inline tail에 표시할 status/prompt text를 만듭니다. 이 파일은 copy를 만들지 않고, 만들어진 line들이
// 화면에서 어디에 놓이고 cursor가 어디로 가야 하는지만 계산합니다.
use super::tail_copy::{
    build_inline_tail_lines_with_context, build_inline_tail_prompt_lines_with_context,
};

// 학습 주석: InlineTailView는 inline terminal adapter가 그릴 tail rendering 계획입니다. Clone은 render 준비 단계와 테스트 fixture가
// 같은 view 값을 복제해 비교할 수 있게 해 주는 얕은 복제 계약입니다.
#[derive(Clone)]
pub(crate) struct InlineTailView {
    // 학습 주석: lines는 status, notice, prompt line이 이미 표시 순서대로 합쳐진 결과입니다. rendering layer는 이 순서를 그대로 그립니다.
    pub(crate) lines: Vec<Line<'static>>,
    // 학습 주석: prompt_cursor_offset은 tail 영역의 왼쪽 위를 기준으로 한 cursor 위치입니다. conversation이 준비되지 않았거나
    // prompt cursor 계산이 불가능하면 None으로 두어 terminal cursor를 강제로 옮기지 않습니다.
    pub(crate) prompt_cursor_offset: Option<(u16, u16)>,
    // 학습 주석: render_from_top은 startup 화면처럼 tail을 아래에 붙이지 않고 위에서부터 보여야 하는 특수 상태를 전달합니다.
    pub(crate) render_from_top: bool,
}

// 학습 주석: build_inline_tail_view는 app 상태를 한 번 읽어 inline tail의 텍스트와 cursor 계획을 함께 만듭니다. copy 생성과 cursor
// 계산이 같은 context를 쓰게 해서 startup/blocked/ready 상태 분기가 서로 엇갈리지 않게 합니다.
pub(crate) fn build_inline_tail_view(app: &NativeTuiApp, content_width: u16) -> InlineTailView {
    // 학습 주석: ShellCorePresentationContext는 app의 큰 상태에서 shell presentation에 필요한 값만 빌려온 snapshot입니다. 아래 tail
    // copy와 cursor 계산 모두 이 context를 공유합니다.
    let context = ShellCorePresentationContext::from_app(app);
    // 학습 주석: tail lines는 status line, notices, prompt line을 포함할 수 있습니다. GitHub review 변경 요약은 tail에서 너무 길어지지
    // 않도록 detail limit로 축약해 넘깁니다.
    let lines = build_inline_tail_lines_with_context(
        app,
        &context,
        app.github_review_recent_changes_summary(INLINE_TAIL_NOTICE_DETAIL_LIMIT),
    );
    // 학습 주석: cursor offset은 lines를 만든 뒤 계산합니다. prompt가 tail 끝부분에 붙기 때문에, prompt 앞의 모든 line이 wrap된
    // row 수를 알아야 실제 terminal cursor row를 결정할 수 있습니다.
    let prompt_cursor_offset =
        build_inline_prompt_cursor_offset_for_lines(app, &context, content_width, &lines);

    InlineTailView {
        lines,
        prompt_cursor_offset,
        // 학습 주석: startup screen은 tail을 화면 하단에 붙이는 일반 conversation layout과 다르게 위에서부터 안내를 보여줍니다.
        render_from_top: context.startup_screen_is_active(),
    }
}

// 학습 주석: build_inline_prompt_cursor_offset_for_lines는 prompt cursor의 y좌표를 "prompt 내부 위치"에서 "tail 전체 내부 위치"로
// 변환합니다. prompt 앞에 있는 status/notice line들이 wrapping되면 그 row 수만큼 cursor y를 밀어야 합니다.
fn build_inline_prompt_cursor_offset_for_lines(
    // 학습 주석: app은 prompt 입력 가능 여부를 판단하는 shell_action_availability를 얻기 위해 필요합니다.
    app: &NativeTuiApp,
    // 학습 주석: context는 conversation state와 prompt line copy를 같은 snapshot에서 가져오기 위한 presentation context입니다.
    context: &ShellCorePresentationContext<'_>,
    // 학습 주석: content_width는 terminal 내부 tail 영역의 가로 폭입니다. 줄바꿈 row 수와 prompt cursor x/y 계산의 공통 기준입니다.
    content_width: u16,
    // 학습 주석: tail_lines는 이미 build_inline_tail_lines_with_context가 만든 최종 표시 line입니다. 여기서 prompt 앞 prefix row를 셉니다.
    tail_lines: &[Line<'static>],
) -> Option<(u16, u16)> {
    // 학습 주석: cursor는 Ready conversation에서만 의미가 있습니다. startup/loading/blocked 상태에서는 prompt가 있어도 실제 입력
    // buffer 위치를 신뢰할 수 없으므로 None으로 빠집니다.
    let ShellConversationState::Ready(conversation) = context.conversation_state else {
        return None;
    };
    // 학습 주석: prompt_lines를 다시 만드는 이유는 tail_lines에서 prompt가 시작되는 index를 알기 위해서입니다. 같은 context와
    // availability를 사용하므로 build_inline_tail_lines_with_context가 붙인 prompt suffix와 길이가 맞습니다.
    let prompt_lines =
        build_inline_tail_prompt_lines_with_context(context, app.shell_action_availability());
    // 학습 주석: prompt_start_index는 tail_lines 안에서 prompt suffix가 시작되는 line index입니다. saturating_sub는 prompt_lines가
    // 예상보다 긴 비정상 상태에서도 slice panic을 피하게 합니다.
    let prompt_start_index = tail_lines.len().saturating_sub(prompt_lines.len());
    // 학습 주석: prompt 앞의 모든 line은 content_width에 따라 여러 terminal row로 wrap될 수 있습니다. 단순 line 개수가 아니라
    // wrapped_row_count 합계를 써야 cursor y좌표가 실제 화면 row와 일치합니다.
    let prompt_start_row = tail_lines[..prompt_start_index]
        .iter()
        .map(|line| wrapped_row_count(line.width(), content_width))
        .sum::<usize>()
        // 학습 주석: ratatui cursor API는 u16 좌표를 쓰므로 usize 합계를 u16으로 줄입니다. 너무 큰 경우에는 아래 fallback을 씁니다.
        .try_into()
        // 학습 주석: 비현실적으로 많은 row가 생기면 u16::MAX로 포화시켜 panic 대신 화면 끝쪽으로 제한합니다.
        .unwrap_or(u16::MAX);
    // 학습 주석: build_prompt_cursor_offset은 prompt 내부에서 cursor가 몇 번째 column/row인지 계산합니다. 여기에는 status/notice prefix
    // row가 아직 포함되어 있지 않습니다.
    let (cursor_x, cursor_y) = build_prompt_cursor_offset(conversation, content_width)?;

    // 학습 주석: 최종 y좌표는 prompt prefix row와 prompt 내부 row를 더한 값입니다. saturating_add는 긴 notice가 있어도 u16 overflow를
    // 일으키지 않고 최대값에 머물게 합니다.
    Some((cursor_x, prompt_start_row.saturating_add(cursor_y)))
}
