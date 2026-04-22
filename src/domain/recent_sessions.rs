use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone)]
pub struct RecentSessions {
    pub items: Vec<SessionSummary>,
    pub warnings: Vec<String>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCatalogTier {
    AttachOnly,
    HandleBasedReattach,
    ProviderBackedCatalog,
}

impl SessionCatalogTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::AttachOnly => "attach-only",
            Self::HandleBasedReattach => "handle-based reattach",
            Self::ProviderBackedCatalog => "provider-backed catalog",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionCatalogStatus {
    pub tier: SessionCatalogTier,
    pub detail: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum SessionCatalog {
    Unsupported(SessionCatalogStatus),
    Partial(SessionCatalogStatus),
    Ready {
        tier: SessionCatalogTier,
        recent_sessions: RecentSessions,
    },
}

impl SessionCatalog {
    pub fn unsupported(
        tier: SessionCatalogTier,
        detail: impl Into<String>,
        warnings: Vec<String>,
    ) -> Self {
        Self::Unsupported(SessionCatalogStatus {
            tier,
            detail: detail.into(),
            warnings,
        })
    }

    pub fn partial(
        tier: SessionCatalogTier,
        detail: impl Into<String>,
        warnings: Vec<String>,
    ) -> Self {
        Self::Partial(SessionCatalogStatus {
            tier,
            detail: detail.into(),
            warnings,
        })
    }

    pub fn ready(tier: SessionCatalogTier, recent_sessions: RecentSessions) -> Self {
        Self::Ready {
            tier,
            recent_sessions,
        }
    }

    pub fn tier(&self) -> SessionCatalogTier {
        match self {
            Self::Unsupported(status) | Self::Partial(status) => status.tier,
            Self::Ready { tier, .. } => *tier,
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Unsupported(status) | Self::Partial(status) => Some(status.detail.as_str()),
            Self::Ready { .. } => None,
        }
    }

    pub fn warnings(&self) -> &[String] {
        match self {
            Self::Unsupported(status) | Self::Partial(status) => status.warnings.as_slice(),
            Self::Ready {
                recent_sessions, ..
            } => recent_sessions.warnings.as_slice(),
        }
    }

    pub fn recent_sessions(&self) -> Option<&RecentSessions> {
        match self {
            Self::Ready {
                recent_sessions, ..
            } => Some(recent_sessions),
            Self::Unsupported(_) | Self::Partial(_) => None,
        }
    }
}

impl From<RecentSessions> for SessionCatalog {
    fn from(recent_sessions: RecentSessions) -> Self {
        Self::ready(SessionCatalogTier::ProviderBackedCatalog, recent_sessions)
    }
}
