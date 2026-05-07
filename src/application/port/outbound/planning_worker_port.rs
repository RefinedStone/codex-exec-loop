// planning worker는 별도 Codex/app-server session을 실행하고, 그 과정에서 stream failure,
// thread bootstrap 실패, workspace 접근 실패를 만날 수 있다. application orchestration은
// 실패 원인을 정책 값으로 재해석하기 전에 adapter의 I/O 오류를 그대로 받아야 하므로 port는
// `anyhow::Result`를 반환한다.
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// `PlanningWorkerOperation`은 planning worker가 수행하는 자동화 목적을 구분한다. 같은 worker
// port가 app-server planning session을 실행하더라도, queue refresh와 authority repair는
// 서로 다른 status label, prompt 구성, 후속 검증을 갖는다. operation을 request/response에 함께
// 싣는 이유는 비동기 worker 결과가 돌아온 뒤에도 caller가 어떤 lifecycle의 결과인지 잃지 않게
// 하기 위해서이다.
pub enum PlanningWorkerOperation {
    // planning DB와 문서 상태를 읽어 다음 queue/head/proposal 상태를 갱신하는 작업이다.
    RefreshQueue,
    // task authority ledger나 planning source-of-truth가 어긋났을 때 복구 prompt를 실행하는 작업이다.
    RepairTaskAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `PlanningWorkerRequest`는 planning orchestration이 outbound worker adapter에 넘기는 실행
// 명세이다. worker orchestration은 domain/application 상태를 읽어 이 값을 만들고, app-server
// planning worker adapter는 `prompt`를 새 planning session의 turn input으로 전달한다.
pub struct PlanningWorkerRequest {
    // 작업의 목적이다. response에도 되돌아와 caller가 완료 로그와 status label을 같은 operation으로 묶는다.
    pub operation: PlanningWorkerOperation,
    // planning worker session을 실행할 workspace root이다. main TUI thread의 cwd와 다를 수
    // 있으므로 adapter가 암묵적인 process cwd에 기대지 않게 명시한다.
    pub workspace_directory: String,
    // planning runtime이 조립한 최종 worker prompt이다. port는 이 문자열을 재해석하지 않고
    // Codex turn으로 전달해 prompt 정책을 application service 안에 남긴다.
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `PlanningWorkerResponse`는 worker session stream을 application orchestration이 다시 쓰기 쉬운
// 형태로 줄인 값이다. adapter는 `ConversationStreamEvent`들을 수집해 최종 agent message와
// planning 파일 변경 목록만 이 구조로 반환하고, 파일 검증과 ledger 반영은 caller 쪽 use case가 담당한다.
pub struct PlanningWorkerResponse {
    // 요청 operation을 response에도 보존해 async orchestration 로그와 후속 분기에서 같은 작업으로 식별한다.
    pub operation: PlanningWorkerOperation,
    // hidden worker session id다. provider 이름과 무관하게 provenance thread_id로 저장된다.
    pub thread_id: Option<String>,
    // hidden worker turn id다. task mutation source_turn_id와 generic provenance turn_id로 연결된다.
    pub turn_id: Option<String>,
    // worker가 마지막으로 완료한 assistant message이다. stream이 tool-only로 끝날 수 있어 optional이다.
    pub final_agent_message: Option<String>,
    // worker turn이 수정했다고 보고한 planning 파일 경로이다. repair/refresh 후 검증과 UI 알림에 연결된다.
    pub changed_planning_file_paths: Vec<String>,
}

// `PlanningWorkerPort`는 planning orchestration이 "별도 agent session을 실행해 planning 작업을
// 수행한다"는 outbound capability를 추상화한다. production은 app-server planning worker
// adapter를 쓰고, planning feature가 꺼진 구성에서는 noop implementation을 주입할 수 있다.
pub trait PlanningWorkerPort: Send + Sync {
    // planning worker session을 실행하고 축약된 결과를 반환한다. stream event 수집, failure event
    // 처리, changed file path 추출은 adapter 책임이고, caller는 response만 보고 후속 orchestration을 진행한다.
    fn run_planning_session(
        &self,
        // operation, workspace, prompt를 포함한 실행 명세이다.
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse>;
}

// `NoopPlanningWorkerPort`는 tests에서 planning worker capability 없이 service graph를 조립하기 위한 fake다.
#[cfg(test)]
pub struct NoopPlanningWorkerPort;

#[cfg(test)]
impl PlanningWorkerPort for NoopPlanningWorkerPort {
    // noop 구현은 요청 operation을 그대로 돌려주고, agent message에 비활성 상태를 남긴다. 이를
    // 통해 caller는 "worker가 성공적으로 아무 것도 하지 않았다"와 "worker 실행 실패"를 구분할 수 있다.
    fn run_planning_session(
        &self,
        // operation만 response에 반영하고 workspace/prompt는 실행하지 않는다.
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse> {
        Ok(PlanningWorkerResponse {
            operation: request.operation,
            thread_id: None,
            turn_id: None,
            // 사람이 로그나 test failure에서 비활성 fallback을 알아볼 수 있는 고정 메시지이다.
            final_agent_message: Some("planning worker disabled".to_string()),
            // 실제 worker가 돌지 않았으므로 변경된 planning 파일은 없다.
            changed_planning_file_paths: Vec::new(),
        })
    }
}
