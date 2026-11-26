use std::fmt;

use crate::application::service::issue_code_validator::IssueCodeValidatorResult;
use crate::domain::issue_validation::{
    IssueRequirements, IssueValidationError, IssueValidationPassed, validate_issue,
};
use crate::domain::issue_work::IssueSharedCopy;

/*
 * 이슈 검증의 두 진입점이다 (#2418): 초안 검증과 게시 관문. 둘 다 domain 의 `validate_issue` 를 부른다.
 *
 * 게시 관문은 외부 쓰기만 막는다. 공유본 조회와 로컬 편집은 이 관문을 지나지 않는다.
 */

/// 게시를 막은 이유이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssuePublishBlocked {
    /// 필수 검증기가 없다: 정책이나 템플릿 요구 사항을 만들지 못했다.
    ValidatorAbsent,
    /// 검증이 실패했다. 필드별 오류이다.
    Invalid(Vec<IssueValidationError>),
    /// 필수 코드 검증기가 거부했거나 실행되지 않았다 (#2431).
    CodeValidator(Vec<IssueCodeValidatorResult>),
    /// 사용자가 결과 확정 전에 게시 검증을 취소했다 (#2723). 늦게 끝난 성공도 쓰지 않는다.
    Cancelled,
}

impl fmt::Display for IssuePublishBlocked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ValidatorAbsent => {
                write!(
                    formatter,
                    "issue publish blocked: required validator is absent"
                )
            }
            Self::Invalid(errors) => write!(
                formatter,
                "issue publish blocked: {} validation error(s)",
                errors.len()
            ),
            Self::CodeValidator(results) => write!(
                formatter,
                "issue publish blocked: {} required code validator(s) did not pass",
                results.len()
            ),
            Self::Cancelled => write!(
                formatter,
                "issue publish blocked: the user cancelled the code validation"
            ),
        }
    }
}

impl std::error::Error for IssuePublishBlocked {}

/// 초안을 검사한다. 결과는 표시용이다. 실패해도 로컬 편집은 이어갈 수 있다.
pub fn validate_issue_draft(
    draft: &IssueSharedCopy,
    requirements: &IssueRequirements,
) -> Result<IssueValidationPassed, Vec<IssueValidationError>> {
    validate_issue(draft, requirements)
}

/// 게시 직전의 관문이다. 외부 쓰기는 이 함수가 준 `IssueValidationPassed` 의 글로만 한다.
/// `requirements` 가 `None` 이면 검증기가 없는 것이고 게시를 막는다.
pub fn gate_issue_publish(
    shared_copy: &IssueSharedCopy,
    requirements: Option<&IssueRequirements>,
) -> Result<IssueValidationPassed, IssuePublishBlocked> {
    let requirements = requirements.ok_or(IssuePublishBlocked::ValidatorAbsent)?;
    validate_issue(shared_copy, requirements).map_err(IssuePublishBlocked::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_validation::{IssueValidationErrorKind, IssueValidationField};

    fn copy(title: &str, body: &str) -> IssueSharedCopy {
        IssueSharedCopy {
            title: title.to_string(),
            body: body.to_string(),
        }
    }

    fn requirements() -> IssueRequirements {
        IssueRequirements {
            template_fields: Vec::new(),
            required_sections: vec!["Steps".to_string()],
        }
    }

    #[test]
    fn draft_and_publish_gate_apply_the_same_rules() {
        let requirements = requirements();
        for shared_copy in [
            copy("Title", "## Steps\n\n1. run\n"),
            copy("", "no sections"),
        ] {
            let draft = validate_issue_draft(&shared_copy, &requirements);
            let gate = gate_issue_publish(&shared_copy, Some(&requirements));

            assert_eq!(draft.map_err(IssuePublishBlocked::Invalid), gate);
        }
    }

    #[test]
    fn publish_is_blocked_when_validation_fails() {
        let blocked = gate_issue_publish(&copy("Title", "no sections"), Some(&requirements()))
            .expect_err("blocked");

        assert_eq!(
            blocked,
            IssuePublishBlocked::Invalid(vec![IssueValidationError {
                field: IssueValidationField::RequiredSection("Steps".to_string()),
                kind: IssueValidationErrorKind::Missing,
            }])
        );
    }

    #[test]
    fn publish_is_blocked_when_the_validator_is_absent() {
        let valid = copy("Title", "## Steps\n\n1. run\n");

        assert_eq!(
            gate_issue_publish(&valid, None),
            Err(IssuePublishBlocked::ValidatorAbsent)
        );
    }

    #[test]
    fn passed_result_carries_exactly_the_validated_text() {
        let valid = copy("Title", "## Steps\n\n1. run\n");

        let passed = gate_issue_publish(&valid, Some(&requirements())).expect("allowed");

        assert_eq!(passed.shared_copy(), &valid);
    }

    #[test]
    fn blocked_publish_does_not_stop_draft_validation_or_change_the_text() {
        // 관문은 외부 쓰기만 막는다: 공유본은 그대로이고 초안 검증(로컬 편집)은 계속 답한다.
        let stored = copy("Title", "## Steps\n\n1. run\n");
        let before = stored.clone();

        assert!(gate_issue_publish(&stored, None).is_err());

        assert_eq!(stored, before);
        assert!(validate_issue_draft(&stored, &requirements()).is_ok());
    }
}
