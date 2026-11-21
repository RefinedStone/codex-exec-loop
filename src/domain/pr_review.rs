//! Repository review policy, findings, and durable review results.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    #[default]
    Off,
    Observe,
    On,
}

impl ReviewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Observe => "observe",
            Self::On => "on",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewResult {
    pub summary: String,
    pub findings: Vec<Finding>,
}

impl ReviewResult {
    pub(crate) fn parse_json(json: &str) -> Option<Self> {
        if json.len() > 48 * 1024 {
            return None;
        }
        let result: Self = serde_json::from_str(json).ok()?;
        if result.summary.trim().is_empty()
            || result.summary.len() > 4000
            || result.findings.len() > 30
            || result.findings.iter().any(|f| {
                f.path.trim().is_empty()
                    || f.path.len() > 512
                    || f.path.chars().any(char::is_control)
                    || f.line == 0
                    || f.body.trim().is_empty()
                    || f.body.len() > 1500
            })
        {
            return None;
        }
        Some(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub path: String,
    pub line: u32,
    pub severity: Severity,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReviewRecord {
    Cancelled,
    DiffTooLarge,
    SourceUnavailable,
    BackendFailed,
    ResultInvalid,
    Reviewed {
        backend: String,
        result: ReviewResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredPrReview {
    pub head: String,
    pub result: Option<ReviewRecord>,
}
