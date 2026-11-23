use super::*;

const OWNER: &str = "CREATE TABLE task_verification_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton=1),
    owner_token TEXT
) STRICT";
const RUNS: &str = "CREATE TABLE task_verification_runs (
    request_id TEXT PRIMARY KEY NOT NULL,
    attempt_id TEXT UNIQUE NOT NULL,
    task_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    criterion_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal>=1),
    claim_fingerprint TEXT NOT NULL,
    owner_token TEXT NOT NULL,
    run_token TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN ('pending','running','finished')),
    lease_json TEXT NOT NULL,
    attempt_json TEXT NOT NULL,
    manifest_json TEXT,
    finish_fingerprint TEXT,
    receipt_json TEXT,
    finished_revision INTEGER,
    failure_limit_application_json TEXT CHECK (failure_limit_application_json IS NULL OR phase='finished'),
    CHECK ((phase='pending' AND manifest_json IS NULL AND receipt_json IS NULL)
        OR (phase='running' AND manifest_json IS NOT NULL AND receipt_json IS NULL)
        OR (phase='finished' AND receipt_json IS NOT NULL)),
    CHECK ((receipt_json IS NULL AND finish_fingerprint IS NULL AND finished_revision IS NULL)
        OR (receipt_json IS NOT NULL AND finish_fingerprint IS NOT NULL AND finished_revision>=1)),
    UNIQUE (scope_id,criterion_id,ordinal)
) STRICT";
const UNFINISHED: &str = "CREATE UNIQUE INDEX task_verification_one_unfinished
    ON task_verification_runs ((1)) WHERE phase!='finished'";
const LATEST: &str = "CREATE INDEX task_verification_latest
    ON task_verification_runs (task_id,criterion_id,scope_id,ordinal DESC)";
const OBJECTS: [(&str, &str); 4] = [
    ("task_verification_state", OWNER),
    ("task_verification_runs", RUNS),
    ("task_verification_one_unfinished", UNFINISHED),
    ("task_verification_latest", LATEST),
];

fn legacy_runs() -> String {
    RUNS.replace(
        "    failure_limit_application_json TEXT CHECK (failure_limit_application_json IS NULL OR phase='finished'),\n",
        "",
    )
}

fn validate(connection: &Connection, allow_legacy: bool) -> Result<bool> {
    let normalized = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut legacy = false;
    for (name, expected) in OBJECTS {
        let sql: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        let actual = sql.as_deref().map(normalized);
        if actual != Some(normalized(expected)) {
            if allow_legacy
                && name == "task_verification_runs"
                && actual == Some(normalized(&legacy_runs()))
            {
                legacy = true;
            } else {
                return Err(TaskEvidenceError::Unavailable.into());
            }
        }
    }
    let count: i64 =
        connection.query_row("SELECT count(*) FROM task_verification_state", [], |row| {
            row.get(0)
        })?;
    if count != 1 {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(legacy)
}

pub(super) fn ensure(connection: &mut Connection) -> Result<()> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name IN ('task_verification_state','task_verification_runs','task_verification_one_unfinished','task_verification_latest')",
        [],|row| row.get(0),
    )?;
    if count != 0 && !validate(connection, true)? {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !table_exists(&transaction, "task_verification_state")? {
        for (_, sql) in OBJECTS {
            transaction.execute_batch(sql)?;
        }
        transaction.execute("INSERT INTO task_verification_state VALUES(1,NULL)", [])?;
    } else if validate(&transaction, true)? {
        // The immutable receipt and all existing ordinals/quota facts stay untouched. A single
        // private decision marker prevents an old automatic request from pausing a resumed task.
        transaction.execute_batch(
            "ALTER TABLE task_verification_runs ADD COLUMN failure_limit_application_json TEXT
             CHECK (failure_limit_application_json IS NULL OR phase='finished')",
        )?;
    }
    validate(&transaction, false)?;
    transaction.commit()?;
    Ok(())
}
