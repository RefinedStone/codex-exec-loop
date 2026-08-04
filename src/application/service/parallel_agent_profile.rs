use std::sync::Arc;

use crate::application::port::inbound::parallel_agent_profile_port::ParallelAgentProfilePort;
use crate::application::port::outbound::parallel_agent_profile_repository_port::ParallelAgentProfileRepositoryPort;
use crate::domain::parallel_agent_profile::{
    ParallelAgentProfileConfig, parse_parallel_agent_profile_config_json,
};

#[derive(Clone)]
pub struct ParallelAgentProfileService {
    repository: Arc<dyn ParallelAgentProfileRepositoryPort>,
}

impl ParallelAgentProfileService {
    pub fn new(repository: Arc<dyn ParallelAgentProfileRepositoryPort>) -> Self {
        Self { repository }
    }

    pub fn load_config(&self, workspace_dir: &str) -> Result<ParallelAgentProfileConfig, String> {
        let Some(body) = self
            .repository
            .load_profile_config_json(workspace_dir)
            .map_err(|error| format!("failed to read parallel agent profiles: {error:#}"))?
        else {
            return Ok(ParallelAgentProfileConfig::default());
        };
        parse_parallel_agent_profile_config_json(&body)
    }

    pub fn save_config(
        &self,
        workspace_dir: &str,
        config: &ParallelAgentProfileConfig,
    ) -> Result<(), String> {
        let config = config.validated()?;
        let body = serde_json::to_string_pretty(&config)
            .map_err(|error| format!("failed to serialize parallel agent profiles: {error}"))?;
        self.repository
            .save_profile_config_json(workspace_dir, &format!("{body}\n"))
            .map_err(|error| format!("failed to write parallel agent profiles: {error:#}"))
    }
}

impl ParallelAgentProfilePort for ParallelAgentProfileService {
    fn load_config(&self, workspace_dir: &str) -> Result<ParallelAgentProfileConfig, String> {
        ParallelAgentProfileService::load_config(self, workspace_dir)
    }

    fn save_config(
        &self,
        workspace_dir: &str,
        config: &ParallelAgentProfileConfig,
    ) -> Result<(), String> {
        ParallelAgentProfileService::save_config(self, workspace_dir, config)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ParallelAgentProfileConfig, ParallelAgentProfileService,
        parse_parallel_agent_profile_config_json,
    };
    use crate::application::port::outbound::parallel_agent_profile_repository_port::ParallelAgentProfileRepositoryPort;
    use anyhow::Result;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct MemoryParallelAgentProfileRepository {
        body: Mutex<Option<String>>,
    }

    impl ParallelAgentProfileRepositoryPort for MemoryParallelAgentProfileRepository {
        fn load_profile_config_json(&self, _workspace_dir: &str) -> Result<Option<String>> {
            Ok(self.body.lock().expect("memory profile lock").clone())
        }

        fn save_profile_config_json(&self, _workspace_dir: &str, body: &str) -> Result<()> {
            *self.body.lock().expect("memory profile lock") = Some(body.to_string());
            Ok(())
        }
    }

    #[test]
    fn missing_profile_config_uses_default_agents() {
        let service = ParallelAgentProfileService::new(Arc::new(
            MemoryParallelAgentProfileRepository::default(),
        ));

        let config = service
            .load_config("workspace")
            .expect("missing config should load");

        assert!(config.profile_for_agent_id("agent-artificer").is_some());
        assert_eq!(config.enabled_profiles().len(), 3);
    }

    #[test]
    fn legacy_profile_json_round_trips_through_repository_port() {
        let repository = Arc::new(MemoryParallelAgentProfileRepository {
            body: Mutex::new(Some(
                r#"{
                  "profiles": [
                    {
                      "agent_id": "legacy-agent",
                      "display_name": "Legacy",
                      "role": "Build",
                      "persona_prompt": "Keep the legacy schema readable.",
                      "avatar_class": "Artificer"
                    }
                  ]
                }"#
                .to_string(),
            )),
        });
        let service = ParallelAgentProfileService::new(repository.clone());

        let loaded = service.load_config("workspace").expect("legacy config");
        assert!(loaded.profiles[0].enabled);
        assert!(loaded.profiles[0].capabilities.is_empty());

        service
            .save_config("workspace", &loaded)
            .expect("save normalized config");
        let reloaded = service.load_config("workspace").expect("reload config");

        assert_eq!(reloaded, loaded);
        assert!(
            repository
                .body
                .lock()
                .expect("memory profile lock")
                .as_deref()
                .is_some_and(|body| body.ends_with('\n'))
        );
    }

    #[test]
    fn available_profile_skips_active_agent_ids() {
        let config = ParallelAgentProfileConfig::default();
        let used = BTreeSet::from(["agent-artificer".to_string()]);

        let profile = config
            .select_available_profile(&used)
            .expect("next profile");

        assert_eq!(profile.agent_id, "agent-scribe");
    }

    #[test]
    fn parser_rejects_duplicate_agent_ids() {
        let error = parse_parallel_agent_profile_config_json(
            r#"{
              "profiles": [
                {"agent_id":"agent-a","display_name":"A","role":"Build","persona_prompt":"","avatar_class":"Artificer"},
                {"agent_id":"agent-a","display_name":"B","role":"Review","persona_prompt":"","avatar_class":"Scribe"}
              ]
            }"#,
        )
        .expect_err("duplicate ids should fail");

        assert!(error.contains("duplicated"));
    }

    #[test]
    fn invalid_in_memory_profiles_fail_closed_in_selection_and_rendering() {
        let mut config = ParallelAgentProfileConfig::default();
        config.profiles[1].agent_id = config.profiles[0].agent_id.clone();

        assert!(config.enabled_profiles().is_empty());
        assert!(config.select_available_profile(&BTreeSet::new()).is_none());
        assert_eq!(config.to_pretty_json(), "{}");
    }
}
