use super::super::SqlitePlanningAuthorityAdapter;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::domain::kernel_event::RecordedKernelEvent;
use anyhow::Result;
use rusqlite::Connection;

#[derive(Default)]
pub(crate) struct StoredAttentionSources {
    pub turns: Vec<RecordedKernelEvent>,
    pub waits: Vec<PlanningAuthorityDistributorQueueRecord>,
}

impl SqlitePlanningAuthorityAdapter {
    pub(crate) fn attention_sources(workspace: &str) -> Result<StoredAttentionSources> {
        let Some(location) = Self::resolve_authority_location_for_read(workspace)? else {
            return Ok(StoredAttentionSources::default());
        };
        let Some(mut store) = super::issue_timeline::read_only::open(&location)? else {
            return Ok(StoredAttentionSources::default());
        };
        let tx = store.connection.transaction()?;
        let sources = StoredAttentionSources::read(&tx)?;
        tx.commit()?;
        Ok(sources)
    }
}

impl StoredAttentionSources {
    fn read(db: &Connection) -> Result<Self> {
        let exists = |table: &str| {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get::<_, bool>(0),
            )
        };
        let mut sources = Self::default();
        if exists("kernel_events")? {
            // A later turn supersedes the prior terminal receipt even before the new turn ends.
            let mut query = db.prepare("WITH latest AS (SELECT max(sequence) AS sequence FROM kernel_events
                WHERE kind IN ('turn_started','turn_ended') GROUP BY thread_id)
                SELECT e.sequence,e.recorded_at,e.payload_json FROM latest JOIN kernel_events e USING(sequence)
                WHERE e.kind='turn_ended' ORDER BY e.sequence")?;
            let mut rows = query.query([])?;
            while let Some(row) = rows.next()? {
                sources.turns.push(RecordedKernelEvent {
                    sequence: row.get(0)?,
                    recorded_at: row.get(1)?,
                    event: serde_json::from_str(&row.get::<_, String>(2)?)?,
                });
            }
        }
        if exists("runtime_distributor_queue")? {
            let mut query = db.prepare(
                "SELECT content FROM runtime_distributor_queue WHERE queue_state='blocked'
                AND json_extract(content,'$.evidence_wait') IS NOT NULL ORDER BY queue_item_id",
            )?;
            let mut rows = query.query([])?;
            while let Some(row) = rows.next()? {
                sources
                    .waits
                    .push(serde_json::from_str(&row.get::<_, String>(0)?)?);
            }
        }
        Ok(sources)
    }
}
