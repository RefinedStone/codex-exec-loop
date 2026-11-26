use anyhow::Result;

use crate::domain::delegation::policy::Route;
use crate::domain::delegation::{ChatSession, Delegation, MyTurn};

pub(crate) struct SubmissionContext<'a> {
    pub account_digest: Option<&'a [u8]>,
    pub task_id: Option<&'a str>,
}

pub(crate) struct DelegationUsageProjection {
    pub delegations: Vec<Delegation>,
    pub unlinked: u64,
    pub truncated: bool,
}

pub(crate) struct Job {
    pub view: Delegation,
    pub instruction: String,
    pub stop_pending: bool,
    pub route: Route,
    pub account_digest: Option<Vec<u8>>,
}

pub(crate) trait DelegationStorePort: Send + Sync {
    #[cfg(test)]
    fn submit(&self, key: &str, instruction: &str, caller: &str) -> Result<Delegation> {
        self.submit_with_route(key, instruction, caller, Ok(Route::default()))
    }
    fn submit_with_route(
        &self,
        key: &str,
        instruction: &str,
        caller: &str,
        route: Result<Route>,
    ) -> Result<Delegation> {
        self.submit_session(key, instruction, caller, route, ChatSession::New, None)
    }
    fn submit_session(
        &self,
        key: &str,
        instruction: &str,
        caller: &str,
        route: Result<Route>,
        session: ChatSession,
        account_digest: Option<&[u8]>,
    ) -> Result<Delegation> {
        self.submit_task_session(
            key,
            instruction,
            caller,
            route,
            session,
            SubmissionContext {
                account_digest,
                task_id: None,
            },
        )
    }
    fn submit_task_session(
        &self,
        key: &str,
        instruction: &str,
        caller: &str,
        route: Result<Route>,
        session: ChatSession,
        context: SubmissionContext<'_>,
    ) -> Result<Delegation>;
    fn task_usage(&self, task_id: &str) -> Result<DelegationUsageProjection>;
    fn find(&self, id: &str) -> Result<Option<Delegation>>;
    fn result(&self, id: &str) -> Result<Option<String>>;
    fn list(&self) -> Result<Vec<Delegation>>;
    fn detail(&self, id: &str) -> Result<Option<Delegation>>;
    fn my_turn(&self) -> Result<Vec<MyTurn>>;
    fn active(&self) -> Result<Vec<Job>>;
    fn update(
        &self,
        old: &Delegation,
        next: &mut Delegation,
        result: Option<&str>,
        stop_pending: bool,
    ) -> Result<bool>;
    fn cancel(&self, id: &str) -> Result<Option<Delegation>>;
    fn take_stop(&self, id: &str) -> Result<bool>;
    fn circuit_until(&self, backend: &str) -> Result<Option<i64>>;
    fn issue_callback_capability(&self, id: &str) -> Result<Option<String>>;
    fn checkpoint(&self, id: &str, token: &str, message: &str) -> Result<Option<Delegation>>;
    fn report(&self, id: &str, token: &str, result: &str) -> Result<Option<Delegation>>;
    fn retry(&self, id: &str, key: &str) -> Result<Option<Delegation>>;
}
