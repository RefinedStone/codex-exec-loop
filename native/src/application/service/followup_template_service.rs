use std::sync::Arc;

use crate::application::port::outbound::followup_template_port::FollowupTemplatePort;
use crate::domain::followup_template::{
    FollowupTemplateCatalog, FollowupTemplateCatalogLoadResult, FollowupTemplateDefinition,
    FollowupTemplateSource,
};
const BUILTIN_TEMPLATE_NEXT_TASK: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 결과를 기준으로 다음 작업 1개만 이어서 진행하세요.
더 이어갈 작업이 없다면 마지막 줄에 {stop_keyword} 만 출력하세요.

직전 답변:
{last_message}"#;
const BUILTIN_TEMPLATE_PLAN_QUEUE: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 결과를 바탕으로 개선점과 다음 작업 후보를 `plan_priority_queue.md` 에 정리하고,
가장 우선순위가 높은 항목 1개를 바로 진행하세요.
더 이어갈 작업이 없다면 마지막 줄에 {stop_keyword} 만 출력하세요.

직전 답변:
{last_message}"#;
const BUILTIN_TEMPLATE_BUGFIX: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

직전 결과 기준으로 아직 남아 있는 버그나 리스크 1개만 골라 수정하세요.
수정이 끝나면 무엇을 고쳤는지 짧게 요약하세요.
더 이어갈 작업이 없다면 마지막 줄에 {stop_keyword} 만 출력하세요.

직전 답변:
{last_message}"#;
const BUILTIN_TEMPLATE_DOCS: &str = r#"대리인입니다.
자동 후속 {auto_turn}/{max_auto_turns} 입니다.

방금 작업을 기준으로 README 또는 사용자 문서에 빠진 내용 1개만 보강하세요.
더 이어갈 작업이 없다면 마지막 줄에 {stop_keyword} 만 출력하세요.

직전 답변:
{last_message}"#;

#[derive(Clone)]
pub struct FollowupTemplateService {
    followup_template_port: Arc<dyn FollowupTemplatePort>,
}

impl FollowupTemplateService {
    pub fn new(followup_template_port: Arc<dyn FollowupTemplatePort>) -> Self {
        Self {
            followup_template_port,
        }
    }

    pub fn load_catalog(&self, workspace_dir: &str) -> FollowupTemplateCatalogLoadResult {
        let mut warnings = Vec::new();
        let mut items = Self::builtin_templates();

        match self
            .followup_template_port
            .load_workspace_templates(workspace_dir)
        {
            Ok(workspace_templates) => {
                for template in workspace_templates {
                    if template.body.trim().is_empty() {
                        warnings.push(format!(
                            "ignored empty follow-up template: {}",
                            template.path
                        ));
                        continue;
                    }

                    items.push(FollowupTemplateDefinition {
                        id: format!("workspace:{}", template.name),
                        label: format!("workspace {}", template.name),
                        body: template.body,
                        source: FollowupTemplateSource::WorkspaceFile {
                            path: template.path,
                        },
                    });
                }
            }
            Err(error) => warnings.push(format!(
                "failed to load workspace follow-up templates: {error}"
            )),
        }

        FollowupTemplateCatalogLoadResult {
            catalog: FollowupTemplateCatalog { items },
            warnings,
        }
    }

    fn builtin_templates() -> Vec<FollowupTemplateDefinition> {
        vec![
            Self::builtin_template(
                "builtin-next-task",
                "builtin next-task",
                BUILTIN_TEMPLATE_NEXT_TASK,
            ),
            Self::builtin_template(
                "builtin-plan-queue",
                "builtin plan-queue",
                BUILTIN_TEMPLATE_PLAN_QUEUE,
            ),
            Self::builtin_template("builtin-bugfix", "builtin bugfix", BUILTIN_TEMPLATE_BUGFIX),
            Self::builtin_template("builtin-docs", "builtin docs", BUILTIN_TEMPLATE_DOCS),
        ]
    }

    fn builtin_template(id: &str, label: &str, body: &str) -> FollowupTemplateDefinition {
        FollowupTemplateDefinition {
            id: id.to_string(),
            label: label.to_string(),
            body: body.to_string(),
            source: FollowupTemplateSource::Builtin,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;

    use super::FollowupTemplateService;
    use crate::application::port::outbound::followup_template_port::{
        FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
    };

    struct FakeFollowupTemplatePort {
        templates: Vec<WorkspaceFollowupTemplateRecord>,
    }

    impl FollowupTemplatePort for FakeFollowupTemplatePort {
        fn load_workspace_templates(
            &self,
            _workspace_dir: &str,
        ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
            Ok(self.templates.clone())
        }
    }

    #[test]
    fn keeps_builtin_templates_and_appends_workspace_templates() {
        let service = FollowupTemplateService::new(Arc::new(FakeFollowupTemplatePort {
            templates: vec![WorkspaceFollowupTemplateRecord {
                name: "custom-review".to_string(),
                path: "/tmp/workspace/.codex-exec-loop/followups/custom-review.md".to_string(),
                body: "workspace body".to_string(),
            }],
        }));

        let result = service.load_catalog("/tmp/workspace");

        assert_eq!(result.catalog.items.len(), 5);
        assert_eq!(result.catalog.items[0].label, "builtin next-task");
        assert_eq!(result.catalog.items[4].label, "workspace custom-review");
    }

    #[test]
    fn ignores_empty_workspace_templates_with_warning() {
        let service = FollowupTemplateService::new(Arc::new(FakeFollowupTemplatePort {
            templates: vec![WorkspaceFollowupTemplateRecord {
                name: "empty-template".to_string(),
                path: "/tmp/workspace/.codex-exec-loop/followups/empty-template.md".to_string(),
                body: "   ".to_string(),
            }],
        }));

        let result = service.load_catalog("/tmp/workspace");

        assert_eq!(result.catalog.items.len(), 4);
        assert_eq!(result.warnings.len(), 1);
    }
}
