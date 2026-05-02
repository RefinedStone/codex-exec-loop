#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptAssemblyRequest<'a> {
    pub operator_prompt: &'a str,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainSessionPromptAssemblyRequest<'a> {
    pub user_prompt: &'a str,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, Default)]
pub struct TurnPromptAssemblyService;

const MAIN_SESSION_SYSTEM_PROMPT: &str = r#"아래 user prompt를 수행하세요.
기존 정책, 런타임 context, 사용자 요청이 충돌하면 더 구체적이고 최신인 지시를 우선하되 전체 의도를 하나의 실행 계획으로 통합하세요.
최종 답변은 간결하게 작성하고, 가능하면 다음 항목을 포함하세요.
- 수정사항: 변경한 파일 위치와 핵심 변경
- 결과: 실행/검증 결과
- 다음 추천: 성능개선, 추천수정, 우려되는 문제"#;

impl TurnPromptAssemblyService {
    pub fn new() -> Self {
        Self
    }

    pub fn build_manual_prompt(&self, request: ManualPromptAssemblyRequest<'_>) -> Option<String> {
        self.build_main_session_prompt(MainSessionPromptAssemblyRequest {
            user_prompt: request.operator_prompt,
            planning_prompt_fragment: request.planning_prompt_fragment,
        })
    }

    pub fn build_main_session_prompt(
        &self,
        request: MainSessionPromptAssemblyRequest<'_>,
    ) -> Option<String> {
        let user_prompt = request.user_prompt.trim();
        if user_prompt.is_empty() {
            return None;
        }

        Some(render_main_session_prompt(
            MAIN_SESSION_SYSTEM_PROMPT,
            user_prompt,
            request.planning_prompt_fragment,
        ))
    }
}

fn render_main_session_prompt(
    system_prompt: &str,
    user_prompt: &str,
    planning_prompt_fragment: Option<&str>,
) -> String {
    let mut result = String::new();
    result.push_str("system prompt:\n");
    result.push_str(system_prompt.trim());

    let Some(planning_prompt_fragment) = planning_prompt_fragment
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        result.push_str("\n\nuser prompt:\n");
        result.push_str(user_prompt.trim());
        return result;
    };

    result.push_str("\n\nruntime context:\n");
    result.push_str(planning_prompt_fragment);
    result.push_str("\n\nuser prompt:\n");
    result.push_str(user_prompt.trim());
    result
}

#[cfg(test)]
mod tests {
    use super::{
        MainSessionPromptAssemblyRequest, ManualPromptAssemblyRequest, TurnPromptAssemblyService,
    };

    #[test]
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
}
