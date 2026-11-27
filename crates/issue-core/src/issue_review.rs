/*
 * LLM 품질 검토의 권고이다 (#2414, #2430). 권고는 글의 제안일 뿐이다. 이 모듈에는 게시, 복원, 검증 통과를
 * 만드는 값이 없다: `IssueValidationPassed` 와 `IssueSharedCopy` 를 만들지 않는다.
 *
 * 적용한 결과(`IssueReviewAppliedDraft`)는 로컬 초안이다. 공유본이 되려면 `mask_issue_text` 와
 * `validate_issue` 를 다시 지나야 한다.
 */
use serde::{Deserialize, Serialize};

/// 권고가 가리키는 칸이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueReviewField {
    Title,
    Body,
}

/// 권고 하나이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueReviewSuggestion {
    pub field: IssueReviewField,
    /// 품질 문제의 설명이다.
    pub problem: String,
    /// 칸에서 바꿀 글이다. 칸에 한 번만 있어야 한다.
    pub original_text: String,
    /// `original_text` 를 대신할 글이다.
    pub suggested_text: String,
}

/// 권고를 적용한 로컬 초안이다. 공유본이 아니다: 마스킹과 정책 검증을 다시 지나야 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueReviewAppliedDraft {
    pub title: String,
    pub body: String,
}

/// 권고를 적용할 수 없는 이유이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueReviewApplyError {
    /// `original_text` 가 비었거나 칸에 없다.
    OriginalTextNotFound,
    /// `original_text` 가 칸에 두 번 이상 있다. 바꿀 자리를 정할 수 없다.
    OriginalTextAmbiguous,
}

/// 권고 하나를 초안에 적용한다. 초안은 바꾸지 않고 새 로컬 초안을 준다.
pub fn apply_issue_review_suggestion(
    title: &str,
    body: &str,
    suggestion: &IssueReviewSuggestion,
) -> Result<IssueReviewAppliedDraft, IssueReviewApplyError> {
    let text = match suggestion.field {
        IssueReviewField::Title => title,
        IssueReviewField::Body => body,
    };
    let original = suggestion.original_text.as_str();
    if original.is_empty() {
        return Err(IssueReviewApplyError::OriginalTextNotFound);
    }
    let position = text
        .find(original)
        .ok_or(IssueReviewApplyError::OriginalTextNotFound)?;
    // Start after one character, not after the whole match: matches may overlap.
    let next_character = position + text[position..].chars().next().unwrap().len_utf8();
    if text[next_character..].contains(original) {
        return Err(IssueReviewApplyError::OriginalTextAmbiguous);
    }
    let mut replaced = text.to_string();
    replaced.replace_range(
        position..position + original.len(),
        &suggestion.suggested_text,
    );
    Ok(match suggestion.field {
        IssueReviewField::Title => IssueReviewAppliedDraft {
            title: replaced,
            body: body.to_string(),
        },
        IssueReviewField::Body => IssueReviewAppliedDraft {
            title: title.to_string(),
            body: replaced,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suggestion(
        field: IssueReviewField,
        original: &str,
        suggested: &str,
    ) -> IssueReviewSuggestion {
        IssueReviewSuggestion {
            field,
            problem: "unclear".to_string(),
            original_text: original.to_string(),
            suggested_text: suggested.to_string(),
        }
    }

    #[test]
    fn suggestion_replaces_one_excerpt_of_its_field_only() {
        let applied = apply_issue_review_suggestion(
            "Crash",
            "## Steps\n\n1. Crash it\n",
            &suggestion(IssueReviewField::Title, "Crash", "Crash on start"),
        )
        .expect("applied");

        assert_eq!(applied.title, "Crash on start");
        assert_eq!(applied.body, "## Steps\n\n1. Crash it\n");
    }

    #[test]
    fn missing_empty_or_repeated_excerpt_is_not_applied() {
        let body = "run\nrun\n";

        assert_eq!(
            apply_issue_review_suggestion(
                "T",
                body,
                &suggestion(IssueReviewField::Body, "stop", "x")
            ),
            Err(IssueReviewApplyError::OriginalTextNotFound)
        );
        assert_eq!(
            apply_issue_review_suggestion("T", body, &suggestion(IssueReviewField::Body, "", "x")),
            Err(IssueReviewApplyError::OriginalTextNotFound)
        );
        assert_eq!(
            apply_issue_review_suggestion(
                "T",
                body,
                &suggestion(IssueReviewField::Body, "run", "x")
            ),
            Err(IssueReviewApplyError::OriginalTextAmbiguous)
        );
    }
    #[test]
    fn overlapping_excerpts_are_ambiguous_including_multibyte_text() {
        for (body, original) in [("....", "..."), ("가가가", "가가"), ("😀😀😀", "😀😀")]
        {
            assert_eq!(
                apply_issue_review_suggestion(
                    "T",
                    body,
                    &suggestion(IssueReviewField::Body, original, "replacement"),
                ),
                Err(IssueReviewApplyError::OriginalTextAmbiguous),
            );
        }
        let applied = apply_issue_review_suggestion(
            "T",
            "앞 가가 뒤",
            &suggestion(IssueReviewField::Body, "가가", "완료"),
        )
        .unwrap();
        assert_eq!(applied.body, "앞 완료 뒤");
    }
}
