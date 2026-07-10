use std::collections::HashMap;
use std::path::{Component, Path};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};

use crate::domain::conversation::{ConversationApprovalDecision, ConversationApprovalRequestKind};

const MAX_PERMISSION_ENTRIES: usize = 32;
const MAX_PERMISSION_STRING_CHARS: usize = 4096;
const MAX_GLOB_SCAN_DEPTH: u64 = 64;
const MAX_DISPLAY_SOURCE_CHARS: usize = 16_384;

pub(crate) const INTERACTIVE_APPROVAL_METHODS: &[&str] = &[
    "item/commandExecution/requestApproval",
    "item/permissions/requestApproval",
];
pub(crate) const UNINSPECTABLE_APPROVAL_METHODS: &[&str] = &["item/fileChange/requestApproval"];
pub(crate) const EXPLICITLY_DECLINED_APPROVAL_METHODS: &[&str] =
    &["execCommandApproval", "applyPatchApproval"];

pub(super) struct AppServerApprovalSpec {
    pub(super) server_request_id: String,
    pub(super) method: String,
    pub(super) thread_id: String,
    pub(super) turn_id: String,
    pub(super) item_id: String,
    pub(super) kind: ConversationApprovalRequestKind,
    pub(super) summary: String,
    pub(super) details: Vec<String>,
    pub(super) accepted_result: Value,
    pub(super) declined_result: Value,
}

pub(super) fn parse_interactive_approval(
    method: &str,
    request_id: &Value,
    params: Option<&Value>,
) -> Result<AppServerApprovalSpec> {
    let params = params
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("approval params must be an object"))?;
    let item_id = bounded_identifier(params, "itemId")?;
    let thread_id = bounded_identifier(params, "threadId")?;
    let turn_id = bounded_identifier(params, "turnId")?;

    let (kind, summary, details, accepted_result, declined_result) = match method {
        "item/commandExecution/requestApproval" => {
            validate_command_approval_params(params)?;
            validate_command_decisions(params.get("availableDecisions"))?;
            let mut details = Vec::new();
            push_optional_display_detail(&mut details, params, "command", "Command", 480)?;
            push_optional_display_detail(&mut details, params, "cwd", "Working directory", 240)?;
            push_optional_display_detail(&mut details, params, "reason", "Reason", 320)?;
            push_command_additional_permissions_details(&mut details, params)?;
            push_network_approval_context_detail(&mut details, params)?;
            push_ignored_policy_amendment_details(&mut details, params)?;
            (
                ConversationApprovalRequestKind::CommandExecution,
                "Command execution requested; values are bounded and normalized for display."
                    .to_string(),
                details,
                json!({ "decision": "accept" }),
                json!({ "decision": "decline" }),
            )
        }
        "item/permissions/requestApproval" => {
            validate_permissions_approval_params(params)?;
            let permissions = sanitize_permission_profile(
                params
                    .get("permissions")
                    .ok_or_else(|| anyhow!("permissions are required"))?,
            )?;
            let summary = permission_summary(&permissions);
            let mut details = Vec::new();
            push_optional_display_detail(&mut details, params, "cwd", "Working directory", 240)?;
            push_optional_display_detail(&mut details, params, "reason", "Reason", 320)?;
            details.extend(permission_details(&permissions)?);
            (
                ConversationApprovalRequestKind::Permissions,
                summary,
                details,
                json!({ "permissions": permissions, "scope": "turn" }),
                json!({ "permissions": {}, "scope": "turn" }),
            )
        }
        _ => return Err(anyhow!("method is not an interactive approval request")),
    };

    let mut bound_details = vec![
        format!("Thread: {}", normalized_display_value(&thread_id, 96)?),
        format!("Turn: {}", normalized_display_value(&turn_id, 96)?),
        format!("Item: {}", normalized_display_value(&item_id, 96)?),
    ];
    bound_details.extend(details);

    Ok(AppServerApprovalSpec {
        server_request_id: safe_request_id(request_id),
        method: method.to_string(),
        thread_id,
        turn_id,
        item_id,
        kind,
        summary,
        details: bound_details,
        accepted_result,
        declined_result,
    })
}

fn validate_command_approval_params(params: &Map<String, Value>) -> Result<()> {
    reject_unknown_non_null_keys(
        params,
        &[
            "additionalPermissions",
            "approvalId",
            "availableDecisions",
            "command",
            "commandActions",
            "cwd",
            "environmentId",
            "itemId",
            "networkApprovalContext",
            "proposedExecpolicyAmendment",
            "proposedNetworkPolicyAmendments",
            "reason",
            "startedAtMs",
            "threadId",
            "turnId",
        ],
    )?;
    require_started_at_ms(params)?;
    let command = non_blank_string(
        params
            .get("command")
            .ok_or_else(|| anyhow!("command is required"))?,
        "command",
    )?;
    normalized_display_value(command, 480)?;
    validate_optional_bounded_string(params, "approvalId")?;
    validate_optional_bounded_string(params, "environmentId")?;
    validate_command_actions(params.get("commandActions"))
}

fn validate_permissions_approval_params(params: &Map<String, Value>) -> Result<()> {
    reject_unknown_non_null_keys(
        params,
        &[
            "cwd",
            "environmentId",
            "itemId",
            "permissions",
            "reason",
            "startedAtMs",
            "threadId",
            "turnId",
        ],
    )?;
    require_started_at_ms(params)?;
    let cwd = bounded_string(
        params
            .get("cwd")
            .ok_or_else(|| anyhow!("cwd is required"))?,
        "cwd",
    )?;
    require_absolute_path(cwd)?;
    validate_optional_bounded_string(params, "environmentId")
}

fn require_started_at_ms(params: &Map<String, Value>) -> Result<i64> {
    params
        .get("startedAtMs")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("startedAtMs must be an int64"))
}

fn validate_optional_bounded_string(params: &Map<String, Value>, key: &str) -> Result<()> {
    let Some(value) = params.get(key).filter(|value| !value.is_null()) else {
        return Ok(());
    };
    bounded_string(value, key).map(|_| ())
}

fn validate_command_actions(value: Option<&Value>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_null() {
        return Err(anyhow!("commandActions cannot be null when present"));
    }
    let actions = bounded_array(value, "commandActions")?;
    if actions.is_empty() {
        return Err(anyhow!("commandActions cannot be empty when present"));
    }
    for action in actions {
        let action = action
            .as_object()
            .ok_or_else(|| anyhow!("command action must be an object"))?;
        let action_type = action
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("command action type is required"))?;
        let allowed = match action_type {
            "read" => &["command", "name", "path", "type"][..],
            "listFiles" => &["command", "path", "type"][..],
            "search" => &["command", "path", "query", "type"][..],
            "unknown" => &["command", "type"][..],
            _ => return Err(anyhow!("command action type is unsupported")),
        };
        reject_unknown_non_null_keys(action, allowed)?;
        let action_command = non_blank_string(
            action
                .get("command")
                .ok_or_else(|| anyhow!("command action command is required"))?,
            "command action command",
        )?;
        normalized_display_value(action_command, 480)?;
        if action_type == "read" {
            let name = bounded_non_blank_string(
                action
                    .get("name")
                    .ok_or_else(|| anyhow!("read command action name is required"))?,
                "read command action name",
            )?;
            normalized_display_value(name, 180)?;
            let path = bounded_non_blank_string(
                action
                    .get("path")
                    .ok_or_else(|| anyhow!("read command action path is required"))?,
                "read command action path",
            )?;
            require_absolute_path(path)?;
            normalized_display_value(path, 240)?;
        } else {
            for key in ["path", "query"] {
                let Some(value) = action.get(key).filter(|value| !value.is_null()) else {
                    continue;
                };
                let value = bounded_non_blank_string(value, key)?;
                normalized_display_value(value, 240)?;
            }
        }
    }
    Ok(())
}

fn reject_unknown_non_null_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    if object
        .iter()
        .any(|(key, value)| !allowed.contains(&key.as_str()) && !value.is_null())
    {
        return Err(anyhow!(
            "approval params contain an unsupported non-null field"
        ));
    }
    Ok(())
}

fn push_command_additional_permissions_details(
    details: &mut Vec<String>,
    params: &Map<String, Value>,
) -> Result<()> {
    let Some(permissions) = params
        .get("additionalPermissions")
        .filter(|value| !value.is_null())
    else {
        return Ok(());
    };
    let permissions = sanitize_permission_profile(permissions)?;
    details.push("Additional command permissions:".to_string());
    details.extend(permission_details(&permissions)?);
    Ok(())
}

fn push_network_approval_context_detail(
    details: &mut Vec<String>,
    params: &Map<String, Value>,
) -> Result<()> {
    let Some(context) = params
        .get("networkApprovalContext")
        .filter(|value| !value.is_null())
    else {
        return Ok(());
    };
    let context = context
        .as_object()
        .ok_or_else(|| anyhow!("networkApprovalContext must be an object"))?;
    reject_unknown_keys(context, &["host", "protocol"])?;
    let host = context
        .get("host")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("networkApprovalContext.host must be a string"))?;
    validate_network_approval_host(host)?;
    let protocol = context
        .get("protocol")
        .and_then(Value::as_str)
        .filter(|protocol| matches!(*protocol, "http" | "https" | "socks5Tcp" | "socks5Udp"))
        .ok_or_else(|| anyhow!("networkApprovalContext.protocol is unsupported"))?;
    details.push(format!("Network target: {protocol}://{host}"));
    Ok(())
}

fn validate_network_approval_host(host: &str) -> Result<()> {
    if host.is_empty()
        || !host.is_ascii()
        || host.chars().count() > 512
        || host.chars().any(|character| {
            unsafe_display_character(character)
                || character.is_whitespace()
                || !(character.is_ascii_alphanumeric()
                    || matches!(character, '.' | '-' | '_' | ':' | '[' | ']' | '%'))
        })
    {
        return Err(anyhow!(
            "networkApprovalContext.host cannot be represented safely"
        ));
    }
    Ok(())
}

fn push_ignored_policy_amendment_details(
    details: &mut Vec<String>,
    params: &Map<String, Value>,
) -> Result<()> {
    if let Some(amendment) = params
        .get("proposedExecpolicyAmendment")
        .filter(|value| !value.is_null())
    {
        let rules = bounded_array(amendment, "proposed exec-policy amendment")?;
        for rule in rules {
            bounded_string(rule, "proposed exec-policy rule")?;
        }
        details.push(format!(
            "Persistent exec-policy proposal: {} rule(s) ignored by this one-shot approval.",
            rules.len()
        ));
    }
    if let Some(amendments) = params
        .get("proposedNetworkPolicyAmendments")
        .filter(|value| !value.is_null())
    {
        let amendments = bounded_array(amendments, "proposed network-policy amendments")?;
        for amendment in amendments {
            let amendment = amendment
                .as_object()
                .ok_or_else(|| anyhow!("network-policy amendment must be an object"))?;
            reject_unknown_keys(amendment, &["action", "host"])?;
            let action = amendment
                .get("action")
                .and_then(Value::as_str)
                .filter(|action| matches!(*action, "allow" | "deny"))
                .ok_or_else(|| anyhow!("network-policy amendment action is invalid"))?;
            let host = amendment
                .get("host")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("network-policy amendment host is required"))?;
            validate_network_approval_host(host)?;
            details.push(format!(
                "Persistent network-policy proposal ignored: {action} {host}."
            ));
        }
    }
    Ok(())
}

fn push_optional_display_detail(
    details: &mut Vec<String>,
    params: &Map<String, Value>,
    key: &str,
    label: &str,
    max_display_chars: usize,
) -> Result<()> {
    let Some(value) = params.get(key).filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let value = value
        .as_str()
        .ok_or_else(|| anyhow!("{key} must be a string when present"))?;
    details.push(format!(
        "{label}: {}",
        normalized_display_value(value, max_display_chars)?
    ));
    Ok(())
}

fn normalized_display_value(value: &str, max_display_chars: usize) -> Result<String> {
    if value.chars().count() > MAX_DISPLAY_SOURCE_CHARS {
        return Err(anyhow!("display value exceeds the accepted source bound"));
    }
    if value.chars().any(dangerous_format_character) {
        return Err(anyhow!(
            "display value contains a Unicode format character that cannot be reviewed safely"
        ));
    }
    if value.is_empty() {
        return Ok("(empty)".to_string());
    }
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\\' => escaped.push_str("\\\\"),
            character if character.is_control() => {
                return Err(anyhow!(
                    "display value contains a control character that cannot be reviewed safely"
                ));
            }
            character => escaped.push(character),
        }
    }
    if escaped.chars().count() > max_display_chars {
        return Err(anyhow!(
            "display value exceeds the complete review display bound"
        ));
    }
    Ok(escaped)
}

fn unsafe_display_character(character: char) -> bool {
    character.is_control() || dangerous_format_character(character)
}

fn dangerous_format_character(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{FEFF}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

fn bounded_identifier(params: &Map<String, Value>, key: &str) -> Result<String> {
    let value = params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{key} must be a string"))?;
    if value.is_empty()
        || value.chars().count() > 256
        || value.chars().any(unsafe_display_character)
    {
        return Err(anyhow!("{key} is outside the accepted bounds"));
    }
    Ok(value.to_string())
}

fn safe_request_id(request_id: &Value) -> String {
    match request_id {
        Value::String(value) => {
            normalized_display_value(value, 64).unwrap_or_else(|_| "opaque".to_string())
        }
        Value::Number(value) => value.to_string(),
        _ => "opaque".to_string(),
    }
}

fn validate_command_decisions(value: Option<&Value>) -> Result<()> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let decisions = value
        .as_array()
        .ok_or_else(|| anyhow!("availableDecisions must be an array"))?;
    if decisions.len() > 16 {
        return Err(anyhow!("availableDecisions exceeds the accepted bound"));
    }
    let has_accept = decisions
        .iter()
        .any(|value| value.as_str() == Some("accept"));
    let has_decline = decisions
        .iter()
        .any(|value| value.as_str() == Some("decline"));
    if !has_accept || !has_decline {
        return Err(anyhow!(
            "approval requires one-turn accept and decline decisions"
        ));
    }
    Ok(())
}

fn sanitize_permission_profile(value: &Value) -> Result<Value> {
    let profile = value
        .as_object()
        .ok_or_else(|| anyhow!("permission profile must be an object"))?;
    reject_unknown_keys(profile, &["fileSystem", "network"])?;
    let mut sanitized = Map::new();
    if let Some(file_system) = profile.get("fileSystem").filter(|value| !value.is_null()) {
        sanitized.insert(
            "fileSystem".to_string(),
            sanitize_file_system_permissions(file_system)?,
        );
    }
    if let Some(network) = profile.get("network").filter(|value| !value.is_null()) {
        let network = network
            .as_object()
            .ok_or_else(|| anyhow!("network permissions must be an object"))?;
        reject_unknown_keys(network, &["enabled"])?;
        let enabled = network
            .get("enabled")
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| anyhow!("network.enabled must be boolean"))
            })
            .transpose()?;
        sanitized.insert("network".to_string(), json!({ "enabled": enabled }));
    }
    Ok(Value::Object(sanitized))
}

fn sanitize_file_system_permissions(value: &Value) -> Result<Value> {
    let permissions = value
        .as_object()
        .ok_or_else(|| anyhow!("file-system permissions must be an object"))?;
    reject_unknown_keys(
        permissions,
        &["entries", "globScanMaxDepth", "read", "write"],
    )?;
    let total_rule_count = ["entries", "read", "write"]
        .into_iter()
        .filter_map(|key| permissions.get(key).filter(|value| !value.is_null()))
        .map(|value| {
            bounded_array(value, "file-system permission rules").map(|values| values.len())
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum::<usize>();
    if total_rule_count > MAX_PERMISSION_ENTRIES {
        return Err(anyhow!(
            "combined file-system permission rules exceed the accepted bound"
        ));
    }
    let mut sanitized = Map::new();

    if let Some(entries) = permissions.get("entries").filter(|value| !value.is_null()) {
        let entries = bounded_array(entries, "file-system entries")?;
        let entries = entries
            .iter()
            .map(sanitize_file_system_entry)
            .collect::<Result<Vec<_>>>()?;
        sanitized.insert("entries".to_string(), Value::Array(entries));
    }
    if let Some(depth) = permissions
        .get("globScanMaxDepth")
        .filter(|value| !value.is_null())
    {
        let depth = depth
            .as_u64()
            .filter(|depth| (1..=MAX_GLOB_SCAN_DEPTH).contains(depth))
            .ok_or_else(|| anyhow!("globScanMaxDepth is outside the accepted bounds"))?;
        sanitized.insert("globScanMaxDepth".to_string(), Value::from(depth));
    }
    for key in ["read", "write"] {
        if let Some(paths) = permissions.get(key).filter(|value| !value.is_null()) {
            let paths = bounded_array(paths, key)?;
            let paths = paths
                .iter()
                .map(|value| {
                    let path = bounded_string(value, key)?;
                    require_absolute_path(path)?;
                    Ok(Value::String(path.to_string()))
                })
                .collect::<Result<Vec<_>>>()?;
            sanitized.insert(key.to_string(), Value::Array(paths));
        }
    }
    Ok(Value::Object(sanitized))
}

fn sanitize_file_system_entry(value: &Value) -> Result<Value> {
    let entry = value
        .as_object()
        .ok_or_else(|| anyhow!("file-system entry must be an object"))?;
    reject_unknown_keys(entry, &["access", "path"])?;
    let access = entry
        .get("access")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "read" | "write" | "deny"))
        .ok_or_else(|| anyhow!("file-system entry has an invalid access mode"))?;
    let path = sanitize_file_system_path(
        entry
            .get("path")
            .ok_or_else(|| anyhow!("file-system entry path is required"))?,
    )?;
    Ok(json!({ "access": access, "path": path }))
}

fn sanitize_file_system_path(value: &Value) -> Result<Value> {
    let path = value
        .as_object()
        .ok_or_else(|| anyhow!("file-system path must be an object"))?;
    match path.get("type").and_then(Value::as_str) {
        Some("path") => {
            reject_unknown_keys(path, &["type", "path"])?;
            let raw = bounded_string(
                path.get("path")
                    .ok_or_else(|| anyhow!("absolute path is required"))?,
                "path",
            )?;
            require_absolute_path(raw)?;
            Ok(json!({ "type": "path", "path": raw }))
        }
        Some("glob_pattern") => {
            reject_unknown_keys(path, &["type", "pattern"])?;
            let pattern = bounded_string(
                path.get("pattern")
                    .ok_or_else(|| anyhow!("glob pattern is required"))?,
                "pattern",
            )?;
            Ok(json!({ "type": "glob_pattern", "pattern": pattern }))
        }
        Some("special") => {
            reject_unknown_keys(path, &["type", "value"])?;
            let special = sanitize_special_path(
                path.get("value")
                    .ok_or_else(|| anyhow!("special path value is required"))?,
            )?;
            Ok(json!({ "type": "special", "value": special }))
        }
        _ => Err(anyhow!("file-system path has an invalid type")),
    }
}

fn sanitize_special_path(value: &Value) -> Result<Value> {
    let special = value
        .as_object()
        .ok_or_else(|| anyhow!("special path must be an object"))?;
    let kind = special
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("special path kind is required"))?;
    match kind {
        "root" | "minimal" | "tmpdir" | "slash_tmp" => {
            reject_unknown_keys(special, &["kind"])?;
            Ok(json!({ "kind": kind }))
        }
        "project_roots" => {
            reject_unknown_keys(special, &["kind", "subpath"])?;
            let subpath = special
                .get("subpath")
                .filter(|value| !value.is_null())
                .map(|value| bounded_string(value, "subpath"))
                .transpose()?;
            if let Some(subpath) = subpath {
                require_relative_subpath(subpath)?;
            }
            Ok(json!({ "kind": kind, "subpath": subpath }))
        }
        _ => Err(anyhow!("special path kind is not supported")),
    }
}

fn reject_unknown_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(anyhow!("permission object contains unsupported fields"));
    }
    Ok(())
}

fn bounded_array<'a>(value: &'a Value, label: &str) -> Result<&'a [Value]> {
    let values = value
        .as_array()
        .ok_or_else(|| anyhow!("{label} must be an array"))?;
    if values.len() > MAX_PERMISSION_ENTRIES {
        return Err(anyhow!("{label} exceeds the accepted bound"));
    }
    Ok(values)
}

fn bounded_string<'a>(value: &'a Value, label: &str) -> Result<&'a str> {
    let value = value
        .as_str()
        .ok_or_else(|| anyhow!("{label} must be a string"))?;
    if value.is_empty()
        || value.chars().count() > MAX_PERMISSION_STRING_CHARS
        || value.chars().any(unsafe_display_character)
    {
        return Err(anyhow!("{label} is outside the accepted bounds"));
    }
    Ok(value)
}

fn bounded_non_blank_string<'a>(value: &'a Value, label: &str) -> Result<&'a str> {
    let value = bounded_string(value, label)?;
    if value.trim().is_empty() {
        return Err(anyhow!("{label} cannot be blank"));
    }
    Ok(value)
}

fn non_blank_string<'a>(value: &'a Value, label: &str) -> Result<&'a str> {
    let value = value
        .as_str()
        .ok_or_else(|| anyhow!("{label} must be a string"))?;
    if value.trim().is_empty() {
        return Err(anyhow!("{label} cannot be blank"));
    }
    Ok(value)
}

fn require_absolute_path(value: &str) -> Result<()> {
    if !Path::new(value).is_absolute() {
        return Err(anyhow!("permission path must be absolute"));
    }
    Ok(())
}

fn require_relative_subpath(value: &str) -> Result<()> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(anyhow!("project-root subpath must remain relative"));
    }
    Ok(())
}

fn permission_summary(permissions: &Value) -> String {
    let file_system = permissions.get("fileSystem").and_then(Value::as_object);
    let entry_count = file_system
        .and_then(|value| value.get("entries"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let legacy_path_count = ["read", "write"]
        .iter()
        .map(|key| {
            file_system
                .and_then(|value| value.get(*key))
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        })
        .sum::<usize>();
    let network = permissions
        .pointer("/network/enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    format!(
        "Additional turn-scoped permissions requested; file-system rules: {}, network: {}. Validated scopes are shown below.",
        entry_count + legacy_path_count,
        if network { "enabled" } else { "unchanged" }
    )
}

fn permission_details(permissions: &Value) -> Result<Vec<String>> {
    let mut details = Vec::new();
    let network = permissions.pointer("/network/enabled");
    if let Some(network) = network {
        details.push(format!(
            "Network access: {}",
            match network.as_bool() {
                Some(true) => "enabled",
                Some(false) => "disabled",
                None => "unchanged",
            }
        ));
    }
    let Some(file_system) = permissions.get("fileSystem").and_then(Value::as_object) else {
        if details.is_empty() {
            details.push("No additional permission fields were requested.".to_string());
        }
        return Ok(details);
    };

    if let Some(depth) = file_system.get("globScanMaxDepth").and_then(Value::as_u64) {
        details.push(format!("Glob scan depth: {depth}"));
    }
    if let Some(entries) = file_system.get("entries").and_then(Value::as_array) {
        for entry in entries {
            let access = entry
                .get("access")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let path = display_permission_path(
                entry
                    .get("path")
                    .ok_or_else(|| anyhow!("validated permission entry lost its path"))?,
            )?;
            details.push(format!("File-system {access}: {path}"));
        }
    }
    for key in ["read", "write"] {
        if let Some(paths) = file_system.get(key).and_then(Value::as_array) {
            for path in paths {
                details.push(format!(
                    "File-system {key}: {}",
                    normalized_display_value(
                        path.as_str()
                            .ok_or_else(|| anyhow!("validated path is not a string"))?,
                        240,
                    )?
                ));
            }
        }
    }
    if details.is_empty() {
        details.push("No additional permission fields were requested.".to_string());
    }
    Ok(details)
}

fn display_permission_path(value: &Value) -> Result<String> {
    match value.get("type").and_then(Value::as_str) {
        Some("path") => normalized_display_value(
            value
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("validated absolute path is missing"))?,
            240,
        ),
        Some("glob_pattern") => Ok(format!(
            "glob {}",
            normalized_display_value(
                value
                    .get("pattern")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("validated glob pattern is missing"))?,
                220,
            )?
        )),
        Some("special") => {
            let special = value
                .get("value")
                .and_then(Value::as_object)
                .ok_or_else(|| anyhow!("validated special path is missing"))?;
            let kind = special
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("validated special path kind is missing"))?;
            let subpath = special
                .get("subpath")
                .and_then(Value::as_str)
                .map(|value| normalized_display_value(value, 180))
                .transpose()?;
            Ok(subpath.map_or_else(
                || format!("special {kind}"),
                |subpath| format!("special {kind}/{subpath}"),
            ))
        }
        _ => Err(anyhow!("validated permission path has an invalid type")),
    }
}

#[derive(Default)]
pub(super) struct AppServerApprovalBroker {
    next_id: AtomicU64,
    pending: Mutex<HashMap<String, SyncSender<ConversationApprovalDecision>>>,
}

impl AppServerApprovalBroker {
    pub(super) fn register(&self) -> Result<(String, Receiver<ConversationApprovalDecision>)> {
        let sequence = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let approval_id = format!("approval-{sequence}");
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending
            .lock()
            .map_err(|_| anyhow!("approval broker mutex was poisoned"))?
            .insert(approval_id.clone(), sender);
        Ok((approval_id, receiver))
    }

    pub(super) fn resolve(
        &self,
        approval_id: &str,
        decision: ConversationApprovalDecision,
    ) -> Result<()> {
        let sender = self
            .pending
            .lock()
            .map_err(|_| anyhow!("approval broker mutex was poisoned"))?
            .remove(approval_id)
            .ok_or_else(|| anyhow!("approval request is no longer pending"))?;
        sender
            .send(decision)
            .map_err(|_| anyhow!("approval request is no longer connected"))
    }

    pub(super) fn cancel(&self, approval_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(approval_id);
        }
    }

    #[cfg(test)]
    pub(super) fn pending_count(&self) -> usize {
        self.pending.lock().map_or(0, |pending| pending.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_resolves_each_request_exactly_once() {
        let broker = AppServerApprovalBroker::default();
        let (approval_id, receiver) = broker.register().expect("register approval");

        broker
            .resolve(&approval_id, ConversationApprovalDecision::Accept)
            .expect("resolve approval");

        assert_eq!(
            receiver.recv().expect("receive decision"),
            ConversationApprovalDecision::Accept
        );
        assert!(
            broker
                .resolve(&approval_id, ConversationApprovalDecision::Decline)
                .is_err()
        );
        assert_eq!(broker.pending_count(), 0);
    }

    #[test]
    fn command_approval_projects_bounded_normalized_details_only() {
        let spec = parse_interactive_approval(
            "item/commandExecution/requestApproval",
            &json!(17),
            Some(&json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": "cargo test\n--lib",
                "cwd": "/workspace\troot",
                "reason": "verify\rchanges",
                "availableDecisions": ["accept", "decline", "acceptForSession"]
            })),
        )
        .expect("command approval should parse");

        assert_eq!(spec.server_request_id, "17");
        assert_eq!(spec.thread_id, "thread-1");
        assert_eq!(spec.turn_id, "turn-1");
        assert_eq!(spec.item_id, "item-1");
        assert_eq!(spec.accepted_result, json!({ "decision": "accept" }));
        assert!(
            spec.details
                .iter()
                .any(|detail| detail == "Command: cargo test\\n--lib")
        );
        assert!(
            spec.details
                .iter()
                .all(|detail| !detail.chars().any(unsafe_display_character))
        );
    }

    #[test]
    fn approvals_reject_details_that_cannot_be_displayed_in_full() {
        for (key, value) in [
            ("command", "x".repeat(481)),
            ("cwd", format!("/{}", "x".repeat(240))),
            ("reason", "x".repeat(321)),
        ] {
            let mut params = json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            });
            params
                .as_object_mut()
                .expect("approval params should be an object")
                .insert(key.to_string(), Value::String(value));

            assert!(
                parse_interactive_approval(
                    "item/commandExecution/requestApproval",
                    &json!("oversized"),
                    Some(&params),
                )
                .is_err(),
                "{key} must not be truncated into an approvable request"
            );
        }
    }

    #[test]
    fn permission_approval_rejects_paths_that_cannot_be_displayed_in_full() {
        let oversized_path = format!("/{}", "x".repeat(240));
        let result = parse_interactive_approval(
            "item/permissions/requestApproval",
            &json!("oversized-permission"),
            Some(&json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "permissions": {
                    "fileSystem": { "read": [oversized_path] }
                }
            })),
        );

        assert!(result.is_err());
    }

    #[test]
    fn approvals_reject_dangerous_unicode_format_characters() {
        for character in ['\u{00ad}', '\u{061c}', '\u{180e}', '\u{206a}', '\u{206f}'] {
            let params = json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": format!("cargo test{character} --lib"),
                "availableDecisions": ["accept", "decline"]
            });

            assert!(
                parse_interactive_approval(
                    "item/commandExecution/requestApproval",
                    &json!("unicode-format"),
                    Some(&params),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn command_approval_preserves_shell_separators_and_repeated_spaces() {
        let spec = parse_interactive_approval(
            "item/commandExecution/requestApproval",
            &json!("meaning-preserving-command"),
            Some(&json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": "echo safe\nrm -rf target\\n-name  &&  cargo test\t--lib",
                "availableDecisions": ["accept", "decline"]
            })),
        )
        .expect("representable shell separators should remain reviewable");

        assert!(spec.details.iter().any(|detail| {
            detail == "Command: echo safe\\nrm -rf target\\\\n-name  &&  cargo test\\t--lib"
        }));
        assert!(
            !spec
                .details
                .iter()
                .any(|detail| detail.contains("echo safe rm"))
        );
    }

    #[test]
    fn command_approval_rejects_session_only_decisions() {
        let result = parse_interactive_approval(
            "item/commandExecution/requestApproval",
            &json!("approval"),
            Some(&json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["acceptForSession", "decline"]
            })),
        );

        assert!(result.is_err());
    }

    #[test]
    fn command_approval_validates_and_displays_privilege_context() {
        let spec = parse_interactive_approval(
            "item/commandExecution/requestApproval",
            &json!("command-privileged"),
            Some(&json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": "curl https://api.example.com",
                "availableDecisions": ["accept", "decline"],
                "additionalPermissions": {
                    "network": { "enabled": true },
                    "fileSystem": { "read": ["/workspace/config"] }
                },
                "networkApprovalContext": {
                    "host": "api.example.com:443",
                    "protocol": "https"
                },
                "proposedExecpolicyAmendment": ["allow curl api.example.com"],
                "proposedNetworkPolicyAmendments": [{
                    "action": "allow",
                    "host": "api.example.com"
                }]
            })),
        )
        .expect("bounded privilege context should parse");

        assert!(
            spec.details
                .iter()
                .any(|detail| detail == "Additional command permissions:")
        );
        assert!(
            spec.details
                .iter()
                .any(|detail| detail == "File-system read: /workspace/config")
        );
        assert!(
            spec.details
                .iter()
                .any(|detail| detail == "Network access: enabled")
        );
        assert!(
            spec.details
                .iter()
                .any(|detail| detail == "Network target: https://api.example.com:443")
        );
        assert!(spec
            .details
            .iter()
            .any(|detail| detail.contains("exec-policy proposal") && detail.contains("ignored")));
        assert!(spec.details.iter().any(|detail| {
            detail == "Persistent network-policy proposal ignored: allow api.example.com."
        }));
        assert_eq!(spec.accepted_result, json!({ "decision": "accept" }));
    }

    #[test]
    fn command_approval_rejects_unrepresentable_privilege_context() {
        let invalid_contexts = [
            json!({ "additionalPermissions": { "fileSystem": { "read": ["relative"] } } }),
            json!({
                "networkApprovalContext": {
                    "host": "api.example.com/path",
                    "protocol": "https"
                }
            }),
            json!({
                "networkApprovalContext": {
                    "host": "api.example.com",
                    "protocol": "ftp"
                }
            }),
            json!({
                "networkApprovalContext": {
                    "host": "api.example.com",
                    "protocol": "https",
                    "futureField": true
                }
            }),
            json!({ "proposedExecpolicyAmendment": { "unexpected": true } }),
            json!({
                "proposedNetworkPolicyAmendments": [{
                    "action": "allow",
                    "host": "api.example.com/path"
                }]
            }),
        ];
        for context in invalid_contexts {
            let mut params = json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": "curl",
                "availableDecisions": ["accept", "decline"]
            });
            params
                .as_object_mut()
                .expect("params should be an object")
                .extend(
                    context
                        .as_object()
                        .expect("context should be an object")
                        .clone(),
                );
            assert!(
                parse_interactive_approval(
                    "item/commandExecution/requestApproval",
                    &json!("command-invalid"),
                    Some(&params),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn command_approval_requires_complete_non_blank_reviewable_execution_text() {
        for command in [Value::Null, json!(""), json!("  \t\n")] {
            let result = parse_interactive_approval(
                "item/commandExecution/requestApproval",
                &json!("blank-command"),
                Some(&json!({
                    "itemId": "item-network",
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "startedAtMs": 1,
                    "command": command,
                    "availableDecisions": ["accept", "decline"],
                    "networkApprovalContext": {
                        "host": "api.example.com",
                        "protocol": "https"
                    }
                })),
            );

            assert!(
                result.is_err(),
                "network context must not make missing command text approvable"
            );
        }

        let missing = parse_interactive_approval(
            "item/commandExecution/requestApproval",
            &json!("missing-command"),
            Some(&json!({
                "itemId": "item-network",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "availableDecisions": ["accept", "decline"]
            })),
        );
        assert!(missing.is_err());
    }

    #[test]
    fn command_approval_rejects_null_empty_incomplete_and_unrenderable_actions() {
        let invalid_actions = [
            Value::Null,
            json!([]),
            json!([{ "type": "unknown" }]),
            json!([{ "type": "unknown", "command": "  " }]),
            json!([{
                "type": "read",
                "command": "cat file",
                "name": "cat",
                "path": "relative/file"
            }]),
            json!([{
                "type": "read",
                "command": "cat file",
                "name": "cat",
                "path": format!("/{}", "x".repeat(241))
            }]),
            json!([{
                "type": "search",
                "command": "rg needle",
                "query": "\u{2066}needle"
            }]),
        ];

        for command_actions in invalid_actions {
            let result = parse_interactive_approval(
                "item/commandExecution/requestApproval",
                &json!("invalid-actions"),
                Some(&json!({
                    "itemId": "item-1",
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "startedAtMs": 1,
                    "command": "cargo test",
                    "commandActions": command_actions,
                    "availableDecisions": ["accept", "decline"]
                })),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn approval_projection_rejects_undisplayable_commands_and_file_changes() {
        let long_command = "x".repeat(2_000);
        let command = parse_interactive_approval(
            "item/commandExecution/requestApproval",
            &json!("command-long"),
            Some(&json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": long_command,
                "availableDecisions": ["accept", "decline"]
            })),
        );
        assert!(
            command.is_err(),
            "a command that cannot be displayed in full must not become approvable"
        );

        let file = parse_interactive_approval(
            "item/fileChange/requestApproval",
            &json!("file-patch"),
            Some(&json!({
                "itemId": "item-2",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "reason": "update source"
            })),
        );
        assert!(
            file.is_err(),
            "file-change approvals must never become interactive"
        );
    }

    #[test]
    fn permission_approval_returns_only_validated_turn_scoped_permissions() {
        let spec = parse_interactive_approval(
            "item/permissions/requestApproval",
            &json!("permission-1"),
            Some(&json!({
                "itemId": "item-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "cwd": "/workspace",
                "permissions": {
                    "network": { "enabled": true },
                    "fileSystem": {
                        "globScanMaxDepth": 8,
                        "entries": [{
                            "access": "write",
                            "path": { "type": "path", "path": "/workspace/output" }
                        }]
                    }
                }
            })),
        )
        .expect("bounded permission request should parse");

        assert_eq!(spec.accepted_result["scope"], "turn");
        assert_eq!(
            spec.accepted_result.pointer("/permissions/network/enabled"),
            Some(&json!(true))
        );
        assert!(
            spec.details
                .iter()
                .any(|detail| detail == "File-system write: /workspace/output")
        );
        assert!(
            spec.details
                .iter()
                .any(|detail| detail == "Network access: enabled")
        );
    }

    #[test]
    fn permission_approval_rejects_unknown_fields_relative_paths_and_excess_rules() {
        let too_many_read_paths = (0..16)
            .map(|index| json!(format!("/read/{index}")))
            .collect::<Vec<_>>();
        let too_many_write_paths = (0..17)
            .map(|index| json!(format!("/write/{index}")))
            .collect::<Vec<_>>();
        for permissions in [
            json!({ "network": { "enabled": true, "token": "secret" } }),
            json!({ "fileSystem": { "write": ["relative/path"] } }),
            json!({
                "fileSystem": {
                    "read": too_many_read_paths,
                    "write": too_many_write_paths
                }
            }),
        ] {
            let result = parse_interactive_approval(
                "item/permissions/requestApproval",
                &json!("permission-invalid"),
                Some(&json!({
                    "itemId": "item-1",
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "startedAtMs": 1,
                    "cwd": "/workspace",
                    "permissions": permissions
                })),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn approval_methods_reject_unknown_non_null_and_invalid_required_timestamp_fields() {
        let cases = [
            (
                "item/commandExecution/requestApproval",
                json!({
                    "itemId": "item-command",
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "startedAtMs": 1,
                    "command": "cargo test",
                    "availableDecisions": ["accept", "decline"]
                }),
            ),
            (
                "item/permissions/requestApproval",
                json!({
                    "itemId": "item-permissions",
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "startedAtMs": 1,
                    "cwd": "/workspace",
                    "permissions": { "network": { "enabled": true } }
                }),
            ),
        ];

        for (method, valid) in cases {
            let mut unknown_null = valid.clone();
            unknown_null["futureField"] = Value::Null;
            assert!(
                parse_interactive_approval(method, &json!("unknown-null"), Some(&unknown_null))
                    .is_ok(),
                "JSON Schema permits inert unknown null fields for {method}"
            );

            let mut unknown_non_null = valid.clone();
            unknown_non_null["futureScope"] = json!("session");
            assert!(
                parse_interactive_approval(
                    method,
                    &json!("unknown-non-null"),
                    Some(&unknown_non_null),
                )
                .is_err(),
                "unknown non-null fields must fail closed for {method}"
            );

            let mut missing_timestamp = valid.clone();
            missing_timestamp
                .as_object_mut()
                .expect("approval params should be an object")
                .remove("startedAtMs");
            assert!(
                parse_interactive_approval(
                    method,
                    &json!("timestamp-missing"),
                    Some(&missing_timestamp),
                )
                .is_err()
            );

            for invalid_timestamp in [json!("1"), json!(u64::MAX)] {
                let mut invalid = valid.clone();
                invalid["startedAtMs"] = invalid_timestamp;
                assert!(
                    parse_interactive_approval(
                        method,
                        &json!("timestamp-invalid"),
                        Some(&invalid),
                    )
                    .is_err(),
                    "invalid int64 timestamp must fail closed for {method}"
                );
            }
        }
    }

    #[test]
    fn permission_approval_requires_a_bounded_absolute_cwd() {
        let valid = json!({
            "itemId": "item-permissions",
            "threadId": "thread-1",
            "turnId": "turn-1",
            "startedAtMs": 1,
            "cwd": "/workspace",
            "permissions": {}
        });
        for invalid_cwd in [Value::Null, json!("relative/path"), json!(42)] {
            let mut invalid = valid.clone();
            invalid["cwd"] = invalid_cwd;
            assert!(
                parse_interactive_approval(
                    "item/permissions/requestApproval",
                    &json!("permission-cwd-invalid"),
                    Some(&invalid),
                )
                .is_err()
            );
        }
    }

    #[test]
    fn codex_0_144_stable_approval_response_shapes_are_exact() {
        let command = parse_interactive_approval(
            "item/commandExecution/requestApproval",
            &json!("command"),
            Some(&json!({
                "itemId": "item-command",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "command": "cargo test",
                "availableDecisions": ["accept", "decline"]
            })),
        )
        .expect("command approval should parse");
        assert_eq!(command.accepted_result, json!({ "decision": "accept" }));
        assert_eq!(command.declined_result, json!({ "decision": "decline" }));

        let permissions = parse_interactive_approval(
            "item/permissions/requestApproval",
            &json!("permissions"),
            Some(&json!({
                "itemId": "item-permissions",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "startedAtMs": 1,
                "cwd": "/workspace",
                "permissions": { "network": { "enabled": true } }
            })),
        )
        .expect("permission approval should parse");
        assert_eq!(
            permissions.accepted_result,
            json!({
                "permissions": { "network": { "enabled": true } },
                "scope": "turn"
            })
        );
        assert_eq!(
            permissions.declined_result,
            json!({ "permissions": {}, "scope": "turn" })
        );
    }
}
