use std::fmt;
use std::path::{Component, Path};
use std::sync::Arc;

use serde::Deserialize;

use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::domain::issue_policy::{
    IssueConvention, IssueDisclosurePolicy, IssuePolicy, IssueSensitiveSpanAction,
};

/*
 * 이슈 정책 파일을 읽고 검증해 `IssuePolicy` 를 만든다 (#2416).
 *
 * 우선순위는 값 하나마다 `프로젝트 파일 > 사용자 파일 > 기본값` 이다. 파일 하나라도 읽지 못하거나 틀리면
 * `IssuePolicyBlocked` 를 돌려준다. 호출자는 그때 게시와 AI 전달을 막는다. 기본값으로 대신하지 않는다.
 */

/// 이슈 정책 파일의 스키마 판이다. `schema_version` 이 없으면 이 판으로 읽는다.
pub const ISSUE_POLICY_SCHEMA_VERSION: u32 = 1;

/// 정책 파일의 범위이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuePolicyScope {
    User,
    Project,
}

impl IssuePolicyScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// 정책을 만들 수 없는 상태이다. 이 상태에서는 그 프로젝트의 게시와 AI 전달을 막는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuePolicyBlocked {
    /// 문제가 있는 파일의 범위이다.
    pub scope: IssuePolicyScope,
    pub reason: String,
}

impl fmt::Display for IssuePolicyBlocked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid {} issue policy: {}",
            self.scope.label(),
            self.reason
        )
    }
}

impl std::error::Error for IssuePolicyBlocked {}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct IssuePolicyLayer {
    schema_version: Option<u32>,
    convention: Option<ConventionLayer>,
    publish: Option<PublishLayer>,
    ai_handoff: Option<AiHandoffLayer>,
    vault: Option<VaultLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConventionLayer {
    template: Option<String>,
    required_sections: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishLayer {
    /// 잠긴 값이다. `true` 만 받는다.
    required_validation: Option<bool>,
    spans: Option<SpansLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiHandoffLayer {
    spans: Option<SpansLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct VaultLayer {
    /// 잠긴 값이다. `true` 만 받는다.
    user_unlock: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpansLayer {
    secret: Option<IssueSensitiveSpanAction>,
    name: Option<IssueSensitiveSpanAction>,
    path: Option<IssueSensitiveSpanAction>,
    log: Option<IssueSensitiveSpanAction>,
}

pub struct IssuePolicyService {
    source: Arc<dyn IssuePolicySourcePort>,
}

impl IssuePolicyService {
    pub fn new(source: Arc<dyn IssuePolicySourcePort>) -> Self {
        Self { source }
    }

    /// `workspace_dir` 프로젝트의 이슈 정책이다.
    pub fn resolve(&self, workspace_dir: &str) -> Result<IssuePolicy, IssuePolicyBlocked> {
        let user = self
            .source
            .load_user_policy_toml()
            .map_err(|error| unreadable(IssuePolicyScope::User, &error))?;
        let project = self
            .source
            .load_project_policy_toml(workspace_dir)
            .map_err(|error| unreadable(IssuePolicyScope::Project, &error))?;
        resolve_issue_policy(user.as_deref(), project.as_deref())
    }
}

/// 두 파일의 글에서 정책을 만든다. 없는 파일은 `None` 이다.
pub fn resolve_issue_policy(
    user_toml: Option<&str>,
    project_toml: Option<&str>,
) -> Result<IssuePolicy, IssuePolicyBlocked> {
    let mut policy = IssuePolicy::builtin();
    // 낮은 우선순위부터 덮어쓴다.
    for (scope, contents) in [
        (IssuePolicyScope::User, user_toml),
        (IssuePolicyScope::Project, project_toml),
    ] {
        if let Some(contents) = contents {
            let layer =
                parse_layer(contents).map_err(|reason| IssuePolicyBlocked { scope, reason })?;
            apply_layer(&mut policy, layer);
        }
    }
    Ok(policy)
}

fn unreadable(scope: IssuePolicyScope, error: &anyhow::Error) -> IssuePolicyBlocked {
    IssuePolicyBlocked {
        scope,
        reason: format!("the file cannot be read: {error:#}"),
    }
}

fn parse_layer(contents: &str) -> Result<IssuePolicyLayer, String> {
    let layer = toml::from_str::<IssuePolicyLayer>(contents).map_err(|error| error.to_string())?;
    if let Some(version) = layer.schema_version
        && version != ISSUE_POLICY_SCHEMA_VERSION
    {
        return Err(format!(
            "unsupported schema_version {version}; expected {ISSUE_POLICY_SCHEMA_VERSION}"
        ));
    }
    if layer
        .publish
        .as_ref()
        .and_then(|publish| publish.required_validation)
        == Some(false)
    {
        return Err("publish.required_validation cannot be turned off".to_string());
    }
    if layer.vault.as_ref().and_then(|vault| vault.user_unlock) == Some(false) {
        return Err("vault.user_unlock cannot be turned off".to_string());
    }
    if let Some(convention) = &layer.convention {
        if let Some(template) = &convention.template {
            validate_template_path(template)?;
        }
        if let Some(sections) = &convention.required_sections
            && sections.iter().any(|section| section.trim().is_empty())
        {
            return Err("convention.required_sections must not contain an empty title".to_string());
        }
    }
    Ok(layer)
}

/// 템플릿은 저장소 안의 파일이어야 한다. 절대 경로와 `..` 는 저장소 밖을 가리킬 수 있다.
fn validate_template_path(template: &str) -> Result<(), String> {
    let inside_repository = !template.trim().is_empty()
        && Path::new(template)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if inside_repository {
        Ok(())
    } else {
        Err(format!(
            "convention.template must be a relative path inside the repository: `{template}`"
        ))
    }
}

fn apply_layer(policy: &mut IssuePolicy, layer: IssuePolicyLayer) {
    if let Some(convention) = layer.convention {
        apply_convention(&mut policy.convention, convention);
    }
    if let Some(spans) = layer.publish.and_then(|publish| publish.spans) {
        apply_spans(&mut policy.publish, spans);
    }
    if let Some(spans) = layer.ai_handoff.and_then(|ai_handoff| ai_handoff.spans) {
        apply_spans(&mut policy.ai_handoff, spans);
    }
}

fn apply_convention(convention: &mut IssueConvention, layer: ConventionLayer) {
    if let Some(template) = layer.template {
        convention.template = Some(template);
    }
    if let Some(required_sections) = layer.required_sections {
        convention.required_sections = required_sections;
    }
}

fn apply_spans(disclosure: &mut IssueDisclosurePolicy, layer: SpansLayer) {
    disclosure.secret = layer.secret.unwrap_or(disclosure.secret);
    disclosure.name = layer.name.unwrap_or(disclosure.name);
    disclosure.path = layer.path.unwrap_or(disclosure.path);
    disclosure.log = layer.log.unwrap_or(disclosure.log);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::{Result, bail};

    use super::{IssuePolicyScope, IssuePolicyService, resolve_issue_policy};
    use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
    use crate::domain::issue_policy::{
        IssueDisclosureTarget, IssuePolicy, IssueSensitiveSpanAction, IssueSensitiveSpanClass,
    };

    struct FixedIssuePolicySource {
        user: Option<&'static str>,
        project: Option<&'static str>,
        project_unreadable: bool,
    }

    impl IssuePolicySourcePort for FixedIssuePolicySource {
        fn load_user_policy_toml(&self) -> Result<Option<String>> {
            Ok(self.user.map(str::to_string))
        }

        fn load_project_policy_toml(&self, _workspace_dir: &str) -> Result<Option<String>> {
            if self.project_unreadable {
                bail!("configuration parent must be a real directory");
            }
            Ok(self.project.map(str::to_string))
        }
    }

    fn blocked_reason(user: Option<&str>, project: Option<&str>) -> (IssuePolicyScope, String) {
        let blocked = resolve_issue_policy(user, project).expect_err("policy must be blocked");
        (blocked.scope, blocked.reason)
    }

    #[test]
    fn no_files_give_the_builtin_policy() {
        let policy = resolve_issue_policy(None, None).expect("builtin policy");

        assert_eq!(policy, IssuePolicy::builtin());
        assert_eq!(policy.convention.template, None);
        for target in [
            IssueDisclosureTarget::Publish,
            IssueDisclosureTarget::AiHandoff,
        ] {
            let disclosure = policy.disclosure_for(target);
            assert_eq!(
                disclosure.action_for(IssueSensitiveSpanClass::Secret),
                IssueSensitiveSpanAction::Remove
            );
            for class in [
                IssueSensitiveSpanClass::Name,
                IssueSensitiveSpanClass::Path,
                IssueSensitiveSpanClass::Log,
            ] {
                assert_eq!(
                    disclosure.action_for(class),
                    IssueSensitiveSpanAction::RestorableMask
                );
            }
        }
    }

    #[test]
    fn project_value_wins_over_user_value_and_other_values_are_inherited() {
        let user = r#"
            [convention]
            template = ".github/ISSUE_TEMPLATE/user.md"
            required_sections = ["Summary"]

            [publish.spans]
            name = "remove"
            path = "user_confirm"
        "#;
        let project = r#"
            [convention]
            template = ".github/ISSUE_TEMPLATE/bug_report.md"

            [publish.spans]
            name = "user_confirm"
        "#;

        let policy = resolve_issue_policy(Some(user), Some(project)).expect("merged policy");

        assert_eq!(
            policy.convention.template.as_deref(),
            Some(".github/ISSUE_TEMPLATE/bug_report.md")
        );
        assert_eq!(policy.convention.required_sections, vec!["Summary"]);
        assert_eq!(policy.publish.name, IssueSensitiveSpanAction::UserConfirm);
        assert_eq!(policy.publish.path, IssueSensitiveSpanAction::UserConfirm);
        assert_eq!(policy.publish.log, IssueSensitiveSpanAction::RestorableMask);
    }

    #[test]
    fn each_span_action_is_read_for_each_class() {
        let project = r#"
            [publish.spans]
            secret = "remove"
            name = "restorable_mask"
            path = "user_confirm"
            log = "remove"
        "#;

        let policy = resolve_issue_policy(None, Some(project)).expect("policy");

        let actions = IssueSensitiveSpanClass::ALL.map(|class| policy.publish.action_for(class));
        assert_eq!(
            actions,
            [
                IssueSensitiveSpanAction::Remove,
                IssueSensitiveSpanAction::RestorableMask,
                IssueSensitiveSpanAction::UserConfirm,
                IssueSensitiveSpanAction::Remove,
            ]
        );
    }

    #[test]
    fn publish_and_ai_handoff_policies_are_separate() {
        let project = r#"
            [publish.spans]
            log = "remove"

            [ai_handoff.spans]
            name = "user_confirm"
        "#;

        let policy = resolve_issue_policy(None, Some(project)).expect("policy");

        assert_eq!(policy.publish.log, IssueSensitiveSpanAction::Remove);
        assert_eq!(
            policy.publish.name,
            IssueSensitiveSpanAction::RestorableMask
        );
        assert_eq!(
            policy.ai_handoff.log,
            IssueSensitiveSpanAction::RestorableMask
        );
        assert_eq!(
            policy.ai_handoff.name,
            IssueSensitiveSpanAction::UserConfirm
        );
    }

    #[test]
    fn mandatory_rules_can_be_stated_but_not_turned_off() {
        let stated = "[publish]\nrequired_validation = true\n[vault]\nuser_unlock = true\n";
        assert_eq!(
            resolve_issue_policy(Some(stated), Some(stated)).expect("stated rules are valid"),
            IssuePolicy::builtin()
        );

        let (scope, reason) =
            blocked_reason(None, Some("[publish]\nrequired_validation = false\n"));
        assert_eq!(scope, IssuePolicyScope::Project);
        assert!(reason.contains("publish.required_validation cannot be turned off"));

        let (scope, reason) = blocked_reason(None, Some("[vault]\nuser_unlock = false\n"));
        assert_eq!(scope, IssuePolicyScope::Project);
        assert!(reason.contains("vault.user_unlock cannot be turned off"));

        let (scope, _) = blocked_reason(Some("[vault]\nuser_unlock = false\n"), None);
        assert_eq!(scope, IssuePolicyScope::User);
    }

    #[test]
    fn invalid_files_block_instead_of_falling_back_to_defaults() {
        let invalid_files = [
            "schema_version = 2\n",
            "not toml [",
            "[publish.spans]\nsecret = \"show\"\n",
            "[publish.spans]\ntoken = \"remove\"\n",
            "[ai_handoff]\nrequired_validation = true\n",
            "[convention]\ntemplate = \"../outside.md\"\n",
            "[convention]\ntemplate = \"/etc/passwd\"\n",
            "[convention]\ntemplate = \"\"\n",
            "[convention]\nrequired_sections = [\" \"]\n",
        ];

        for contents in invalid_files {
            let (scope, _) = blocked_reason(None, Some(contents));
            assert_eq!(scope, IssuePolicyScope::Project, "{contents}");
        }
        // 올바른 프로젝트 파일이 있어도 틀린 사용자 파일은 막는다.
        let (scope, _) = blocked_reason(Some("schema_version = 2\n"), Some("schema_version = 1\n"));
        assert_eq!(scope, IssuePolicyScope::User);
    }

    #[test]
    fn service_reads_both_files_and_blocks_when_one_is_unreadable() {
        let readable = IssuePolicyService::new(Arc::new(FixedIssuePolicySource {
            user: Some("[publish.spans]\nlog = \"remove\"\n"),
            project: Some("[ai_handoff.spans]\nlog = \"user_confirm\"\n"),
            project_unreadable: false,
        }));
        let policy = readable.resolve("/workspace").expect("policy");
        assert_eq!(policy.publish.log, IssueSensitiveSpanAction::Remove);
        assert_eq!(policy.ai_handoff.log, IssueSensitiveSpanAction::UserConfirm);

        let unreadable = IssuePolicyService::new(Arc::new(FixedIssuePolicySource {
            user: None,
            project: None,
            project_unreadable: true,
        }));
        let blocked = unreadable.resolve("/workspace").expect_err("blocked");
        assert_eq!(blocked.scope, IssuePolicyScope::Project);
        assert!(blocked.to_string().contains("cannot be read"));
    }

    #[test]
    fn shipped_example_is_a_valid_project_policy() {
        let example = include_str!("../../../schema/akra-issue-policy.example.toml");

        let policy = resolve_issue_policy(None, Some(example)).expect("example must stay valid");

        assert_eq!(
            policy.convention.template.as_deref(),
            Some(".github/ISSUE_TEMPLATE/bug_report.md")
        );
        assert_eq!(policy.ai_handoff.log, IssueSensitiveSpanAction::Remove);
    }
}
