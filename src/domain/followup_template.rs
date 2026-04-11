pub const BUILTIN_NEXT_TASK_TEMPLATE_ID: &str = "builtin-next-task";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowupTemplateDefinition {
    pub id: String,
    pub label: String,
    pub body: String,
    pub source: FollowupTemplateSource,
}

impl FollowupTemplateDefinition {
    pub fn is_builtin_next_task(&self) -> bool {
        self.id == BUILTIN_NEXT_TASK_TEMPLATE_ID
    }

    pub fn source_label(&self) -> String {
        match &self.source {
            FollowupTemplateSource::Builtin => "builtin".to_string(),
            FollowupTemplateSource::WorkspaceFile { path } => {
                format!("workspace file: {path}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowupTemplateSource {
    Builtin,
    WorkspaceFile { path: String },
}

#[derive(Debug, Clone)]
pub struct FollowupTemplateCatalog {
    pub items: Vec<FollowupTemplateDefinition>,
}

#[derive(Debug, Clone)]
pub struct FollowupTemplateCatalogLoadResult {
    pub catalog: FollowupTemplateCatalog,
    pub warnings: Vec<String>,
}
