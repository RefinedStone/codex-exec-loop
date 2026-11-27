//! Same private runtime directory for all worktrees. Never store rules in repository configuration.
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::domain::git_protection::ProtectedBranchPolicy;
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;

pub(crate) struct PolicyStore(Connection);
impl PolicyStore {
    pub(crate) fn open(cwd: &str) -> Result<Self> {
        let location = SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(cwd)?;
        Self::at(Path::new(&location.runtime_dir))
    }
    pub(super) fn at(dir: &Path) -> Result<Self> {
        let connection = Connection::open(dir.join("git-branch-protection.db"))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS branch_policy (destination TEXT PRIMARY KEY, state TEXT NOT NULL); CREATE TABLE IF NOT EXISTS policy_log (id INTEGER PRIMARY KEY, at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, destination TEXT NOT NULL, action TEXT NOT NULL, state TEXT NOT NULL);")?;
        Ok(Self(connection))
    }
    fn read(db: &Connection, destination: &str) -> Result<ProtectedBranchPolicy> {
        let raw: Option<String> = db
            .query_row(
                "SELECT state FROM branch_policy WHERE destination=?",
                [destination],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match raw {
            Some(s) => serde_json::from_str(&s)?,
            None => ProtectedBranchPolicy::default(),
        })
    }
    pub(crate) fn policy(&self, destination: &str) -> Result<ProtectedBranchPolicy> {
        Self::read(&self.0, destination)
    }
    pub(crate) fn save(
        &mut self,
        destination: &str,
        seen: &ProtectedBranchPolicy,
        next: &ProtectedBranchPolicy,
    ) -> Result<()> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            Self::read(&tx, destination)? == *seen,
            "the protection list changed; review it again"
        );
        let state = serde_json::to_string(next)?;
        tx.execute("INSERT INTO branch_policy VALUES (?1,?2) ON CONFLICT(destination) DO UPDATE SET state=excluded.state",params![destination,state])?;
        tx.execute(
            "INSERT INTO policy_log(destination,action,state) VALUES(?1,'review rules',?2)",
            params![destination, state],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn guarded<T>(
        &mut self,
        destination: &str,
        run: impl FnOnce(&ProtectedBranchPolicy) -> Result<T>,
    ) -> Result<T> {
        // Serialize updates against the actual remote write, not only against planning.
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = run(&Self::read(&tx, destination)?)?;
        tx.commit()?;
        Ok(result)
    }
}
