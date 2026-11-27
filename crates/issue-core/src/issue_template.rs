/*
 * 대상 저장소의 이슈 템플릿에서 읽은 규약이다 (#2414, #2417). 템플릿의 경로와 판, 필수 입력을 가진다.
 *
 * 이 값은 템플릿이 요구하는 것만 말한다. 초안이 요구를 지키는지는 검증기(#2418)가 본다.
 */

/// 템플릿 파일의 형식이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IssueTemplateFormat {
    /// Markdown 템플릿이다 (`.md`).
    Markdown,
    /// YAML Issue Form 이다 (`.yml`, `.yaml`).
    IssueForm,
}

/// 필수 입력의 종류이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTemplateInputKind {
    /// Markdown 템플릿의 제목 아래 구간이다.
    Section,
    /// Issue Form 의 한 줄 입력이다 (`input`).
    Input,
    /// Issue Form 의 여러 줄 입력이다 (`textarea`).
    Textarea,
    /// Issue Form 의 선택 입력이다 (`dropdown`). 값은 `options` 중 하나여야 한다.
    Dropdown { options: Vec<String> },
    /// Issue Form 의 체크 상자 묶음(`checkboxes`)에서 꼭 체크해야 하는 항목 하나이다.
    Checkbox { option: String },
}

/// 템플릿이 요구하는 입력 하나이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTemplateRequiredInput {
    /// 게시본에서 이 입력의 제목이 되는 글이다 (Markdown 제목 또는 Issue Form 의 `label`).
    pub label: String,
    pub kind: IssueTemplateInputKind,
}

/// 템플릿이 요구하지만 Akra 가 해석하지 못하는 입력이다. 이 입력은 결정적 검증을 할 수 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTemplateUnsupportedInput {
    /// 입력을 가리키는 이름이다. `label`, 없으면 `id`, 없으면 `type` 이다.
    pub label: String,
    pub reason: String,
}

/// 템플릿 하나에서 읽은 이슈 규약이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTemplateConvention {
    /// 저장소 뿌리에서 본 템플릿의 상대 경로이다.
    pub path: String,
    /// 템플릿의 판이다. 파일 내용의 SHA-256 이다. 초안은 작성할 때의 이 값을 가진다.
    pub version: String,
    pub format: IssueTemplateFormat,
    pub required_inputs: Vec<IssueTemplateRequiredInput>,
    /// 지원 범위 밖의 필수 입력이다. 비어 있지 않으면 사용자가 직접 확인해야 한다.
    pub unsupported_required_inputs: Vec<IssueTemplateUnsupportedInput>,
}

/// 프로젝트의 템플릿을 읽은 결과이다. 실패의 종류를 구분한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTemplateResolution {
    /// 이슈 정책에 템플릿이 없다. 템플릿의 요구도 없다.
    NotConfigured,
    Resolved(IssueTemplateConvention),
    /// 정책이 가리킨 파일이 저장소에 없다.
    Missing {
        path: String,
    },
    /// 파일이 있지만 읽지 못했다.
    Unreadable {
        path: String,
        reason: String,
    },
    /// 파일을 읽었지만 지원하는 템플릿 형식으로 해석하지 못했다.
    Invalid {
        path: String,
        version: String,
        reason: String,
    },
}

impl IssueTemplateResolution {
    /// 지금 템플릿의 판이다. 읽지 못한 템플릿은 판이 없다.
    pub fn version(&self) -> Option<&str> {
        match self {
            Self::Resolved(convention) => Some(&convention.version),
            Self::Invalid { version, .. } => Some(version),
            Self::NotConfigured | Self::Missing { .. } | Self::Unreadable { .. } => None,
        }
    }
}

/// 게시 전에 초안의 템플릿 판을 지금 판과 비교한 결과이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTemplateVersionCheck {
    /// 초안을 쓴 뒤에 템플릿이 바뀌지 않았다.
    Unchanged,
    /// 템플릿이 바뀌었다. 사용자가 초안을 다시 검토해야 한다. 없는 판은 `None` 이다.
    ReviewRequired {
        drafted_version: Option<String>,
        current_version: Option<String>,
    },
}

/// `drafted_version` 은 초안을 쓸 때의 `IssueTemplateResolution::version` 이다.
pub fn check_issue_template_version(
    drafted_version: Option<&str>,
    current: &IssueTemplateResolution,
) -> IssueTemplateVersionCheck {
    let current_version = current.version();
    // 지금 템플릿을 읽지 못하면 판을 비교할 수 없다. 같다고 보지 않는다.
    let comparable = matches!(
        current,
        IssueTemplateResolution::NotConfigured
            | IssueTemplateResolution::Resolved(_)
            | IssueTemplateResolution::Invalid { .. }
    );
    if comparable && drafted_version == current_version {
        IssueTemplateVersionCheck::Unchanged
    } else {
        IssueTemplateVersionCheck::ReviewRequired {
            drafted_version: drafted_version.map(str::to_string),
            current_version: current_version.map(str::to_string),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IssueTemplateConvention, IssueTemplateFormat, IssueTemplateResolution,
        IssueTemplateVersionCheck, check_issue_template_version,
    };

    fn resolved(version: &str) -> IssueTemplateResolution {
        IssueTemplateResolution::Resolved(IssueTemplateConvention {
            path: ".github/ISSUE_TEMPLATE/bug.md".to_string(),
            version: version.to_string(),
            format: IssueTemplateFormat::Markdown,
            required_inputs: Vec::new(),
            unsupported_required_inputs: Vec::new(),
        })
    }

    #[test]
    fn the_same_template_version_needs_no_review() {
        assert_eq!(
            check_issue_template_version(Some("v1"), &resolved("v1")),
            IssueTemplateVersionCheck::Unchanged
        );
        assert_eq!(
            check_issue_template_version(None, &IssueTemplateResolution::NotConfigured),
            IssueTemplateVersionCheck::Unchanged
        );
    }

    #[test]
    fn a_changed_template_version_requests_review() {
        assert_eq!(
            check_issue_template_version(Some("v1"), &resolved("v2")),
            IssueTemplateVersionCheck::ReviewRequired {
                drafted_version: Some("v1".to_string()),
                current_version: Some("v2".to_string()),
            }
        );
        // 초안 뒤에 템플릿이 새로 생겼다.
        assert_eq!(
            check_issue_template_version(None, &resolved("v1")),
            IssueTemplateVersionCheck::ReviewRequired {
                drafted_version: None,
                current_version: Some("v1".to_string()),
            }
        );
    }

    #[test]
    fn a_template_that_cannot_be_read_now_requests_review() {
        let missing = IssueTemplateResolution::Missing {
            path: "bug.md".to_string(),
        };
        assert_eq!(
            check_issue_template_version(Some("v1"), &missing),
            IssueTemplateVersionCheck::ReviewRequired {
                drafted_version: Some("v1".to_string()),
                current_version: None,
            }
        );
        // 초안 때도 읽지 못했어도 같다고 보지 않는다.
        assert!(matches!(
            check_issue_template_version(None, &missing),
            IssueTemplateVersionCheck::ReviewRequired { .. }
        ));
    }
}
