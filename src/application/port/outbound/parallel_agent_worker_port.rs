// 학습 주석: parallel worker는 별도 dispatch worker에서 conversation stream을 수집합니다.
// `Sender`는 app-server adapter가 그 dispatch worker의 event loop로 stream event를 밀어 넣는 통로입니다.
use std::sync::mpsc::Sender;

// 학습 주석: isolated worker thread 시작은 app-server I/O와 worktree 실행 경계에 닿으므로 실패할 수 있습니다.
use anyhow::Result;

// 학습 주석: parallel worker도 일반 conversation stream과 같은 `ConversationStreamEvent` 계약을 씁니다.
// 그래서 dispatch worker는 final assistant text, completion, failure를 기존 reducer vocabulary로 관찰할 수 있습니다.
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;

// 학습 주석: `ParallelAgentWorkerPort`는 parallel mode가 leased worktree에서 isolated Codex session을 시작하기 위해
// outbound app-server adapter에 요구하는 최소 계약입니다. 일반 `InteractiveTurnRuntimePort`와 달리 기존 user thread에
// 붙지 않고, slot별 작업을 새 thread로 분리해 실행합니다.
//
// 학습 주석: 이 port가 별도 trait인 이유는 parallel dispatch가 main conversation UI와 다른 lifecycle을 갖기 때문입니다.
// dispatch worker는 stream을 수집해 slot result로 환원하고, PR/merge/delivery는 상위 distributor가 나중에 담당합니다.
pub trait ParallelAgentWorkerPort: Send + Sync {
    // 학습 주석: leased worktree directory에서 isolated new thread를 시작합니다.
    // 성공은 worker stream이 시작되었다는 뜻이고, 실제 결과는 event_sender로 들어오는 completion/failure events에서 결정됩니다.
    fn run_isolated_new_thread_stream(
        &self,
        // 학습 주석: slot이 lease한 worktree root입니다. main workspace와 분리되어 worker 변경 범위를 제한합니다.
        cwd: &str,
        // 학습 주석: distributor가 만든 handoff prompt입니다. worker는 이 한 작업 범위만 수행해야 합니다.
        prompt: &str,
        // 학습 주석: dispatch worker가 소유한 stream receiver와 짝을 이루는 sender입니다.
        // outbound adapter는 이 sender로 thread prepared, message completed, tool activity, terminal completion을 보냅니다.
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
}

#[derive(Debug, Default)]
// 학습 주석: `NoopParallelAgentWorkerPort`는 테스트 fixture나 parallel worker capability가 비활성인 구성에서 쓰는 fallback입니다.
// stream을 시작하지 않고 즉시 성공을 돌려주므로, caller는 별도 fake 없이 TUI shell runtime을 구성할 수 있습니다.
pub struct NoopParallelAgentWorkerPort;

impl ParallelAgentWorkerPort for NoopParallelAgentWorkerPort {
    // 학습 주석: noop 구현은 실제 isolated worker를 만들지 않습니다.
    // 인자를 모두 underscore로 받아 "계약은 만족하지만 사용하지 않는다"는 의도를 Rust 경고 없이 표현합니다.
    fn run_isolated_new_thread_stream(
        &self,
        // 학습 주석: noop에서는 worktree root를 사용하지 않습니다.
        _cwd: &str,
        // 학습 주석: noop에서는 handoff prompt를 실행하지 않습니다.
        _prompt: &str,
        // 학습 주석: noop은 stream events를 보내지 않으므로 sender도 사용하지 않습니다.
        _event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        // 학습 주석: 성공을 반환해 shell/runtime 구성 테스트가 parallel worker adapter 없이도 진행되게 합니다.
        Ok(())
    }
}
