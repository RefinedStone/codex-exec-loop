use crate::domain::recent_sessions::SessionCatalog;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCatalogReadySnapshot {
    pub catalog: Box<SessionCatalog>,
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
            catalog: Box::new(catalog),
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

impl SessionCatalogSnapshot {
    /// 읽은 세션 목록에 이 thread 가 있다 (plan 18 K3-3b).
    pub(crate) fn lists_thread(&self, thread_id: &str) -> bool {
        let Self::Ready(ready) = self else {
            return false;
        };
        ready
            .catalog
            .recent_sessions()
            .is_some_and(|sessions| sessions.items.iter().any(|item| item.id == thread_id))
    }

    /// 읽은 세션 목록이 적은 이 thread 의 작업 디렉터리이다. 목록은 모든 저장소의 thread 를 담는다 (#2303).
    pub(crate) fn thread_cwd(&self, thread_id: &str) -> Option<&str> {
        let Self::Ready(ready) = self else {
            return None;
        };
        ready
            .catalog
            .recent_sessions()?
            .items
            .iter()
            .find(|item| item.id == thread_id)
            .map(|item| item.cwd.as_str())
    }
}

/*
 * `path` 가 `workspace` 이거나 그 안이다 (#2303). 글자로만 비교한다: `\\?\` 머리, `\` 구분자, 끝 구분자를 맞추고
 * Windows 에서는 대소문자를 가리지 않는다. 빈 `path` 는 어디에도 속하지 않는다.
 * shortcut: symlink, junction, 8.3 이름은 같은 디렉터리라도 다르다고 본다. 거짓 거절이 보이면 adapter 가 정규화한 값을 넘긴다.
 */
pub(crate) fn workspace_contains(workspace: &str, path: &str) -> bool {
    let workspace = comparable_path(workspace);
    let path = comparable_path(path);
    !path.is_empty()
        && (path == workspace
            || path
                .strip_prefix(&workspace)
                .is_some_and(|rest| rest.starts_with('/')))
}

fn comparable_path(path: &str) -> String {
    let path = path.trim();
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    let path = path.replace('\\', "/");
    let path = path.trim_end_matches('/');
    if cfg!(windows) {
        path.to_lowercase()
    } else {
        path.to_string()
    }
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
                catalog: Box::new(
                    RecentSessions {
                        items: Vec::new(),
                        warnings: vec!["partial catalog".to_string()],
                        next_cursor: None,
                    }
                    .into()
                ),
                tier_label: SessionCatalogTier::ProviderBackedCatalog
                    .label()
                    .to_string(),
                item_count: 0,
                warnings: vec!["partial catalog".to_string()],
            }
        );
    }

    #[test]
    fn workspace_contains_only_the_workspace_and_its_subdirectories() {
        assert!(workspace_contains("/repo/a", "/repo/a"));
        assert!(workspace_contains("/repo/a/", "/repo/a/.akra-pool/slot-1"));
        assert!(workspace_contains(r"\\?\C:\repo\a", "C:/repo/a/src"));
        assert!(!workspace_contains("/repo/a", "/repo/ab"));
        assert!(!workspace_contains("/repo/a", "/repo/b"));
        assert!(!workspace_contains("/repo/a", "/repo"));
        assert!(!workspace_contains("/repo/a", ""));
    }
}
