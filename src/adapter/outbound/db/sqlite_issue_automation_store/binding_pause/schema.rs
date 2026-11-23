use super::*;

pub(super) const STATE_TABLE: &str = "issue_binding_pause_states";
pub(super) const COMMAND_TABLE: &str = "issue_binding_pause_commands";
pub(super) const FENCE_TABLE: &str = "issue_binding_pause_work_fences";

const STATE_SCHEMA: &str = r#"
CREATE TABLE issue_binding_pause_states (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    binding_id TEXT COLLATE BINARY NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    paused INTEGER NOT NULL CHECK (paused IN (0,1)),
    pending_command_id TEXT COLLATE BINARY,
    PRIMARY KEY (workspace_dir,binding_id),
    CHECK (revision > 0 OR (paused=0 AND pending_command_id IS NULL)),
    FOREIGN KEY (binding_id) REFERENCES issue_automation_bindings(binding_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (workspace_dir,pending_command_id)
        REFERENCES issue_binding_pause_commands(workspace_dir,command_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
const COMMAND_SCHEMA: &str = r#"
CREATE TABLE issue_binding_pause_commands (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    command_id TEXT COLLATE BINARY NOT NULL,
    binding_id TEXT COLLATE BINARY NOT NULL,
    intent_json TEXT NOT NULL,
    accepted_revision INTEGER NOT NULL CHECK (accepted_revision >= 0),
    receipt_json TEXT,
    event_json TEXT,
    acknowledged INTEGER NOT NULL DEFAULT 0 CHECK (acknowledged IN (0,1)),
    PRIMARY KEY (workspace_dir,command_id),
    CHECK ((receipt_json IS NULL) = (event_json IS NULL)),
    CHECK (receipt_json IS NOT NULL OR acknowledged=0),
    FOREIGN KEY (workspace_dir,binding_id)
        REFERENCES issue_binding_pause_states(workspace_dir,binding_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
const FENCE_SCHEMA: &str = r#"
CREATE TABLE issue_binding_pause_work_fences (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    pause_command_id TEXT COLLATE BINARY NOT NULL,
    work_order_id TEXT COLLATE BINARY NOT NULL,
    control_command_id TEXT COLLATE BINARY NOT NULL,
    PRIMARY KEY (workspace_dir,pause_command_id,work_order_id),
    UNIQUE (workspace_dir,control_command_id),
    FOREIGN KEY (workspace_dir,pause_command_id)
        REFERENCES issue_binding_pause_commands(workspace_dir,command_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (workspace_dir,work_order_id)
        REFERENCES issue_work_checkpoints(workspace_dir,work_order_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (workspace_dir,control_command_id)
        REFERENCES issue_intervention_commands(workspace_dir,command_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
const PENDING_INDEX: &str = "CREATE INDEX issue_binding_pause_pending ON issue_binding_pause_states(workspace_dir,binding_id) WHERE pending_command_id IS NOT NULL";
const EVENT_INDEX: &str = "CREATE INDEX issue_binding_pause_unacknowledged ON issue_binding_pause_commands(workspace_dir,binding_id,command_id) WHERE event_json IS NOT NULL AND acknowledged=0";

pub(in super::super) fn schema_objects() -> Vec<(&'static str, &'static str)> {
    vec![
        (STATE_TABLE, STATE_SCHEMA),
        (COMMAND_TABLE, COMMAND_SCHEMA),
        (FENCE_TABLE, FENCE_SCHEMA),
        ("issue_binding_pause_pending", PENDING_INDEX),
        ("issue_binding_pause_unacknowledged", EVENT_INDEX),
    ]
}

pub(in super::super) fn create_schema(connection: &Connection) -> Result<()> {
    for (_, sql) in schema_objects() {
        connection.execute_batch(sql)?;
    }
    connection.execute(
        "INSERT INTO issue_binding_pause_states(workspace_dir,binding_id,revision,paused)
         SELECT workspace_dir,binding_id,0,0 FROM issue_automation_bindings",
        [],
    )?;
    Ok(())
}

pub(in super::super) fn initialize_binding(
    connection: &Connection,
    workspace: &str,
    binding_id: &IssueAutomationBindingId,
) -> Result<()> {
    connection.execute(
        "INSERT INTO issue_binding_pause_states(workspace_dir,binding_id,revision,paused)
         VALUES (?1,?2,0,0) ON CONFLICT(workspace_dir,binding_id) DO NOTHING",
        params![workspace, binding_id.as_str()],
    )?;
    Ok(())
}
