use serde::{Deserialize, Serialize};

use crate::application::port::outbound::issue_review_model_port::{
    IssueReviewModelPort, IssueReviewModelRequest,
};
use crate::domain::issue_review::IssueReviewSuggestion;
// 사용자가 고른 권고의 적용이다. 클라이언트는 이 서비스로 검토와 적용을 함께 쓴다.
pub use crate::domain::issue_review::{
    IssueReviewAppliedDraft, IssueReviewApplyError, apply_issue_review_suggestion,
};
use crate::domain::issue_validation::IssueValidationPassed;

/*
 * 검증된 공유 초안의 품질을 모델로 검토한다 (#2430).
 *
 * 입력은 `IssueValidationPassed` 뿐이다: 가리고 필수 검증을 통과한 글만 모델에 간다. 출력은 권고뿐이다.
 * 이 서비스는 게시, 복원, 검증 규칙 변경을 하지 않고 `IssueValidationPassed` 를 만들지 않는다.
 * 사용자가 권고를 적용하면 (`apply_issue_review_suggestion`) 그 결과는 로컬 초안이고, 마스킹과
 * 정책 검증을 다시 지난다.
 */

/// 검토 스킬의 지시문이다.
pub const ISSUE_DRAFT_REVIEW_SKILL_TEXT: &str =
    include_str!("../../../assets/app-server/skills/akra-issue-draft-review/SKILL.md");
pub(crate) const ISSUE_DRAFT_REVIEW_FEATURE: &str = "issue_draft_review";

#[derive(Serialize)]
struct IssueReviewDraftInput<'a> {
    title: &'a str,
    body: &'a str,
}

/// 모델의 답이다. `refusal` 이 있으면 거부이다.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IssueReviewModelReply {
    refusal: Option<String>,
    suggestions: Option<Vec<IssueReviewSuggestion>>,
}

/// 검토 한 번의 결과이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IssueReviewOutcome {
    /// 구조화된 권고이다. 빈 목록은 "문제 없음"이다.
    Suggestions(Vec<IssueReviewSuggestion>),
    /// 모델이 검토를 거부했다.
    Refused { reason: String },
    /// 모델을 부르지 못했거나 답을 해석하지 못했다.
    Failed { reason: String },
}

/// 검증을 통과한 공유 초안을 검토한다. 실패와 거부는 초안, 검증 결과, 게시 상태를 바꾸지 않는다.
pub fn review_issue_draft(
    passed: &IssueValidationPassed,
    model: &dyn IssueReviewModelPort,
) -> IssueReviewOutcome {
    match model.review(model_request(passed)) {
        Ok(reply) => parse_model_reply(&reply),
        Err(error) => IssueReviewOutcome::Failed {
            reason: format!("the review model failed: {error:#}"),
        },
    }
}

pub(crate) fn model_request(passed: &IssueValidationPassed) -> IssueReviewModelRequest {
    let shared_copy = passed.shared_copy();
    IssueReviewModelRequest {
        instructions: ISSUE_DRAFT_REVIEW_SKILL_TEXT.to_owned(),
        draft_json: serde_json::to_string(&IssueReviewDraftInput {
            title: &shared_copy.title,
            body: &shared_copy.body,
        })
        .expect("issue review input is plain text fields"),
    }
}

pub(crate) fn parse_model_reply(reply: &str) -> IssueReviewOutcome {
    // 모델은 JSON 을 코드 울타리로 감싸기도 한다.
    let json = reply
        .trim()
        .trim_start_matches("```json")
        .trim_matches('`')
        .trim();
    match serde_json::from_str::<IssueReviewModelReply>(json) {
        Ok(IssueReviewModelReply {
            refusal: Some(reason),
            ..
        }) => IssueReviewOutcome::Refused { reason },
        Ok(IssueReviewModelReply {
            suggestions: Some(suggestions),
            ..
        }) => IssueReviewOutcome::Suggestions(suggestions),
        Ok(_) => IssueReviewOutcome::Failed {
            reason: "the review reply has no suggestions and no refusal".to_string(),
        },
        Err(error) => IssueReviewOutcome::Failed {
            reason: format!("the review reply is not valid: {error}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::issue_review_model_fake::FakeIssueReviewModelAdapter;
    use crate::application::service::issue_masking::mask_issue_text;
    use crate::application::service::issue_validation::gate_issue_publish;
    use crate::domain::issue_policy::IssueDisclosurePolicy;
    use crate::domain::issue_review::IssueReviewField;
    use crate::domain::issue_validation::{IssueRequirements, validate_issue};
    use crate::domain::issue_work::IssueSharedCopy;

    fn requirements() -> IssueRequirements {
        IssueRequirements {
            template_fields: Vec::new(),
            required_sections: vec!["Steps".to_string()],
        }
    }

    fn passed() -> IssueValidationPassed {
        let shared_copy = IssueSharedCopy {
            title: "Crash".to_string(),
            body: "## Steps\n\n1. Run it with [masked-1]\n".to_string(),
        };
        validate_issue(&shared_copy, &requirements()).expect("valid draft")
    }

    fn suggestions_reply(original: &str, suggested: &str) -> String {
        serde_json::json!({
            "suggestions": [{
                "field": "body",
                "problem": "The steps do not say which command fails.",
                "originalText": original,
                "suggestedText": suggested,
            }]
        })
        .to_string()
    }

    #[test]
    fn model_gets_only_the_validated_shared_draft_and_returns_structured_suggestions() {
        let model =
            FakeIssueReviewModelAdapter::replying(&suggestions_reply("1. Run it", "1. Run `akra`"));

        let outcome = review_issue_draft(&passed(), &model);

        let IssueReviewOutcome::Suggestions(suggestions) = outcome else {
            panic!("expected suggestions: {outcome:?}");
        };
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].field, IssueReviewField::Body);
        assert_eq!(suggestions[0].original_text, "1. Run it");
        assert_eq!(suggestions[0].suggested_text, "1. Run `akra`");
        let requests = model.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].instructions, ISSUE_DRAFT_REVIEW_SKILL_TEXT);
        assert_eq!(
            requests[0].draft_json,
            r###"{"title":"Crash","body":"## Steps\n\n1. Run it with [masked-1]\n"}"###
        );
    }

    #[test]
    fn refusal_returns_the_reason_and_no_suggestions() {
        let model = FakeIssueReviewModelAdapter::replying(
            "```json\n{\"refusal\": \"The draft is not an issue report.\"}\n```",
        );

        assert_eq!(
            review_issue_draft(&passed(), &model),
            IssueReviewOutcome::Refused {
                reason: "The draft is not an issue report.".to_string()
            }
        );
    }

    #[test]
    fn model_failure_and_invalid_replies_fail_without_changing_the_draft() {
        let before = passed();
        let failing = FakeIssueReviewModelAdapter::failing("model is unavailable");
        let outcome = review_issue_draft(&before, &failing);
        assert!(
            matches!(&outcome, IssueReviewOutcome::Failed { reason } if reason.contains("model is unavailable")),
            "{outcome:?}"
        );

        for reply in [
            "I think the draft is fine.",
            "{}",
            // 권고는 글만 바꾼다. 게시나 검증 변경 같은 다른 칸이 있는 답은 받지 않는다.
            r#"{"suggestions": [], "publish": true}"#,
            r#"{"suggestions": [{"field": "labels", "problem": "p", "originalText": "a", "suggestedText": "b"}]}"#,
            r#"{"suggestions": [{"field": "body", "problem": "p", "originalText": "a", "suggestedText": "b", "requiredValidation": false}]}"#,
        ] {
            let model = FakeIssueReviewModelAdapter::replying(reply);
            assert!(
                matches!(
                    review_issue_draft(&before, &model),
                    IssueReviewOutcome::Failed { .. }
                ),
                "{reply}"
            );
        }
        assert_eq!(before, passed());
    }

    #[test]
    fn applied_suggestion_is_a_new_draft_that_passes_masking_and_validation_again() {
        let passed = passed();
        let shared_copy = passed.shared_copy();
        let apply = |original: &str, suggested: &str| {
            let model =
                FakeIssueReviewModelAdapter::replying(&suggestions_reply(original, suggested));
            let IssueReviewOutcome::Suggestions(suggestions) = review_issue_draft(&passed, &model)
            else {
                panic!("expected suggestions");
            };
            let applied = apply_issue_review_suggestion(
                &shared_copy.title,
                &shared_copy.body,
                &suggestions[0],
            )
            .expect("applied");
            // 적용한 결과는 로컬 초안이다: 마스킹과 게시 관문을 다시 지난다.
            let masked = mask_issue_text(
                &applied.title,
                &applied.body,
                &[],
                &IssueDisclosurePolicy::builtin(),
            )
            .expect("masked");
            gate_issue_publish(&masked.shared_copy, Some(&requirements()))
        };

        let allowed = apply("1. Run it", "1. Run `akra`").expect("valid after the edit");
        assert_eq!(
            allowed.shared_copy().body,
            "## Steps\n\n1. Run `akra` with [masked-1]\n"
        );
        // 필수 섹션을 없애는 권고는 적용해도 게시 관문을 지나지 못한다.
        assert!(apply("## Steps", "Steps").is_err());
        // 처음의 통과 결과는 그대로이다.
        assert_eq!(
            passed.shared_copy().body,
            "## Steps\n\n1. Run it with [masked-1]\n"
        );
    }
}
