// interactive turn runtime은 별도 worker/thread에서 stream event를 밀어 넣는다.
// `Sender`를 port 메서드 인자로 받으면 TUI 쪽 수신 루프가 만든 채널에 outbound adapter가 직접 이벤트를 보낼 수 있다.
use std::sync::mpsc::Sender;

// app-server 실행, snapshot 조회, stop 요청은 모두 I/O 경계라 실패할 수 있다.
// application service는 구체 오류 타입보다 failure context 보존이 중요하므로 `anyhow::Result`를 그대로 사용한다.
use anyhow::Result;

// `ConversationStreamEvent`는 outbound runtime이 TUI로 보내는 application-level stream contract이다.
// port는 app-server protocol event가 아니라 이 정규화된 enum만 노출한다.
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
// snapshot은 저장된 conversation read model이고, runtime control truth는 중단/제어의 실제 소유자를 나타낸다.
// 둘 다 TUI가 구체 adapter 타입을 몰라도 대화 화면과 제어 버튼을 구성하게 해 주는 domain 값이다.
use crate::domain::conversation::{ConversationRuntimeControlTruth, ConversationSnapshot};

// `InteractiveTurnRuntimePort`는 `ConversationService`가 outbound runtime에 요구하는 대화 실행 계약이다.
// 실제 구현은 Codex app-server adapter이지만, application 계층은 새 thread 실행, 기존 thread 실행, snapshot 조회,
// 전체 중단 요청이라는 사용 사례만 알고 있으면 된다.
//
// 이 trait가 `Send + Sync`인 이유는 TUI runtime이 background task와 UI event loop 사이에서 service를
// 복제해 사용하기 때문이다. port 구현은 여러 thread에서 공유되어도 안전해야 한다.
pub trait InteractiveTurnRuntimePort: Send + Sync {
    // runtime control truth는 "stop all sessions"나 실행 상태 판단을 실제로 어느 runtime이 담당하는지
    // TUI에 알려 주는 값이다. AppRuntime은 이 값을 초기화 시 읽어 중단 버튼과 상태 문구의 근거로 삼는다.
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth;

    // 저장된 conversation snapshot을 읽는다. TUI가 session list에서 thread를 열거나 background update를
    // 반영할 때, app-server/session store 세부사항 없이 thread_id 하나로 대화 read model을 요청하는 경계이다.
    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot>;

    // 현재 runtime이 관리하는 모든 interactive session에 중단을 요청한다.
    // TUI controller는 사용자 명령을 이 메서드 하나로 전달하고, adapter는 app-server connection/turn interrupt 구현을 소유한다.
    fn request_stop_all_sessions(&self) -> Result<()>;

    // 아직 thread_id가 없는 새 대화를 시작하고 첫 prompt를 stream으로 실행한다.
    // 성공은 "stream worker를 시작했다"는 의미이고, 실제 메시지/완료/실패 상태는 `event_sender`로 이어서 전달된다.
    fn run_new_thread_stream(
        &self,
        // 새 app-server thread가 실행될 workspace directory이다.
        cwd: &str,
        // 사용자 prompt 또는 조립된 main-session prompt이다. adapter가 Codex protocol request로 매핑한다.
        prompt: &str,
        // outbound runtime이 `ThreadPrepared`, `TurnStarted`, delta, tool activity, completion/failure를 보낼 채널이다.
        // sender 소유권을 넘기는 이유는 runtime worker가 호출 stack보다 오래 살아 있을 수 있기 때문이다.
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;

    // 이미 존재하는 conversation thread에 후속 prompt를 stream으로 실행한다.
    // 새 thread와 같은 event contract를 쓰므로 TUI 수신 루프는 두 실행 경로를 거의 같은 reducer로 처리할 수 있다.
    fn run_turn_stream(
        &self,
        // app-server/session store가 알고 있는 conversation thread 식별자이다.
        thread_id: &str,
        // 기존 thread에 이어 붙일 prompt이다.
        prompt: &str,
        // 후속 turn의 stream event를 전달할 채널이다. 실패도 panic이 아니라 `Failed` 이벤트나 `Result` 오류로 표현된다.
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
}
