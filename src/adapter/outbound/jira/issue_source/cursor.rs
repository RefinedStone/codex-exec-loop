//! A completed updated window and an in-progress page share the existing atomic cursor column.
//! Search is eventually consistent: overlapping windows plus periodic full backfill are required.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::issue_automation::policy::IssueAutomationJiraBinding;
use crate::domain::issue_automation::source::IssueSourceFailure;
use crate::domain::provider_credentials::{
    ConnectionRef, Provider, valid_connection_id, valid_profile_id, valid_revision,
};

const OVERLAP_MS: u64 = 10 * 60 * 1000;
pub(super) const BACKFILL_MS: u64 = 24 * 60 * 60 * 1000;
const MAX_TIME: u64 = 9_007_199_254_740_991;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Cursor {
    version: u8,
    revision: u64,
    connection: ConnectionRef,
    filter_sha256: String,
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
        next_page_token: Option<String>,
    },
}

impl Cursor {
    pub(super) fn begin(
        raw: Option<&str>,
        binding: &IssueAutomationJiraBinding,
        connection: &ConnectionRef,
        now: u64,
    ) -> Result<Self, IssueSourceFailure> {
        if now == 0 || now > MAX_TIME {
            return Err(IssueSourceFailure::InvalidRequest);
        }
        validate_filter(&binding.jql)?;
        let filter_sha256 = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    &binding.site_url,
                    &binding.cloud_id,
                    &binding.project_id,
                    &binding.expected_account_id,
                    &binding.jql,
                ))
                .map_err(|_| IssueSourceFailure::InvalidRequest)?
            )
        );
        let mut cursor = match raw {
            Some(raw) => {
                let parsed: Self =
                    serde_json::from_str(raw).map_err(|_| IssueSourceFailure::InvalidCursor)?;
                parsed.validate()?;
                if parsed.connection == *connection && parsed.filter_sha256 == filter_sha256 {
                    parsed
                } else {
                    Self::new(connection.clone(), filter_sha256)
                }
            }
            None => Self::new(connection.clone(), filter_sha256),
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
                next_page_token: None,
            };
        }
        Ok(cursor)
    }

    fn new(connection: ConnectionRef, filter_sha256: String) -> Self {
        Self {
            version: 1,
            revision: 0,
            connection,
            filter_sha256,
            watermark: 0,
            last_backfill: 0,
            phase: Phase::Idle {},
        }
    }

    pub(super) fn restart_full(&self, now: u64) -> Self {
        let mut next = Self::new(self.connection.clone(), self.filter_sha256.clone());
        next.revision = self.revision;
        next.phase = Phase::Pages {
            from: 0,
            through: now.max(self.watermark),
            full: true,
            next_page_token: None,
        };
        next
    }

    pub(super) fn query(
        &self,
        binding: &IssueAutomationJiraBinding,
    ) -> Result<String, IssueSourceFailure> {
        let Phase::Pages { from, through, .. } = self.phase else {
            return Err(IssueSourceFailure::InvalidCursor);
        };
        // Unquoted integers are epoch milliseconds, avoiding the account timezone used by date strings.
        // https://support.atlassian.com/jira-software-cloud/docs/jql-fields/#Updated
        Ok(format!(
            "project = {} AND ({}) AND statusCategory != done AND updated >= {from} AND updated < {through} ORDER BY updated ASC, id ASC",
            binding.project_id, binding.jql
        ))
    }

    pub(super) fn token(&self) -> Option<&str> {
        match &self.phase {
            Phase::Pages {
                next_page_token, ..
            } => next_page_token.as_deref(),
            Phase::Idle {} => None,
        }
    }

    pub(super) fn finish_page(
        mut self,
        is_last: bool,
        token: Option<String>,
    ) -> Result<String, IssueSourceFailure> {
        let Phase::Pages {
            from,
            through,
            full,
            ..
        } = self.phase
        else {
            return Err(IssueSourceFailure::InvalidCursor);
        };
        if is_last {
            self.watermark = through;
            if full {
                self.last_backfill = through;
            }
            self.phase = Phase::Idle {};
        } else {
            let token = token
                .filter(|value| valid_token(value))
                .ok_or(IssueSourceFailure::MalformedResponse)?;
            if self.token() == Some(token.as_str()) {
                return Err(IssueSourceFailure::MalformedResponse);
            }
            self.phase = Phase::Pages {
                from,
                through,
                full,
                next_page_token: Some(token),
            };
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
        let digest = self.filter_sha256.len() == 64
            && self
                .filter_sha256
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        if self.version != 1
            || self.revision > MAX_TIME
            || !digest
            || self.connection.provider != Provider::Jira
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
            next_page_token,
            ..
        } = &self.phase
            && (*from > *through
                || *through == 0
                || *through > MAX_TIME
                || *through < self.watermark
                || next_page_token
                    .as_ref()
                    .is_some_and(|value| !valid_token(value)))
        {
            return Err(IssueSourceFailure::InvalidCursor);
        }
        Ok(())
    }
}

fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 2048 && !value.chars().any(char::is_control)
}

/// Only grouping/quoted-text safety is interpreted locally. Jira validates the JQL grammar.
/// A predicate cannot close the host's parentheses and bypass its project/window clauses.
pub(super) fn validate_filter(value: &str) -> Result<(), IssueSourceFailure> {
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 0u32;
    let mut words = String::new();
    for character in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if let Some(active) = quote {
            if character == '\\' {
                escaped = true;
            } else if character == active {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                words.push(' ');
            }
            '(' => {
                depth = depth
                    .checked_add(1)
                    .filter(|depth| *depth <= 64)
                    .ok_or(IssueSourceFailure::InvalidRequest)?;
                words.push(' ');
            }
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or(IssueSourceFailure::InvalidRequest)?;
                words.push(' ');
            }
            c if c.is_ascii_alphabetic() => words.push(c.to_ascii_lowercase()),
            _ => words.push(' '),
        }
    }
    if quote.is_some()
        || escaped
        || depth != 0
        || words
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| pair == ["order", "by"])
    {
        return Err(IssueSourceFailure::InvalidRequest);
    }
    Ok(())
}
