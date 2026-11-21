use anyhow::Result;

use crate::domain::delegation::Delegation;

pub(crate) struct Job {
    pub view: Delegation,
    pub instruction: String,
    pub stop_pending: bool,
}

pub(crate) trait DelegationStorePort: Send + Sync {
    fn submit(&self, key: &str, instruction: &str, caller: &str) -> Result<Delegation>;
    fn find(&self, id: &str) -> Result<Option<Delegation>>;
    fn result(&self, id: &str) -> Result<Option<String>>;
    fn list(&self) -> Result<Vec<Delegation>>;
    fn active(&self) -> Result<Vec<Job>>;
    fn update(
        &self,
        old: &Delegation,
        next: &mut Delegation,
        result: Option<&str>,
        stop_pending: bool,
    ) -> Result<bool>;
    fn cancel(&self, id: &str) -> Result<Option<Delegation>>;
    fn stopped(&self, id: &str) -> Result<()>;
}
