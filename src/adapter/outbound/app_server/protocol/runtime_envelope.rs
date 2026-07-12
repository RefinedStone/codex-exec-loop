use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde_json::{Map, Value};

use super::super::{MAX_STREAM_IDENTIFIER_BYTES, MAX_STREAM_METADATA_BYTES, bounded_stream_text};
use super::{
    ApprovalPolicyValue, ApprovalsReviewerValue, ReasoningEffortValue, SandboxModeValue,
    ThreadRecord, ThreadStatus,
};
use crate::domain::conversation_runtime_envelope::{
    ConversationRuntimeApprovalPolicy, ConversationRuntimeApprovalsReviewer,
    ConversationRuntimeConfigurationObservation, ConversationRuntimeConfigurationRequest,
    ConversationRuntimeEnvelope, ConversationRuntimeLaunchEnvironment,
    ConversationRuntimeMalformedValue, ConversationRuntimeModelReroute,
    ConversationRuntimeModelRerouteReason, ConversationRuntimeObservedValue,
    ConversationRuntimePermissionProfile, ConversationRuntimeRequestedValue,
    ConversationRuntimeSandboxPolicy, ConversationRuntimeThreadStatus,
};

const MAX_SANDBOX_WRITABLE_ROOTS: usize = 32;

pub(in crate::adapter::outbound::app_server) fn to_runtime_envelope(
    request: ConversationRuntimeConfigurationRequest,
    response_fields: &BTreeMap<String, Value>,
    thread: &ThreadRecord,
    launch_environment: ConversationRuntimeLaunchEnvironment,
) -> Result<ConversationRuntimeEnvelope> {
    Ok(ConversationRuntimeEnvelope::prepared(
        request,
        configuration_observation_from_response(response_fields, thread)?,
        launch_environment,
        thread_status_observation(&thread.status),
    ))
}

pub(in crate::adapter::outbound::app_server) fn runtime_configuration_request(
    model: Option<&str>,
    effort: Option<ReasoningEffortValue>,
    cwd: Option<&str>,
    approval_policy: Option<ApprovalPolicyValue>,
    approvals_reviewer: Option<ApprovalsReviewerValue>,
    sandbox: Option<SandboxModeValue>,
) -> ConversationRuntimeConfigurationRequest {
    ConversationRuntimeConfigurationRequest {
        model: requested_string(model, MAX_STREAM_IDENTIFIER_BYTES),
        reasoning_effort: requested_string(
            effort.map(ReasoningEffortValue::label),
            MAX_STREAM_IDENTIFIER_BYTES,
        ),
        cwd: requested_string(cwd, MAX_STREAM_METADATA_BYTES),
        approval_policy: requested_value(approval_policy.map(ApprovalPolicyValue::runtime_policy)),
        approvals_reviewer: requested_value(
            approvals_reviewer.map(ApprovalsReviewerValue::runtime_reviewer),
        ),
        sandbox: requested_value(sandbox.map(SandboxModeValue::runtime_policy)),
        ..ConversationRuntimeConfigurationRequest::default()
    }
}

pub(in crate::adapter::outbound::app_server) fn settings_observation(
    params: &Value,
) -> Result<ConversationRuntimeConfigurationObservation> {
    let Some(settings) = params.get("threadSettings").and_then(Value::as_object) else {
        bail!("thread/settings/updated omitted a valid threadSettings object");
    };

    if !settings
        .get("collaborationMode")
        .is_some_and(Value::is_object)
    {
        bail!("thread/settings/updated omitted required collaborationMode object");
    }
    let observation = ConversationRuntimeConfigurationObservation {
        model: observed_string(settings.get("model"), MAX_STREAM_IDENTIFIER_BYTES),
        model_provider: observed_string(settings.get("modelProvider"), MAX_STREAM_IDENTIFIER_BYTES),
        reasoning_effort: observed_string(settings.get("effort"), MAX_STREAM_IDENTIFIER_BYTES),
        service_tier: observed_string(settings.get("serviceTier"), MAX_STREAM_IDENTIFIER_BYTES),
        cwd: observed_string(settings.get("cwd"), MAX_STREAM_METADATA_BYTES),
        approval_policy: observed_approval_policy(settings.get("approvalPolicy")),
        approvals_reviewer: observed_approvals_reviewer(settings.get("approvalsReviewer")),
        sandbox: observed_sandbox_policy(settings.get("sandboxPolicy")),
        permission_profile: observed_permission_profile(settings.get("activePermissionProfile")),
        source: ConversationRuntimeObservedValue::Missing,
    };
    validate_required_configuration_fields(&observation, "thread/settings/updated")?;
    Ok(observation)
}

pub(in crate::adapter::outbound::app_server) fn model_reroute(
    params: &Value,
) -> Option<ConversationRuntimeModelReroute> {
    let from_model =
        nonempty_bounded_string(params.get("fromModel")?, MAX_STREAM_IDENTIFIER_BYTES)?;
    let to_model = nonempty_bounded_string(params.get("toModel")?, MAX_STREAM_IDENTIFIER_BYTES)?;
    let reason = nonempty_bounded_string(params.get("reason")?, MAX_STREAM_IDENTIFIER_BYTES)?;
    let reason = match reason.as_str() {
        "highRiskCyberActivity" => ConversationRuntimeModelRerouteReason::HighRiskCyberActivity,
        _ => ConversationRuntimeModelRerouteReason::Unknown(reason),
    };

    Some(ConversationRuntimeModelReroute {
        from_model,
        to_model,
        reason,
    })
}

pub(in crate::adapter::outbound::app_server) fn status_observation(
    value: Option<&Value>,
) -> ConversationRuntimeObservedValue<ConversationRuntimeThreadStatus> {
    let Some(value) = value else {
        return ConversationRuntimeObservedValue::Missing;
    };
    if value.is_null() {
        return ConversationRuntimeObservedValue::Null;
    }
    let Some(status) = value.as_object() else {
        return ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::ExpectedObject,
        );
    };
    let Some(status_type) = status.get("type").and_then(Value::as_str) else {
        return ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::MissingDiscriminator,
        );
    };
    let status_type = bounded_text(status_type, MAX_STREAM_IDENTIFIER_BYTES);
    let status = match status_type.as_str() {
        "notLoaded" => ConversationRuntimeThreadStatus::NotLoaded,
        "idle" => ConversationRuntimeThreadStatus::Idle,
        "systemError" => ConversationRuntimeThreadStatus::SystemError,
        "active" => {
            let Some(flags) = status.get("activeFlags").and_then(Value::as_array) else {
                return ConversationRuntimeObservedValue::Malformed(
                    ConversationRuntimeMalformedValue::InvalidObject,
                );
            };
            match active_thread_status(flags) {
                Ok(status) => status,
                Err(issue) => return ConversationRuntimeObservedValue::Malformed(issue),
            }
        }
        _ => ConversationRuntimeThreadStatus::Unknown(status_type),
    };
    ConversationRuntimeObservedValue::Observed(status)
}

fn configuration_observation_from_response(
    response_fields: &BTreeMap<String, Value>,
    thread: &ThreadRecord,
) -> Result<ConversationRuntimeConfigurationObservation> {
    let observation = ConversationRuntimeConfigurationObservation {
        model: observed_string(response_fields.get("model"), MAX_STREAM_IDENTIFIER_BYTES),
        model_provider: observed_string(
            response_fields.get("modelProvider"),
            MAX_STREAM_IDENTIFIER_BYTES,
        ),
        reasoning_effort: observed_string(
            response_fields.get("reasoningEffort"),
            MAX_STREAM_IDENTIFIER_BYTES,
        ),
        service_tier: observed_string(
            response_fields.get("serviceTier"),
            MAX_STREAM_IDENTIFIER_BYTES,
        ),
        cwd: observed_string(response_fields.get("cwd"), MAX_STREAM_METADATA_BYTES),
        approval_policy: observed_approval_policy(response_fields.get("approvalPolicy")),
        approvals_reviewer: observed_approvals_reviewer(response_fields.get("approvalsReviewer")),
        sandbox: observed_sandbox_policy(response_fields.get("sandbox")),
        // Named profile provenance on start/resume is experimental. Akra
        // initializes stable app-server capabilities only, so even a schema-only
        // field must not be promoted as a stable applied fact.
        permission_profile: ConversationRuntimeObservedValue::UnavailableOnStableResponse,
        source: ConversationRuntimeObservedValue::Observed(thread.source.runtime_source()),
    };
    validate_required_configuration_fields(&observation, "thread start/resume response")?;
    Ok(observation)
}

fn observed_string(
    value: Option<&Value>,
    max_bytes: usize,
) -> ConversationRuntimeObservedValue<String> {
    let Some(value) = value else {
        return ConversationRuntimeObservedValue::Missing;
    };
    if value.is_null() {
        return ConversationRuntimeObservedValue::Null;
    }
    let Some(value) = value.as_str().filter(|value| !value.is_empty()) else {
        return ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::ExpectedString,
        );
    };
    ConversationRuntimeObservedValue::Observed(bounded_text(value, max_bytes))
}

fn observed_approval_policy(
    value: Option<&Value>,
) -> ConversationRuntimeObservedValue<ConversationRuntimeApprovalPolicy> {
    let Some(value) = value else {
        return ConversationRuntimeObservedValue::Missing;
    };
    if value.is_null() {
        return ConversationRuntimeObservedValue::Null;
    }
    if let Some(label) = value.as_str() {
        let label = bounded_text(label, MAX_STREAM_IDENTIFIER_BYTES);
        let policy = match label.as_str() {
            "untrusted" => ConversationRuntimeApprovalPolicy::Untrusted,
            "on-request" => ConversationRuntimeApprovalPolicy::OnRequest,
            "never" => ConversationRuntimeApprovalPolicy::Never,
            _ => ConversationRuntimeApprovalPolicy::Unknown(label),
        };
        return ConversationRuntimeObservedValue::Observed(policy);
    }
    let Some(granular) = value.get("granular").and_then(Value::as_object) else {
        return ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::InvalidObject,
        );
    };
    let Some(sandbox_approval) = required_bool(granular, "sandbox_approval") else {
        return malformed_boolean();
    };
    let Some(rules) = required_bool(granular, "rules") else {
        return malformed_boolean();
    };
    let Some(mcp_elicitations) = required_bool(granular, "mcp_elicitations") else {
        return malformed_boolean();
    };
    let Ok(request_permissions) = optional_bool(granular, "request_permissions") else {
        return malformed_boolean();
    };
    let Ok(skill_approval) = optional_bool(granular, "skill_approval") else {
        return malformed_boolean();
    };
    let defaulted = request_permissions.is_none() || skill_approval.is_none();
    let policy = ConversationRuntimeApprovalPolicy::Granular {
        sandbox_approval,
        rules,
        mcp_elicitations,
        request_permissions: Some(request_permissions.unwrap_or(false)),
        skill_approval: Some(skill_approval.unwrap_or(false)),
    };
    if defaulted {
        ConversationRuntimeObservedValue::Defaulted(policy)
    } else {
        ConversationRuntimeObservedValue::Observed(policy)
    }
}

fn observed_approvals_reviewer(
    value: Option<&Value>,
) -> ConversationRuntimeObservedValue<ConversationRuntimeApprovalsReviewer> {
    observed_string(value, MAX_STREAM_IDENTIFIER_BYTES).map(|label| match label.as_str() {
        "user" => ConversationRuntimeApprovalsReviewer::User,
        "auto_review" => ConversationRuntimeApprovalsReviewer::AutoReview,
        "guardian_subagent" => ConversationRuntimeApprovalsReviewer::GuardianSubagent,
        _ => ConversationRuntimeApprovalsReviewer::Unknown(label),
    })
}

fn observed_sandbox_policy(
    value: Option<&Value>,
) -> ConversationRuntimeObservedValue<ConversationRuntimeSandboxPolicy> {
    let Some(value) = value else {
        return ConversationRuntimeObservedValue::Missing;
    };
    if value.is_null() {
        return ConversationRuntimeObservedValue::Null;
    }
    let Some(policy) = value.as_object() else {
        return ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::ExpectedObject,
        );
    };
    let Some(policy_type) = policy.get("type").and_then(Value::as_str) else {
        return ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::MissingDiscriminator,
        );
    };
    let policy_type = bounded_text(policy_type, MAX_STREAM_IDENTIFIER_BYTES);
    let policy = match policy_type.as_str() {
        "readOnly" => {
            let Ok(network_access) = optional_bool(policy, "networkAccess") else {
                return malformed_boolean();
            };
            let defaulted = network_access.is_none();
            let policy = ConversationRuntimeSandboxPolicy::ReadOnly {
                network_access: Some(network_access.unwrap_or(false)),
            };
            return if defaulted {
                ConversationRuntimeObservedValue::Defaulted(policy)
            } else {
                ConversationRuntimeObservedValue::Observed(policy)
            };
        }
        "workspaceWrite" => {
            let Ok(network_access) = optional_bool(policy, "networkAccess") else {
                return malformed_boolean();
            };
            let Ok(exclude_tmpdir_env_var) = optional_bool(policy, "excludeTmpdirEnvVar") else {
                return malformed_boolean();
            };
            let Ok(exclude_slash_tmp) = optional_bool(policy, "excludeSlashTmp") else {
                return malformed_boolean();
            };
            let (writable_roots, writable_roots_defaulted, writable_roots_truncated) =
                match optional_bounded_string_array(
                    policy,
                    "writableRoots",
                    MAX_SANDBOX_WRITABLE_ROOTS,
                    MAX_STREAM_IDENTIFIER_BYTES,
                ) {
                    Ok(Some((roots, truncated))) => (roots, false, truncated),
                    Ok(None) => (Vec::new(), true, false),
                    Err(issue) => return ConversationRuntimeObservedValue::Malformed(issue),
                };
            let defaulted = network_access.is_none()
                || writable_roots_defaulted
                || exclude_tmpdir_env_var.is_none()
                || exclude_slash_tmp.is_none();
            let policy = ConversationRuntimeSandboxPolicy::WorkspaceWrite {
                network_access: Some(network_access.unwrap_or(false)),
                writable_roots: Some(writable_roots),
                writable_roots_truncated,
                exclude_tmpdir_env_var: Some(exclude_tmpdir_env_var.unwrap_or(false)),
                exclude_slash_tmp: Some(exclude_slash_tmp.unwrap_or(false)),
            };
            return if defaulted {
                ConversationRuntimeObservedValue::Defaulted(policy)
            } else {
                ConversationRuntimeObservedValue::Observed(policy)
            };
        }
        "dangerFullAccess" => ConversationRuntimeSandboxPolicy::DangerFullAccess,
        "externalSandbox" => {
            let (network_access, defaulted) = match policy.get("networkAccess") {
                None => (Some("restricted".to_string()), true),
                Some(Value::Null) => {
                    return ConversationRuntimeObservedValue::Malformed(
                        ConversationRuntimeMalformedValue::ExpectedString,
                    );
                }
                Some(value) => match value.as_str() {
                    Some(value) => (
                        Some(bounded_text(value, MAX_STREAM_IDENTIFIER_BYTES)),
                        false,
                    ),
                    None => {
                        return ConversationRuntimeObservedValue::Malformed(
                            ConversationRuntimeMalformedValue::ExpectedString,
                        );
                    }
                },
            };
            let policy = ConversationRuntimeSandboxPolicy::ExternalSandbox { network_access };
            return if defaulted {
                ConversationRuntimeObservedValue::Defaulted(policy)
            } else {
                ConversationRuntimeObservedValue::Observed(policy)
            };
        }
        _ => ConversationRuntimeSandboxPolicy::Unknown { policy_type },
    };
    ConversationRuntimeObservedValue::Observed(policy)
}

fn observed_permission_profile(
    value: Option<&Value>,
) -> ConversationRuntimeObservedValue<ConversationRuntimePermissionProfile> {
    let Some(value) = value else {
        return ConversationRuntimeObservedValue::Missing;
    };
    if value.is_null() {
        return ConversationRuntimeObservedValue::Null;
    }
    let Some(profile) = value.as_object() else {
        return ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::ExpectedObject,
        );
    };
    let Some(id) = profile
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return ConversationRuntimeObservedValue::Malformed(
            ConversationRuntimeMalformedValue::InvalidObject,
        );
    };
    let extends = match profile.get("extends") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_str() {
            Some(value) => Some(bounded_text(value, MAX_STREAM_IDENTIFIER_BYTES)),
            None => {
                return ConversationRuntimeObservedValue::Malformed(
                    ConversationRuntimeMalformedValue::ExpectedString,
                );
            }
        },
    };
    ConversationRuntimeObservedValue::Observed(ConversationRuntimePermissionProfile {
        id: bounded_text(id, MAX_STREAM_IDENTIFIER_BYTES),
        extends,
    })
}

fn thread_status_observation(
    status: &ThreadStatus,
) -> ConversationRuntimeObservedValue<ConversationRuntimeThreadStatus> {
    let mut value = Map::new();
    value.insert(
        "type".to_string(),
        Value::String(status.status_type.clone()),
    );
    if !status.active_flags.is_null() {
        value.insert("activeFlags".to_string(), status.active_flags.clone());
    }
    status_observation(Some(&Value::Object(value)))
}

fn active_thread_status(
    flags: &[Value],
) -> std::result::Result<ConversationRuntimeThreadStatus, ConversationRuntimeMalformedValue> {
    let mut waiting_on_approval = false;
    let mut waiting_on_user_input = false;
    let mut unknown_flags = Vec::new();
    let mut unknown_flags_truncated = false;
    for flag in flags {
        let Some(flag) = flag.as_str().filter(|flag| !flag.is_empty()) else {
            return Err(ConversationRuntimeMalformedValue::ExpectedString);
        };
        match flag {
            "waitingOnApproval" => waiting_on_approval = true,
            "waitingOnUserInput" => waiting_on_user_input = true,
            _ if unknown_flags.len() < 32 => {
                unknown_flags.push(bounded_text(flag, MAX_STREAM_IDENTIFIER_BYTES));
            }
            _ => unknown_flags_truncated = true,
        }
    }
    Ok(ConversationRuntimeThreadStatus::Active {
        waiting_on_approval,
        waiting_on_user_input,
        unknown_flags,
        unknown_flags_truncated,
    })
}

fn nonempty_bounded_string(value: &Value, max_bytes: usize) -> Option<String> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|value| bounded_text(value, max_bytes))
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    bounded_stream_text(value.to_string(), max_bytes)
}

fn required_bool(object: &Map<String, Value>, key: &str) -> Option<bool> {
    object.get(key).and_then(Value::as_bool)
}

fn optional_bool(object: &Map<String, Value>, key: &str) -> Result<Option<bool>, ()> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::Null) => Err(()),
        Some(value) => value.as_bool().map(Some).ok_or(()),
    }
}

fn optional_bounded_string_array(
    object: &Map<String, Value>,
    key: &str,
    max_items: usize,
    max_item_bytes: usize,
) -> std::result::Result<Option<(Vec<String>, bool)>, ConversationRuntimeMalformedValue> {
    let Some(value) = object.get(key) else {
        return Ok(None);
    };
    let Some(values) = value.as_array() else {
        return Err(ConversationRuntimeMalformedValue::ExpectedArray);
    };
    let mut bounded = Vec::with_capacity(values.len().min(max_items));
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value.as_str().filter(|value| !value.is_empty()) else {
            return Err(ConversationRuntimeMalformedValue::ExpectedString);
        };
        if index < max_items {
            bounded.push(bounded_text(value, max_item_bytes));
        }
    }
    Ok(Some((bounded, values.len() > max_items)))
}

fn requested_string(
    value: Option<&str>,
    max_bytes: usize,
) -> ConversationRuntimeRequestedValue<String> {
    value.map_or(ConversationRuntimeRequestedValue::Omitted, |value| {
        ConversationRuntimeRequestedValue::Value(bounded_text(value, max_bytes))
    })
}

fn requested_value<T>(value: Option<T>) -> ConversationRuntimeRequestedValue<T> {
    value.map_or(
        ConversationRuntimeRequestedValue::Omitted,
        ConversationRuntimeRequestedValue::Value,
    )
}

fn validate_required_configuration_fields(
    observation: &ConversationRuntimeConfigurationObservation,
    context: &str,
) -> Result<()> {
    require_observed(&observation.model, context, "model")?;
    require_observed(&observation.model_provider, context, "modelProvider")?;
    require_observed(&observation.cwd, context, "cwd")?;
    require_observed(&observation.approval_policy, context, "approvalPolicy")?;
    require_observed(
        &observation.approvals_reviewer,
        context,
        "approvalsReviewer",
    )?;
    require_observed(&observation.sandbox, context, "sandbox")?;
    Ok(())
}

fn require_observed<T>(
    value: &ConversationRuntimeObservedValue<T>,
    context: &str,
    field: &str,
) -> Result<()> {
    match value {
        ConversationRuntimeObservedValue::Observed(_)
        | ConversationRuntimeObservedValue::Defaulted(_) => Ok(()),
        ConversationRuntimeObservedValue::Null => {
            bail!("{context} required field `{field}` was null")
        }
        ConversationRuntimeObservedValue::Missing => {
            bail!("{context} omitted required field `{field}`")
        }
        ConversationRuntimeObservedValue::Malformed(_) => {
            bail!("{context} required field `{field}` was malformed")
        }
        ConversationRuntimeObservedValue::UnavailableOnStableResponse => {
            bail!("{context} required field `{field}` was unavailable")
        }
        ConversationRuntimeObservedValue::UnavailableAfterObservationGap => {
            bail!("{context} required field `{field}` had an observation gap")
        }
    }
}

fn malformed_boolean<T>() -> ConversationRuntimeObservedValue<T> {
    ConversationRuntimeObservedValue::Malformed(ConversationRuntimeMalformedValue::ExpectedBoolean)
}

trait MapObservedValue<T> {
    fn map<U>(self, mapper: impl FnOnce(T) -> U) -> ConversationRuntimeObservedValue<U>;
}

impl<T> MapObservedValue<T> for ConversationRuntimeObservedValue<T> {
    fn map<U>(self, mapper: impl FnOnce(T) -> U) -> ConversationRuntimeObservedValue<U> {
        match self {
            ConversationRuntimeObservedValue::Observed(value) => {
                ConversationRuntimeObservedValue::Observed(mapper(value))
            }
            ConversationRuntimeObservedValue::Defaulted(value) => {
                ConversationRuntimeObservedValue::Defaulted(mapper(value))
            }
            ConversationRuntimeObservedValue::Null => ConversationRuntimeObservedValue::Null,
            ConversationRuntimeObservedValue::Missing => ConversationRuntimeObservedValue::Missing,
            ConversationRuntimeObservedValue::Malformed(issue) => {
                ConversationRuntimeObservedValue::Malformed(issue)
            }
            ConversationRuntimeObservedValue::UnavailableOnStableResponse => {
                ConversationRuntimeObservedValue::UnavailableOnStableResponse
            }
            ConversationRuntimeObservedValue::UnavailableAfterObservationGap => {
                ConversationRuntimeObservedValue::UnavailableAfterObservationGap
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SECRET_CANARY: &str = "AKRA_TEST_SECRET_CANARY_RUNTIME_ENVELOPE";

    fn fixtures() -> Value {
        serde_json::from_str(include_str!("fixtures/runtime_envelopes.json"))
            .expect("runtime envelope fixture should parse")
    }

    fn fixture_fields(fixtures: &Value, key: &str) -> BTreeMap<String, Value> {
        serde_json::from_value(fixtures[key].clone())
            .unwrap_or_else(|error| panic!("runtime envelope fixture `{key}` should map: {error}"))
    }

    fn thread_record() -> ThreadRecord {
        serde_json::from_value(json!({
            "id": "thread-1",
            "name": "Envelope",
            "preview": "",
            "cwd": "/repo",
            "source": "appServer",
            "modelProvider": "openai",
            "updatedAt": 1,
            "path": "/tmp/thread.jsonl",
            "status": { "type": "idle" },
            "gitInfo": null,
            "turns": []
        }))
        .expect("thread fixture should deserialize")
    }

    #[test]
    fn checked_in_fixtures_cover_stable_start_resume_and_observation_provenance() {
        let fixtures = fixtures();
        let thread: ThreadRecord = serde_json::from_value(fixtures["thread"].clone())
            .expect("fixture thread should deserialize");

        let equal = to_runtime_envelope(
            ConversationRuntimeConfigurationRequest {
                model: ConversationRuntimeRequestedValue::Value("gpt-equal".to_string()),
                ..ConversationRuntimeConfigurationRequest::default()
            },
            &fixture_fields(&fixtures, "startEqualsRequest"),
            &thread,
            ConversationRuntimeLaunchEnvironment::unknown(),
        )
        .expect("equal start response should map");
        assert_eq!(
            equal.thread_request.model.as_value().map(String::as_str),
            Some("gpt-equal")
        );
        assert_eq!(
            equal.applied.model,
            ConversationRuntimeObservedValue::Observed("gpt-equal".to_string())
        );
        assert!(matches!(
            equal.applied.sandbox,
            ConversationRuntimeObservedValue::Observed(
                ConversationRuntimeSandboxPolicy::WorkspaceWrite {
                    writable_roots: Some(ref roots),
                    writable_roots_truncated: false,
                    exclude_tmpdir_env_var: Some(true),
                    exclude_slash_tmp: Some(false),
                    ..
                }
            ) if roots == &["/repo".to_string(), "/tmp/akra".to_string()]
        ));

        let overridden = to_runtime_envelope(
            ConversationRuntimeConfigurationRequest {
                model: ConversationRuntimeRequestedValue::Value("gpt-requested".to_string()),
                ..ConversationRuntimeConfigurationRequest::default()
            },
            &fixture_fields(&fixtures, "resumeOverrideResponse"),
            &thread,
            ConversationRuntimeLaunchEnvironment::unknown(),
        )
        .expect("override resume response should map");
        assert_eq!(
            overridden
                .thread_request
                .model
                .as_value()
                .map(String::as_str),
            Some("gpt-requested")
        );
        assert_eq!(
            overridden.applied.model,
            ConversationRuntimeObservedValue::Observed("gpt-applied".to_string())
        );
        assert_eq!(
            overridden.applied.reasoning_effort,
            ConversationRuntimeObservedValue::Null
        );
        assert_eq!(
            overridden.applied.service_tier,
            ConversationRuntimeObservedValue::Missing
        );
        assert_eq!(
            overridden.applied.permission_profile,
            ConversationRuntimeObservedValue::UnavailableOnStableResponse
        );

        let missing = to_runtime_envelope(
            ConversationRuntimeConfigurationRequest::default(),
            &fixture_fields(&fixtures, "optionalMissingResponse"),
            &thread,
            ConversationRuntimeLaunchEnvironment::unknown(),
        )
        .expect("missing optional fields should map");
        let explicit_null = to_runtime_envelope(
            ConversationRuntimeConfigurationRequest::default(),
            &fixture_fields(&fixtures, "optionalNullResponse"),
            &thread,
            ConversationRuntimeLaunchEnvironment::unknown(),
        )
        .expect("null optional fields should map");
        assert_eq!(
            missing.applied.reasoning_effort,
            ConversationRuntimeObservedValue::Missing
        );
        assert_eq!(
            explicit_null.applied.reasoning_effort,
            ConversationRuntimeObservedValue::Null
        );

        let unknown = to_runtime_envelope(
            ConversationRuntimeConfigurationRequest::default(),
            &fixture_fields(&fixtures, "unknownResponse"),
            &thread,
            ConversationRuntimeLaunchEnvironment::unknown(),
        )
        .expect("unknown stable values should remain typed");
        let settings = settings_observation(&fixtures["settingsUpdated"])
            .expect("complete settings fixture should map atomically");
        let reroute =
            model_reroute(&fixtures["modelRerouted"]).expect("reroute fixture should map");
        let status = status_observation(fixtures["statusChanged"].get("status"));
        let public_debug = format!("{unknown:?}{settings:?}{reroute:?}{status:?}");
        assert!(!public_debug.contains(SECRET_CANARY));
        assert!(!public_debug.contains("providerCredential"));
        assert!(!public_debug.contains("instructionSources"));

        let error = to_runtime_envelope(
            ConversationRuntimeConfigurationRequest::default(),
            &fixture_fields(&fixtures, "malformedRequiredResponse"),
            &thread,
            ConversationRuntimeLaunchEnvironment::unknown(),
        )
        .expect_err("malformed required response must fail closed");
        assert!(error.to_string().contains("model"));
    }

    #[test]
    fn response_mapping_keeps_requested_applied_missing_and_null_distinct() {
        let fields = BTreeMap::from([
            ("model".to_string(), json!("applied-model")),
            ("modelProvider".to_string(), json!("provider-a")),
            ("reasoningEffort".to_string(), Value::Null),
            ("cwd".to_string(), json!("/applied")),
            ("approvalPolicy".to_string(), json!("on-request")),
            ("approvalsReviewer".to_string(), json!("user")),
            ("sandbox".to_string(), json!({ "type": "readOnly" })),
            ("activePermissionProfile".to_string(), Value::Null),
        ]);
        let request = ConversationRuntimeConfigurationRequest {
            model: ConversationRuntimeRequestedValue::Value("requested-model".to_string()),
            ..ConversationRuntimeConfigurationRequest::default()
        };

        let envelope = to_runtime_envelope(
            request,
            &fields,
            &thread_record(),
            ConversationRuntimeLaunchEnvironment::unknown(),
        )
        .expect("required response envelope should parse");

        assert_eq!(
            envelope.thread_request.model.as_value().map(String::as_str),
            Some("requested-model")
        );
        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("applied-model".to_string())
        );
        assert_eq!(
            envelope.applied.reasoning_effort,
            ConversationRuntimeObservedValue::Null
        );
        assert_eq!(
            envelope.applied.service_tier,
            ConversationRuntimeObservedValue::Missing
        );
        assert_eq!(
            envelope.applied.permission_profile,
            ConversationRuntimeObservedValue::UnavailableOnStableResponse
        );
    }

    #[test]
    fn response_mapping_preserves_unknown_values_and_drops_raw_extra_fields() {
        let fields = BTreeMap::from([
            ("model".to_string(), json!("gpt-next")),
            ("modelProvider".to_string(), json!("provider-next")),
            ("reasoningEffort".to_string(), json!("ultra")),
            ("cwd".to_string(), json!("/repo")),
            ("approvalPolicy".to_string(), json!("future-policy")),
            ("approvalsReviewer".to_string(), json!("future-reviewer")),
            (
                "sandbox".to_string(),
                json!({ "type": "futureSandbox", "credential": "do-not-project" }),
            ),
            (
                "providerMetadata".to_string(),
                json!({ "secret": "do-not-project" }),
            ),
        ]);

        let envelope = to_runtime_envelope(
            ConversationRuntimeConfigurationRequest::default(),
            &fields,
            &thread_record(),
            ConversationRuntimeLaunchEnvironment::unknown(),
        )
        .expect("unknown but structurally valid values should parse");
        assert_eq!(
            envelope.applied.reasoning_effort,
            ConversationRuntimeObservedValue::Observed("ultra".to_string())
        );
        assert!(matches!(
            envelope.applied.approval_policy,
            ConversationRuntimeObservedValue::Observed(
                ConversationRuntimeApprovalPolicy::Unknown(ref label)
            ) if label == "future-policy"
        ));
        assert!(matches!(
            envelope.applied.sandbox,
            ConversationRuntimeObservedValue::Observed(
                ConversationRuntimeSandboxPolicy::Unknown { ref policy_type }
            ) if policy_type == "futureSandbox"
        ));
        let debug = format!("{envelope:?}");
        assert!(!debug.contains("do-not-project"));
        assert!(!debug.contains("credential"));
    }

    #[test]
    fn response_mapping_rejects_missing_null_and_malformed_required_fields() {
        let base = BTreeMap::from([
            ("model".to_string(), json!("gpt-next")),
            ("modelProvider".to_string(), json!("openai")),
            ("cwd".to_string(), json!("/repo")),
            ("approvalPolicy".to_string(), json!("on-request")),
            ("approvalsReviewer".to_string(), json!("user")),
            ("sandbox".to_string(), json!({ "type": "readOnly" })),
        ]);
        for (field, replacement) in [
            ("model", None),
            ("cwd", Some(Value::Null)),
            ("sandbox", Some(json!("read-only"))),
            (
                "sandbox",
                Some(json!({
                    "type": "workspaceWrite",
                    "writableRoots": null
                })),
            ),
        ] {
            let mut fields = base.clone();
            match replacement {
                Some(value) => {
                    fields.insert(field.to_string(), value);
                }
                None => {
                    fields.remove(field);
                }
            }
            let error = to_runtime_envelope(
                ConversationRuntimeConfigurationRequest::default(),
                &fields,
                &thread_record(),
                ConversationRuntimeLaunchEnvironment::unknown(),
            )
            .expect_err("invalid required field must reject the response envelope");
            assert!(error.to_string().contains(field));
        }
    }

    #[test]
    fn settings_parser_preserves_granular_approval_and_profile_provenance() {
        let settings = settings_observation(&json!({
            "threadSettings": {
                "model": "gpt-next",
                "modelProvider": "openai",
                "effort": "max",
                "serviceTier": null,
                "cwd": "/repo",
                "approvalPolicy": {
                    "granular": {
                        "sandbox_approval": true,
                        "rules": false,
                        "mcp_elicitations": true,
                        "request_permissions": true
                    }
                },
                "approvalsReviewer": "auto_review",
                "sandboxPolicy": { "type": "workspaceWrite", "networkAccess": false },
                "activePermissionProfile": { "id": ":workspace", "extends": null },
                "collaborationMode": {}
            }
        }))
        .expect("complete settings snapshot should parse");

        assert_eq!(
            settings.reasoning_effort,
            ConversationRuntimeObservedValue::Observed("max".to_string())
        );
        assert!(matches!(
            settings.approval_policy,
            ConversationRuntimeObservedValue::Defaulted(
                ConversationRuntimeApprovalPolicy::Granular {
                    request_permissions: Some(true),
                    ..
                }
            )
        ));
        assert!(matches!(
            settings.permission_profile,
            ConversationRuntimeObservedValue::Observed(
                ConversationRuntimePermissionProfile { ref id, .. }
            ) if id == ":workspace"
        ));
        assert!(matches!(
            settings.sandbox,
            ConversationRuntimeObservedValue::Defaulted(
                ConversationRuntimeSandboxPolicy::WorkspaceWrite {
                    network_access: Some(false),
                    writable_roots: Some(ref roots),
                    writable_roots_truncated: false,
                    exclude_tmpdir_env_var: Some(false),
                    exclude_slash_tmp: Some(false),
                }
            ) if roots.is_empty()
        ));
    }

    #[test]
    fn status_and_reroute_parsers_preserve_unknown_values_without_raw_payloads() {
        assert_eq!(
            status_observation(Some(&json!({
                "type": "active",
                "activeFlags": ["waitingOnApproval", "futureFlag"]
            }))),
            ConversationRuntimeObservedValue::Observed(ConversationRuntimeThreadStatus::Active {
                waiting_on_approval: true,
                waiting_on_user_input: false,
                unknown_flags: vec!["futureFlag".to_string()],
                unknown_flags_truncated: false,
            })
        );
        assert_eq!(
            model_reroute(&json!({
                "fromModel": "gpt-a",
                "toModel": "gpt-b",
                "reason": "futureReason"
            })),
            Some(ConversationRuntimeModelReroute {
                from_model: "gpt-a".to_string(),
                to_model: "gpt-b".to_string(),
                reason: ConversationRuntimeModelRerouteReason::Unknown("futureReason".to_string()),
            })
        );
    }

    #[test]
    fn sandbox_roots_and_active_flags_preserve_explicit_truncation() {
        let mut roots = (0..(MAX_SANDBOX_WRITABLE_ROOTS + 8))
            .map(|index| format!("/repo/root-{index}"))
            .collect::<Vec<_>>();
        roots[0] = format!("/repo/{}", "x".repeat(MAX_STREAM_IDENTIFIER_BYTES + 32));
        let settings = settings_observation(&json!({
            "threadSettings": {
                "model": "gpt-next",
                "modelProvider": "openai",
                "effort": null,
                "serviceTier": null,
                "cwd": "/repo",
                "approvalPolicy": "on-request",
                "approvalsReviewer": "user",
                "sandboxPolicy": {
                    "type": "workspaceWrite",
                    "writableRoots": roots
                },
                "activePermissionProfile": null,
                "collaborationMode": {}
            }
        }))
        .expect("bounded workspace roots should remain a valid settings snapshot");
        let ConversationRuntimeObservedValue::Defaulted(
            ConversationRuntimeSandboxPolicy::WorkspaceWrite {
                writable_roots: Some(roots),
                writable_roots_truncated,
                ..
            },
        ) = settings.sandbox
        else {
            panic!("workspace-write settings should retain bounded roots");
        };
        assert_eq!(roots.len(), MAX_SANDBOX_WRITABLE_ROOTS);
        assert!(writable_roots_truncated);
        assert!(roots[0].contains("[truncated by Akra"));

        let flags = (0..40)
            .map(|index| Value::String(format!("futureFlag{index}")))
            .collect::<Vec<_>>();
        assert!(matches!(
            status_observation(Some(&json!({
                "type": "active",
                "activeFlags": flags
            }))),
            ConversationRuntimeObservedValue::Observed(
                ConversationRuntimeThreadStatus::Active {
                    ref unknown_flags,
                    unknown_flags_truncated: true,
                    ..
                }
            ) if unknown_flags.len() == 32
        ));
    }

    #[test]
    fn sandbox_roots_reject_malformed_values_after_the_projection_limit() {
        let mut roots = (0..MAX_SANDBOX_WRITABLE_ROOTS)
            .map(|index| Value::String(format!("/repo/root-{index}")))
            .collect::<Vec<_>>();
        roots.push(Value::Bool(false));

        assert_eq!(
            observed_sandbox_policy(Some(&json!({
                "type": "workspaceWrite",
                "writableRoots": roots
            }))),
            ConversationRuntimeObservedValue::Malformed(
                ConversationRuntimeMalformedValue::ExpectedString
            )
        );
    }

    #[test]
    fn active_flags_scan_known_values_after_the_unknown_projection_limit() {
        let mut flags = (0..33)
            .map(|index| Value::String(format!("futureFlag{index}")))
            .collect::<Vec<_>>();
        flags.push(Value::String("waitingOnApproval".to_string()));

        assert!(matches!(
            status_observation(Some(&json!({
                "type": "active",
                "activeFlags": flags
            }))),
            ConversationRuntimeObservedValue::Observed(
                ConversationRuntimeThreadStatus::Active {
                    waiting_on_approval: true,
                    ref unknown_flags,
                    unknown_flags_truncated: true,
                    ..
                }
            ) if unknown_flags.len() == 32
        ));
    }

    #[test]
    fn active_flags_reject_malformed_values_after_the_projection_limit() {
        let mut flags = (0..32)
            .map(|index| Value::String(format!("futureFlag{index}")))
            .collect::<Vec<_>>();
        flags.push(Value::Bool(false));

        assert_eq!(
            status_observation(Some(&json!({
                "type": "active",
                "activeFlags": flags
            }))),
            ConversationRuntimeObservedValue::Malformed(
                ConversationRuntimeMalformedValue::ExpectedString
            )
        );
    }
}
