use std::sync::Arc;

pub use crate::domain::parallel_agent_profile::{
    ParallelAgentProfile, ParallelAgentProfileConfig, parse_parallel_agent_profile_config_json,
};

pub trait ParallelAgentProfilePort: Send + Sync {
    fn load_config(&self, workspace_dir: &str) -> Result<ParallelAgentProfileConfig, String>;

    fn save_config(
        &self,
        workspace_dir: &str,
        config: &ParallelAgentProfileConfig,
    ) -> Result<(), String>;
}

impl<T> ParallelAgentProfilePort for Arc<T>
where
    T: ParallelAgentProfilePort + ?Sized,
{
    fn load_config(&self, workspace_dir: &str) -> Result<ParallelAgentProfileConfig, String> {
        self.as_ref().load_config(workspace_dir)
    }

    fn save_config(
        &self,
        workspace_dir: &str,
        config: &ParallelAgentProfileConfig,
    ) -> Result<(), String> {
        self.as_ref().save_config(workspace_dir, config)
    }
}
