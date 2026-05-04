// parallel worker는 slot별 worktree에서 별도 Codex session을 실행하지만, 결과 수집은 TUI
// dispatch worker가 맡는다. `Sender`는 app-server adapter가 그 dispatch worker의 event
// loop로 stream event를 밀어 넣는 단방향 통로이다.
use std::sync::mpsc::Sender;

// isolated worker thread 시작은 app-server I/O, 현재 repo의 worktree 상태, Codex session
// bootstrap에 닿는 outbound 작업이다. 실패는 정책 오류가 아니라 실행 환경의 정상적인 결과일 수
// 있으므로 port는 `anyhow::Result`로 adapter 실패를 그대로 올린다.
use anyhow::Result;

// parallel worker도 일반 conversation stream과 같은 `ConversationStreamEvent` 계약을 쓴다.
// 이 공유 vocabulary 덕분에 dispatch worker는 worker 전용 protocol을 새로 만들지 않고도
// final assistant text, completion, failure, tool activity를 기존 reducer 관점으로 관찰할 수 있다.
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
// `ParallelAgentWorkerStreamRequest`는 application layer가 확정한 sub-session prompt 경계이다.
// adapter는 thread metadata와 turn prompt를 재조립하지 않고 app-server protocol로만 매핑한다.
pub struct ParallelAgentWorkerStreamRequest<'a> {
    pub cwd: &'a str,
    pub prompt: &'a str,
    pub developer_instructions: &'a str,
    pub service_name: &'a str,
}

// `ParallelAgentWorkerPort`는 parallel mode가 leased worktree에서 isolated Codex session을
// 시작하기 위해 outbound app-server adapter에 요구하는 최소 계약이다. 일반
// `InteractiveTurnRuntimePort`와 달리 기존 user thread에 붙지 않고, distributor가 slot에 배정한
// 단일 작업을 새 thread로 분리해 실행한다.
//
// 이 port가 별도 trait인 이유는 parallel dispatch가 main conversation UI와 다른 lifecycle을 갖기
// 때문이다. dispatch worker는 stream을 수집해 slot result로 환원하고, PR/merge/delivery는 상위
// distributor가 나중에 담당한다. port를 분리하면 adapter는 같은 app-server transport를 쓰더라도
// "사용자 대화 turn"과 "고립된 작업자 실행"의 계약을 혼동하지 않는다.
pub trait ParallelAgentWorkerPort: Send + Sync {
    // leased worktree directory에서 isolated new thread를 시작한다. 성공은 worker stream이
    // 시작되었다는 뜻일 뿐이며, 실제 작업 성공/실패는 `event_sender`로 들어오는
    // completion/failure event를 dispatch worker가 환원해 결정한다.
    fn run_isolated_new_thread_stream(
        &self,
        // slot worktree, turn prompt, thread metadata를 담은 실행 요청이다.
        request: ParallelAgentWorkerStreamRequest<'_>,
        // dispatch worker가 소유한 stream receiver와 짝을 이루는 sender이다. outbound adapter는
        // thread prepared, message completed, tool activity, terminal completion 같은 app-server
        // event를 이 통로로 전달하고, dispatch worker는 그 흐름을 slot 상태로 축약한다.
        event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()>;
}

#[derive(Debug, Default)]
// `NoopParallelAgentWorkerPort`는 테스트 fixture나 parallel worker capability가 비활성인 구성에서
// 쓰는 fallback이다. stream을 시작하지 않고 즉시 성공을 돌려주므로, caller는 별도 fake를 만들지
// 않아도 TUI shell runtime의 dependency graph를 구성할 수 있다.
pub struct NoopParallelAgentWorkerPort;

impl ParallelAgentWorkerPort for NoopParallelAgentWorkerPort {
    // noop 구현은 실제 isolated worker를 만들지 않는다. 인자를 모두 underscore로 받아 "계약은
    // 만족하지만 사용하지 않는다"는 의도를 Rust 경고 없이 표현한다.
    fn run_isolated_new_thread_stream(
        &self,
        // noop에서는 실행 요청을 사용하지 않는다.
        _request: ParallelAgentWorkerStreamRequest<'_>,
        // noop은 stream events를 보내지 않으므로 sender도 사용하지 않는다.
        _event_sender: Sender<ConversationStreamEvent>,
    ) -> Result<()> {
        // 성공을 반환해 shell/runtime 구성 테스트가 parallel worker adapter 없이도 진행되게 한다.
        Ok(())
    }
}
