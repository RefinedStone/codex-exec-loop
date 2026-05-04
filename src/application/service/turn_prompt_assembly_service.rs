#[derive(Debug, Clone, PartialEq, Eq)]
// `ManualPromptAssemblyRequest`는 사람이 TUI에서 직접 입력한 prompt를 main session용
// 실행 prompt로 감싸기 위한 요청이다. manual 입력도 queue에서 온 작업과 같은 main-session guardrail을 타야 하므로,
// 별도 타입으로 의미를 드러낸 뒤 내부에서는 `MainSessionPromptAssemblyRequest`로 변환한다.
pub struct ManualPromptAssemblyRequest<'a> {
    // operator가 입력한 원문이다. 서비스는 앞뒤 공백만 정리하고 의미를 재작성하지 않는다.
    pub operator_prompt: &'a str,
    // planning runtime이 현재 queue/readiness/context를 요약해 붙일 수 있는 선택 fragment이다.
    // 없거나 공백뿐이면 prompt에서 runtime context section 자체가 빠진다.
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `MainSessionPromptAssemblyRequest`는 실제 주 작업 세션에 들어갈 prompt 조립 요청이다.
// main session은 commit/push/PR/merge 같은 delivery 권한을 가진 흐름이므로, system prompt가 결과 형식과 지시 우선순위를
// 분명히 잡아 준다.
pub struct MainSessionPromptAssemblyRequest<'a> {
    // 사용자 요청 또는 distributor가 main session에 넘긴 queue handoff 본문이다.
    pub user_prompt: &'a str,
    // planning fragment는 user prompt보다 앞의 `runtime context` section에 들어간다.
    // 모델이 현재 계획/큐 상태를 먼저 읽고 그 다음 사용자 요청을 실행하게 하는 배치이다.
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// `SubSessionPromptAssemblyRequest`는 parallel mode의 leased worktree에서 실행될 하위 세션 prompt이다.
// sub session은 코드를 고치거나 작은 commit을 만들 수 있지만 delivery는 distributor가 담당하므로,
// main session과 다른 system prompt로 권한 경계를 강하게 제한한다.
pub struct SubSessionPromptAssemblyRequest<'a> {
    // distributor가 만든 queued-task handoff 원문이다. 이 값이 sub session의 유일한 작업 범위이다.
    pub handoff_prompt: &'a str,
}

#[derive(Debug, Clone, Default)]
// `TurnPromptAssemblyService`는 Codex turn에 실제로 넣을 최종 prompt 문자열을 만드는 application service이다.
// 상태를 들고 있지 않기 때문에 값 자체는 빈 struct이고, 호출자는 shared service 구성에서 cheap clone/default로 주입한다.
//
// 이 계층을 따로 둔 이유는 "사용자 입력 그대로"를 app-server에 보내지 않고, main/sub session별
// 시스템 지시와 runtime context를 일관되게 감싸기 위해서이다. prompt 정책이 흩어지면 병렬 세션 권한 경계가 쉽게 깨진다.
pub struct TurnPromptAssemblyService;

// main session system prompt이다. main session은 사용자의 실제 요청을 완료하고 최종 답변을 돌려주는
// 주 실행 경로이므로, 지시 충돌 해소 기준과 결과 보고 형식을 함께 지정한다.
const MAIN_SESSION_SYSTEM_PROMPT: &str = r#"아래 user prompt를 수행하세요.
기존 정책, 런타임 context, 사용자 요청이 충돌하면 더 구체적이고 최신인 지시를 우선하되 전체 의도를 하나의 실행 계획으로 통합하세요.
최종 답변은 간결하게 작성하고, 가능하면 다음 항목을 포함하세요.
- 수정사항: 변경한 파일 위치와 핵심 변경
- 결과: 실행/검증 결과
- 다음 추천: 성능개선, 추천수정, 우려되는 문제"#;

// sub session system prompt이다. 핵심은 "handoff 하나만 수행"과 "delivery 금지"이다.
// 하위 작업자가 shared branch rebase나 PR merge를 직접 수행하면 distributor의 통합 순서와 worktree 정리가 무너질 수 있다.
const SUB_SESSION_SYSTEM_PROMPT: &str = r#"아래 queued-task handoff만 수행하세요.
이 세션은 leased worktree에서 실행되는 Akra sub session입니다.
작업 범위는 handoff의 task 하나로 제한하고, 의미 있는 코드 변경이 있으면 작은 reviewable commit을 남기세요.
push, PR 생성, merge, shared branch rebase, worktree cleanup은 수행하지 마세요. 완료 후 Akra distributor가 delivery를 처리합니다.
최종 답변에는 변경 요약, 검증 결과, 남은 작업만 간결하게 포함하세요."#;

impl TurnPromptAssemblyService {
    // 상태 없는 서비스 생성자이다. shared service composition에서 다른 service와 같은 형태로 주입하기 위해
    // `new`를 제공하고, 테스트도 이 생성자를 통해 실제 production 경로와 같은 API를 사용한다.
    pub fn new() -> Self {
        Self
    }

    // manual prompt는 사람이 직접 입력한 요청을 main session prompt로 승격한다.
    // 별도 렌더러를 만들지 않고 main session 렌더러를 재사용해, manual 실행과 queue 실행이 같은 guardrail을 공유한다.
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn build_manual_prompt(&self, request: ManualPromptAssemblyRequest<'_>) -> Option<String> {
        /*
         * manual turn도 여전히 main-session turn이다.
         * build_main_session_prompt를 거치면 직접 operator 입력도 queue handoff와 같은 delivery/reporting 계약을 탄다.
         * 동시에 operator가 입력한 원문은 마지막 user prompt section으로 보존된다.
         */
        self.build_main_session_prompt(MainSessionPromptAssemblyRequest {
            // operator prompt는 main session 관점에서는 user prompt이다.
            user_prompt: request.operator_prompt,
            // planning fragment도 그대로 전달해 manual turn이 현재 planning context를 잃지 않게 한다.
            planning_prompt_fragment: request.planning_prompt_fragment,
        })
    }

    // main session prompt를 만든다. 반환이 `Option<String>`인 이유는 공백뿐인 user prompt를
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

        Some(render_main_session_prompt(
            MAIN_SESSION_SYSTEM_PROMPT,
            user_prompt,
            request.planning_prompt_fragment,
        ))
    }

    // sub session prompt를 만든다. sub session은 handoff 하나가 작업 범위이므로,
    // handoff가 비어 있으면 session을 시작하지 않는 것이 맞다.
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn build_sub_session_prompt(
        &self,
        // distributor가 lease한 slot에 전달할 handoff 요청이다.
        request: SubSessionPromptAssemblyRequest<'_>,
    ) -> Option<String> {
        // 빈 handoff는 범위 없는 sub session을 만들기 때문에 `None`으로 막는다.
        let handoff_prompt = request.handoff_prompt.trim();
        if handoff_prompt.is_empty() {
            /*
             * handoff 없는 sub-session은 bounded task가 없다.
             * 여기서 None을 돌리면 distributor가 generic system instruction만 보고 움직일 worker lane을 lease하지 않는다.
             */
            return None;
        }

        Some(render_sub_session_prompt(
            SUB_SESSION_SYSTEM_PROMPT,
            handoff_prompt,
        ))
    }
}

// main session prompt의 실제 문자열 레이아웃을 담당한다.
// 형식은 system prompt, 선택 runtime context, user prompt 순서이다. 이 순서는 모델이 전역 실행 규칙을 먼저 읽고,
// 현재 계획 상태를 다음에 읽은 뒤, 마지막으로 수행할 사용자 요청을 보도록 의도한 것이다.
#[tracing::instrument(level = "trace")]
fn render_main_session_prompt(
    // production에서는 `MAIN_SESSION_SYSTEM_PROMPT`를 넘기고, 함수 분리 덕분에 테스트나 미래 확장에서
    // 다른 system prompt를 주입해 렌더링 규칙만 따로 확인할 수 있다.
    system_prompt: &str,
    // 최종 prompt의 `user prompt:` section에 들어갈 실행 요청이다.
    user_prompt: &str,
    // 있을 때만 `runtime context:` section을 삽입할 planning fragment이다.
    planning_prompt_fragment: Option<&str>,
) -> String {
    // `String`에 순서대로 push해 정확한 구분자와 section 순서를 제어한다.
    // PromptDocument builder를 쓰지 않는 이유는 이 prompt는 `system prompt:` 같은 낮은 수준 label 형식을 이미 갖고 있어
    // 기존 app-server 입력과의 호환성을 유지해야 하기 때문이다.
    let mut result = String::new();
    result.push_str("system prompt:\n");
    result.push_str(system_prompt.trim());

    /*
     * runtime context는 optional이지만, 있으면 system prompt와 final user prompt 사이에 있어야 한다.
     * 이 위치는 model에게 현재 planning state를 제공하면서도 operator의 직접 요청이 마지막 concrete task instruction으로
     * 남게 한다.
     */
    // planning fragment가 없으면 runtime context section 전체를 생략한다.
    // 빈 section을 남기지 않는 정책은 prompt_component builder와 같은 방향이다.
    let Some(planning_prompt_fragment) = planning_prompt_fragment
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        // context가 없는 단순 경로이다. system prompt 다음에 곧바로 user prompt를 붙인다.
        result.push_str("\n\nuser prompt:\n");
        result.push_str(user_prompt.trim());
        return result;
    };

    // context가 있는 경로이다. runtime context를 user prompt보다 앞에 둬,
    // 사용자의 직접 요청을 마지막 section으로 남기면서도 현재 계획 상태를 실행 근거로 제공한다.
    result.push_str("\n\nruntime context:\n");
    result.push_str(planning_prompt_fragment);
    result.push_str("\n\nuser prompt:\n");
    result.push_str(user_prompt.trim());
    result
}

// sub session prompt의 문자열 레이아웃이다. main session과 달리 runtime context를 따로 받지 않고,
// `queued-task handoff:` 하나만 작업 범위로 전달한다.
#[tracing::instrument(level = "trace")]
fn render_sub_session_prompt(system_prompt: &str, handoff_prompt: &str) -> String {
    /*
     * sub-session rendering에는 의도적으로 runtime-context slot이 없다.
     * 유일한 task body는 queued handoff이므로, parallel worker가 ambient main-session context를 leased-worktree scope에
     * 우연히 섞을 수 없다.
     */
    // system prompt가 먼저 권한 제한을 선언하고, 그 다음 handoff가 실제 작업 내용을 제공한다.
    let mut result = String::new();
    result.push_str("system prompt:\n");
    result.push_str(system_prompt.trim());
    result.push_str("\n\nqueued-task handoff:\n");
    result.push_str(handoff_prompt.trim());
    result
}

#[cfg(test)]
mod tests {
    use super::{
        MainSessionPromptAssemblyRequest, ManualPromptAssemblyRequest,
        SubSessionPromptAssemblyRequest, TurnPromptAssemblyService,
    };

    #[test]
    // manual prompt가 공백을 정리하고 빈 planning fragment를 runtime context로 렌더링하지 않는지 확인한다.
    // 이 테스트가 깨지면 TUI에서 단순 manual 실행을 했을 때 불필요한 빈 context section이 agent 입력에 섞일 수 있다.
    fn manual_prompt_is_trimmed_and_keeps_empty_planning_fragment_out() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_manual_prompt(ManualPromptAssemblyRequest {
            operator_prompt: "  ship it  ",
            planning_prompt_fragment: Some("   "),
        });

        let rendered = prompt.expect("manual prompt should render");
        assert!(rendered.starts_with("system prompt:\n"));
        assert!(rendered.contains("아래 user prompt를 수행하세요."));
        assert!(rendered.ends_with("user prompt:\nship it"));
        assert!(!rendered.contains("runtime context:"));
    }

    #[test]
    // planning fragment가 있을 때 system prompt와 user prompt 사이에 runtime context로 들어가는지 확인한다.
    // 이 위치가 바뀌면 main session이 현재 계획 상태를 읽는 순서가 달라진다.
    fn manual_prompt_appends_planning_fragment_when_present() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_manual_prompt(ManualPromptAssemblyRequest {
            operator_prompt: "ship it",
            planning_prompt_fragment: Some("Planning Context\nQueue Summary"),
        });

        let rendered = prompt.expect("manual prompt should render");
        assert!(rendered.contains("\nruntime context:\nPlanning Context\nQueue Summary\n\n"));
        assert!(rendered.ends_with("user prompt:\nship it"));
    }

    #[test]
    // queue handoff가 main session으로 들어갈 때도 일반 user prompt section으로 감싸지는지 확인한다.
    // main session은 delivery 권한이 있으므로 결과 보고 형식 guardrail을 포함해야 한다.
    fn main_session_prompt_wraps_queue_handoff_as_user_prompt() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_main_session_prompt(MainSessionPromptAssemblyRequest {
            user_prompt: "# queued-task-handoff\n\n[task]\nintent=Continue",
            planning_prompt_fragment: None,
        });

        let rendered = prompt.expect("queue prompt should render");
        assert!(rendered.starts_with("system prompt:\n"));
        assert!(rendered.contains("- 수정사항: 변경한 파일 위치와 핵심 변경"));
        assert!(
            rendered.ends_with("user prompt:\n# queued-task-handoff\n\n[task]\nintent=Continue")
        );
    }

    #[test]
    // sub session prompt가 delivery 금지 guardrail과 handoff section을 함께 포함하는지 확인한다.
    // parallel worker가 이 문구를 잃으면 push/PR/merge를 직접 수행해 distributor의 통합 책임을 침범할 수 있다.
    fn sub_session_prompt_has_delivery_guardrails() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_sub_session_prompt(SubSessionPromptAssemblyRequest {
            handoff_prompt: "# queued-task-handoff\n\n[task]\nintent=Continue",
        });

        let rendered = prompt.expect("sub session prompt should render");
        assert!(rendered.starts_with("system prompt:\n"));
        assert!(rendered.contains("Akra sub session"));
        assert!(rendered.contains("push, PR 생성, merge"));
        assert!(
            rendered.ends_with(
                "queued-task handoff:\n# queued-task-handoff\n\n[task]\nintent=Continue"
            )
        );
    }
}
