//! Main-only confirmation envelopes. Ordinary CommandRequest stays unchanged.
use super::CommandRequest;
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationBinding, IssueConfirmationDisplay, IssueConfirmationIntent,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmationRegisterRequest {
    pub registration_id: String,
    pub session_file: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmationRegisterResponse {
    pub binding: IssueConfirmationBinding,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmationPrepareRequest {
    pub request: CommandRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmationPrepareResponse {
    pub intent: IssueConfirmationIntent,
    pub display: IssueConfirmationDisplay,
}

/// Never implement Debug for envelopes that contain raw proof or pre-sharing note text.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HumanConfirmationSubmission {
    pub registration_id: String,
    pub proof_id: String,
    pub intent: IssueConfirmationIntent,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmedCommandRequest {
    pub request: CommandRequest,
    pub confirmation: HumanConfirmationSubmission,
}

pub fn is_confirmation_method(method: &str) -> bool {
    method.starts_with("akra/kernel/confirmation/") || method == super::METHOD_COMMAND_CONFIRMED
}

/// Invalid confirmation JSON must not fall through to app-server or raw recordings.
#[derive(Deserialize)]
struct ConfirmationMethodProbe {
    #[serde(default)]
    method: Option<String>,
}

pub fn is_confirmation_frame(text: &str) -> bool {
    match serde_json::from_str::<ConfirmationMethodProbe>(text) {
        Ok(envelope) => envelope
            .method
            .as_deref()
            .is_some_and(is_confirmation_method),
        // Batch, duplicate method, or malformed JSON must not leak a confirmation envelope.
        // Decode individual JSON string tokens: a later parse failure cannot erase a method
        // that already appeared, including escaped slashes and Unicode escapes.
        Err(_) => contains_encoded_confirmation_method(text),
    }
}

fn contains_encoded_confirmation_method(text: &str) -> bool {
    let mut start = None;
    let mut escaped = false;
    let mut previous_is_method = false;
    let mut previous_end = 0;
    for (index, byte) in text.bytes().enumerate() {
        let Some(open) = start else {
            if byte == b'"' {
                start = Some(index);
            }
            continue;
        };
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte != b'"' {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<String>(&text[open..=index]) {
            if previous_is_method
                && text[previous_end..open].trim() == ":"
                && is_confirmation_method(&value)
            {
                return true;
            }
            previous_is_method = value == "method";
        } else {
            previous_is_method = false;
        }
        previous_end = index + 1;
        start = None;
    }
    // Also recognize a complete encoded method value whose closing quote was truncated.
    if let Some(open) = start
        && previous_is_method
        && text[previous_end..open].trim() == ":"
        && let Ok(value) = serde_json::from_str::<String>(&format!("{}\"", &text[open..]))
    {
        return is_confirmation_method(&value);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::kernel_event::ReasonCode;
    use crate::domain::kernel_spec::KernelCommandKind;
    use serde_json::json;

    #[test]
    fn proof_is_only_an_explicit_gateway_envelope_and_not_a_normal_command_field() {
        let request = json!({"command":{"type":"set_issue_automation_mode","mode":"observe"},
            "idempotency_key":"mode-key","confirmation":{"proof_id":"private-proof"}});
        assert!(serde_json::from_value::<CommandRequest>(request).is_err());
        assert!(serde_json::from_value::<ConfirmationRegisterRequest>(
            json!({"registration_id":"a".repeat(32),"session_file":"/private/path","token":"raw-token"})
        ).is_err());
        assert!(
            serde_json::from_value::<ConfirmedCommandRequest>(
                json!({"request":{"command":{"type":"set_issue_automation_mode","mode":"observe"}},
                "confirmation":{"proof_id":"private-proof"},"by":"human"})
            )
            .is_err()
        );
    }

    #[test]
    fn confirmation_methods_describe_their_real_envelopes_and_human_rejections() {
        let spec = serde_json::to_value(super::super::describe()).unwrap();
        for (method, input, output) in [
            (
                super::super::METHOD_CONFIRMATION_REGISTER,
                "ConfirmationRegisterRequest",
                "ConfirmationRegisterResponse",
            ),
            (
                super::super::METHOD_CONFIRMATION_PREPARE,
                "ConfirmationPrepareRequest",
                "ConfirmationPrepareResponse",
            ),
            (
                super::super::METHOD_COMMAND_CONFIRMED,
                "ConfirmedCommandRequest",
                "CommandResponse",
            ),
        ] {
            let entry = spec["methods"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["name"] == method)
                .unwrap();
            assert_eq!(entry["params"]["$ref"], format!("#/$defs/{input}"));
            assert_eq!(entry["result"]["$ref"], format!("#/$defs/{output}"));
            assert_eq!(spec["$defs"][input]["additionalProperties"], false);
        }
        for command in [
            KernelCommandKind::AnswerIssueDecision,
            KernelCommandKind::SetIssueAutomationMode,
            KernelCommandKind::SetIssueAutonomy,
            KernelCommandKind::SetIssueBindingPaused,
            KernelCommandKind::WaiveCriterion,
        ] {
            assert!(
                command
                    .spec()
                    .rejections
                    .contains(&ReasonCode::HumanConfirmationRequired)
            );
        }
        assert_eq!(
            serde_json::to_value(super::super::reason_next(
                ReasonCode::HumanConfirmationRequired
            ))
            .unwrap(),
            json!([{"step":"ask_user"}])
        );
        assert!(
            spec["$defs"]["CommandRequest"]["properties"]
                .get("confirmation")
                .is_none()
        );
    }

    #[test]
    fn malformed_confirmation_text_is_private_while_normal_app_server_text_keeps_its_route() {
        assert!(is_confirmation_frame(
            r#"{"method":"akra/kernel/command_confirmed","params":{"proof_id":"private"#
        ));
        assert!(is_confirmation_frame(
            r#"{"method":"akra/kernel/confirmation/unknown","params":{"token":"private"}}"#
        ));
        assert!(!is_confirmation_frame(
            r#"{"method":"turn/start","params":{"input":"akra/kernel/command_confirmed"}}"#
        ));
    }
    #[test]
    fn unsupported_batches_cannot_bypass_the_private_confirmation_route() {
        assert!(is_confirmation_frame(&json!([
            {"jsonrpc":"2.0","id":1,"method":"akra/kernel/command_confirmed","params":{"proof_id":"private"}}
        ]).to_string()));
        assert!(is_confirmation_frame(
            &json!([
                [{"method":"akra/kernel/confirmation/register","params":{"session_file":"private"}}]
            ])
            .to_string()
        ));
        assert!(!is_confirmation_frame(
            &json!([
                {"method":"turn/start","params":{"input":"akra/kernel/command_confirmed"}}
            ])
            .to_string()
        ));
    }

    #[test]
    fn malformed_escaped_methods_and_duplicate_method_fields_remain_private() {
        for text in [
            r#"{"method":"akra\/kernel\/command_confirmed","params":{"proof_id":"private"#,
            r#"{"method":"akra\u002fkernel\u002fcommand_confirmed","params":{"proof_id":"private"#,
            r#"{"\u006dethod":"akra\u002fkernel\u002fcommand_confirmed","params":{"proof_id":"private"#,
            r#"{"method":"akra/kernel/command_confirmed","method":"turn/start","params":{"proof_id":"private"}}"#,
            r#"{"method":"akra\u002fkernel\u002fcommand_confirmed"#,
        ] {
            assert!(is_confirmation_frame(text));
        }
        assert!(!is_confirmation_frame(
            r#"{"method":"turn/start","params":{"input":{"method":"akra/kernel/command_confirmed"}}}"#
        ));
    }
}
