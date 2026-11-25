pub const CONTROL_TABLE: &str = "issue_collection_control";
pub const OBSERVATIONS_TABLE: &str = "issue_source_observations";

pub const CONTROL_SCHEMA: &str = r#"
CREATE TABLE issue_collection_control (
    workspace_dir TEXT COLLATE BINARY NOT NULL PRIMARY KEY,
    mode TEXT NOT NULL CHECK (mode IN ('off', 'observe')),
    epoch INTEGER NOT NULL CHECK (epoch >= 1),
    revision INTEGER NOT NULL CHECK (revision >= epoch),
    binding_id TEXT COLLATE BINARY,
    discovery_cursor TEXT,
    resync_cursor TEXT,
    last_attempt_at_ms INTEGER CHECK (last_attempt_at_ms IS NULL OR last_attempt_at_ms >= 0),
    last_success_at_ms INTEGER CHECK (last_success_at_ms IS NULL OR last_success_at_ms >= 0),
    last_failure TEXT,
    consecutive_failures INTEGER NOT NULL CHECK (consecutive_failures BETWEEN 0 AND 4294967295),
    CHECK (discovery_cursor IS NULL OR binding_id IS NOT NULL),
    CHECK ((last_failure IS NULL) = (consecutive_failures = 0)),
    CHECK (last_success_at_ms IS NULL OR last_attempt_at_ms IS NOT NULL),
    CHECK (last_failure IS NULL OR last_attempt_at_ms IS NOT NULL),
    FOREIGN KEY (binding_id) REFERENCES issue_automation_bindings (binding_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;

pub const OBSERVATIONS_SCHEMA: &str = r#"
CREATE TABLE issue_source_observations (
    provider TEXT COLLATE BINARY NOT NULL CHECK (provider IN ('github', 'jira', 'linear')),
    server_id TEXT COLLATE BINARY NOT NULL,
    repository_id TEXT COLLATE BINARY NOT NULL,
    issue_id TEXT COLLATE BINARY NOT NULL,
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    binding_id TEXT COLLATE BINARY NOT NULL,
    current_read TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    PRIMARY KEY (provider, server_id, repository_id, issue_id, workspace_dir),
    FOREIGN KEY (binding_id, provider, server_id, repository_id, workspace_dir)
        REFERENCES issue_automation_bindings
            (binding_id, provider, server_id, repository_id, workspace_dir)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID
"#;
