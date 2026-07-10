use std::sync::Arc;

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::application::port::outbound::parallel_agent_profile_repository_port::ParallelAgentProfileRepositoryPort;

const AVATAR_CLASSES: &[&str] = &[
    "Artificer",
    "Scribe",
    "Guardian",
    "Ranger",
    "Seer",
    "Runner",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelAgentProfile {
    pub agent_id: String,
    pub display_name: String,
    pub role: String,
    pub persona_prompt: String,
    pub avatar_class: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl ParallelAgentProfile {
    pub fn prompt_lines(&self) -> Vec<String> {
        self.persona_prompt
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    pub fn prompt_label(&self) -> String {
        if self.role.trim().is_empty() {
            self.display_name.clone()
        } else {
            format!("{} / {}", self.display_name, self.role)
        }
    }

    fn normalized(&self, index: usize) -> Result<Self, String> {
        let agent_id = self.agent_id.trim();
        if agent_id.is_empty() {
            return Err(format!("agent profile #{} is missing agent_id", index + 1));
        }
        if !is_stable_agent_id(agent_id) {
            return Err(format!(
                "agent profile `{agent_id}` must use only ASCII letters, digits, dash, or underscore"
            ));
        }
        let display_name = self.display_name.trim();
        let role = self.role.trim();
        let avatar_class = self.avatar_class.trim();
        Ok(Self {
            agent_id: agent_id.to_string(),
            display_name: if display_name.is_empty() {
                agent_id.to_string()
            } else {
                display_name.to_string()
            },
            role: if role.is_empty() {
                "작업자".to_string()
            } else {
                role.to_string()
            },
            persona_prompt: self.persona_prompt.trim().to_string(),
            avatar_class: if avatar_class.is_empty() {
                AVATAR_CLASSES[index % AVATAR_CLASSES.len()].to_string()
            } else {
                avatar_class.to_string()
            },
            capabilities: self
                .capabilities
                .iter()
                .map(|capability| capability.trim())
                .filter(|capability| !capability.is_empty())
                .map(str::to_string)
                .collect(),
            enabled: self.enabled,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelAgentProfileConfig {
    #[serde(default = "default_agent_profiles")]
    pub profiles: Vec<ParallelAgentProfile>,
}

impl Default for ParallelAgentProfileConfig {
    fn default() -> Self {
        Self {
            profiles: default_agent_profiles(),
        }
    }
}

impl ParallelAgentProfileConfig {
    pub fn validated(&self) -> Result<Self, String> {
        let mut seen = BTreeSet::new();
        let mut profiles = Vec::new();
        for (index, profile) in self.profiles.iter().enumerate() {
            let profile = profile.normalized(index)?;
            if !seen.insert(profile.agent_id.clone()) {
                return Err(format!(
                    "agent profile `{}` is duplicated",
                    profile.agent_id
                ));
            }
            profiles.push(profile);
        }
        if profiles.is_empty() {
            profiles = default_agent_profiles();
        }
        Ok(Self { profiles })
    }

    pub fn enabled_profiles(&self) -> Vec<ParallelAgentProfile> {
        let Ok(config) = self.validated() else {
            return Vec::new();
        };
        config
            .profiles
            .into_iter()
            .filter(|profile| profile.enabled)
            .collect()
    }

    pub fn select_available_profile(
        &self,
        used_agent_ids: &BTreeSet<String>,
    ) -> Option<ParallelAgentProfile> {
        self.enabled_profiles()
            .into_iter()
            .find(|profile| !used_agent_ids.contains(&profile.agent_id))
    }

    pub fn profile_for_agent_id(&self, agent_id: &str) -> Option<ParallelAgentProfile> {
        let agent_id = agent_id.trim();
        self.enabled_profiles()
            .into_iter()
            .find(|profile| profile.agent_id == agent_id)
    }

    pub fn to_pretty_json(&self) -> String {
        self.validated()
            .and_then(|config| {
                serde_json::to_string_pretty(&config).map_err(|error| {
                    format!("failed to serialize parallel agent profiles: {error}")
                })
            })
            .unwrap_or_else(|_| "{}".to_string())
    }
}

pub fn parse_parallel_agent_profile_config_json(
    body: &str,
) -> Result<ParallelAgentProfileConfig, String> {
    let config = serde_json::from_str::<ParallelAgentProfileConfig>(body)
        .map_err(|error| format!("failed to parse parallel agent profiles: {error}"))?;
    config.validated()
}

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

fn default_agent_profiles() -> Vec<ParallelAgentProfile> {
    vec![
        ParallelAgentProfile {
            agent_id: "agent-artificer".to_string(),
            display_name: "아티피서".to_string(),
            role: "구현 담당".to_string(),
            persona_prompt: "You are an implementation-focused Akra agent.\nPrefer the smallest coherent code change that completes the assigned task.\nKeep tests and user-visible behavior aligned with the existing codebase.".to_string(),
            avatar_class: "Artificer".to_string(),
            capabilities: vec!["implementation".to_string(), "tests".to_string()],
            enabled: true,
        },
        ParallelAgentProfile {
            agent_id: "agent-scribe".to_string(),
            display_name: "서기관".to_string(),
            role: "정리 담당".to_string(),
            persona_prompt: "You are a documentation and cleanup Akra agent.\nClarify naming, copy, and small structural issues without expanding scope.\nKeep the final report concise and grounded in changed files.".to_string(),
            avatar_class: "Scribe".to_string(),
            capabilities: vec!["documentation".to_string(), "cleanup".to_string()],
            enabled: true,
        },
        ParallelAgentProfile {
            agent_id: "agent-guardian".to_string(),
            display_name: "가디언".to_string(),
            role: "검증 담당".to_string(),
            persona_prompt: "You are a verification-focused Akra agent.\nLook for regressions, missing edge cases, and test gaps inside the assigned task scope.\nPrefer actionable fixes over broad review commentary.".to_string(),
            avatar_class: "Guardian".to_string(),
            capabilities: vec!["review".to_string(), "verification".to_string()],
            enabled: true,
        },
    ]
}

fn default_enabled() -> bool {
    true
}

fn is_stable_agent_id(value: &str) -> bool {
    value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
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
