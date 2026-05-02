// `Arc`는 여러 런타임 구성 요소가 같은 conversation runtime 구현을 공유하게 해 주는
// 원자적 참조 카운터이다. TUI app runtime, shell entrypoint, 테스트 fixture는 service를 복제해도
// 실제 app-server adapter 인스턴스는 하나의 port 객체로 유지된다.
use std::sync::Arc;
// `Sender`는 런타임 작업자에서 TUI 수신 루프로 이벤트를 보내는 통로이다.
// 이 service는 채널을 해석하지 않고 port에 그대로 넘겨, 스트리밍 세부 처리를 outbound adapter에 맡긴다.
use std::sync::mpsc::Sender;

// `anyhow::Result`는 application service가 adapter 오류를 상위 TUI 흐름에 전달하는 공통 결과 타입이다.
// 여기서는 오류 종류를 새 도메인 enum으로 재포장하지 않고, runtime port의 실패 맥락을 그대로 보존한다.
use anyhow::Result;

// `InteractiveTurnRuntimePort`는 application 계층이 outbound runtime에 기대하는 최소 계약이다.
// 실제 구현은 Codex app-server adapter이지만, TUI와 service는 trait object만 보므로 테스트 fake나 다른 runtime으로
// 교체해도 호출 코드는 바뀌지 않는다.
use crate::application::port::outbound::interactive_turn_runtime_port::InteractiveTurnRuntimePort;
// conversation runtime event는 이전 계층에서 정리한 스트림 계약이다.
// service는 이 이벤트 타입을 알고 있지만 이벤트 payload를 직접 만들거나 줄이지 않는다.
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
// snapshot은 저장된 대화 상태를 읽는 결과이고, control truth는 "중단/실행 제어를 누가 담당하는가"를
// 나타내는 도메인 값이다. 둘 다 TUI가 런타임 구현 세부사항 없이 화면 상태를 구성하는 데 쓰인다.
use crate::domain::conversation::{ConversationRuntimeControlTruth, ConversationSnapshot};

#[derive(Clone)]
// `ConversationService`는 TUI inbound adapter와 outbound interactive runtime port 사이의
// application facade이다. 현재 메서드는 대부분 얇은 위임이지만, 이 얇은 층이 중요한 이유는
// TUI가 `CodexAppServerAdapter` 같은 구체 adapter를 직접 잡지 않게 하고 application 언어로만 대화 기능을
// 호출하게 만들기 때문이다.
//
// 이 구조는 adapter -> application -> domain 방향을 지키는 경계이다. inbound TUI는 service를 호출하고,
// service는 port trait을 호출하며, outbound adapter는 그 port를 구현한다. 나중에 캐싱, 정책 검증, telemetry가 필요하면
// TUI나 app-server adapter를 흔들지 않고 이 service에 추가할 수 있다.
pub struct ConversationService {
    // trait object를 `Arc`에 담아 소유한다. `dyn InteractiveTurnRuntimePort`는 런타임의 실제 타입을
    // 숨기고, `Arc`는 service clone이 많아져도 같은 runtime 제어면을 공유하게 한다.
    interactive_turn_runtime_port: Arc<dyn InteractiveTurnRuntimePort>,
}

impl ConversationService {
    // service 생성자는 runtime port를 주입받는다. shell entrypoint에서는 실제 app-server adapter를 넘기고,
    // TUI 테스트 fixture에서는 fake port를 넘겨 같은 application API를 검증한다.
    pub fn new(interactive_turn_runtime_port: Arc<dyn InteractiveTurnRuntimePort>) -> Self {
        Self {
            interactive_turn_runtime_port,
        }
    }

    // 저장된 conversation snapshot을 읽는 조회 메서드이다. TUI는 thread id만 알고 있고,
    // snapshot 저장 위치나 app-server 세션 디테일을 알 필요가 없으므로 port로 위임한다.
    pub fn load_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        self.interactive_turn_runtime_port
            // port 메서드 이름에는 `conversation`을 포함해 outbound 경계에서의 책임을 더 분명히 한다.
            // service 메서드는 TUI 쪽 호출 문맥에 맞춰 더 짧은 `load_snapshot`으로 노출한다.
            .load_conversation_snapshot(thread_id)
    }

    // runtime control truth는 "중단 버튼, 전체 세션 정지, 실행 상태 판단을 어느 runtime이
    // 실제로 담당하는지"를 알려 주는 값이다. AppRuntime 초기화 시 이 값을 읽어 TUI 제어 모델을 맞춘다.
    pub fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        self.interactive_turn_runtime_port.runtime_control_truth()
    }

    // 사용자가 전체 대화 실행을 멈추려 할 때 호출되는 명령 메서드이다.
    // 실제로 어떤 프로세스/세션을 멈출지는 outbound runtime이 알고 있으므로 service는 명령만 전달한다.
    pub fn request_stop_all_sessions(&self) -> Result<()> {
        self.interactive_turn_runtime_port
            // 실패를 그대로 반환해야 TUI가 "중단 요청 자체가 실패했다"는 상태를 사용자에게 표시할 수 있다.
            .request_stop_all_sessions()
    }

    // 새 thread를 만들며 첫 prompt를 실행하는 스트리밍 진입점이다.
    // TUI의 turn submission runtime은 현재 thread_id가 없을 때 이 메서드를 호출하고, 이후 ThreadPrepared/TurnStarted 같은
    // `ConversationStreamEvent`를 수신해 세션 상태를 채운다.
    pub fn run_new_thread_stream(
        &self,
        // cwd는 app-server가 새 대화를 어느 workspace에서 시작할지 결정하는 실행 문맥이다.
        cwd: &str,
        // prompt는 사용자 입력 원문이다. service는 prompt를 변형하지 않아 adapter가 Codex 프로토콜로 매핑한다.
        prompt: &str,
        // event_sender는 호출자가 만든 수신 루프와 짝을 이룬다. 소유권을 넘기는 이유는
        // runtime worker가 thread 종료까지 이 sender를 들고 스트림 이벤트를 계속 보낼 수 있어야 하기 때문이다.
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.interactive_turn_runtime_port
            // 새 thread 생성, app-server launch/reattach, protocol notification 해석은 모두 outbound 구현 책임이다.
            .run_new_thread_stream(cwd, prompt, event_sender)
    }

    // 이미 준비된 thread에 후속 prompt를 실행하는 스트리밍 진입점이다.
    // 새 thread 흐름과 같은 이벤트 계약을 사용하므로 TUI 수신 루프는 "새 대화"와 "기존 대화"를 거의 같은 방식으로 처리한다.
    pub fn run_turn_stream(
        &self,
        // thread_id는 이전 `ThreadPrepared`나 세션 목록에서 얻은 대화 식별자이다.
        // 이 값으로 outbound runtime은 올바른 app-server conversation에 prompt를 붙인다.
        thread_id: &str,
        // 후속 turn의 사용자 입력이다. service는 validation/prompt rewrite를 하지 않는 얇은 경계이다.
        prompt: &str,
        // 같은 `ConversationStreamEvent` 채널을 사용해 delta, 도구 활동, 승인 상태, 완료/실패를 돌려받는다.
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        self.interactive_turn_runtime_port
            // 기존 thread에서의 turn 실행도 service가 직접 구현하지 않는다.
            // port 경계를 통과시켜 app-server adapter가 프로토콜과 세션 저장 책임을 계속 소유하게 한다.
            .run_turn_stream(thread_id, prompt, event_sender)
    }
}
