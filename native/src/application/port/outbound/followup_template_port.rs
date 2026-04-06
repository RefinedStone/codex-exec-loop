use anyhow::Result;

#[derive(Debug, Clone)]
pub struct WorkspaceFollowupTemplateRecord {
    pub name: String,
    pub path: String,
    pub body: String,
}

pub trait FollowupTemplatePort: Send + Sync {
    fn load_workspace_templates(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<WorkspaceFollowupTemplateRecord>>;
}
