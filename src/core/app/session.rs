use crate::domain::recent_sessions::SessionCatalog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCatalogReadySnapshot {
    pub tier_label: String,
    pub item_count: usize,
    pub warnings: Vec<String>,
}

impl SessionCatalogReadySnapshot {
    pub(crate) fn from_catalog(catalog: SessionCatalog) -> Self {
        let tier_label = catalog.tier().label().to_string();
        let item_count = catalog
            .recent_sessions()
            .map(|recent_sessions| recent_sessions.items.len())
            .unwrap_or(0);
        let warnings = catalog.warnings().to_vec();
        Self {
            tier_label,
            item_count,
            warnings,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCatalogSnapshot {
    Idle,
    Loading,
    Ready(SessionCatalogReadySnapshot),
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SessionCatalogState {
    #[default]
    Idle,
    Loading,
    Ready(SessionCatalogReadySnapshot),
    Failed(String),
}

impl SessionCatalogState {
    pub fn snapshot(&self) -> SessionCatalogSnapshot {
        match self {
            Self::Idle => SessionCatalogSnapshot::Idle,
            Self::Loading => SessionCatalogSnapshot::Loading,
            Self::Ready(ready) => SessionCatalogSnapshot::Ready(ready.clone()),
            Self::Failed(message) => SessionCatalogSnapshot::Failed {
                message: message.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalogTier};

    #[test]
    fn ready_snapshot_projects_provider_catalog_summary() {
        let catalog = RecentSessions {
            items: Vec::new(),
            warnings: vec!["partial catalog".to_string()],
            next_cursor: None,
        }
        .into();

        assert_eq!(
            SessionCatalogReadySnapshot::from_catalog(catalog),
            SessionCatalogReadySnapshot {
                tier_label: SessionCatalogTier::ProviderBackedCatalog
                    .label()
                    .to_string(),
                item_count: 0,
                warnings: vec!["partial catalog".to_string()],
            }
        );
    }
}
