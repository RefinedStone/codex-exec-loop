use crate::adapter::inbound::kernel_protocol::{
    AttentionItem, delivery_wait_items, terminal_turn_item,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::domain::issue_automation::decision::StoredIssueDecision;
use crate::domain::kernel_event::RecordedKernelEvent;
use anyhow::Result;
use std::collections::HashSet;

pub(crate) struct LiveAttentionSources {
    pub decisions: Vec<StoredIssueDecision>,
    pub recent_turns: Vec<RecordedKernelEvent>,
}

pub(crate) fn stored_items(
    workspace: &str,
    recent_turns: Vec<RecordedKernelEvent>,
) -> Result<Vec<AttentionItem>> {
    let mut sources = SqlitePlanningAuthorityAdapter::attention_sources(workspace)?;
    // The reader can see a retry in SQLite that is newer than the runtime sample.
    // Conversely, published events can arrive before their SQLite commit.
    sources.turns.extend(
        recent_turns
            .into_iter()
            .filter(|r| r.sequence > sources.stored_through),
    );
    let mut seen = HashSet::new();
    sources.turns.reverse();
    sources
        .turns
        .retain(|r| seen.insert(r.event.thread_id().to_owned()));
    sources.turns.reverse();
    Ok(sources
        .turns
        .into_iter()
        .filter_map(terminal_turn_item)
        .chain(sources.waits.into_iter().flat_map(delivery_wait_items))
        .collect())
}
