#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualPromptAssemblyRequest<'a> {
    pub operator_prompt: &'a str,
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

#[cfg(test)]
mod tests {
    use super::{ManualPromptAssemblyRequest, TurnPromptAssemblyService};

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
}
