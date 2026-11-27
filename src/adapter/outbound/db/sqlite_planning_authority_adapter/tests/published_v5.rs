use std::path::Path;

use rusqlite::{Connection, params};

use super::super::{
    AUTHORITY_STORE_SCHEMA_VERSION, SqlitePlanningAuthorityAdapter, open_authority_connection,
    prepare_private_authority_directory_tree, prepare_private_authority_store_file,
};
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::domain::planning::{PlanningAuthorityLocation, TaskStatus};

const PRIMARY: &str = "task-worker-20261127T205920Z-af05a4c04a1c";
const DEPENDENT: &str = "task-worker-20261127T205948Z-55522956acfc";

struct PublishedFixture {
    workspace: String,
    location: PlanningAuthorityLocation,
}

impl PublishedFixture {
    fn new() -> Self {
        let workspace = super::temp_workspace("published-v5");
        let location =
            SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace(&workspace)
                .expect("isolated location");
        let fixture = Self {
            workspace,
            location,
        };
        let path = Path::new(&fixture.location.authority_store_path);
        let _directories =
            prepare_private_authority_directory_tree(path.parent().expect("runtime parent"))
                .expect("private runtime");
        let _file = prepare_private_authority_store_file(path).expect("private fixture file");
        let connection = Connection::open(path).expect("fixture connection");
        connection
            .execute_batch(include_str!("published_v5.sql"))
            .expect("genuine published schema and synthetic rows");
        for (key, value) in [
            ("canonical_repo_root", &fixture.location.canonical_repo_root),
            ("workspace_root", &fixture.location.workspace_root),
        ] {
            connection
                .execute(
                    "UPDATE authority_metadata SET value = ?1 WHERE key = ?2",
                    params![value, key],
                )
                .expect("bind the disposable workspace");
        }
        fixture
    }

    fn connection(&self) -> Connection {
        Connection::open(&self.location.authority_store_path).expect("fixture read")
    }
}

impl Drop for PublishedFixture {
    fn drop(&mut self) {
        for path in [
            Path::new(&self.location.runtime_dir)
                .parent()
                .expect("owned project directory"),
            Path::new(&self.workspace),
        ] {
            if let Err(error) = std::fs::remove_dir_all(path) {
                assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
            }
        }
    }
}

fn task_rows(connection: &Connection) -> Vec<(String, String)> {
    connection
        .prepare("SELECT task_id, content_json FROM planning_tasks ORDER BY task_id")
        .expect("task row query")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("task rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("complete task rows")
}

fn schema_version(connection: &Connection) -> String {
    connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema marker")
}

#[test]
fn published_v5_preserves_tasks_dependencies_queue_revision_and_repository_binding() {
    let fixture = PublishedFixture::new();
    let before = task_rows(&fixture.connection());
    let adapter = SqlitePlanningAuthorityAdapter::new();

    let snapshot = adapter
        .load_task_authority_snapshot(&fixture.workspace)
        .expect("published v5 migrates")
        .expect("existing task authority");

    assert_eq!(snapshot.planning_revision, 4);
    assert_eq!(snapshot.task_authority.tasks.len(), 2);
    let primary = snapshot
        .task_authority
        .tasks
        .iter()
        .find(|task| task.id == PRIMARY)
        .expect("original primary ID");
    assert_eq!(primary.base_priority, 87);
    assert_eq!(
        primary.description,
        "Synthetic published-release task.\nKeep this description unchanged."
    );
    let dependent = snapshot
        .task_authority
        .tasks
        .iter()
        .find(|task| task.id == DEPENDENT)
        .expect("original dependent ID");
    assert_eq!(dependent.status, TaskStatus::Blocked);
    assert_eq!(dependent.depends_on, [PRIMARY]);
    assert_eq!(
        snapshot
            .queue_projection
            .next_task
            .as_ref()
            .expect("queue head")
            .task_id,
        PRIMARY
    );
    let connection = fixture.connection();
    assert_eq!(task_rows(&connection), before);
    assert_eq!(
        schema_version(&connection),
        AUTHORITY_STORE_SCHEMA_VERSION.to_string()
    );
    let identity: String = connection
        .query_row(
            "SELECT value FROM authority_metadata WHERE key = 'repository_identity'",
            [],
            |row| row.get(0),
        )
        .expect("migrated identity");
    assert_eq!(identity, fixture.location.repository_identity);
    drop(connection);
    assert_eq!(
        SqlitePlanningAuthorityAdapter::new()
            .load_task_authority_snapshot(&fixture.workspace)
            .expect("independent reopen"),
        Some(snapshot)
    );
}

#[test]
fn published_v5_migration_failure_rolls_back_schema_and_data_before_retry() {
    let fixture = PublishedFixture::new();
    let connection = fixture.connection();
    let before = task_rows(&connection);
    connection
        .execute_batch(
            "CREATE TRIGGER fail_schema_advance BEFORE UPDATE OF value ON authority_metadata
             WHEN NEW.key = 'schema_version' AND NEW.value != '5'
             BEGIN SELECT RAISE(ABORT, 'synthetic migration commit failure'); END;",
        )
        .expect("late migration failure");

    let error = open_authority_connection(&fixture.location).expect_err("migration must fail");

    assert!(format!("{error:#}").contains("synthetic migration commit failure"));
    assert_eq!(schema_version(&connection), "5");
    assert_eq!(task_rows(&connection), before);
    let new_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'planning_task_mutation_events'",
            [],
            |row| row.get(0),
        )
        .expect("rollback schema");
    assert_eq!(new_tables, 0);
    connection
        .execute_batch("DROP TRIGGER fail_schema_advance")
        .expect("remove only the injected fixture fault");
    drop(connection);
    let migrated = open_authority_connection(&fixture.location).expect("explicit retry");
    assert_eq!(task_rows(&migrated), before);
    assert_eq!(
        schema_version(&migrated),
        AUTHORITY_STORE_SCHEMA_VERSION.to_string()
    );
}

#[test]
fn published_v5_admission_keeps_version_mode_and_repository_fences() {
    for (key, value, reason) in [
        (
            "schema_version",
            "4",
            "unsupported authority-store schema version",
        ),
        ("mode", "foreign-store", "unsupported authority-store mode"),
        (
            "canonical_repo_root",
            "foreign-repository",
            "different canonical repository",
        ),
    ] {
        let fixture = PublishedFixture::new();
        let connection = fixture.connection();
        let before = task_rows(&connection);
        connection
            .execute(
                "UPDATE authority_metadata SET value = ?1 WHERE key = ?2",
                params![value, key],
            )
            .expect("install invalid boundary marker");

        let error = open_authority_connection(&fixture.location).expect_err("admission must fail");

        assert!(format!("{error:#}").contains(reason));
        assert_eq!(task_rows(&connection), before);
    }
}
