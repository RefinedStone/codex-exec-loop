/*
 * 이슈의 필수 규약을 검사하는 순수한 규칙이다 (#2414, #2418). 시계, 난수, I/O 가 없다. 같은 제목, 본문,
 * 요구 사항은 같은 결과를 준다.
 *
 * 초안 검증과 게시 직전 검증은 둘 다 `validate_issue` 를 부른다. `IssueValidationPassed` 는 이 모듈만
 * 만든다: 필드가 비공개이고 `Deserialize` 가 없다. LLM 제안이나 클라이언트 입력은 이 값을 만들 수 없다.
 */
use crate::domain::issue_work::IssueSharedCopy;

/// GitHub 이슈 폼이 빈 답 대신 본문에 쓰는 글이다. 빈 값으로 본다.
const NO_RESPONSE_PLACEHOLDER: &str = "_No response_";

/// 필드 값에 허용하는 형식이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueFieldFormat {
    /// 한 줄이다.
    SingleLine,
    /// 목록의 값 중 하나이다.
    OneOf(Vec<String>),
}

/// 템플릿의 필드 하나에 대한 요구이다. 본문에서 필드는 `label` 과 같은 제목의 섹션이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueFieldRequirement {
    pub label: String,
    pub required: bool,
    /// 값이 있을 때 검사하는 형식이다.
    pub format: Option<IssueFieldFormat>,
}

/// 검증기의 입력이다. 템플릿 해석 결과(#2417)와 정책의 필수 섹션(#2416)을 이 값으로 옮긴다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueRequirements {
    pub template_fields: Vec<IssueFieldRequirement>,
    /// 프로젝트가 요구하는 섹션의 제목이다 (`IssueConvention::required_sections`).
    pub required_sections: Vec<String>,
}

/// 오류가 난 자리이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueValidationField {
    Title,
    /// 템플릿 필드이다. 값은 필드의 `label` 이다.
    TemplateField(String),
    /// 프로젝트 필수 섹션이다. 값은 섹션의 제목이다.
    RequiredSection(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueValidationErrorKind {
    /// 본문에 그 제목의 섹션이 없다.
    Missing,
    /// 값이 비어 있다.
    Empty,
    /// 값이 허용 형식이 아니다.
    InvalidFormat,
}

/// 필드 하나의 오류이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueValidationError {
    pub field: IssueValidationField,
    pub kind: IssueValidationErrorKind,
}

/// 검증을 통과한 공유본이다. `validate_issue` 만 만든다. 통과한 글과 묶여 있어서 통과 뒤에 글을 바꿀 수 없다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueValidationPassed {
    shared_copy: IssueSharedCopy,
}

impl IssueValidationPassed {
    /// 검증을 통과한 글이다.
    pub fn shared_copy(&self) -> &IssueSharedCopy {
        &self.shared_copy
    }
}

/// 제목과 본문을 요구 사항으로 검사한다. 오류의 순서는 제목, 템플릿 필드, 필수 섹션이다.
pub fn validate_issue(
    shared_copy: &IssueSharedCopy,
    requirements: &IssueRequirements,
) -> Result<IssueValidationPassed, Vec<IssueValidationError>> {
    let mut errors = Vec::new();
    if shared_copy.title.trim().is_empty() {
        errors.push(IssueValidationError {
            field: IssueValidationField::Title,
            kind: IssueValidationErrorKind::Empty,
        });
    }

    let sections = parse_sections(&shared_copy.body);
    for field in &requirements.template_fields {
        let value = section_value(&sections, &field.label);
        if let Some(kind) = field_error(field, value.as_deref()) {
            errors.push(IssueValidationError {
                field: IssueValidationField::TemplateField(field.label.clone()),
                kind,
            });
        }
    }
    for title in &requirements.required_sections {
        if let Some(kind) = presence_error(section_value(&sections, title).as_deref()) {
            errors.push(IssueValidationError {
                field: IssueValidationField::RequiredSection(title.clone()),
                kind,
            });
        }
    }

    if errors.is_empty() {
        Ok(IssueValidationPassed {
            shared_copy: shared_copy.clone(),
        })
    } else {
        Err(errors)
    }
}

fn field_error(
    field: &IssueFieldRequirement,
    value: Option<&str>,
) -> Option<IssueValidationErrorKind> {
    match presence_error(value) {
        Some(kind) => field.required.then_some(kind),
        None => {
            let value = value.unwrap_or_default();
            let valid = match &field.format {
                None => true,
                Some(IssueFieldFormat::SingleLine) => !value.contains('\n'),
                Some(IssueFieldFormat::OneOf(options)) => {
                    options.iter().any(|option| option.trim() == value)
                }
            };
            (!valid).then_some(IssueValidationErrorKind::InvalidFormat)
        }
    }
}

/// `value` 는 섹션의 다듬은 글이다. 섹션이 없으면 `None` 이다.
fn presence_error(value: Option<&str>) -> Option<IssueValidationErrorKind> {
    match value {
        None => Some(IssueValidationErrorKind::Missing),
        Some(value) if value.is_empty() || value == NO_RESPONSE_PLACEHOLDER => {
            Some(IssueValidationErrorKind::Empty)
        }
        Some(_) => None,
    }
}

struct Section<'a> {
    level: usize,
    title: &'a str,
    /// 이 제목 다음 줄부터 다음 제목 전까지의 줄이다.
    lines: Vec<&'a str>,
}

/// 본문을 Markdown 제목(`#` 1~6개)으로 나눈다. 코드 울타리 안의 `#` 는 제목이 아니다.
fn parse_sections(body: &str) -> Vec<Section<'_>> {
    let mut sections: Vec<Section<'_>> = Vec::new();
    let mut in_fence = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        match (in_fence, heading(trimmed)) {
            (false, Some((level, title))) => sections.push(Section {
                level,
                title,
                lines: Vec::new(),
            }),
            _ => {
                if let Some(section) = sections.last_mut() {
                    section.lines.push(line);
                }
            }
        }
    }
    sections
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    let rest = &line[level..];
    let is_heading = (1..=6).contains(&level) && (rest.is_empty() || rest.starts_with(' '));
    is_heading.then(|| (level, rest.trim().trim_end_matches('#').trim_end()))
}

/// `title` 섹션의 글이다. 하위 제목과 그 글을 포함하고, 같거나 높은 수준의 다음 제목에서 끝난다.
fn section_value(sections: &[Section<'_>], title: &str) -> Option<String> {
    let title = title.trim();
    let start = sections.iter().position(|section| section.title == title)?;
    let level = sections[start].level;
    let mut lines = sections[start].lines.clone();
    for section in sections[start + 1..]
        .iter()
        .take_while(|section| section.level > level)
    {
        lines.push(section.title);
        lines.extend(&section.lines);
    }
    Some(lines.join("\n").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(title: &str, body: &str) -> IssueSharedCopy {
        IssueSharedCopy {
            title: title.to_string(),
            body: body.to_string(),
        }
    }

    fn field(
        label: &str,
        required: bool,
        format: Option<IssueFieldFormat>,
    ) -> IssueFieldRequirement {
        IssueFieldRequirement {
            label: label.to_string(),
            required,
            format,
        }
    }

    fn requirements() -> IssueRequirements {
        IssueRequirements {
            template_fields: vec![
                field("Version", true, Some(IssueFieldFormat::SingleLine)),
                field(
                    "Severity",
                    true,
                    Some(IssueFieldFormat::OneOf(vec![
                        "low".to_string(),
                        "high".to_string(),
                    ])),
                ),
                field("Notes", false, Some(IssueFieldFormat::SingleLine)),
            ],
            required_sections: vec!["Steps".to_string()],
        }
    }

    const VALID_BODY: &str = "### Version\n\n1.2.3\n\n### Severity\n\nhigh\n\n### Notes\n\n_No response_\n\n## Steps\n\n1. run\n";

    fn error(field: IssueValidationField, kind: IssueValidationErrorKind) -> IssueValidationError {
        IssueValidationError { field, kind }
    }

    #[test]
    fn valid_issue_passes_and_keeps_the_validated_text() {
        let shared_copy = copy("Crash on start", VALID_BODY);

        let passed = validate_issue(&shared_copy, &requirements()).expect("passes");

        assert_eq!(passed.shared_copy(), &shared_copy);
    }

    #[test]
    fn each_broken_rule_returns_an_error_for_its_field() {
        let body = "### Version\n\n1.2.3\nand more\n\n### Severity\n\nurgent\n\n## Steps\n\n_No response_\n";

        let errors = validate_issue(&copy("  ", body), &requirements()).expect_err("fails");

        assert_eq!(
            errors,
            vec![
                error(IssueValidationField::Title, IssueValidationErrorKind::Empty),
                error(
                    IssueValidationField::TemplateField("Version".to_string()),
                    IssueValidationErrorKind::InvalidFormat
                ),
                error(
                    IssueValidationField::TemplateField("Severity".to_string()),
                    IssueValidationErrorKind::InvalidFormat
                ),
                error(
                    IssueValidationField::RequiredSection("Steps".to_string()),
                    IssueValidationErrorKind::Empty
                ),
            ]
        );
    }

    #[test]
    fn missing_required_field_and_section_are_reported() {
        let errors =
            validate_issue(&copy("Title", "free text"), &requirements()).expect_err("fails");

        assert_eq!(
            errors,
            vec![
                error(
                    IssueValidationField::TemplateField("Version".to_string()),
                    IssueValidationErrorKind::Missing
                ),
                error(
                    IssueValidationField::TemplateField("Severity".to_string()),
                    IssueValidationErrorKind::Missing
                ),
                error(
                    IssueValidationField::RequiredSection("Steps".to_string()),
                    IssueValidationErrorKind::Missing
                ),
            ]
        );
    }

    #[test]
    fn same_issue_and_requirements_give_the_same_result() {
        let shared_copy = copy("", "### Severity\n\nurgent\n");

        assert_eq!(
            validate_issue(&shared_copy, &requirements()),
            validate_issue(&shared_copy, &requirements())
        );
    }

    #[test]
    fn section_holds_its_subsections_and_ignores_headings_in_code_fences() {
        let body = "## Steps\n\n### First\n\n```\n## Steps\n# not a heading\n```\n\n## Other\n";
        let sections = parse_sections(body);

        assert_eq!(
            section_value(&sections, "Steps").as_deref(),
            Some("First\n\n```\n## Steps\n# not a heading\n```")
        );
        assert_eq!(section_value(&sections, "Other").as_deref(), Some(""));
        assert_eq!(section_value(&sections, "Absent"), None);
    }

    #[test]
    fn empty_requirements_only_check_the_title() {
        let requirements = IssueRequirements::default();

        assert!(validate_issue(&copy("Title", ""), &requirements).is_ok());
        assert!(validate_issue(&copy("", ""), &requirements).is_err());
    }
}
