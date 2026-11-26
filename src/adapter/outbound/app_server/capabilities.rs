use crate::domain::runtime_capabilities::{ConversationRuntimeCapabilities, RuntimeFeatureSupport};

pub(super) fn codex_runtime_capabilities() -> ConversationRuntimeCapabilities {
    ConversationRuntimeCapabilities {
        contract_version: 1,
        runtime_namespace: "codex-app-server".to_string(),
        protocol_baseline: Some("v2 / codex-cli 0.144.0".to_string()),
        // The checked-in schema version is not the installed or connected server.
        observed_runtime_version: None,
        start: RuntimeFeatureSupport::supported(
            "new thread in a validated workspace",
            "thread/start then turn/start; effective policy and server admission apply",
        ),
        resume: RuntimeFeatureSupport::supported(
            "existing thread in the same host and runtime namespace",
            "thread/read and thread/resume validate identity and applied workspace before turn/start",
        ),
        interrupt: RuntimeFeatureSupport::supported(
            "active thread/turn owned by this adapter process",
            "request acknowledgement is not terminal confirmation; completion may win the race",
        ),
        steer: RuntimeFeatureSupport::supported(
            "owned active main or worker turn with an expected turn id",
            "current broker binding and deadline apply; stale or terminal turns are rejected",
        ),
        approval: RuntimeFeatureSupport::supported(
            "pending commandExecution or permissions approval owned by this stream",
            "fileChange and legacy requests are declined; expired, cancelled or consumed requests cannot authorize later work",
        ),
        usage: RuntimeFeatureSupport::supported(
            "token usage notifications observed on a bound thread/turn",
            "delivery is optional and may repeat; missing usage is unobserved, not zero; totals are not monetary cost",
        ),
    }
}

#[cfg(test)]
mod tests;
