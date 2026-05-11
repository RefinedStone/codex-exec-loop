use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG_RELATIVE_PATH: &[&str] = &[".akra", "parallel-agent-profiles.json"];
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
        self.validated()
            .unwrap_or_default()
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
        serde_json::to_string_pretty(&self.validated().unwrap_or_default())
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

pub fn load_parallel_agent_profile_config(
    workspace_dir: &str,
) -> Result<ParallelAgentProfileConfig, String> {
    let path = config_path(workspace_dir);
    if !path.exists() {
        return Ok(ParallelAgentProfileConfig::default());
    }
    let body = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read parallel agent profiles: {error}"))?;
    parse_parallel_agent_profile_config_json(&body)
}

pub fn save_parallel_agent_profile_config(
    workspace_dir: &str,
    config: &ParallelAgentProfileConfig,
) -> Result<(), String> {
    let config = config.validated()?;
    let path = config_path(workspace_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create parallel agent profile dir: {error}"))?;
    }
    let body = serde_json::to_string_pretty(&config)
        .map_err(|error| format!("failed to serialize parallel agent profiles: {error}"))?;
    fs::write(&path, format!("{body}\n"))
        .map_err(|error| format!("failed to write parallel agent profiles: {error}"))
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

fn config_path(workspace_dir: &str) -> PathBuf {
    let mut path = Path::new(workspace_dir).to_path_buf();
    for segment in CONFIG_RELATIVE_PATH {
        path.push(segment);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::{
        ParallelAgentProfileConfig, load_parallel_agent_profile_config,
        parse_parallel_agent_profile_config_json, save_parallel_agent_profile_config,
    };
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("akra-agent-profile-test-{label}-{unique}"));
        fs::create_dir_all(&path).expect("temp workspace");
        path
    }

    #[test]
    fn missing_profile_config_uses_default_agents() {
        let temp = temp_workspace("missing");

        let config = load_parallel_agent_profile_config(temp.to_str().unwrap())
            .expect("missing config should load");

        assert!(config.profile_for_agent_id("agent-artificer").is_some());
        assert_eq!(config.enabled_profiles().len(), 3);
    }

    #[test]
    fn profile_config_round_trips_through_workspace_file() {
        let temp = temp_workspace("round-trip");
        let config = ParallelAgentProfileConfig::default();

        save_parallel_agent_profile_config(temp.to_str().unwrap(), &config).expect("save config");
        let loaded =
            load_parallel_agent_profile_config(temp.to_str().unwrap()).expect("load config");

        assert_eq!(loaded.enabled_profiles()[0].agent_id, "agent-artificer");
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
}
