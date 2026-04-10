use chrono::{Local, TimeZone};

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub name: Option<String>,
    pub preview: String,
    pub cwd: String,
    pub source: String,
    pub model_provider: String,
    pub updated_at_epoch: i64,
    pub status_type: String,
    pub path: String,
    pub git_branch: Option<String>,
}

impl SessionSummary {
    pub fn short_id(&self) -> String {
        self.id.chars().take(8).collect()
    }

    pub fn title(&self) -> String {
        self.name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.first_preview_line())
    }

    pub fn first_preview_line(&self) -> String {
        self.preview
            .lines()
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Self::truncate)
            .unwrap_or_else(|| "(empty preview)".to_string())
    }

    pub fn preview_block(&self) -> String {
        let preview = self.preview.trim();
        if preview.is_empty() {
            "(empty preview)".to_string()
        } else {
            preview.to_string()
        }
    }

    pub fn workspace_label(&self) -> String {
        self.cwd
            .rsplit('/')
            .next()
            .filter(|value| !value.is_empty())
            .unwrap_or(self.cwd.as_str())
            .to_string()
    }

    pub fn updated_at_label(&self) -> String {
        Local
            .timestamp_opt(self.updated_at_epoch, 0)
            .single()
            .map(|value| value.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| self.updated_at_epoch.to_string())
    }

    fn truncate(value: &str) -> String {
        const LIMIT: usize = 72;
        let count = value.chars().count();
        if count <= LIMIT {
            return value.to_string();
        }

        value.chars().take(LIMIT - 1).collect::<String>() + "..."
    }
}
