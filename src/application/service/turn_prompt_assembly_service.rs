use crate::domain::followup_template::FollowupTemplateDefinition;

use super::planning_auto_follow_copy::PLANNING_AUTO_FOLLOW_REFRESH_QUEUE_BODY;

pub(crate) const PREVIEW_THREAD_ID_PLACEHOLDER: &str = "draft-thread";
pub(crate) const PREVIEW_LAST_MESSAGE_PLACEHOLDER: &str = "(waiting for next agent reply)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptAssemblyRequest<'a> {
    pub operator_prompt: &'a str,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoFollowPromptAssemblyRequest<'a> {
    pub template: &'a FollowupTemplateDefinition,
    pub auto_turn: usize,
    pub max_auto_turns: usize,
    pub session_id: &'a str,
    pub stop_keyword: &'a str,
    pub last_message: &'a str,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoFollowPromptPreviewRequest<'a> {
    pub template: &'a FollowupTemplateDefinition,
    pub auto_turn: usize,
    pub max_auto_turns: usize,
    pub session_id: &'a str,
    pub stop_keyword: &'a str,
    pub last_message: Option<&'a str>,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningAutoFollowOperation {
    RefreshQueueFromLatestAnswer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningAutoFollowPromptAssemblyRequest<'a> {
    pub operation: PlanningAutoFollowOperation,
    pub auto_turn: usize,
    pub max_auto_turns: usize,
    pub session_id: &'a str,
    pub stop_keyword: &'a str,
    pub last_message: &'a str,
    pub planning_prompt_fragment: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningAutoFollowPromptPreviewRequest<'a> {
    pub operation: PlanningAutoFollowOperation,
    pub auto_turn: usize,
    pub max_auto_turns: usize,
    pub session_id: &'a str,
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

    pub fn build_auto_follow_prompt(&self, request: AutoFollowPromptAssemblyRequest<'_>) -> String {
        append_planning_fragment(
            render_template_body(
                request.template.body.as_str(),
                request.auto_turn,
                request.max_auto_turns,
                request.session_id,
                request.stop_keyword,
                request.last_message,
            ),
            request.planning_prompt_fragment,
        )
    }

    pub fn build_auto_follow_prompt_preview(
        &self,
        request: AutoFollowPromptPreviewRequest<'_>,
    ) -> String {
        let preview_session_id = normalized_preview_session_id(request.session_id);
        let preview_last_message = normalized_preview_last_message(request.last_message);

        self.build_auto_follow_prompt(AutoFollowPromptAssemblyRequest {
            template: request.template,
            auto_turn: request.auto_turn,
            max_auto_turns: request.max_auto_turns,
            session_id: preview_session_id,
            stop_keyword: request.stop_keyword,
            last_message: preview_last_message,
            planning_prompt_fragment: request.planning_prompt_fragment,
        })
    }

    pub fn build_planning_auto_follow_prompt(
        &self,
        request: PlanningAutoFollowPromptAssemblyRequest<'_>,
    ) -> String {
        append_planning_fragment(
            render_template_body(
                planning_auto_follow_operation_body(request.operation),
                request.auto_turn,
                request.max_auto_turns,
                request.session_id,
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
        let preview_session_id = normalized_preview_session_id(request.session_id);
        let preview_last_message = normalized_preview_last_message(request.last_message);

        self.build_planning_auto_follow_prompt(PlanningAutoFollowPromptAssemblyRequest {
            operation: request.operation,
            auto_turn: request.auto_turn,
            max_auto_turns: request.max_auto_turns,
            session_id: preview_session_id,
            stop_keyword: request.stop_keyword,
            last_message: preview_last_message,
            planning_prompt_fragment: request.planning_prompt_fragment,
        })
    }
}

fn render_template_body(
    template_body: &str,
    auto_turn: usize,
    max_auto_turns: usize,
    session_id: &str,
    stop_keyword: &str,
    last_message: &str,
) -> String {
    let max_auto_turns = render_max_auto_turns(max_auto_turns);
    template_body
        .replace("{auto_turn}", &auto_turn.to_string())
        .replace("{max_auto_turns}", &max_auto_turns)
        .replace("{session_id}", session_id)
        .replace("{stop_keyword}", stop_keyword)
        .replace("{last_message}", last_message)
}

fn render_max_auto_turns(max_auto_turns: usize) -> String {
    if max_auto_turns == usize::MAX {
        "infinite".to_string()
    } else {
        max_auto_turns.to_string()
    }
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

fn normalized_preview_session_id(session_id: &str) -> &str {
    if session_id.trim().is_empty() {
        PREVIEW_THREAD_ID_PLACEHOLDER
    } else {
        session_id
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
        AutoFollowPromptAssemblyRequest, AutoFollowPromptPreviewRequest,
        ManualPromptAssemblyRequest, PlanningAutoFollowOperation,
        PlanningAutoFollowPromptAssemblyRequest, PlanningAutoFollowPromptPreviewRequest,
        TurnPromptAssemblyService,
    };
    use crate::domain::followup_template::{FollowupTemplateDefinition, FollowupTemplateSource};

    fn sample_template() -> FollowupTemplateDefinition {
        FollowupTemplateDefinition {
            id: "builtin-next-task".to_string(),
            label: "builtin next-task".to_string(),
            body: "session={session_id}\nauto={auto_turn}/{max_auto_turns}\nlast={last_message}\nstop={stop_keyword}".to_string(),
            source: FollowupTemplateSource::Builtin,
        }
    }

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
    fn auto_follow_prompt_renders_template_and_appends_planning_fragment() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_auto_follow_prompt(AutoFollowPromptAssemblyRequest {
            template: &sample_template(),
            auto_turn: 2,
            max_auto_turns: 5,
            session_id: "thread-1",
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            planning_prompt_fragment: Some("Planning Context"),
        });

        assert!(prompt.contains("session=thread-1"));
        assert!(prompt.contains("auto=2/5"));
        assert!(prompt.contains("last=latest answer"));
        assert!(prompt.contains("stop=AUTO_STOP"));
        assert!(prompt.ends_with("\n\nPlanning Context"));
    }

    #[test]
    fn auto_follow_preview_uses_placeholders_for_blank_runtime_values() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_auto_follow_prompt_preview(AutoFollowPromptPreviewRequest {
            template: &sample_template(),
            auto_turn: 1,
            max_auto_turns: 3,
            session_id: "",
            stop_keyword: "AUTO_STOP",
            last_message: Some("   "),
            planning_prompt_fragment: None,
        });

        assert!(prompt.contains("session=draft-thread"));
        assert!(prompt.contains("auto=1/3"));
        assert!(prompt.contains("last=(waiting for next agent reply)"));
    }

    #[test]
    fn auto_follow_prompt_renders_infinite_limit_label() {
        let service = TurnPromptAssemblyService::new();

        let prompt = service.build_auto_follow_prompt(AutoFollowPromptAssemblyRequest {
            template: &sample_template(),
            auto_turn: 4,
            max_auto_turns: usize::MAX,
            session_id: "thread-1",
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            planning_prompt_fragment: None,
        });

        assert!(prompt.contains("auto=4/infinite"));
    }

    #[test]
    fn auto_follow_prompt_uses_fragment_without_leading_blank_lines_when_template_is_empty() {
        let service = TurnPromptAssemblyService::new();
        let template = FollowupTemplateDefinition {
            id: "empty".to_string(),
            label: "empty".to_string(),
            body: "   ".to_string(),
            source: FollowupTemplateSource::Builtin,
        };

        let prompt = service.build_auto_follow_prompt(AutoFollowPromptAssemblyRequest {
            template: &template,
            auto_turn: 1,
            max_auto_turns: 2,
            session_id: "thread-1",
            stop_keyword: "AUTO_STOP",
            last_message: "latest answer",
            planning_prompt_fragment: Some("Planning Context"),
        });

        assert_eq!(prompt, "Planning Context");
    }

    #[test]
    fn planning_auto_follow_prompt_builds_queue_refresh_instruction() {
        let service = TurnPromptAssemblyService::new();

        let prompt =
            service.build_planning_auto_follow_prompt(PlanningAutoFollowPromptAssemblyRequest {
                operation: PlanningAutoFollowOperation::RefreshQueueFromLatestAnswer,
                auto_turn: 1,
                max_auto_turns: 3,
                session_id: "thread-1",
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
                auto_turn: 1,
                max_auto_turns: 3,
                session_id: "",
                stop_keyword: "AUTO_STOP",
                last_message: None,
                planning_prompt_fragment: None,
            },
        );

        assert!(prompt.contains("자동 후속 1/3 입니다."));
        assert!(prompt.contains("(waiting for next agent reply)"));
    }
}
