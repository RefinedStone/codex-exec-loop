// 학습 주석: planning worker는 별도 Codex/app-server session을 실행하거나 stream failure를 받을 수 있으므로
// I/O 경계 오류를 `anyhow::Result`로 application orchestration에 되돌립니다.
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 학습 주석: `PlanningWorkerOperation`은 planning worker가 수행하는 자동화 목적을 구분합니다.
// 같은 worker port가 app-server planning session을 실행하더라도, 호출자는 queue refresh와 authority repair를
// 서로 다른 lifecycle/status로 기록해야 하므로 operation을 request/response에 함께 싣습니다.
pub enum PlanningWorkerOperation {
    // 학습 주석: planning DB와 문서 상태를 읽어 다음 queue/head/proposal 상태를 갱신하는 작업입니다.
    RefreshQueue,
    // 학습 주석: task authority ledger나 planning source-of-truth가 어긋났을 때 복구 prompt를 실행하는 작업입니다.
    RepairTaskAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `PlanningWorkerRequest`는 planning orchestration이 outbound worker adapter에 넘기는 실행 명세입니다.
// worker orchestration은 이 값을 만들고, app-server planning worker adapter는 prompt를 새 planning session으로 실행합니다.
pub struct PlanningWorkerRequest {
    // 학습 주석: 작업의 목적입니다. response에도 되돌아와 caller가 완료 로그와 status label을 같은 operation으로 묶습니다.
    pub operation: PlanningWorkerOperation,
    // 학습 주석: planning worker session을 실행할 workspace root입니다. main TUI thread의 cwd와 다를 수 있으므로 명시합니다.
    pub workspace_directory: String,
    // 학습 주석: planning runtime이 조립한 최종 worker prompt입니다. port는 이 문자열을 재해석하지 않고 Codex turn으로 전달합니다.
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `PlanningWorkerResponse`는 worker session stream을 application orchestration이 다시 쓰기 쉬운 형태로 줄인 값입니다.
// adapter는 `ConversationStreamEvent`들을 수집해 최종 agent message와 planning 파일 변경 목록만 이 구조로 반환합니다.
pub struct PlanningWorkerResponse {
    // 학습 주석: 요청 operation을 response에도 보존해 async orchestration 로그와 후속 분기에서 같은 작업으로 식별합니다.
    pub operation: PlanningWorkerOperation,
    // 학습 주석: worker가 마지막으로 완료한 assistant message입니다. stream이 tool-only로 끝날 수 있어 optional입니다.
    pub final_agent_message: Option<String>,
    // 학습 주석: worker turn이 수정했다고 보고한 planning 파일 경로입니다. repair/refresh 후 검증과 UI 알림에 연결됩니다.
    pub changed_planning_file_paths: Vec<String>,
}

// 학습 주석: `PlanningWorkerPort`는 planning orchestration이 "별도 agent session을 실행해 planning 작업을 수행한다"는
// outbound capability를 추상화합니다. production은 app-server planning worker adapter를 쓰고, planning feature가 꺼진
// 구성에서는 noop implementation을 주입할 수 있습니다.
pub trait PlanningWorkerPort: Send + Sync {
    // 학습 주석: planning worker session을 실행하고 축약된 결과를 반환합니다. stream event 수집, failure event 처리,
    // changed file path 추출은 adapter 책임이고, caller는 response만 보고 후속 orchestration을 진행합니다.
    fn run_planning_session(
        &self,
        // 학습 주석: operation, workspace, prompt를 포함한 실행 명세입니다.
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse>;
}

// 학습 주석: `NoopPlanningWorkerPort`는 planning worker capability가 없는 구성에서 쓰는 안전한 fallback입니다.
// worker가 실제 파일을 바꾸지 않았음을 명확히 하면서도 orchestration이 panic 없이 진행되도록 response 형태를 맞춥니다.
pub struct NoopPlanningWorkerPort;

impl PlanningWorkerPort for NoopPlanningWorkerPort {
    // 학습 주석: noop 구현은 요청 operation을 그대로 돌려주고, agent message에 비활성 상태를 남깁니다.
    // 이를 통해 caller는 "worker가 성공적으로 아무 것도 하지 않았다"와 "worker 실행 실패"를 구분할 수 있습니다.
    fn run_planning_session(
        &self,
        // 학습 주석: operation만 response에 반영하고 workspace/prompt는 실행하지 않습니다.
        request: PlanningWorkerRequest,
    ) -> Result<PlanningWorkerResponse> {
        Ok(PlanningWorkerResponse {
            operation: request.operation,
            // 학습 주석: 사람이 로그나 test failure에서 비활성 fallback을 알아볼 수 있는 고정 메시지입니다.
            final_agent_message: Some("planner worker disabled".to_string()),
            // 학습 주석: 실제 worker가 돌지 않았으므로 변경된 planning 파일은 없습니다.
            changed_planning_file_paths: Vec::new(),
        })
    }
}
