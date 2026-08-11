use anyhow::Result;

use crate::domain::conversation::ConversationTurnOptions;

/// Durable per-thread interactive turn options.  `None` from `load` means the
/// thread predates Akra ownership (or was created externally), which is
/// intentionally different from `Some(ConversationTurnOptions { None, None })`.
pub trait ConversationThreadTurnOptionsPort: Send + Sync {
    /// A no-op construction is useful for standalone adapter tests, but it
    /// must not turn every resumed test thread into an app-server-default
    /// thread. Production persistence adapters keep the default `true`.
    fn is_durable(&self) -> bool {
        true
    }

    fn load_turn_options(
        &self,
        workspace_dir: &str,
        thread_id: &str,
    ) -> Result<Option<ConversationTurnOptions>>;

    fn store_turn_options(
        &self,
        workspace_dir: &str,
        thread_id: &str,
        options: &ConversationTurnOptions,
    ) -> Result<()>;
}

/// Standalone adapter construction and narrow tests do not need a SQLite
/// authority store.  The production composition injects the durable adapter.
#[derive(Default)]
pub struct NoopConversationThreadTurnOptionsPort;

impl ConversationThreadTurnOptionsPort for NoopConversationThreadTurnOptionsPort {
    fn is_durable(&self) -> bool {
        false
    }

    fn load_turn_options(
        &self,
        _workspace_dir: &str,
        _thread_id: &str,
    ) -> Result<Option<ConversationTurnOptions>> {
        Ok(None)
    }

    fn store_turn_options(
        &self,
        _workspace_dir: &str,
        _thread_id: &str,
        _options: &ConversationTurnOptions,
    ) -> Result<()> {
        Ok(())
    }
}
