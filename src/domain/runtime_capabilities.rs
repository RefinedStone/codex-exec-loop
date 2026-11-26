//! Adapter declarations describe available operations, not permission to execute them.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "support", rename_all = "snake_case")]
pub enum RuntimeFeatureSupport {
    Supported { scope: String, limitations: String },
    Unsupported { reason: String },
    NotDeclared { reason: String },
}

impl RuntimeFeatureSupport {
    pub fn supported(scope: &str, limitations: &str) -> Self {
        Self::Supported {
            scope: scope.to_string(),
            limitations: limitations.to_string(),
        }
    }

    pub fn not_declared() -> Self {
        Self::NotDeclared {
            reason: "runtime adapter has not declared this capability".to_string(),
        }
    }
}

/// This is an implementation baseline. Account, policy, ownership and live server
/// admission still decide whether a particular request can execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRuntimeCapabilities {
    pub contract_version: u32,
    pub runtime_namespace: String,
    pub protocol_baseline: Option<String>,
    pub observed_runtime_version: Option<String>,
    pub start: RuntimeFeatureSupport,
    pub resume: RuntimeFeatureSupport,
    pub interrupt: RuntimeFeatureSupport,
    pub steer: RuntimeFeatureSupport,
    pub approval: RuntimeFeatureSupport,
    pub usage: RuntimeFeatureSupport,
}

impl Default for ConversationRuntimeCapabilities {
    fn default() -> Self {
        // Existing adapters may override optional trait methods without declaring
        // capabilities. Absence of a declaration must not become a support claim.
        Self {
            contract_version: 1,
            runtime_namespace: "undeclared".to_string(),
            protocol_baseline: None,
            observed_runtime_version: None,
            start: RuntimeFeatureSupport::not_declared(),
            resume: RuntimeFeatureSupport::not_declared(),
            interrupt: RuntimeFeatureSupport::not_declared(),
            steer: RuntimeFeatureSupport::not_declared(),
            approval: RuntimeFeatureSupport::not_declared(),
            usage: RuntimeFeatureSupport::not_declared(),
        }
    }
}
