use super::*;

const STATE: &str = "CREATE TABLE task_evidence_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    entry_count INTEGER NOT NULL CHECK (entry_count >= 0),
    artifact_bytes INTEGER NOT NULL CHECK (artifact_bytes >= 0),
    reserved_bytes INTEGER NOT NULL CHECK (reserved_bytes >= 0)
) STRICT";
const SCOPES: &str = "CREATE TABLE task_evidence_scopes (
    task_id TEXT PRIMARY KEY NOT NULL,
    scope_id TEXT UNIQUE NOT NULL,
    acceptance_revision INTEGER NOT NULL CHECK (acceptance_revision >= 0),
    criteria_sha256 TEXT NOT NULL,
    FOREIGN KEY (task_id) REFERENCES planning_tasks(task_id) ON DELETE CASCADE
) STRICT";
const ATTEMPTS: &str = "CREATE TABLE task_evidence_attempts (
    request_id TEXT PRIMARY KEY NOT NULL,
    attempt_id TEXT UNIQUE NOT NULL,
    task_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    criterion_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    prepare_fingerprint TEXT NOT NULL,
    lease_json TEXT NOT NULL,
    attempt_json TEXT NOT NULL,
    record_fingerprint TEXT,
    record_revision INTEGER,
    receipt_json TEXT,
    CHECK ((receipt_json IS NULL AND record_fingerprint IS NULL AND record_revision IS NULL)
        OR (receipt_json IS NOT NULL AND record_fingerprint IS NOT NULL AND record_revision >= 1)),
    UNIQUE (scope_id, criterion_id, ordinal)
) STRICT";
const WAIVERS: &str = "CREATE TABLE task_criterion_waivers (
    request_id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    scope_id TEXT NOT NULL,
    criterion_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    evidence_revision INTEGER NOT NULL CHECK (evidence_revision >= 1),
    receipt_json TEXT NOT NULL
) STRICT";
const EVENTS: &str = "CREATE TABLE task_evidence_events (
    event_id TEXT PRIMARY KEY NOT NULL,
    evidence_revision INTEGER UNIQUE NOT NULL CHECK (evidence_revision >= 1),
    payload_json TEXT NOT NULL,
    acknowledged INTEGER NOT NULL CHECK (acknowledged IN (0, 1))
) STRICT";
const ATTEMPTS_LOOKUP: &str = "CREATE INDEX task_evidence_attempts_latest
    ON task_evidence_attempts (task_id, criterion_id, record_revision DESC)";
const WAIVERS_LOOKUP: &str = "CREATE INDEX task_criterion_waivers_latest
    ON task_criterion_waivers (task_id, criterion_id, evidence_revision DESC)";

const OBJECTS: [(&str, &str); 7] = [
    ("task_evidence_state", STATE),
    ("task_evidence_scopes", SCOPES),
    ("task_evidence_attempts", ATTEMPTS),
    ("task_criterion_waivers", WAIVERS),
    ("task_evidence_events", EVENTS),
    ("task_evidence_attempts_latest", ATTEMPTS_LOOKUP),
    ("task_criterion_waivers_latest", WAIVERS_LOOKUP),
];

fn normalized_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate_schema(connection: &Connection) -> Result<()> {
    for (name, expected) in OBJECTS {
        let sql: Option<String> = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        if sql.as_deref().map(normalized_sql).as_deref() != Some(normalized_sql(expected).as_str())
        {
            return Err(TaskEvidenceError::Unavailable.into());
        }
    }
    let state_count: i64 =
        connection.query_row("SELECT count(*) FROM task_evidence_state", [], |row| {
            row.get(0)
        })?;
    if state_count != 1 {
        return Err(TaskEvidenceError::Unavailable.into());
    }
    Ok(())
}

pub(super) fn ensure(connection: &mut Connection) -> Result<()> {
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name IN (
        'task_evidence_state','task_evidence_scopes','task_evidence_attempts',
        'task_criterion_waivers','task_evidence_events','task_evidence_attempts_latest',
        'task_criterion_waivers_latest')",
        [],
        |row| row.get(0),
    )?;
    if count != 0 {
        return validate_schema(connection);
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // First feature open is serialized. Another connection may have installed the additive
    // schema while this one waited for the write lock.
    if table_exists(&transaction, "task_evidence_state")? {
        validate_schema(&transaction)?;
        transaction.commit()?;
        return Ok(());
    }
    for (_, sql) in OBJECTS {
        transaction.execute_batch(sql)?;
    }
    transaction.execute("INSERT INTO task_evidence_state VALUES (1,0,0,0,0)", [])?;
    let tasks = {
        let mut statement =
            transaction.prepare("SELECT content_json FROM planning_tasks ORDER BY task_id")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for json in tasks {
        let task: TaskDefinition = serde_json::from_str(&json)?;
        sync_task_scope(&transaction, &task)?;
    }
    validate_schema(&transaction)?;
    transaction.commit()?;
    Ok(())
}

pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter) fn sync_task_scope(
    transaction: &Transaction<'_>,
    task: &TaskDefinition,
) -> Result<()> {
    // Initial base-schema migration can save task rows before this additive feature exists.
    if !table_exists(transaction, "task_evidence_scopes")? {
        return Ok(());
    }
    let valid = validate_evidence_identifier(&task.id)
        .and({
            if task.acceptance_revision <= MAX_TASK_EVIDENCE_REVISION {
                Ok(())
            } else {
                Err(TaskEvidenceError::Limit)
            }
        })
        .and_then(|()| {
            crate::domain::planning::validate_task_acceptance(
                &task.acceptance,
                task.acceptance_revision,
            )
            .map_err(|_| TaskEvidenceError::Invalid)
        });
    let hash = criteria_sha256(&task.acceptance);
    if valid.is_err() || hash.is_err() {
        // Older valid task formats remain usable. Evidence for unsupported authority is closed.
        return retire_task_scope(transaction, &task.id);
    }
    let hash = hash?;
    let prior = read_scope(transaction, &task.id)?;
    if prior.as_ref().is_some_and(|scope| {
        scope.acceptance_revision == task.acceptance_revision && scope.criteria_sha256 == hash
    }) {
        return Ok(());
    }
    transaction.execute(
        "INSERT INTO task_evidence_scopes(task_id,scope_id,acceptance_revision,criteria_sha256)
        VALUES (?1,?2,?3,?4) ON CONFLICT(task_id) DO UPDATE SET
        scope_id=excluded.scope_id,acceptance_revision=excluded.acceptance_revision,criteria_sha256=excluded.criteria_sha256",
        params![task.id, random_token()?, task.acceptance_revision as i64, hash])?;
    bump_revision(transaction)?;
    Ok(())
}

pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter) fn retire_task_scope(
    transaction: &Transaction<'_>,
    task_id: &str,
) -> Result<()> {
    if table_exists(transaction, "task_evidence_scopes")?
        && transaction.execute(
            "DELETE FROM task_evidence_scopes WHERE task_id=?1",
            [task_id],
        )? != 0
    {
        bump_revision(transaction)?;
    }
    Ok(())
}

pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter) fn retire_all_scopes(
    transaction: &Transaction<'_>,
) -> Result<()> {
    if table_exists(transaction, "task_evidence_scopes")?
        && transaction.execute("DELETE FROM task_evidence_scopes", [])? != 0
    {
        bump_revision(transaction)?;
    }
    Ok(())
}
