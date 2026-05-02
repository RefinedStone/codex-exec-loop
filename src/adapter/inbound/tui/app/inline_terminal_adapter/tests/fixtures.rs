/*
학습 주석: inline terminal adapter 테스트는 terminal drawing, viewport replay, host scrollback insertion을
검증하지만, 입력은 거의 항상 완성된 `NativeTuiApp`에서 나옵니다. 이 fixture는 실제 app-server 없이도
startup/session/conversation/planning service graph를 조립해 렌더링 테스트가 infrastructure 대신 화면 계약에 집중하게 합니다.
*/
use std::sync::Arc;

use anyhow::Result;

use crate::adapter::inbound::tui::app::{ConversationState, NativeTuiApp};
use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
use crate::application::port::outbound::codex_app_server_port::{
    AppServerStartupContext, CodexAppServerPort,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::conversation_service::ConversationService;
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};
use crate::application::service::session_service::SessionService;
use crate::application::service::startup_service::StartupService;
use crate::domain::conversation::ConversationSnapshot;
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog};

// 학습 주석: FakeCodexAppServerPort는 inline terminal 테스트가 app-server process, network stream, session
// catalog 구현을 요구하지 않게 하는 공유 fake입니다. 성공 경로만 제공해 테스트 실패가 renderer 변화에서만 나오게 합니다.
struct FakeCodexAppServerPort;

impl CodexAppServerPort for FakeCodexAppServerPort {
    // 학습 주석: startup context는 shell chrome이 "app-server bridge usable" 상태로 렌더링되는지 결정합니다.
    // 여기서는 account/initialize를 모두 정상으로 두어 terminal adapter 테스트가 startup failure copy에 묶이지 않게 합니다.
    fn load_startup_context(&self) -> Result<AppServerStartupContext> {
        Ok(AppServerStartupContext {
            attachment_profile:
                crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_detail: "ok".to_string(),
            account_detail: "ok".to_string(),
            account_ok: true,
            warnings: Vec::new(),
        })
    }

    // 학습 주석: session browser 테스트가 아닌 inline terminal 테스트에서는 catalog content가 중요하지 않습니다.
    // 빈 Ready catalog를 돌려 shell/runtime이 session capability를 성공적으로 통과하되 row rendering은 발생하지 않게 합니다.
    fn load_recent_sessions(&self, _limit: usize) -> Result<SessionCatalog> {
        Ok(RecentSessions {
            items: Vec::new(),
            warnings: Vec::new(),
            next_cursor: None,
        }
        .into())
    }

    // 학습 주석: resumed-session 경로에는 deterministic한 loaded conversation이 필요합니다. 반환 snapshot은
    // 요청된 thread id를 보존하되 title/cwd/message를 고정해 viewport snapshot이 외부 상태에 흔들리지 않게 합니다.
    fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
        Ok(ConversationSnapshot {
            thread_id: thread_id.to_string(),
            title: "Loaded thread".to_string(),
            cwd: "/tmp/root".to_string(),
            messages: Vec::new(),
            warnings: Vec::new(),
            runtime_notices: Vec::new(),
        })
    }

    // 학습 주석: stop-all도 렌더링 setup과 같은 runtime port에 있지만 inline terminal 테스트는 process control을
    // 검증하지 않습니다. 성공으로 닫아 fixture construction이 unrelated shutdown wiring에 막히지 않게 합니다.
    fn request_stop_all_sessions(&self) -> Result<()> {
        Ok(())
    }

    // 학습 주석: new-thread streaming은 no-op입니다. 이 테스트들은 conversation/runtime state를 app에 직접 넣고
    // render하므로, 여기서 stream event를 만들면 terminal 테스트가 async turn behavior에 의존하게 됩니다.
    fn run_new_thread_stream(
        &self,
        _cwd: &str,
        _prompt: &str,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        Ok(())
    }

    // 학습 주석: existing-thread turn streaming도 의도적으로 조용히 성공합니다. live delta가 필요한 테스트는
    // model/runtime event를 app state로 직접 주입하므로, 이 fixture는 deterministic하고 synchronous하게 남습니다.
    fn run_turn_stream(
        &self,
        _thread_id: &str,
        _prompt: &str,
        _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        Ok(())
    }
}

// 학습 주석: make_test_app은 production과 같은 service graph를 만들되 외부 boundary를 deterministic local fake로
// 바꿉니다. inline terminal 테스트는 app-server 시작이나 실제 planning workspace 조작 없이도 현실적인
// `NativeTuiApp`을 대상으로 shell rendering, viewport bookkeeping, planning copy를 검증할 수 있습니다.
pub(super) fn make_test_app() -> NativeTuiApp {
    // 학습 주석: 하나의 Arc-backed fake를 startup/session/conversation service가 공유합니다. production에서도
    // 이 service들이 Codex app-server port family를 통해 같은 outbound boundary를 바라보므로 wiring shape가 맞춰집니다.
    let codex_port = Arc::new(FakeCodexAppServerPort);
    let mut app = NativeTuiApp::new(
        StartupService::new(codex_port.clone()),
        SessionService::new(codex_port.clone()),
        ConversationService::new(codex_port),
        // 학습 주석: parallel worker는 inline terminal rendering contract 밖에 있습니다. noop worker를 넣으면
        // app construction은 production 형태를 유지하되 terminal 테스트 중 background agent work는 시작되지 않습니다.
        Arc::new(
            crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort,
        ),
        // 학습 주석: test parallel-mode service는 production과 같은 control surface를 제공하지만 실제
        // slot/worktree orchestration은 피합니다.
        crate::adapter::inbound::tui::app::test_helpers::test_parallel_mode_service(),
        // 학습 주석: 여러 shell panel이 planning snapshot shape를 읽으므로 PlanningServices에는 filesystem workspace
        // adapter를 그대로 주입합니다. 아래에서 uninitialized로 시작시켜 rendered copy는 예측 가능하게 유지합니다.
        PlanningServices::from_workspace_port(Arc::new(FilesystemPlanningWorkspaceAdapter::new())),
    );
    // 학습 주석: NativeTuiApp::new는 terminal 테스트용 ready draft conversation을 만들어야 합니다. 이 불변식이
    // 바뀌면 대부분의 inline rendering path가 `/tmp/root`의 editable draft를 전제하므로 fixture가 즉시 실패해야 합니다.
    let ConversationState::Ready(conversation) = &mut app.conversation_state else {
        panic!("test app should start with a ready draft conversation");
    };
    // 학습 주석: 고정된 cwd/draft workspace는 snapshot text와 workspace-aware planning copy의 기준점입니다.
    // 두 값을 같게 두면 terminal UI만 보려는 테스트가 우연히 workspace mismatch behavior를 덮지 않습니다.
    conversation.cwd = "/tmp/root".to_string();
    conversation.draft_workspace_directory = "/tmp/root".to_string();
    // 학습 주석: uninitialized planning runtime snapshot은 inline shell 테스트의 중립 baseline입니다. 각 테스트는
    // fixture에서 seeded task를 물려받는 대신 필요할 때만 더 풍부한 planning state를 명시적으로 넣을 수 있습니다.
    conversation.replace_planning_runtime_snapshot(PlanningRuntimeSnapshot::uninitialized());
    app
}
