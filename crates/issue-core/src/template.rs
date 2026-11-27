use std::sync::Arc;

use serde::Deserialize;
use serde_yaml_ng::Value;
use sha2::{Digest, Sha256};

use crate::issue_markdown::IssueMarkdownHeadings;
use crate::issue_template::{
    IssueTemplateConvention, IssueTemplateFormat, IssueTemplateInputKind,
    IssueTemplateRequiredInput, IssueTemplateResolution, IssueTemplateUnsupportedInput,
    IssueTemplateVersionCheck, check_issue_template_version,
};
use crate::issue_template_source_port::IssueTemplateSourcePort;

/*
 * 대상 저장소의 이슈 템플릿을 읽어 `IssueTemplateResolution` 을 만든다 (#2417).
 *
 * Markdown 템플릿은 제목마다 필수 구간 하나이다. YAML Issue Form 은 `input`, `textarea`, `dropdown`,
 * `checkboxes` 의 필수 표시를 읽는다. 다른 종류의 필수 입력은 `unsupported_required_inputs` 로 간다.
 * 없는 파일, 읽기 실패, 해석 실패는 서로 다른 결과이다. 호출자는 이들을 "요구 없음"으로 바꾸지 않는다.
 */
pub struct IssueTemplateService {
    source: Arc<dyn IssueTemplateSourcePort>,
}

impl IssueTemplateService {
    pub fn new(source: Arc<dyn IssueTemplateSourcePort>) -> Self {
        Self { source }
    }

    /// `template_path` 는 이슈 정책의 `IssueConvention::template` 이다.
    pub fn resolve(
        &self,
        workspace_dir: &str,
        template_path: Option<&str>,
    ) -> IssueTemplateResolution {
        let Some(path) = template_path else {
            return IssueTemplateResolution::NotConfigured;
        };
        match self.source.load_template(workspace_dir, path) {
            Ok(Some(contents)) => parse_issue_template(path, &contents),
            Ok(None) => IssueTemplateResolution::Missing {
                path: path.to_string(),
            },
            Err(error) => IssueTemplateResolution::Unreadable {
                path: path.to_string(),
                reason: format!("{error:#}"),
            },
        }
    }

    /// 게시 전에 부른다. `drafted_version` 은 초안을 쓸 때의 `IssueTemplateResolution::version` 이다.
    pub fn check_draft_version(
        &self,
        workspace_dir: &str,
        template_path: Option<&str>,
        drafted_version: Option<&str>,
    ) -> IssueTemplateVersionCheck {
        check_issue_template_version(drafted_version, &self.resolve(workspace_dir, template_path))
    }
}

/// 템플릿 파일의 글에서 규약을 만든다. 형식은 확장자로 정한다.
pub fn parse_issue_template(path: &str, contents: &str) -> IssueTemplateResolution {
    let version = format!("{:x}", Sha256::digest(contents.as_bytes()));
    let extension = path.rsplit_once('.').map(|(_, extension)| extension);
    let parsed = match extension.map(str::to_ascii_lowercase).as_deref() {
        Some("md") => Ok((
            IssueTemplateFormat::Markdown,
            markdown_required_sections(contents),
            Vec::new(),
        )),
        Some("yml" | "yaml") => parse_issue_form(contents)
            .map(|(required, unsupported)| (IssueTemplateFormat::IssueForm, required, unsupported)),
        _ => Err("the template must be a `.md`, `.yml`, or `.yaml` file".to_string()),
    };
    match parsed {
        Ok((format, required_inputs, unsupported_required_inputs)) => {
            IssueTemplateResolution::Resolved(IssueTemplateConvention {
                path: path.to_string(),
                version,
                format,
                required_inputs,
                unsupported_required_inputs,
            })
        }
        Err(reason) => IssueTemplateResolution::Invalid {
            path: path.to_string(),
            version,
            reason,
        },
    }
}

/// Markdown 템플릿의 제목이다. 머리말(front matter)과 코드 블록 안의 줄은 제목이 아니다.
fn markdown_required_sections(contents: &str) -> Vec<IssueTemplateRequiredInput> {
    let mut lines = contents.lines().peekable();
    if lines.peek().map(|line| line.trim_end()) == Some("---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim_end() == "---" {
                break;
            }
        }
    }
    let mut sections = Vec::new();
    let mut headings = IssueMarkdownHeadings::default();
    for line in lines {
        if let Some((_, label)) = headings.heading(line)
            && !label.is_empty()
        {
            sections.push(IssueTemplateRequiredInput {
                label: label.to_string(),
                kind: IssueTemplateInputKind::Section,
            });
        }
    }
    sections
}

#[derive(Debug, Deserialize)]
struct IssueForm {
    body: Vec<IssueFormField>,
}

#[derive(Debug, Deserialize)]
struct IssueFormField {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    #[serde(default)]
    attributes: IssueFormAttributes,
    #[serde(default)]
    validations: IssueFormValidations,
}

#[derive(Debug, Default, Deserialize)]
struct IssueFormAttributes {
    label: Option<String>,
    #[serde(default)]
    options: Vec<Value>,
    #[serde(default)]
    multiple: bool,
}

#[derive(Debug, Default, Deserialize)]
struct IssueFormValidations {
    #[serde(default)]
    required: bool,
}

type IssueFormInputs = (
    Vec<IssueTemplateRequiredInput>,
    Vec<IssueTemplateUnsupportedInput>,
);

fn parse_issue_form(contents: &str) -> Result<IssueFormInputs, String> {
    let form = serde_yaml_ng::from_str::<IssueForm>(contents)
        .map_err(|error| format!("the Issue Form cannot be parsed: {error}"))?;
    let mut required = Vec::new();
    let mut unsupported = Vec::new();
    for field in form.body {
        match issue_form_field_inputs(&field) {
            Ok(inputs) => {
                if inputs
                    .iter()
                    .any(|input| matches!(input.kind, IssueTemplateInputKind::Checkbox { .. }))
                {
                    unsupported.push(IssueTemplateUnsupportedInput {
                        label: field_label(&field)?,
                        reason: "required checkbox validation is not supported".to_string(),
                    });
                }
                required.extend(inputs);
            }
            Err(reason) => unsupported.push(IssueTemplateUnsupportedInput {
                label: field
                    .attributes
                    .label
                    .or(field.id)
                    .unwrap_or_else(|| field.kind.clone()),
                reason,
            }),
        }
    }
    Ok((required, unsupported))
}

/// 필드 하나의 필수 입력이다. 필수가 아닌 필드는 빈 목록이다. 해석하지 못하는 필수 필드는 오류이다.
fn issue_form_field_inputs(
    field: &IssueFormField,
) -> Result<Vec<IssueTemplateRequiredInput>, String> {
    let kind = match field.kind.as_str() {
        // `markdown` 은 안내 글이다. 입력이 아니다.
        "markdown" => return Ok(Vec::new()),
        "checkboxes" => return required_checkboxes(field),
        _ if !field.validations.required => return Ok(Vec::new()),
        "input" => IssueTemplateInputKind::Input,
        "textarea" => IssueTemplateInputKind::Textarea,
        "dropdown" if field.attributes.multiple => {
            return Err("required multiple-selection dropdowns are not supported".to_string());
        }
        "dropdown" => IssueTemplateInputKind::Dropdown {
            options: field
                .attributes
                .options
                .iter()
                .map(scalar_text)
                .collect::<Result<_, _>>()?,
        },
        other => return Err(format!("unsupported field type `{other}`")),
    };
    Ok(vec![IssueTemplateRequiredInput {
        label: field_label(field)?,
        kind,
    }])
}

/// 체크 상자는 묶음이 아니라 항목마다 필수 표시가 있다.
fn required_checkboxes(field: &IssueFormField) -> Result<Vec<IssueTemplateRequiredInput>, String> {
    let mut inputs = Vec::new();
    for option in &field.attributes.options {
        if option.get("required").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let option_label = option
            .get("label")
            .ok_or_else(|| "a required checkbox has no label".to_string())
            .and_then(scalar_text)?;
        inputs.push(IssueTemplateRequiredInput {
            label: field_label(field)?,
            kind: IssueTemplateInputKind::Checkbox {
                option: option_label,
            },
        });
    }
    Ok(inputs)
}

fn field_label(field: &IssueFormField) -> Result<String, String> {
    match field.attributes.label.as_deref().map(str::trim) {
        Some(label) if !label.is_empty() => Ok(label.to_string()),
        _ => Err("a required field has no label".to_string()),
    }
}

fn scalar_text(value: &Value) -> Result<String, String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        _ => Err("an option is not a plain value".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, bail};

    use super::{IssueTemplateService, parse_issue_template};
    use crate::issue_template::{
        IssueTemplateConvention, IssueTemplateFormat, IssueTemplateInputKind,
        IssueTemplateRequiredInput, IssueTemplateResolution, IssueTemplateVersionCheck,
    };
    use crate::issue_template_source_port::IssueTemplateSourcePort;

    const MARKDOWN_TEMPLATE: &str = "---
name: Bug report
about: Report a defect
# not a heading
---

## Summary

<!-- one line -->

### Steps to reproduce ###

```sh
# a comment in a code block
```

#no-space is not a heading
";

    const ISSUE_FORM: &str = r#"
name: Bug report
description: Report a defect
labels: [bug]
body:
  - type: markdown
    attributes:
      value: Thanks for the report.
  - type: input
    id: version
    attributes:
      label: Version
    validations:
      required: true
  - type: textarea
    id: steps
    attributes:
      label: Steps to reproduce
    validations:
      required: true
  - type: textarea
    id: notes
    attributes:
      label: Notes
  - type: dropdown
    id: os
    attributes:
      label: Operating system
      options: [macOS, Windows, 11]
    validations:
      required: true
  - type: checkboxes
    id: terms
    attributes:
      label: Checks
      options:
        - label: I searched existing issues
          required: true
        - label: I can send a fix
  - type: upload
    id: logs
    attributes:
      label: Log files
    validations:
      required: true
  - type: upload
    id: screenshots
    attributes:
      label: Screenshots
  - type: input
    id: contact
    validations:
      required: true
"#;

    fn convention(resolution: IssueTemplateResolution) -> IssueTemplateConvention {
        match resolution {
            IssueTemplateResolution::Resolved(convention) => convention,
            other => panic!("expected a resolved template, got {other:?}"),
        }
    }

    fn input(label: &str, kind: IssueTemplateInputKind) -> IssueTemplateRequiredInput {
        IssueTemplateRequiredInput {
            label: label.to_string(),
            kind,
        }
    }

    #[test]
    fn markdown_template_returns_path_version_and_heading_sections() {
        let convention = convention(parse_issue_template(
            ".github/ISSUE_TEMPLATE/bug.md",
            MARKDOWN_TEMPLATE,
        ));

        assert_eq!(convention.path, ".github/ISSUE_TEMPLATE/bug.md");
        assert_eq!(convention.format, IssueTemplateFormat::Markdown);
        assert_eq!(convention.version.len(), 64);
        assert_eq!(
            convention.required_inputs,
            vec![
                input("Summary", IssueTemplateInputKind::Section),
                input("Steps to reproduce", IssueTemplateInputKind::Section),
            ]
        );
        assert!(convention.unsupported_required_inputs.is_empty());
    }

    #[test]
    fn template_version_changes_only_when_the_contents_change() {
        let first = parse_issue_template("bug.md", "## Summary\n");
        let same = parse_issue_template("bug.md", "## Summary\n");
        let changed = parse_issue_template("bug.md", "## Summary\n## Impact\n");

        assert_eq!(first.version(), same.version());
        assert_ne!(first.version(), changed.version());
    }

    #[test]
    fn issue_form_returns_required_inputs_and_separates_unsupported_ones() {
        let convention = convention(parse_issue_template(
            ".github/ISSUE_TEMPLATE/bug.yml",
            ISSUE_FORM,
        ));

        assert_eq!(convention.format, IssueTemplateFormat::IssueForm);
        assert_eq!(
            convention.required_inputs,
            vec![
                input("Version", IssueTemplateInputKind::Input),
                input("Steps to reproduce", IssueTemplateInputKind::Textarea),
                input(
                    "Operating system",
                    IssueTemplateInputKind::Dropdown {
                        options: vec!["macOS".to_string(), "Windows".to_string(), "11".to_string()],
                    }
                ),
                input(
                    "Checks",
                    IssueTemplateInputKind::Checkbox {
                        option: "I searched existing issues".to_string(),
                    }
                ),
            ]
        );
        // 필수가 아닌 `upload` 는 결과에 없다. 필수인 것과 이름 없는 필수 입력만 남는다.
        let unsupported = convention
            .unsupported_required_inputs
            .iter()
            .map(|input| (input.label.as_str(), input.reason.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            unsupported,
            vec![
                ("Checks", "required checkbox validation is not supported"),
                ("Log files", "unsupported field type `upload`"),
                ("contact", "a required field has no label"),
            ]
        );
    }

    #[test]
    fn a_template_that_cannot_be_parsed_is_invalid_and_keeps_its_version() {
        for (path, contents) in [
            ("bug.yml", "body: [unterminated"),
            ("bug.yml", "name: Bug report\n"),
            ("bug.txt", "## Summary\n"),
        ] {
            let resolution = parse_issue_template(path, contents);
            assert!(
                matches!(resolution, IssueTemplateResolution::Invalid { .. }),
                "{path}: {resolution:?}"
            );
            assert!(resolution.version().is_some());
        }
    }

    struct FixedIssueTemplateSource {
        contents: Option<&'static str>,
        unreadable: bool,
    }

    impl IssueTemplateSourcePort for FixedIssueTemplateSource {
        fn load_template(
            &self,
            _workspace_dir: &str,
            _template_path: &str,
        ) -> Result<Option<String>> {
            if self.unreadable {
                bail!("configuration must be a regular non-symlink file");
            }
            Ok(self.contents.map(str::to_string))
        }
    }

    fn resolve(
        contents: Option<&'static str>,
        unreadable: bool,
        template_path: Option<&str>,
    ) -> IssueTemplateResolution {
        IssueTemplateService::new(Arc::new(FixedIssueTemplateSource {
            contents,
            unreadable,
        }))
        .resolve("/workspace", template_path)
    }

    #[test]
    fn service_distinguishes_no_template_missing_and_unreadable() {
        assert_eq!(
            resolve(Some("## Summary\n"), false, None),
            IssueTemplateResolution::NotConfigured
        );
        assert_eq!(
            resolve(None, false, Some("bug.md")),
            IssueTemplateResolution::Missing {
                path: "bug.md".to_string(),
            }
        );
        assert_eq!(
            resolve(None, true, Some("bug.md")),
            IssueTemplateResolution::Unreadable {
                path: "bug.md".to_string(),
                reason: "configuration must be a regular non-symlink file".to_string(),
            }
        );
        assert!(matches!(
            resolve(Some("## Summary\n"), false, Some("bug.md")),
            IssueTemplateResolution::Resolved(_)
        ));
    }

    #[test]
    fn a_draft_needs_review_when_the_template_changed_before_publish() {
        let drafted = parse_issue_template("bug.md", "## Summary\n");
        let service = |contents| {
            IssueTemplateService::new(Arc::new(FixedIssueTemplateSource {
                contents: Some(contents),
                unreadable: false,
            }))
        };

        assert_eq!(
            service("## Summary\n").check_draft_version(
                "/workspace",
                Some("bug.md"),
                drafted.version()
            ),
            IssueTemplateVersionCheck::Unchanged
        );
        assert!(matches!(
            service("## Summary\n## Impact\n").check_draft_version(
                "/workspace",
                Some("bug.md"),
                drafted.version()
            ),
            IssueTemplateVersionCheck::ReviewRequired { .. }
        ));
    }
    #[test]
    fn markdown_templates_share_fence_rules_and_preserve_csharp_labels() {
        for body in [
            "\x60\x60\x60\x60md\n\x60\x60\x60\n## Hidden\n\x60\x60\x60\n\x60\x60\x60\x60\n### C# ###",
            "~~~\n\x60\x60\x60\n## Hidden\n\x60\x60\x60\n~~~\n### C#",
        ] {
            let parsed = convention(parse_issue_template("bug.md", body));
            assert_eq!(
                parsed.required_inputs,
                [input("C#", IssueTemplateInputKind::Section)]
            );
        }
        let unclosed = convention(parse_issue_template("bug.md", "\x60\x60\x60\n## Example"));
        assert!(unclosed.required_inputs.is_empty());
    }

    #[test]
    fn required_multiple_dropdowns_are_explicitly_unsupported() {
        for (multiple, required, expected_required, expected_unsupported) in
            [(false, true, 1, 0), (true, true, 0, 1), (true, false, 0, 0)]
        {
            let yaml = format!(
                "body:\n  - type: dropdown\n    attributes:\n      label: Systems\n      options: [Windows, macOS]\n      multiple: {multiple}\n    validations:\n      required: {required}\n"
            );
            let parsed = convention(parse_issue_template("bug.yml", &yaml));
            assert_eq!(parsed.required_inputs.len(), expected_required);
            assert_eq!(
                parsed.unsupported_required_inputs.len(),
                expected_unsupported
            );
        }
    }
}
