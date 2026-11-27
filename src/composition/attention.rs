use crate::adapter::inbound::kernel_protocol::{
    AttentionItem, delivery_wait_items, terminal_turn_item,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use anyhow::Result;

pub(crate) fn stored_items(workspace: &str) -> Result<Vec<AttentionItem>> {
    let sources = SqlitePlanningAuthorityAdapter::attention_sources(workspace)?;
    Ok(sources
        .turns
        .into_iter()
        .filter_map(terminal_turn_item)
        .chain(sources.waits.into_iter().flat_map(delivery_wait_items))
        .collect())
}
