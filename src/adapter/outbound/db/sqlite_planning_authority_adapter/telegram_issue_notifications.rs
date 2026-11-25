use anyhow::{Context, Result, bail};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::application::port::outbound::telegram_issue_notification_port::{
    TelegramIssueNotificationEnd, TelegramIssueNotificationLedgerPort,
};

use super::{SqlitePlanningAuthorityAdapter, open_authority_connection};

const MAX_STREAM_KEY_BYTES: usize = 128;
const MAX_INTENT_KEY_BYTES: usize = 512;
// 끝난 intent 는 cursor 뒤의 사건이므로 다시 읽히지 않는다. 최근 행만 남긴다.
const FINISHED_NOTIFICATION_RETENTION: i64 = 4096;

/*
 * 표는 `ensure_schema` 가 아니라 여기서 만든다 (kernel_events 와 같다). 현재 판의 기존 DB 는 `ensure_schema` 가
 * DDL 전에 끝나고, schema version 을 올리면 이전 binary 가 이 DB 를 열지 못한다.
 */
fn open_ledger(adapter_workspace: &str) -> Result<Connection> {
    let location = SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(
        adapter_workspace,
    )?;
    let connection = open_authority_connection(&location)?;
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS telegram_issue_notification_cursors (
                stream_key TEXT PRIMARY KEY,
                after_sequence INTEGER NOT NULL CHECK (after_sequence >= 0),
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS telegram_issue_notifications (
                stream_key TEXT NOT NULL,
                intent_key TEXT NOT NULL,
                chat_id INTEGER NOT NULL,
                delivery_state TEXT NOT NULL
                    CHECK (delivery_state IN ('sending', 'sent', 'unknown')),
                claimed_at TEXT NOT NULL,
                finished_at TEXT,
                PRIMARY KEY (stream_key, intent_key, chat_id)
            );",
        )
        .context("failed to ensure the Telegram issue notification tables")?;
    Ok(connection)
}

impl TelegramIssueNotificationLedgerPort for SqlitePlanningAuthorityAdapter {
    fn recover_issue_notifications(
        &self,
        workspace_dir: &str,
        stream_key: &str,
    ) -> Result<Option<i64>> {
        validate_key(stream_key, MAX_STREAM_KEY_BYTES)?;
        let mut connection = open_ledger(workspace_dir)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram issue notification recovery")?;
        // runner 는 하나이고 주기마다 전송을 끝낸다. 남은 sending 은 끊긴 전송이다.
        transaction
            .execute(
                "UPDATE telegram_issue_notifications
                 SET delivery_state = 'unknown', finished_at = ?2
                 WHERE stream_key = ?1 AND delivery_state = 'sending'",
                params![stream_key, Utc::now().to_rfc3339()],
            )
            .context("failed to recover interrupted Telegram issue notifications")?;
        let cursor = transaction
            .query_row(
                "SELECT after_sequence FROM telegram_issue_notification_cursors
                 WHERE stream_key = ?1",
                [stream_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .context("failed to load Telegram issue notification cursor")?;
        transaction
            .commit()
            .context("failed to commit Telegram issue notification recovery")?;
        Ok(cursor)
    }

    fn save_issue_notification_cursor(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        after: i64,
    ) -> Result<()> {
        validate_key(stream_key, MAX_STREAM_KEY_BYTES)?;
        if after < 0 {
            bail!("Telegram issue notification cursor must be non-negative");
        }
        let connection = open_ledger(workspace_dir)?;
        // 기록이 초기화되면 cursor 는 뒤로 간다. 그래서 MAX 로 묶지 않는다.
        connection
            .execute(
                "INSERT INTO telegram_issue_notification_cursors (stream_key, after_sequence, updated_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(stream_key) DO UPDATE SET
                     after_sequence = excluded.after_sequence,
                     updated_at = excluded.updated_at",
                params![stream_key, after, Utc::now().to_rfc3339()],
            )
            .context("failed to persist Telegram issue notification cursor")?;
        Ok(())
    }

    fn claim_issue_notification(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        intent_key: &str,
        chat_id: i64,
    ) -> Result<bool> {
        validate_key(stream_key, MAX_STREAM_KEY_BYTES)?;
        validate_key(intent_key, MAX_INTENT_KEY_BYTES)?;
        let connection = open_ledger(workspace_dir)?;
        let inserted = connection
            .execute(
                "INSERT OR IGNORE INTO telegram_issue_notifications
                 (stream_key, intent_key, chat_id, delivery_state, claimed_at)
                 VALUES (?1, ?2, ?3, 'sending', ?4)",
                params![stream_key, intent_key, chat_id, Utc::now().to_rfc3339()],
            )
            .context("failed to claim a Telegram issue notification")?;
        Ok(inserted == 1)
    }

    fn finish_issue_notification(
        &self,
        workspace_dir: &str,
        stream_key: &str,
        intent_key: &str,
        chat_id: i64,
        end: TelegramIssueNotificationEnd,
    ) -> Result<()> {
        validate_key(stream_key, MAX_STREAM_KEY_BYTES)?;
        validate_key(intent_key, MAX_INTENT_KEY_BYTES)?;
        let state = match end {
            TelegramIssueNotificationEnd::Sent => "sent",
            TelegramIssueNotificationEnd::Unknown => "unknown",
        };
        let mut connection = open_ledger(workspace_dir)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to open Telegram issue notification completion")?;
        let changed = transaction
            .execute(
                "UPDATE telegram_issue_notifications
                 SET delivery_state = ?4, finished_at = ?5
                 WHERE stream_key = ?1 AND intent_key = ?2 AND chat_id = ?3
                   AND delivery_state = 'sending'",
                params![
                    stream_key,
                    intent_key,
                    chat_id,
                    state,
                    Utc::now().to_rfc3339()
                ],
            )
            .context("failed to finish a Telegram issue notification")?;
        if changed != 1 {
            bail!("Telegram issue notification was not claimed for sending");
        }
        transaction
            .execute(
                "DELETE FROM telegram_issue_notifications
                 WHERE stream_key = ?1 AND delivery_state != 'sending'
                   AND rowid < COALESCE(
                       (SELECT rowid FROM telegram_issue_notifications
                        WHERE stream_key = ?1
                        ORDER BY rowid DESC
                        LIMIT 1 OFFSET ?2),
                       0
                   )",
                params![stream_key, FINISHED_NOTIFICATION_RETENTION],
            )
            .context("failed to prune Telegram issue notifications")?;
        transaction
            .commit()
            .context("failed to commit Telegram issue notification completion")?;
        Ok(())
    }
}

fn validate_key(key: &str, max_bytes: usize) -> Result<()> {
    if key.is_empty() || key.len() > max_bytes || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        bail!(
            "Telegram issue notification key must be printable ASCII and at most {max_bytes} bytes"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(label: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-telegram-issue-notifications-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn one_intent_is_claimed_once_and_an_interrupted_send_becomes_unknown_after_restart() {
        let path = workspace("restart");
        let workspace = path.to_string_lossy().into_owned();
        let stream = "bot-id:42";
        let ledger = SqlitePlanningAuthorityAdapter::new();
        // 이전 binary 가 같은 schema version 으로 만든 DB 이다. 새 표가 없어도 처음 쓸 때 만든다.
        let location =
            SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace)
                .unwrap();
        open_authority_connection(&location)
            .unwrap()
            .execute_batch(
                "DROP TABLE IF EXISTS telegram_issue_notification_cursors;
                 DROP TABLE IF EXISTS telegram_issue_notifications;",
            )
            .unwrap();
        assert_eq!(
            ledger
                .recover_issue_notifications(&workspace, stream)
                .unwrap(),
            None
        );
        ledger
            .save_issue_notification_cursor(&workspace, stream, 7)
            .unwrap();
        assert!(
            ledger
                .claim_issue_notification(&workspace, stream, "decision_requested:d1", 5)
                .unwrap()
        );
        ledger
            .finish_issue_notification(
                &workspace,
                stream,
                "decision_requested:d1",
                5,
                TelegramIssueNotificationEnd::Sent,
            )
            .unwrap();
        // 전송 중에 끊겼다. 다음 runner 는 이 intent 를 다시 보내지 않는다.
        assert!(
            ledger
                .claim_issue_notification(&workspace, stream, "decision_requested:d2", 5)
                .unwrap()
        );

        let restarted = SqlitePlanningAuthorityAdapter::new();
        assert_eq!(
            restarted
                .recover_issue_notifications(&workspace, stream)
                .unwrap(),
            Some(7)
        );
        for intent in ["decision_requested:d1", "decision_requested:d2"] {
            assert!(
                !restarted
                    .claim_issue_notification(&workspace, stream, intent, 5)
                    .unwrap(),
                "{intent}"
            );
        }
        let state: String = open_authority_connection(&location)
            .unwrap()
            .query_row(
                "SELECT delivery_state FROM telegram_issue_notifications WHERE intent_key = ?1",
                ["decision_requested:d2"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "unknown");
        // 같은 intent 도 다른 chat 에는 한 번 보낸다.
        assert!(
            restarted
                .claim_issue_notification(&workspace, stream, "decision_requested:d1", 6)
                .unwrap()
        );
        let _ = std::fs::remove_dir_all(path);
    }
}
