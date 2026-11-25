//! A completed `updatedAt` window and an in-progress page share the existing atomic cursor column.
//! The provider page cursor and the incremental window are separate: the watermark advances only
//! after the last page of a window. Overlapping windows plus a periodic full backfill re-read
//! issues near a boundary; the store deduplicates them by stable ID.
use chrono::{DateTime, SecondsFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::issue_automation::policy::IssueAutomationLinearBinding;
use crate::domain::issue_automation::source::IssueSourceFailure;
use crate::domain::provider_credentials::{
    ConnectionRef, Provider, valid_connection_id, valid_profile_id, valid_revision,
};

const OVERLAP_MS: u64 = 10 * 60 * 1000;
pub(super) const BACKFILL_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_TIME: u64 = 9_007_199_254_740_991;
const MAX_PROVIDER_CURSOR_BYTES: usize = 2048;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Cursor {
    version: u8,
    revision: u64,
    connection: ConnectionRef,
    binding_sha256: String,
    watermark: u64,
    last_backfill: u64,
    phase: Phase,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Phase {
    Idle {},
    Pages {
        from: u64,
        through: u64,
        full: bool,
        after: Option<String>,
    },
}

/// The typed window of one provider page read.
pub(super) struct Window {
    pub(super) from: String,
    pub(super) before: String,
    pub(super) after: Option<String>,
}

impl Cursor {
    pub(super) fn begin(
        raw: Option<&str>,
        binding: &IssueAutomationLinearBinding,
        connection: &ConnectionRef,
        now: u64,
    ) -> Result<Self, IssueSourceFailure> {
        if now == 0 || now > MAX_TIME {
            return Err(IssueSourceFailure::InvalidRequest);
        }
        let binding_sha256 = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&binding.team_id, &binding.expected_viewer_id))
                    .map_err(|_| IssueSourceFailure::InvalidRequest)?
            )
        );
        let mut cursor = match raw {
            Some(raw) => {
                let parsed: Self =
                    serde_json::from_str(raw).map_err(|_| IssueSourceFailure::InvalidCursor)?;
                parsed.validate()?;
                // Another credential generation or team starts a new full read.
                if parsed.connection == *connection && parsed.binding_sha256 == binding_sha256 {
                    parsed
                } else {
                    Self::new(connection.clone(), binding_sha256)
                }
            }
            None => Self::new(connection.clone(), binding_sha256),
        };
        if matches!(cursor.phase, Phase::Idle {}) {
            let through = now.max(cursor.watermark);
            let full = cursor.last_backfill == 0
                || through.saturating_sub(cursor.last_backfill) >= BACKFILL_MS;
            cursor.phase = Phase::Pages {
                from: if full {
                    0
                } else {
                    cursor.watermark.saturating_sub(OVERLAP_MS)
                },
                through,
                full,
                after: None,
            };
        }
        Ok(cursor)
    }

    fn new(connection: ConnectionRef, binding_sha256: String) -> Self {
        Self {
            version: 1,
            revision: 0,
            connection,
            binding_sha256,
            watermark: 0,
            last_backfill: 0,
            phase: Phase::Idle {},
        }
    }

    /// A rejected continuation is a new full read, never advancement past an unread page.
    pub(super) fn restart_full(&self, now: u64) -> Self {
        let mut next = Self::new(self.connection.clone(), self.binding_sha256.clone());
        next.revision = self.revision;
        next.phase = Phase::Pages {
            from: 0,
            through: now.max(self.watermark),
            full: true,
            after: None,
        };
        next
    }

    pub(super) fn window(&self) -> Result<Window, IssueSourceFailure> {
        let Phase::Pages {
            from,
            through,
            after,
            ..
        } = &self.phase
        else {
            return Err(IssueSourceFailure::InvalidCursor);
        };
        Ok(Window {
            from: rfc3339(*from)?,
            before: rfc3339(*through)?,
            after: after.clone(),
        })
    }

    pub(super) fn finish_page(
        mut self,
        has_next_page: bool,
        end_cursor: Option<String>,
    ) -> Result<String, IssueSourceFailure> {
        let Phase::Pages {
            from,
            through,
            full,
            after,
        } = &self.phase
        else {
            return Err(IssueSourceFailure::InvalidCursor);
        };
        let (from, through, full) = (*from, *through, *full);
        if has_next_page {
            let end_cursor = end_cursor
                .filter(|value| valid_provider_cursor(value))
                .ok_or(IssueSourceFailure::MalformedResponse)?;
            if after.as_deref() == Some(end_cursor.as_str()) {
                return Err(IssueSourceFailure::MalformedResponse);
            }
            self.phase = Phase::Pages {
                from,
                through,
                full,
                after: Some(end_cursor),
            };
        } else {
            self.watermark = through;
            if full {
                self.last_backfill = through;
            }
            self.phase = Phase::Idle {};
        }
        // A repeated or backwards clock must still publish a different atomic checkpoint.
        self.revision = self
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_TIME)
            .ok_or(IssueSourceFailure::InvalidCursor)?;
        self.validate()?;
        let raw =
            serde_json::to_string(&self).map_err(|_| IssueSourceFailure::MalformedResponse)?;
        if raw.len() > 4096 {
            return Err(IssueSourceFailure::MalformedResponse);
        }
        Ok(raw)
    }

    fn validate(&self) -> Result<(), IssueSourceFailure> {
        let digest = self.binding_sha256.len() == 64
            && self
                .binding_sha256
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        if self.version != 1
            || self.revision > MAX_TIME
            || !digest
            || self.connection.provider != Provider::Linear
            || !valid_profile_id(&self.connection.profile_id)
            || !valid_connection_id(&self.connection.connection_id)
            || self.connection.revision == 0
            || !valid_revision(self.connection.revision)
            || self.watermark > MAX_TIME
            || self.last_backfill > self.watermark
        {
            return Err(IssueSourceFailure::InvalidCursor);
        }
        if let Phase::Pages {
            from,
            through,
            after,
            ..
        } = &self.phase
            && (*from > *through
                || *through == 0
                || *through > MAX_TIME
                || *through < self.watermark
                || after
                    .as_ref()
                    .is_some_and(|value| !valid_provider_cursor(value)))
        {
            return Err(IssueSourceFailure::InvalidCursor);
        }
        Ok(())
    }
}

fn valid_provider_cursor(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROVIDER_CURSOR_BYTES
        && !value.chars().any(char::is_control)
}

fn rfc3339(milliseconds: u64) -> Result<String, IssueSourceFailure> {
    let value = i64::try_from(milliseconds).map_err(|_| IssueSourceFailure::InvalidCursor)?;
    DateTime::from_timestamp_millis(value)
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or(IssueSourceFailure::InvalidCursor)
}
