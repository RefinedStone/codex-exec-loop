use std::sync::Arc;

use anyhow::Result;

use super::codex_runtime_capabilities;
use crate::adapter::outbound::app_server::CodexAppServerAdapter;
use crate::application::port::conversation_stream::ConversationStreamSender;
use crate::application::port::outbound::interactive_turn_runtime_port::{
    ConversationRuntimeCapabilities, InteractiveTurnRuntimePort, RuntimeFeatureSupport,
};
use crate::application::service::conversation_service::ConversationService;
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationRuntimeControlTruth, ConversationSnapshot,
    ConversationTurnOptions, ConversationTurnSteerRequest,
};
use crate::domain::turn_terminal::ConversationTurnTerminalReceipt;

struct NonCodexRuntime;

impl InteractiveTurnRuntimePort for NonCodexRuntime {
    fn runtime_control_truth(&self) -> ConversationRuntimeControlTruth {
        ConversationRuntimeControlTruth::default()
    }

    fn load_conversation_snapshot(&self, _thread_id: &str) -> Result<ConversationSnapshot> {
        anyhow::bail!("fixture has no stored snapshots")
    }

    fn request_stop_all_sessions(&self) -> Result<()> {
        Ok(())
    }

    fn run_new_thread_stream(
        &self,
        _cwd: &str,
        _prompt: &str,
        _image_paths: &[String],
        _options: ConversationTurnOptions,
        _event_sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        anyhow::bail!("fixture deliberately has no execution")
    }

    fn run_turn_stream(
        &self,
        _thread_id: &str,
        _prompt: &str,
        _image_paths: &[String],
        _options: ConversationTurnOptions,
        _event_sender: ConversationStreamSender,
    ) -> Result<ConversationTurnTerminalReceipt> {
        anyhow::bail!("fixture deliberately has no execution")
    }
}

#[test]
fn runtime_capabilities_codex_manifest_matches_schema_and_does_not_probe_a_server() {
    let adapter = CodexAppServerAdapter::new("capability-contract", "test");
    let manifest = adapter.runtime_capabilities();
    assert_eq!(manifest, codex_runtime_capabilities());
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../schema/codex_app_server_protocol.v2.schemas.json"
    ))
    .unwrap();
    assert_eq!(
        manifest.protocol_baseline,
        Some(format!(
            "{} / {}",
            schema["version"].as_str().unwrap(),
            schema["x-source-cli-version"].as_str().unwrap()
        ))
    );
    assert_eq!(manifest.observed_runtime_version, None);
    assert_eq!(manifest.runtime_namespace, "codex-app-server");
    for feature in [
        &manifest.start,
        &manifest.resume,
        &manifest.interrupt,
        &manifest.steer,
        &manifest.approval,
        &manifest.usage,
    ] {
        assert!(
            matches!(feature, RuntimeFeatureSupport::Supported { scope, limitations }
            if !scope.is_empty() && !limitations.is_empty())
        );
    }
    let encoded = serde_json::to_string(&manifest).unwrap();
    assert_eq!(
        serde_json::from_str::<ConversationRuntimeCapabilities>(&encoded).unwrap(),
        manifest
    );
}

#[test]
fn runtime_capabilities_undeclared_fake_does_not_inherit_codex_or_claim_live_support() {
    let service = ConversationService::new(Arc::new(NonCodexRuntime));
    let manifest = service.runtime_capabilities();
    assert_eq!(manifest.runtime_namespace, "undeclared");
    assert_eq!(manifest.protocol_baseline, None);
    assert_eq!(manifest.observed_runtime_version, None);
    for feature in [
        manifest.start,
        manifest.resume,
        manifest.interrupt,
        manifest.steer,
        manifest.approval,
        manifest.usage,
    ] {
        assert!(
            matches!(feature, RuntimeFeatureSupport::NotDeclared { reason } if !reason.is_empty())
        );
    }
    assert!(
        service
            .request_turn_interrupt("same-thread-id", "turn-1")
            .unwrap_err()
            .to_string()
            .contains("not supported")
    );
    assert!(
        service
            .resolve_approval_request("approval-1", ConversationApprovalDecision::Decline)
            .unwrap_err()
            .to_string()
            .contains("not supported")
    );
    assert!(
        service
            .steer_turn(ConversationTurnSteerRequest {
                thread_id: "same-thread-id".to_string(),
                expected_turn_id: "turn-1".to_string(),
                prompt: "fixture".to_string(),
            })
            .unwrap_err()
            .to_string()
            .contains("not supported")
    );
}

#[test]
fn runtime_capabilities_unsupported_and_undeclared_remain_distinct_on_the_wire() {
    let unsupported = RuntimeFeatureSupport::Unsupported {
        reason: "fixture engine does not report token usage".to_string(),
    };
    let value = serde_json::to_value(&unsupported).unwrap();
    assert_eq!(value["support"], "unsupported");
    assert_eq!(
        serde_json::from_value::<RuntimeFeatureSupport>(value).unwrap(),
        unsupported
    );
    assert_eq!(
        serde_json::to_value(RuntimeFeatureSupport::not_declared()).unwrap()["support"],
        "not_declared"
    );
}
