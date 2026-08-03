const MAIN_SESSION_DOCUMENT_TITLE: &str = "# akra-main-session-turn";
const MANUAL_INTAKE_DOCUMENT_TITLE: &str = "# manual-intake-task-handoff";
const USER_PROMPT_SECTION: &str = "\n[user-prompt]\n";
const DESCRIPTION_USER_PROMPT_SECTION: &str = "\n[description]\nUser prompt:\n\n";
const ORIGINAL_USER_PROMPT_SECTION: &str = "\n[original-user-prompt]\n";
const MANUAL_INTAKE_RULES_SECTION: &str = "\n[rules]\n";

/// Projects an Akra-generated app-server prompt envelope back to the operator
/// text that belongs in the visible conversation transcript.
///
/// The app-server durably stores the actual `userMessage` sent to Codex. Akra's
/// main-session request intentionally contains execution/reporting contracts,
/// but those contracts are transport input rather than user-authored history.
/// Snapshot hydration therefore unwraps only the exact document grammar Akra
/// emits and otherwise fails closed by returning the original wire text.
pub(super) fn project_akra_user_message(raw_text: String) -> String {
    visible_akra_user_prompt(&raw_text)
        .map(str::to_string)
        .unwrap_or(raw_text)
}

pub(super) fn visible_akra_user_prompt(raw_text: &str) -> Option<&str> {
    if !has_document_title(raw_text, MAIN_SESSION_DOCUMENT_TITLE) {
        return None;
    }

    // `user-prompt` is deliberately the final main-session section, so the
    // body may itself contain bracketed Markdown without being truncated as a
    // synthetic outer section.
    let user_prompt = raw_text
        .split_once(USER_PROMPT_SECTION)
        .map(|(_, body)| body.trim())
        .filter(|body| !body.is_empty())?;

    if !has_document_title(user_prompt, MANUAL_INTAKE_DOCUMENT_TITLE) {
        return Some(user_prompt);
    }

    visible_manual_intake_user_prompt(user_prompt)
}

fn visible_manual_intake_user_prompt(user_prompt: &str) -> Option<&str> {
    // Hidden ManualPromptIntake stores the operator prompt twice: once in the
    // generated description and once in `original-user-prompt`. Matching those
    // copies identifies the generated boundary without mistaking user-authored
    // `[original-user-prompt]` or `[rules]` Markdown for transport structure.
    let before_generated_rules = user_prompt
        .rsplit_once(MANUAL_INTAKE_RULES_SECTION)
        .map(|(body, _)| body)?;
    if let Some((_, duplicated_prompt_body)) =
        before_generated_rules.split_once(DESCRIPTION_USER_PROMPT_SECTION)
    {
        for (marker_index, _) in duplicated_prompt_body.match_indices(ORIGINAL_USER_PROMPT_SECTION)
        {
            let description_prompt = duplicated_prompt_body[..marker_index].trim();
            let original_prompt =
                duplicated_prompt_body[marker_index + ORIGINAL_USER_PROMPT_SECTION.len()..].trim();
            if !original_prompt.is_empty() && description_prompt == original_prompt {
                return Some(original_prompt);
            }
        }
    }

    // Older/minimal envelopes without the duplicated description remain safe
    // only when exactly one original-prompt delimiter exists.
    let mut markers = before_generated_rules.match_indices(ORIGINAL_USER_PROMPT_SECTION);
    let (marker_index, _) = markers.next()?;
    if markers.next().is_some() {
        return None;
    }
    before_generated_rules
        .get(marker_index + ORIGINAL_USER_PROMPT_SECTION.len()..)
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
}

fn has_document_title(text: &str, title: &str) -> bool {
    text == title || text.starts_with(&format!("{title}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_user_messages_are_not_reinterpreted() {
        let text = "Please inspect [user-prompt] literally.";

        assert_eq!(visible_akra_user_prompt(text), None);
        assert_eq!(project_akra_user_message(text.to_string()), text);
    }

    #[test]
    fn main_session_envelope_projects_only_its_final_user_prompt() {
        let raw = concat!(
            "# akra-main-session-turn\n\n",
            "[execution-contract]\ninternal execution rule\n\n",
            "[reporting-contract]\ninternal reporting rule\n\n",
            "[user-prompt]\n실제 사용자 요청\n[bracketed-user-content]"
        );

        assert_eq!(
            visible_akra_user_prompt(raw),
            Some("실제 사용자 요청\n[bracketed-user-content]")
        );
    }

    #[test]
    fn manual_intake_envelope_restores_the_original_operator_prompt() {
        let raw = concat!(
            "# akra-main-session-turn\n\n",
            "[execution-contract]\ninternal execution rule\n\n",
            "[reporting-contract]\ninternal reporting rule\n\n",
            "[user-prompt]\n",
            "# manual-intake-task-handoff\n\n",
            "[task]\nintent=Execute the hidden task.\n\n",
            "[description]\nUser prompt:\n\n안녕하세요 event 관련 rs 파일 두개만 찾아줄래요?\n\n",
            "[original-user-prompt]\n안녕하세요 event 관련 rs 파일 두개만 찾아줄래요?\n\n",
            "[rules]\n- Keep authority unchanged."
        );

        let projected = project_akra_user_message(raw.to_string());

        assert_eq!(
            projected,
            "안녕하세요 event 관련 rs 파일 두개만 찾아줄래요?"
        );
        assert!(!projected.contains("execution-contract"));
        assert!(!projected.contains("manual-intake-task-handoff"));
    }

    #[test]
    fn manual_intake_projection_preserves_section_like_user_markdown() {
        let operator_prompt = concat!(
            "첫 번째 문단\n",
            "[original-user-prompt]\n",
            "이 표시는 사용자 본문입니다.\n",
            "[rules]\n",
            "이 규칙도 사용자 본문입니다."
        );
        let raw = format!(
            concat!(
                "# akra-main-session-turn\n\n",
                "[execution-contract]\ninternal execution rule\n\n",
                "[reporting-contract]\ninternal reporting rule\n\n",
                "[user-prompt]\n",
                "# manual-intake-task-handoff\n\n",
                "[description]\nUser prompt:\n\n{0}\n\n",
                "[original-user-prompt]\n{0}\n\n",
                "[rules]\n- Keep authority unchanged."
            ),
            operator_prompt
        );

        assert_eq!(visible_akra_user_prompt(&raw), Some(operator_prompt));
    }
}
