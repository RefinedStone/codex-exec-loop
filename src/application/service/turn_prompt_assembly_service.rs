use crate::application::service::prompt_component::PromptDocument;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
// `SubSessionAgentPrompt`는 특정 agent profile이 제공하는 prompt 주입값이다.
// 비어 있으면 profile prompt 없이 기본 sub-session 계약만 사용한다.
pub struct SubSessionAgentPrompt {
    pub label: String,
    pub lines: Vec<String>,
}

impl SubSessionAgentPrompt {
    pub fn new(label: impl Into<String>, lines: Vec<String>) -> Self {
        Self {
            label: label.into().trim().to_string(),
            lines: lines
                .into_iter()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect(),
        }
    }

    fn has_lines(&self) -> bool {
        !self.lines.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `ManualPromptAssemblyRequest`는 사람이 TUI에서 직접 입력한 prompt를 main-session용
// 실행 prompt로 감싸기 위한 요청이다. manual 입력도 queue에서 온 작업과 같은 main-session guardrail을 타야 하므로,
// 별도 타입으로 의미를 드러낸 뒤 내부에서는 `MainSessionPromptAssemblyRequest`로 변환한다.
pub struct ManualPromptAssemblyRequest<'a> {
    // operator가 입력한 원문이다. 서비스는 앞뒤 공백만 정리하고 의미를 재작성하지 않는다.
    pub operator_prompt: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `MainSessionPromptAssemblyRequest`는 실제 주 작업 세션에 들어갈 prompt 조립 요청이다.
// main-session은 commit/push/PR/merge 같은 delivery 권한을 가진 흐름이므로, system prompt가 결과 형식과 지시 우선순위를
// 분명히 잡아 준다.
pub struct MainSessionPromptAssemblyRequest<'a> {
    // 사용자 요청 또는 distributor가 main-session에 넘긴 queue handoff 본문이다.
    pub user_prompt: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `SubSessionPromptAssemblyRequest`는 parallel mode의 leased worktree에서 실행될 하위 세션 prompt이다.
// sub-session은 코드를 고치거나 작은 commit을 만들 수 있지만 delivery는 distributor가 담당하므로,
// main-session과 다른 system prompt로 권한 경계를 강하게 제한한다.
pub struct SubSessionPromptAssemblyRequest<'a> {
    // distributor가 만든 queued-task handoff 원문이다. 이 값이 sub-session의 유일한 작업 범위이다.
    pub handoff_prompt: &'a str,
    // 선택된 agent profile에서 온 persona prompt다.
    pub agent_prompt: SubSessionAgentPrompt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `SubSessionPromptAssembly`는 app-server thread metadata와 turn prompt를 한 application 경계에서 묶는다.
// outbound adapter는 이 값을 직렬화만 하고, Akra-specific worker 계약 문구를 새로 조립하지 않는다.
pub struct SubSessionPromptAssembly {
    pub turn_prompt: String,
    pub developer_instructions: String,
    pub service_name: String,
}

#[derive(Debug, Clone, Default)]
// `TurnPromptAssemblyService`는 Codex turn에 실제로 넣을 최종 prompt 문자열을 만드는 application service이다.
// 상태를 들고 있지 않기 때문에 값 자체는 빈 struct이고, 호출자는 shared service 구성에서 cheap clone/default로 주입한다.
//
// 이 계층을 따로 둔 이유는 "사용자 입력 그대로"를 app-server에 보내지 않고, main/sub-session별
// 시스템 지시와 runtime context를 일관되게 감싸기 위해서이다. prompt 정책이 흩어지면 병렬 세션 권한 경계가 쉽게 깨진다.
pub struct TurnPromptAssemblyService;

// main-session은 사용자의 실제 요청을 완료하고 최종 답변을 돌려주는 주 실행 경로이다.
// 지시 충돌 해소, 실행 범위, 결과 보고 형식을 서로 다른 section으로 나눠 모델이 계약을 덮어 읽지 않게 한다.
fn main_session_execution_contract_lines() -> Vec<String> {
    vec![
        "아래 `user-prompt`를 수행하세요.".to_string(),
        "기존 정책과 사용자 요청이 충돌하면 더 구체적이고 최신인 지시를 우선하되 전체 의도를 하나의 실행 계획으로 통합하세요.".to_string(),
        "task authority, planning queue, direction authority를 직접 생성/수정/삭제하지 마세요. 필요한 후속 작업은 최종 답변의 follow-up suggestion으로만 남기세요.".to_string(),
    ]
}

fn main_session_reporting_contract_lines() -> Vec<String> {
    vec![
        "최종 답변은 간결하게 작성하세요.".to_string(),
        "가능하면 `수정사항`, `결과`, `다음 추천`을 포함하세요.".to_string(),
        "`수정사항`에는 변경한 파일 위치와 핵심 변경을 적으세요.".to_string(),
        "`결과`에는 실행/검증 결과를 적으세요.".to_string(),
        "`다음 추천`에는 성능개선, 추천수정, 우려되는 문제를 적으세요.".to_string(),
    ]
}

// sub-session의 핵심은 "handoff 하나만 수행"과 "delivery 금지"이다.
// 하위 작업자가 shared branch rebase나 PR merge를 직접 수행하면 distributor의 통합 순서와 worktree 정리가 무너질 수 있다.
fn sub_session_execution_contract_lines() -> Vec<String> {
    vec![
        "아래 `queued-task-handoff`만 수행하세요.".to_string(),
        "이 세션은 leased worktree에서 실행되는 Akra sub-session입니다.".to_string(),
        "작업 범위는 handoff의 task 하나로 제한하세요.".to_string(),
        "의미 있는 코드 변경이 있으면 작은 reviewable commit을 남기세요.".to_string(),
    ]
}

fn sub_session_delivery_boundary_lines() -> Vec<String> {
    vec![
        "push, PR 생성, merge, shared branch rebase, worktree cleanup은 수행하지 마세요."
            .to_string(),
        "완료 후 Akra distributor가 delivery를 처리합니다.".to_string(),
    ]
}

fn sub_session_reporting_contract_lines() -> Vec<String> {
    vec!["최종 답변에는 변경 요약, 검증 결과, 남은 작업만 간결하게 포함하세요.".to_string()]
}

fn sub_session_prompt_heading(agent_prompt: &SubSessionAgentPrompt) -> String {
    if !agent_prompt.has_lines() {
        return "Agent profile prompt:".to_string();
    }
    if agent_prompt.label.is_empty() {
        "Agent profile prompt:".to_string()
    } else {
        format!("Agent profile prompt: {}", agent_prompt.label)
    }
}

fn sub_session_developer_instructions(
    persona_lines: &[String],
    agent_prompt: &SubSessionAgentPrompt,
) -> String {
    let mut lines = vec![
        "You are an Akra parallel task sub-session running in a leased worktree.",
        "Execute only the queued-task handoff supplied in the turn prompt.",
        "Keep changes scoped to that task and leave a small reviewable commit when source changes are needed.",
        "Do not push, open pull requests, merge, rebase shared branches, or clean up the worktree; Akra distributor handles delivery after completion.",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();
    if !persona_lines.is_empty() {
        lines.push(String::new());
        lines.push(sub_session_prompt_heading(agent_prompt));
        lines.extend(persona_lines.iter().cloned());
    }
    lines.join("\n")
}

fn sub_session_service_name() -> String {
    "akra-parallel-worker".to_string()
}

impl TurnPromptAssemblyService {
    // 상태 없는 서비스 생성자이다. shared service composition에서 다른 service와 같은 형태로 주입하기 위해
    // `new`를 제공하고, 테스트도 이 생성자를 통해 실제 production 경로와 같은 API를 사용한다.
    pub fn new() -> Self {
        Self
    }

    // manual prompt는 사람이 직접 입력한 요청을 main-session prompt로 승격한다.
    // 별도 렌더러를 만들지 않고 main-session 렌더러를 재사용해, manual 실행과 queue 실행이 같은 guardrail을 공유한다.
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn build_manual_prompt(&self, request: ManualPromptAssemblyRequest<'_>) -> Option<String> {
        /*
         * manual turn도 여전히 main-session turn이다.
         * build_main_session_prompt를 거치면 직접 operator 입력도 queue handoff와 같은 delivery/reporting 계약을 탄다.
         * 동시에 operator가 입력한 원문은 마지막 user prompt section으로 보존된다.
         */
        self.build_main_session_prompt(MainSessionPromptAssemblyRequest {
            // operator prompt는 main-session 관점에서는 user prompt이다.
            user_prompt: request.operator_prompt,
        })
    }

    // main-session prompt를 만든다. 반환이 `Option<String>`인 이유는 공백뿐인 user prompt를
    // app-server로 보내지 않기 위해서이다. 호출자는 `None`을 "실행할 turn 없음"으로 처리할 수 있다.
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn build_main_session_prompt(
        &self,
        // 사용자 요청과 선택 planning context를 담은 조립 요청이다.
        request: MainSessionPromptAssemblyRequest<'_>,
    ) -> Option<String> {
        // user prompt는 전체 prompt의 핵심 payload이므로 앞뒤 공백을 정리한 뒤 비어 있는지 먼저 본다.
        let user_prompt = request.user_prompt.trim();
        if user_prompt.is_empty() {
            /*
             * 빈 user prompt는 prompt rendering 전에 멈춘다.
             * user payload 없는 system prompt를 보내면 authority rule만 있고 task가 없는 turn이 생긴다.
             * caller 입장에서는 그런 turn보다 None 결과가 훨씬 판단하기 쉽다.
             */
            return None;
        }

        Some(render_main_session_prompt(user_prompt))
    }

    // sub-session prompt를 만든다. sub-session은 handoff 하나가 작업 범위이므로,
    // handoff가 비어 있으면 session을 시작하지 않는 것이 맞다.
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn build_sub_session_prompt(
        &self,
        // distributor가 lease한 slot에 전달할 handoff 요청이다.
        request: SubSessionPromptAssemblyRequest<'_>,
    ) -> Option<SubSessionPromptAssembly> {
        // 빈 handoff는 범위 없는 sub-session을 만들기 때문에 `None`으로 막는다.
        let handoff_prompt = request.handoff_prompt.trim();
        if handoff_prompt.is_empty() {
            /*
             * handoff 없는 sub-session은 bounded task가 없다.
             * 여기서 None을 돌리면 distributor가 generic system instruction만 보고 움직일 worker lane을 lease하지 않는다.
             */
            return None;
        }

        let persona_lines = request.agent_prompt.lines.clone();
        Some(SubSessionPromptAssembly {
            turn_prompt: render_sub_session_prompt(handoff_prompt, &persona_lines),
            developer_instructions: sub_session_developer_instructions(
                &persona_lines,
                &request.agent_prompt,
            ),
            service_name: sub_session_service_name(),
        })
    }
}

// main-session prompt의 실제 문자열 레이아웃을 담당한다.
// 형식은 실행 계약, 보고 계약, user prompt 순서이다. planning context와 task authority mutation 규칙은
// hidden intake/planning worker 계층에서만 소비되고 main-session에는 compact handoff만 들어온다.
#[tracing::instrument(level = "trace")]
fn render_main_session_prompt(
    // 최종 prompt의 `user prompt:` section에 들어갈 실행 요청이다.
    user_prompt: &str,
) -> String {
    PromptDocument::builder("akra-main-session-turn")
        .lines(
            "execution-contract",
            main_session_execution_contract_lines(),
        )
        .lines(
            "reporting-contract",
            main_session_reporting_contract_lines(),
        )
        .text("user-prompt", user_prompt)
        .build()
        .render()
}

// sub-session prompt의 문자열 레이아웃이다. main-session과 달리 runtime context를 따로 받지 않고,
// `queued-task-handoff` 하나만 작업 범위로 전달한다.
#[tracing::instrument(level = "trace")]
fn render_sub_session_prompt(handoff_prompt: &str, persona_lines: &[String]) -> String {
    /*
     * sub-session rendering에는 의도적으로 runtime-context slot이 없다.
     * 유일한 task body는 queued handoff이므로, parallel worker가 ambient main-session context를 leased-worktree scope에
     * 우연히 섞을 수 없다.
     */
    PromptDocument::builder("akra-sub-session-turn")
        .lines("execution-contract", sub_session_execution_contract_lines())
        .lines("delivery-boundary", sub_session_delivery_boundary_lines())
        .lines("persona-prompt", persona_lines.to_vec())
        .lines("reporting-contract", sub_session_reporting_contract_lines())
        .text("queued-task-handoff", handoff_prompt)
        .build()
        .render()
}

#[cfg(test)]
mod tests {
    use super::{
        MainSessionPromptAssemblyRequest, ManualPromptAssemblyRequest, SubSessionAgentPrompt,
        SubSessionPromptAssemblyRequest, TurnPromptAssemblyService,
    };

    #[test]
    // manual prompt가 공백을 정리하고 runtime context를 렌더링하지 않는지 확인한다.
    // 이 테스트가 깨지면 TUI manual 실행에 hidden intake 전용 context가 섞일 수 있다.
    fn manual_prompt_is_trimmed_and_keeps_empty_planning_fragment_out() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_manual_prompt(ManualPromptAssemblyRequest {
            operator_prompt: "  ship it  ",
        });

        let rendered = prompt.expect("manual prompt should render");
        assert!(rendered.starts_with("# akra-main-session-turn\n"));
        assert!(rendered.contains("[execution-contract]"));
        assert!(rendered.contains("아래 `user-prompt`를 수행하세요."));
        assert!(rendered.ends_with("[user-prompt]\nship it"));
        assert!(!rendered.contains("[runtime-context]"));
    }

    #[test]
    // manual prompt는 planning fragment를 main-session prompt에 붙이지 않는다.
    // task authority context는 hidden intake/planning worker 쪽에서만 소비되어야 한다.
    fn manual_prompt_keeps_planning_fragment_out() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_manual_prompt(ManualPromptAssemblyRequest {
            operator_prompt: "ship it",
        });

        let rendered = prompt.expect("manual prompt should render");
        assert!(!rendered.contains("[runtime-context]"));
        assert!(!rendered.contains("Planning Context"));
        assert!(!rendered.contains("Queue Summary"));
        assert!(rendered.ends_with("[user-prompt]\nship it"));
    }

    #[test]
    // queue handoff가 main-session으로 들어갈 때도 일반 user prompt section으로 감싸지는지 확인한다.
    // main-session은 delivery 권한이 있으므로 결과 보고 형식 guardrail을 포함해야 한다.
    fn main_session_prompt_wraps_queue_handoff_as_user_prompt() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_main_session_prompt(MainSessionPromptAssemblyRequest {
            user_prompt: "# queued-task-handoff\n\n[task]\nintent=Continue",
        });

        let rendered = prompt.expect("queue prompt should render");
        assert!(rendered.starts_with("# akra-main-session-turn\n"));
        assert!(rendered.contains("`수정사항`에는 변경한 파일 위치와 핵심 변경을 적으세요."));
        assert!(
            rendered.ends_with("[user-prompt]\n# queued-task-handoff\n\n[task]\nintent=Continue")
        );
    }

    #[test]
    // sub-session prompt가 delivery 금지 guardrail과 handoff section을 함께 포함하는지 확인한다.
    // parallel worker가 이 문구를 잃으면 push/PR/merge를 직접 수행해 distributor의 통합 책임을 침범할 수 있다.
    fn sub_session_prompt_has_delivery_guardrails() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_sub_session_prompt(SubSessionPromptAssemblyRequest {
            handoff_prompt: "# queued-task-handoff\n\n[task]\nintent=Continue",
            agent_prompt: SubSessionAgentPrompt::default(),
        });

        let assembly = prompt.expect("sub-session prompt should render");
        let rendered = assembly.turn_prompt;
        assert!(rendered.starts_with("# akra-sub-session-turn\n"));
        assert!(rendered.contains("Akra sub-session"));
        assert!(rendered.contains("push, PR 생성, merge"));
        assert!(
            rendered.ends_with(
                "[queued-task-handoff]\n# queued-task-handoff\n\n[task]\nintent=Continue"
            )
        );
        assert!(!rendered.contains("[runtime-context]"));
        assert!(!rendered.contains("[persona-prompt]"));
        assert_eq!(assembly.service_name, "akra-parallel-worker");
        assert!(
            assembly
                .developer_instructions
                .contains("parallel task sub-session")
        );
        assert!(assembly.developer_instructions.contains("Do not push"));
        assert!(!assembly.developer_instructions.contains("Persona prompt:"));
    }

    #[test]
    // 선택된 agent profile의 persona prompt를 sub-session prompt와 developer metadata에 같은 응집 프롬프트로 주입한다.
    fn sub_session_prompt_includes_agent_profile_prompt() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_sub_session_prompt(SubSessionPromptAssemblyRequest {
            handoff_prompt: "# queued-task-handoff\n\n[task]\nintent=Continue",
            agent_prompt: SubSessionAgentPrompt::new(
                "아티피서 / 구현 담당",
                vec!["You are a careful implementation agent.".to_string()],
            ),
        });

        let assembly = prompt.expect("sub-session prompt should render");
        assert!(assembly.turn_prompt.contains("[persona-prompt]"));
        assert!(
            assembly
                .turn_prompt
                .contains("You are a careful implementation agent.")
        );
        assert!(
            assembly
                .developer_instructions
                .contains("Agent profile prompt: 아티피서 / 구현 담당")
        );
        assert!(
            assembly
                .developer_instructions
                .contains("You are a careful implementation agent.")
        );
    }
}
