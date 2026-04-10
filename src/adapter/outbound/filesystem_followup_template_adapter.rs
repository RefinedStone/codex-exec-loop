use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::followup_template_port::{
    FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
};

#[derive(Default)]
pub struct FilesystemFollowupTemplateAdapter;

impl FilesystemFollowupTemplateAdapter {
    pub fn new() -> Self {
        Self
    }

    fn followup_directory(workspace_dir: &str) -> PathBuf {
        Path::new(workspace_dir)
            .join(".codex-exec-loop")
            .join("followups")
    }

    fn should_include(path: &Path) -> bool {
        path.is_file()
            && matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("md") | Some("txt")
            )
    }
}

impl FollowupTemplatePort for FilesystemFollowupTemplateAdapter {
    fn load_workspace_templates(
        &self,
        workspace_dir: &str,
    ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
        let followup_directory = Self::followup_directory(workspace_dir);
        if !followup_directory.exists() {
            return Ok(Vec::new());
        }
        if !followup_directory.is_dir() {
            bail!(
                "follow-up template path is not a directory: {}",
                followup_directory.display()
            );
        }

        let mut entries = fs::read_dir(&followup_directory)
            .with_context(|| format!("failed to read {}", followup_directory.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| Self::should_include(path))
            .collect::<Vec<_>>();
        entries.sort();

        entries
            .into_iter()
            .map(|path| {
                let body = fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                let name = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("workspace-template")
                    .to_string();

                Ok(WorkspaceFollowupTemplateRecord {
                    name,
                    path: path.display().to_string(),
                    body,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::FilesystemFollowupTemplateAdapter;
    use crate::application::port::outbound::followup_template_port::FollowupTemplatePort;

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    #[test]
    fn loads_workspace_templates_in_sorted_order() {
        let workspace_dir = create_temp_workspace("followup-template-adapter");
        let followup_dir = Path::new(&workspace_dir)
            .join(".codex-exec-loop")
            .join("followups");
        fs::create_dir_all(&followup_dir).expect("followup directory should exist");
        fs::write(followup_dir.join("20-second.md"), "second template")
            .expect("template should be written");
        fs::write(followup_dir.join("10-first.txt"), "first template")
            .expect("template should be written");
        fs::write(followup_dir.join("ignore.json"), "{}").expect("ignored file should be written");

        let adapter = FilesystemFollowupTemplateAdapter::new();
        let templates = adapter
            .load_workspace_templates(&workspace_dir)
            .expect("workspace templates should load");

        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].name, "10-first");
        assert_eq!(templates[1].name, "20-second");

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
