use std::{path::PathBuf, time::Duration};

use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::application::service::pr_review::{
    CommentReservation, PrReviewJournal, ReviewMode, ReviewRecord, StoredPrReview,
};
use crate::domain::github_review::GithubPullRequestTarget;

pub struct ReviewJournal(PathBuf);

impl ReviewJournal {
    pub fn new(runtime: PathBuf) -> Self {
        Self(runtime.join("pr-review.sqlite3"))
    }

    fn open(&self) -> Result<Connection> {
        let db = Connection::open(&self.0)?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS bindings (
                 repo TEXT PRIMARY KEY COLLATE NOCASE, mode TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS reviews (
                 repo TEXT COLLATE NOCASE, pr INTEGER, head TEXT, result TEXT,
                 PRIMARY KEY(repo, pr, head)
             );
             CREATE TABLE IF NOT EXISTS review_comment_consent (
                 repo TEXT COLLATE NOCASE, pr INTEGER, head TEXT, PRIMARY KEY(repo,pr,head)
             );
             CREATE TABLE IF NOT EXISTS comments (
                 repo TEXT COLLATE NOCASE, pr INTEGER, head TEXT NOT NULL, id INTEGER,
                 pending INTEGER NOT NULL, PRIMARY KEY(repo, pr)
             );",
        )?;
        Ok(db)
    }

    /// Expose only through an authenticated human settings path.
    pub fn set_mode(&self, repository: &str, mode: ReviewMode) -> Result<()> {
        self.open()?.execute(
            "INSERT INTO bindings VALUES (?1, ?2) ON CONFLICT(repo) DO UPDATE SET mode=excluded.mode",
            params![repository, serde_json::to_string(&mode)?],
        )?;
        Ok(())
    }

    pub fn result(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
    ) -> Result<Option<ReviewRecord>> {
        let value: Option<String> = self
            .open()?
            .query_row(
                "SELECT result FROM reviews WHERE repo=?1 AND pr=?2 AND head=?3",
                params![target.repository, target.number, head],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub fn latest(&self, target: &GithubPullRequestTarget) -> Result<Option<StoredPrReview>> {
        if !self.0.exists() {
            return Ok(None);
        }
        let stored = self.open()?.query_row(
            "SELECT head, result FROM reviews WHERE repo=?1 AND pr=?2 ORDER BY rowid DESC LIMIT 1",
            params![target.repository, target.number],
            |row| Ok((row.get(0)?, row.get::<_, Option<String>>(1)?)),
        ).optional()?;
        stored
            .map(|(head, result)| {
                Ok(StoredPrReview {
                    head,
                    result: result
                        .map(|result| serde_json::from_str(&result))
                        .transpose()?,
                })
            })
            .transpose()
    }
}

impl PrReviewJournal for ReviewJournal {
    fn mode(&self, repository: &str) -> Result<ReviewMode> {
        if !self.0.exists() {
            return Ok(ReviewMode::Off);
        }
        let mode: Option<String> = self
            .open()?
            .query_row(
                "SELECT mode FROM bindings WHERE repo=?1",
                [repository],
                |row| row.get(0),
            )
            .optional()?;
        Ok(mode
            .map(|mode| serde_json::from_str(&mode))
            .transpose()?
            .unwrap_or_default())
    }

    fn claim(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
        may_comment: bool,
    ) -> Result<bool> {
        let mut db = self.open()?;
        let txn = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = txn.execute(
            "INSERT OR IGNORE INTO reviews VALUES (?1, ?2, ?3, NULL)",
            params![target.repository, target.number, head],
        )?;
        if inserted == 0 {
            return Ok(false);
        }
        if may_comment {
            txn.execute(
                "INSERT INTO review_comment_consent(repo,pr,head) VALUES (?1, ?2, ?3)",
                params![target.repository, target.number, head],
            )?;
        }
        txn.execute(
            "UPDATE reviews SET result=?4 WHERE repo=?1 AND pr=?2 AND head<>?3 AND result IS NULL",
            params![
                target.repository,
                target.number,
                head,
                serde_json::to_string(&ReviewRecord::Cancelled)?
            ],
        )?;
        txn.commit()?;
        Ok(true)
    }

    fn pending(&self, target: &GithubPullRequestTarget, head: &str) -> Result<bool> {
        Ok(self.open()?.query_row("SELECT EXISTS(SELECT 1 FROM reviews WHERE repo=?1 AND pr=?2 AND head=?3 AND result IS NULL)", params![target.repository, target.number, head], |row| row.get(0))?)
    }

    fn finish(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
        result: &ReviewRecord,
    ) -> Result<bool> {
        Ok(self.open()?.execute(
            "UPDATE reviews SET result=?4 WHERE repo=?1 AND pr=?2 AND head=?3 AND result IS NULL",
            params![
                target.repository,
                target.number,
                head,
                serde_json::to_string(result)?
            ],
        )? == 1)
    }

    fn reserve_comment(
        &self,
        target: &GithubPullRequestTarget,
        head: &str,
    ) -> Result<CommentReservation> {
        let mut db = self.open()?;
        let txn = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mode: Option<String> = txn
            .query_row(
                "SELECT mode FROM bindings WHERE repo=?1",
                [&target.repository],
                |row| row.get(0),
            )
            .optional()?;
        if mode.as_deref() != Some("\"on\"") || !txn.query_row(
            "SELECT EXISTS(SELECT 1 FROM review_comment_consent WHERE repo=?1 AND pr=?2 AND head=?3)",
            params![target.repository, target.number, head], |r| r.get::<_, bool>(0))? {
            return Ok(CommentReservation::Suppressed);
        }
        let existing: Option<(String, Option<u64>, bool)> = txn
            .query_row(
                "SELECT head, id, pending FROM comments WHERE repo=?1 AND pr=?2",
                params![target.repository, target.number],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if existing
            .as_ref()
            .is_some_and(|(previous, _, pending)| *pending || previous == head)
        {
            return Ok(CommentReservation::Suppressed);
        }
        txn.execute(
            "INSERT INTO comments VALUES (?1, ?2, ?3, NULL, 1)
             ON CONFLICT(repo, pr) DO UPDATE SET head=excluded.head, pending=1",
            params![target.repository, target.number, head],
        )?;
        txn.commit()?;
        Ok(CommentReservation::Write(
            existing.and_then(|(_, id, _)| id),
        ))
    }

    fn record_comment(&self, target: &GithubPullRequestTarget, head: &str, id: u64) -> Result<()> {
        ensure!(id > 0, "invalid GitHub comment id");
        let changed = self.open()?.execute(
            "UPDATE comments SET id=?4, pending=0 WHERE repo=?1 AND pr=?2 AND head=?3 AND pending=1",
            params![target.repository, target.number, head, id],
        )?;
        ensure!(changed == 1, "PR review comment reservation was lost");
        Ok(())
    }
}
