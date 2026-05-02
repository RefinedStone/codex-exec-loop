// `Sender`는 표준 라이브러리 mpsc 채널의 송신 끝이다. 이 파일의 이벤트는
// app-server를 읽는 런타임 스레드에서 만들어지고, TUI 쪽 수신 루프가 하나씩 받아 화면 상태로 줄인다.
use std::sync::mpsc::Sender;

// 승인 검토와 도구 활동은 이미 도메인 타입으로 정규화되어 있다.
// 스트림 이벤트는 JSON 원문이나 adapter 전용 구조를 노출하지 않고, TUI가 바로 소비할 수 있는
// application 계층의 언어로만 경계를 통과하게 한다.
use crate::domain::conversation::{ConversationApprovalReview, ConversationToolActivity};
// 터미널 브리지 attachment 프로필은 "어떤 방식으로 app-server 세션에 붙었는지"를
// 사용자에게 보여 주기 위한 작은 도메인 값이다. 이벤트에 싣는 순간부터 TUI는 launch/reattach를
// 별도 adapter 지식 없이 표현할 수 있다.
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

#[derive(Debug, Clone, PartialEq, Eq)]
// `ConversationStreamEvent`는 한 턴의 app-server 실행 결과를 TUI 런타임으로 흘려보내는
// 공개 스트림 계약이다. outbound adapter는 Codex app-server의 알림, 완료 item, 오류를 이 enum으로
// 변환하고, inbound TUI는 `conversation_runtime`에서 이 enum을 패턴 매칭해 화면 모델과 세션 상태를 갱신한다.
//
// enum으로 계약을 세우면 새 이벤트 종류를 추가할 때 컴파일러가 모든 소비자에게 처리를 요구한다.
// 이 프로젝트에서는 adapter -> application -> domain 방향을 유지해야 하므로, TUI가 app-server JSON 스키마에
// 직접 의존하지 않게 만드는 완충층 역할도 한다.
pub enum ConversationStreamEvent {
    // 터미널 브리지가 새 app-server를 띄웠거나 기존 세션에 다시 붙었다는 관찰 이벤트이다.
    // 메시지 본문과 별개로 먼저 흘려보내면 TUI가 "연결 방식"을 대화 로그의 attachment로 표현할 수 있다.
    AttachmentObserved {
        // 프로필 안에는 launch/reattach 같은 의도와 표시 문자열이 들어 있으므로,
        // 소비자는 adapter 세부 구현을 몰라도 동일한 attachment 렌더링 경로를 사용할 수 있다.
        profile: TerminalBridgeAttachmentProfile,
    },
    // app-server가 대화 thread를 준비한 뒤 알려 주는 식별 정보이다.
    // 새 thread 시작과 기존 thread 재개 모두 이 이벤트를 통해 TUI 세션 모델의 기준 thread_id를 확정한다.
    ThreadPrepared {
        // thread_id는 후속 turn, 재첨부, session detail 저장에서 같은 대화를 가리키는 키이다.
        thread_id: String,
        // title은 app-server가 알고 있는 thread 표시 이름이며 세션 목록과 헤더에 연결된다.
        title: String,
        // cwd는 해당 thread가 어느 작업 디렉터리 문맥에서 실행되는지 보여 주는 값이다.
        cwd: String,
    },
    // 실제 agent turn이 시작되었음을 알린다. ThreadPrepared가 대화 컨테이너라면,
    // TurnStarted는 그 안에서 하나의 사용자 요청/agent 응답 사이클이 열렸다는 신호이다.
    TurnStarted {
        // turn_id는 완료, 중단, 병렬 모드 상태 추적에서 현재 실행 단위를 묶는 식별자이다.
        turn_id: String,
    },
    // app-server나 adapter가 "분석 중", "도구 실행 중" 같은 짧은 상태 문자열을 갱신할 때 쓴다.
    // 최종 답변 텍스트와 분리되어 있으므로 TUI는 상태 라인만 바꾸고 대화 로그는 흔들지 않을 수 있다.
    StatusUpdated {
        // 사람이 읽는 상태 문구이다. 의미 있는 상태 전이는 도메인 타입이 아니라 이 얕은 문자열로 충분하다.
        text: String,
    },
    // agent 메시지의 스트리밍 조각이다. app-server는 긴 응답을 여러 delta로 보내므로
    // TUI는 같은 item_id의 delta를 누적해 사용자가 실시간으로 응답이 자라는 모습을 보게 한다.
    AgentMessageDelta {
        // item_id는 같은 assistant message 조각들을 한 버퍼로 합치기 위한 키이다.
        item_id: String,
        // phase는 reasoning/output 같은 부분 스트림을 구분할 수 있을 때만 채워진다.
        // `Option`인 이유는 모든 app-server 알림이 phase를 제공하지 않기 때문이다.
        phase: Option<String>,
        // delta는 이번 알림에서 새로 추가된 텍스트 조각이며, 완성본이 아니다.
        delta: String,
    },
    // 하나의 agent 메시지가 완료되어 최종 텍스트가 확정되었음을 알린다.
    // Delta와 Completed를 분리하면 TUI는 스트리밍 중 임시 버퍼와 완료된 로그 항목을 다르게 다룰 수 있다.
    AgentMessageCompleted {
        // 완료 이벤트도 item_id를 유지해 기존 delta 버퍼를 최종 텍스트로 교체하거나 닫을 수 있게 한다.
        item_id: String,
        // 완료된 메시지도 phase별로 올 수 있으므로 delta와 같은 선택 필드를 둔다.
        phase: Option<String>,
        // text는 해당 item/phase의 확정된 전체 텍스트이다.
        text: String,
    },
    // shell command, 파일 작업, 기타 도구 호출의 상태를 도메인 값으로 전달한다.
    // adapter는 app-server completed item을 해석해 이 값으로 바꾸고, TUI는 도구 패널이나 대화 로그에 반영한다.
    ToolActivity {
        // activity는 도구 이름, 상태, 표시할 요약을 담는 도메인 구조이다.
        activity: ConversationToolActivity,
    },
    // 승인 요청/검토 상태가 바뀌었음을 알린다. 사용자 승인 흐름은 일반 텍스트와 달리
    // 버튼, 상태 배지, pending/approved/rejected 상태를 요구하므로 별도 이벤트로 유지한다.
    ApprovalReviewUpdated {
        // review는 승인 대상, 결정 상태, 표시 텍스트를 담은 도메인 모델이다.
        review: ConversationApprovalReview,
    },
    // 현재 turn의 스트림이 정상적으로 끝났다는 종료 신호이다.
    // 수신 루프는 이 이벤트를 보고 스트림 읽기를 끝내고, 세션 저장과 후속 작업 가능 상태로 넘어간다.
    TurnCompleted {
        // 완료된 turn_id를 싣기 때문에 늦게 도착한 이벤트를 현재 실행 단위와 대조할 수 있다.
        turn_id: String,
        // planning 파일 변경 목록은 답변 텍스트와 별개의 산출물이다.
        // 병렬/계획 흐름은 이 경로 목록을 사용해 후속 merge, repair, UI 알림을 연결한다.
        changed_planning_file_paths: Vec<String>,
    },
    // 스트림 중 복구하지 못한 오류를 TUI에 전달하는 종료성 이벤트이다.
    // panic 대신 이벤트로 실패를 표현하면 화면은 에러 메시지를 남기고 런타임을 정상적으로 정리할 수 있다.
    Failed {
        // message는 사용자와 개발자가 볼 수 있는 오류 요약이다. 원인 체인은 adapter 쪽 로그에 남긴다.
        message: String,
    },
}

impl ConversationStreamEvent {
    // attachment 이벤트 생성 로직을 함수로 두면 호출자는 enum 필드 이름을 반복하지 않아도 된다.
    // `const fn`인 이유는 단순 포장 생성자라서 런타임 상태가 필요 없고, 테스트/상수 문맥에서도 같은 값을 만들 수 있기 때문이다.
    pub const fn attachment_observed(profile: TerminalBridgeAttachmentProfile) -> Self {
        Self::AttachmentObserved { profile }
    }

    // 새 codex app-server 프로세스를 띄운 경우의 표준 attachment 이벤트이다.
    // outbound adapter와 fake port 테스트가 같은 helper를 쓰면 launch 표시 계약이 한곳에서 유지된다.
    pub const fn codex_app_server_launch_attachment() -> Self {
        Self::attachment_observed(TerminalBridgeAttachmentProfile::codex_app_server_launch())
    }

    // 기존 codex app-server 세션에 재첨부한 경우의 표준 attachment 이벤트이다.
    // launch와 reattach를 같은 enum variant로 보내되 프로필만 다르게 하여 렌더링 경로를 공유한다.
    pub const fn codex_app_server_reattach_attachment() -> Self {
        Self::attachment_observed(TerminalBridgeAttachmentProfile::codex_app_server_reattach())
    }
}

// 공통 송신 helper이다. 여러 adapter 위치에서 attachment 이벤트를 보낼 때
// 같은 생성자와 같은 실패 처리 정책을 쓰도록 작은 함수로 묶었다.
pub(crate) fn emit_attachment_observed(
    // sender는 런타임 작업자에서 TUI 수신 루프로 이어지는 채널의 송신 끝이다.
    event_sender: &Sender<ConversationStreamEvent>,
    // 호출자가 이미 선택한 attachment 프로필이다. helper는 이것을 이벤트로 감싸기만 한다.
    profile: TerminalBridgeAttachmentProfile,
) {
    // 수신자가 이미 닫힌 경우 `send`는 실패한다. 이 이벤트는 관찰/표시용 보조 신호라서
    // 스트림 종료 중 panic을 일으키지 않고 best-effort로 흘려보내는 것이 맞다.
    let _ = event_sender.send(ConversationStreamEvent::attachment_observed(profile));
}

// 새 app-server launch attachment를 보내는 의도 드러난 wrapper이다.
// 호출 지점에서 프로필 생성 세부사항을 읽지 않아도 "launch를 알린다"는 목적이 바로 보인다.
pub(crate) fn emit_codex_app_server_launch_attachment(
    event_sender: &Sender<ConversationStreamEvent>,
) {
    emit_attachment_observed(
        event_sender,
        TerminalBridgeAttachmentProfile::codex_app_server_launch(),
    );
}

// 기존 app-server reattach attachment를 보내는 wrapper이다.
// launch와 같은 송신 helper를 공유하므로 실패 처리와 이벤트 형태가 두 경로에서 갈라지지 않는다.
pub(crate) fn emit_codex_app_server_reattach_attachment(
    event_sender: &Sender<ConversationStreamEvent>,
) {
    emit_attachment_observed(
        event_sender,
        TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
    );
}

#[cfg(test)]
mod tests {
    use super::ConversationStreamEvent;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

    #[test]
    // 이 테스트는 launch/reattach helper가 정확한 attachment profile을 감싼다는 계약을 고정한다.
    // 이벤트 소비자는 variant만 보고 렌더링하고 profile 내용으로 표시 차이를 만들기 때문에, helper가 잘못된 profile을
    // 선택하면 사용자에게 연결 방식이 거꾸로 보일 수 있다.
    fn codex_attachment_helpers_build_expected_profiles() {
        assert_eq!(
            ConversationStreamEvent::codex_app_server_launch_attachment(),
            ConversationStreamEvent::AttachmentObserved {
                profile: TerminalBridgeAttachmentProfile::codex_app_server_launch(),
            }
        );
        assert_eq!(
            ConversationStreamEvent::codex_app_server_reattach_attachment(),
            ConversationStreamEvent::AttachmentObserved {
                profile: TerminalBridgeAttachmentProfile::codex_app_server_reattach(),
            }
        );
    }
}
