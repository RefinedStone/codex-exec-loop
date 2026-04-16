use super::planning_auto_follow_copy::PLANNING_AUTO_FOLLOW_REFRESH_QUEUE_BODY;

pub(crate) const PREVIEW_LAST_MESSAGE_PLACEHOLDER: &str = "(waiting for next agent reply)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptAssemblyRequest<'a> {
    pub operator_prompt: &'a str,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAutoFollowOperation {
    RefreshQueueFromLatestAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningAutoFollowPromptAssemblyRequest<'a> {
    pub operation: PlanningAutoFollowOperation,
    pub stop_keyword: &'a str,
    pub last_message: &'a str,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningAutoFollowPromptPreviewRequest<'a> {
    pub operation: PlanningAutoFollowOperation,
    pub stop_keyword: &'a str,
    pub last_message: Option<&'a str>,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, Default)]
pub struct TurnPromptAssemblyService;

impl TurnPromptAssemblyService {
    pub fn new() -> Self {
        Self
    }

    pub fn build_manual_prompt(&self, request: ManualPromptAssemblyRequest<'_>) -> Option<String> {
        let operator_prompt = request.operator_prompt.trim();
        if operator_prompt.is_empty() {
            return None;
        }

        Some(append_planning_fragment(
            operator_prompt.to_string(),
            request.planning_prompt_fragment,
        ))
    }

    pub fn build_planning_auto_follow_prompt(
        &self,
        request: PlanningAutoFollowPromptAssemblyRequest<'_>,
    ) -> String {
        append_planning_fragment(
            render_planning_auto_follow_body(
                planning_auto_follow_operation_body(request.operation),
                request.stop_keyword,
                request.last_message,
            ),
            request.planning_prompt_fragment,
        )
    }

    pub fn build_planning_auto_follow_prompt_preview(
        &self,
        request: PlanningAutoFollowPromptPreviewRequest<'_>,
    ) -> String {
        self.build_planning_auto_follow_prompt(PlanningAutoFollowPromptAssemblyRequest {
            operation: request.operation,
            stop_keyword: request.stop_keyword,
            last_message: normalized_preview_last_message(request.last_message),
            planning_prompt_fragment: request.planning_prompt_fragment,
        })
    }
}

fn render_planning_auto_follow_body(
    prompt_body: &str,
    stop_keyword: &str,
    last_message: &str,
) -> String {
    prompt_body
        .replace("{stop_keyword}", stop_keyword)
        .replace("{last_message}", last_message)
}

fn append_planning_fragment(
    rendered_prompt: String,
    planning_prompt_fragment: Option<&str>,
) -> String {
    let Some(planning_prompt_fragment) = planning_prompt_fragment
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return rendered_prompt;
    };

    let mut result = rendered_prompt;
    let trimmed_len = result.trim_end().len();
    result.truncate(trimmed_len);
    if !result.is_empty() {
        result.push_str("\n\n");
    }
    result.push_str(planning_prompt_fragment);
    result
}

fn planning_auto_follow_operation_body(operation: PlanningAutoFollowOperation) -> &'static str {
    match operation {
        PlanningAutoFollowOperation::RefreshQueueFromLatestAnswer => {
            PLANNING_AUTO_FOLLOW_REFRESH_QUEUE_BODY
        }
    }
}

fn normalized_preview_last_message(last_message: Option<&str>) -> &str {
    last_message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(PREVIEW_LAST_MESSAGE_PLACEHOLDER)
}

#[cfg(test)]
mod tests {
    use super::{
        ManualPromptAssemblyRequest, PlanningAutoFollowOperation,
        PlanningAutoFollowPromptAssemblyRequest, PlanningAutoFollowPromptPreviewRequest,
        TurnPromptAssemblyService,
    };

    #[test]
    fn manual_prompt_is_trimmed_and_keeps_empty_planning_fragment_out() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_manual_prompt(ManualPromptAssemblyRequest {
            operator_prompt: "  ship it  ",
            planning_prompt_fragment: Some("   "),
        });

        assert_eq!(prompt.as_deref(), Some("ship it"));
    }

    #[test]
    fn manual_prompt_appends_planning_fragment_when_present() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_manual_prompt(ManualPromptAssemblyRequest {
            operator_prompt: "ship it",
            planning_prompt_fragment: Some("Planning Context\nQueue Summary"),
        });

        assert_eq!(
            prompt.as_deref(),
            Some("ship it\n\nPlanning Context\nQueue Summary")
        );
    }

    #[test]
    fn planning_auto_follow_prompt_builds_queue_refresh_instruction() {
        let service = TurnPromptAssemblyService::new();

        let prompt =
            service.build_planning_auto_follow_prompt(PlanningAutoFollowPromptAssemblyRequest {
                operation: PlanningAutoFollowOperation::RefreshQueueFromLatestAnswer,
                stop_keyword: "AUTO_STOP",
                last_message: "latest answer",
                planning_prompt_fragment: Some("Planning Context"),
            });

        assert!(prompt.contains("planning priority queue"));
        assert!(prompt.contains("latest answer"));
        assert!(prompt.contains("작업 목록 전체를 queue에 반영하되"));
        assert!(prompt.contains("이번 턴에서는 가장 높은 우선순위의 executable task 1개만 수행"));
        assert!(prompt.ends_with("\n\nPlanning Context"));
    }

    #[test]
    fn planning_auto_follow_preview_uses_placeholder_last_message() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_planning_auto_follow_prompt_preview(
            PlanningAutoFollowPromptPreviewRequest {
                operation: PlanningAutoFollowOperation::RefreshQueueFromLatestAnswer,
                stop_keyword: "AUTO_STOP",
                last_message: None,
                planning_prompt_fragment: None,
            },
        );

        assert!(!prompt.contains("자동 후속"));
        assert!(prompt.contains("(waiting for next agent reply)"));
        assert!(prompt.contains("마지막 줄에 AUTO_STOP 만 출력하세요."));
    }
}
