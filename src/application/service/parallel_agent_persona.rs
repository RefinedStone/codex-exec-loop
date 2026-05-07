use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG_RELATIVE_PATH: &[&str] = &[".akra", "parallel-agent-persona.json"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ParallelAgentPersona {
    #[default]
    None,
    Sample,
}

impl ParallelAgentPersona {
    pub fn from_form_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "" => Some(Self::None),
            "sample" => Some(Self::Sample),
            _ => None,
        }
    }

    pub fn form_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sample => "sample",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Sample => "Sample careful implementer",
        }
    }

    pub fn prompt_lines(self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::Sample => vec![
                "You are a careful implementation agent.".to_string(),
                "Prefer the smallest coherent change that completes the queued task.".to_string(),
                "Call out uncertainty briefly instead of expanding the task scope.".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ParallelAgentPersonaConfig {
    #[serde(default)]
    pub persona: ParallelAgentPersona,
}

impl ParallelAgentPersonaConfig {
    pub fn new(persona: ParallelAgentPersona) -> Self {
        Self { persona }
    }

    pub fn options() -> Vec<ParallelAgentPersonaOption> {
        vec![
            ParallelAgentPersonaOption::new(ParallelAgentPersona::None),
            ParallelAgentPersonaOption::new(ParallelAgentPersona::Sample),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelAgentPersonaOption {
    pub value: &'static str,
    pub label: &'static str,
    pub prompt_preview: String,
}

impl ParallelAgentPersonaOption {
    fn new(persona: ParallelAgentPersona) -> Self {
        Self {
            value: persona.form_value(),
            label: persona.label(),
            prompt_preview: persona.prompt_lines().join(" "),
        }
    }
}

pub fn load_parallel_agent_persona_config(
    workspace_dir: &str,
) -> Result<ParallelAgentPersonaConfig, String> {
    let path = config_path(workspace_dir);
    if !path.exists() {
        return Ok(ParallelAgentPersonaConfig::default());
    }
    let body = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read parallel agent persona config: {error}"))?;
    serde_json::from_str(&body)
        .map_err(|error| format!("failed to parse parallel agent persona config: {error}"))
}

pub fn save_parallel_agent_persona_config(
    workspace_dir: &str,
    config: &ParallelAgentPersonaConfig,
) -> Result<(), String> {
    let path = config_path(workspace_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!("failed to create parallel agent persona config dir: {error}")
        })?;
    }
    let body = serde_json::to_string_pretty(config)
        .map_err(|error| format!("failed to serialize parallel agent persona config: {error}"))?;
    fs::write(&path, format!("{body}\n"))
        .map_err(|error| format!("failed to write parallel agent persona config: {error}"))
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
        ParallelAgentPersona, ParallelAgentPersonaConfig, load_parallel_agent_persona_config,
        save_parallel_agent_persona_config,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("akra-persona-test-{label}-{unique}"));
        fs::create_dir_all(&path).expect("temp workspace");
        path
    }

    #[test]
    fn missing_persona_config_defaults_to_none() {
        let temp = temp_workspace("missing");

        let config = load_parallel_agent_persona_config(temp.to_str().unwrap())
            .expect("missing config should load");

        assert_eq!(config.persona, ParallelAgentPersona::None);
        fs::remove_dir_all(temp).ok();
    }

    #[test]
    fn persona_config_round_trips_sample_choice() {
        let temp = temp_workspace("round-trip");
        let workspace = temp.to_str().unwrap();

        save_parallel_agent_persona_config(
            workspace,
            &ParallelAgentPersonaConfig::new(ParallelAgentPersona::Sample),
        )
        .expect("config should save");

        let config = load_parallel_agent_persona_config(workspace).expect("config should load");
        assert_eq!(config.persona, ParallelAgentPersona::Sample);
        assert!(fs::metadata(temp.join(".akra/parallel-agent-persona.json")).is_ok());
        fs::remove_dir_all(temp).ok();
    }
}
