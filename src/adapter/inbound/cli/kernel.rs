//! Machine CLI. The embedded protocol specification owns command names and input fields.

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use rand::RngCore;
use serde_json::{Map, Value, json};

use crate::adapter::inbound::kernel_protocol::{CommandRequest, KernelQuery};
use crate::application::port::inbound::kernel_command_port::KernelCommandPort;
use crate::composition::production;
use crate::domain::kernel_spec::KernelCallError;

const SPEC: &str = include_str!("../../../../schema/akra-kernel-spec.json");

#[derive(Debug)]
struct Invocation {
    workspace: PathBuf,
    request: Request,
    wait: bool,
}

#[derive(Debug)]
enum Request {
    Command(CommandRequest),
    Query(KernelQuery),
}

pub(super) fn describe(args: &[OsString], stdout: &mut impl Write) -> Result<i32> {
    let result = if args == [OsString::from("--json")] {
        serde_json::from_str(SPEC).context("invalid embedded kernel specification")
    } else {
        Err(anyhow::anyhow!("usage: akra describe --json"))
    };
    render(result, None, stdout)
}

pub(super) fn run(args: &[OsString], stdout: &mut impl Write) -> Result<i32> {
    run_with(args, execute, stdout)
}

fn run_with(
    args: &[OsString],
    execute: impl FnOnce(Invocation) -> Result<Value>,
    stdout: &mut impl Write,
) -> Result<i32> {
    let invocation = parse(args);
    // A command that fails after it was sent may have run. The output carries its key, so that the
    // caller can send the same command again with that key and the kernel does not run it twice.
    let sent_key = invocation
        .as_ref()
        .ok()
        .and_then(|invocation| match &invocation.request {
            Request::Command(request) => request.idempotency_key.clone(),
            Request::Query(_) => None,
        });
    render(invocation.and_then(execute), sent_key.as_deref(), stdout)
}

fn execute(invocation: Invocation) -> Result<Value> {
    let port = production::build_kernel_command_port(&invocation.workspace)?;
    execute_with_port(invocation, port.as_ref())
}

fn execute_with_port(invocation: Invocation, port: &dyn KernelCommandPort) -> Result<Value> {
    match invocation.request {
        Request::Command(request) => port.command(
            request.command.kind(),
            serde_json::to_value(request.command)?,
            request.idempotency_key,
        ),
        Request::Query(query) => port.query(serde_json::to_value(query)?, invocation.wait),
    }
}

fn render(result: Result<Value>, sent_key: Option<&str>, stdout: &mut impl Write) -> Result<i32> {
    let (output, code) = match result {
        Ok(output) => {
            let code = if !output["rejection"].is_null()
                || output["operation"]["state"] == "rejected"
                || output["state"] == "rejected"
            {
                2
            } else if output["processed"] == false {
                1
            } else {
                0
            };
            (output, code)
        }
        Err(error) => {
            let mut output = json!({"error": {"message": format!("{error:#}")}});
            // The code finds the entry in the `errors` list of the specification and its next steps.
            if let Some(kernel_error) = error.downcast_ref::<KernelCallError>() {
                output["error"]["code"] = json!(kernel_error.code);
            }
            if let Some(key) = sent_key {
                output["idempotency_key"] = json!(key);
            }
            (output, 1)
        }
    };
    serde_json::to_writer(&mut *stdout, &output)?;
    writeln!(stdout)?;
    Ok(code)
}

fn parse(args: &[OsString]) -> Result<Invocation> {
    let words = args
        .iter()
        .map(|word| {
            word.to_str()
                .context("kernel arguments must be valid UTF-8")
        })
        .collect::<Result<Vec<_>>>()?;
    let spec: Value = serde_json::from_str(SPEC)?;
    let mut positional = Vec::new();
    let mut fields = Map::new();
    let mut workspace = std::env::current_dir()?;
    let mut key = None;
    let mut wait = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut index = 0;
    while index < words.len() {
        let word = words[index];
        if let Some(flag) = word.strip_prefix("--") {
            ensure!(seen.insert(flag), "duplicate option --{flag}");
            match flag {
                "json" => {}
                "wait" => wait = true,
                _ => {
                    index += 1;
                    let value = words.get(index).context("option requires a value")?;
                    ensure!(!value.starts_with("--"), "option --{flag} requires a value");
                    match flag {
                        "workspace" => workspace = PathBuf::from(value),
                        "idempotency-key" => key = Some((*value).to_string()),
                        _ => {
                            ensure!(flag != "type", "--type is not an input field");
                            let name = flag.replace('-', "_");
                            ensure!(!fields.contains_key(&name), "duplicate field {name}");
                            fields.insert(name, Value::String((*value).to_string()));
                        }
                    }
                }
            }
        } else {
            positional.push(word);
        }
        index += 1;
    }
    let (section, name, scope) = match positional.as_slice() {
        ["query", name] => ("queries", *name, None),
        ["history"] => {
            ensure!(fields.contains_key("after"), "history requires --after");
            ("queries", "history", None)
        }
        ["operation", id] => {
            ensure!(!fields.contains_key("op_id"), "duplicate operation id");
            fields.insert("op_id".into(), Value::String((*id).into()));
            ("queries", "operation", None)
        }
        [scope, name] => ("commands", *name, Some(*scope)),
        _ => bail!("usage: akra kernel <scope> <command> [--field value] --json"),
    };
    let entry = spec[section]
        .as_array()
        .context("missing embedded specification entries")?
        .iter()
        .find(|entry| entry["name"] == name)
        .with_context(|| format!("unknown kernel {section} name: {name}"))?;
    if let Some(scope) = scope {
        ensure!(
            entry["scope"] == scope,
            "command {name} does not belong to scope {scope}"
        );
        ensure!(!wait, "--wait is only valid for history and operation");
    } else {
        ensure!(
            key.is_none(),
            "--idempotency-key is only valid for commands"
        );
        ensure!(
            !wait || matches!(name, "history" | "operation"),
            "--wait is only valid for history and operation"
        );
    }
    let properties = entry["input"]["properties"]
        .as_object()
        .context("missing input properties")?;
    let mut input = Map::new();
    input.insert("type".into(), json!(name));
    for (name, raw) in fields {
        let property = properties
            .get(&name)
            .with_context(|| format!("unknown input field: {name}"))?;
        let property = resolve(property, &spec)?;
        let text = raw.as_str().context("expected argument text")?;
        let string_type = property["type"] == "string"
            || property["type"]
                .as_array()
                .is_some_and(|types| types.contains(&json!("string")));
        let value = if string_type {
            json!(text)
        } else {
            serde_json::from_str(text).with_context(|| format!("{name} requires a JSON value"))?
        };
        input.insert(name, value);
    }
    let input = Value::Object(input);
    validate(&entry["input"], &input, &spec, name)?;
    let request = if section == "commands" {
        let request: CommandRequest = serde_json::from_value(json!({
            "command": input,
            "idempotency_key": key.unwrap_or_else(random_key)
        }))?;
        request
            .descriptor()
            .map_err(|error| anyhow::anyhow!(error.message))?;
        request
            .command
            .clone()
            .into_core(&workspace.to_string_lossy())
            .map_err(|error| anyhow::anyhow!(error.message))?;
        Request::Command(request)
    } else {
        Request::Query(serde_json::from_value(input)?)
    };
    Ok(Invocation {
        workspace,
        request,
        wait,
    })
}

fn random_key() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn resolve<'a>(schema: &'a Value, spec: &'a Value) -> Result<&'a Value> {
    match schema["$ref"].as_str() {
        Some(reference) => spec
            .pointer(
                reference
                    .strip_prefix('#')
                    .context("nonlocal schema reference")?,
            )
            .context("unknown schema reference"),
        None => Ok(schema),
    }
}

fn validate(schema: &Value, value: &Value, spec: &Value, path: &str) -> Result<()> {
    let schema = resolve(schema, spec)?;
    let matches_type = |kind: &str| match kind {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => false,
    };
    if let Some(kind) = schema.get("type") {
        let valid = kind.as_str().is_some_and(matches_type)
            || kind
                .as_array()
                .is_some_and(|types| types.iter().filter_map(Value::as_str).any(matches_type));
        ensure!(valid, "{path}: expected {kind}");
    }
    if let Some(expected) = schema.get("const") {
        ensure!(expected == value, "{path}: expected {expected}");
    }
    if let Some(allowed) = schema["enum"].as_array() {
        ensure!(allowed.contains(value), "{path}: invalid enum value");
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branches) = schema[keyword].as_array() {
            let count = branches
                .iter()
                .filter(|branch| validate(branch, value, spec, path).is_ok())
                .count();
            ensure!(
                count > 0 && (keyword != "oneOf" || count == 1),
                "{path}: invalid {keyword} value"
            );
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        ensure!(
            schema["minLength"].as_u64().is_none_or(|min| length >= min)
                && schema["maxLength"].as_u64().is_none_or(|max| length <= max),
            "{path}: invalid string length"
        );
        if let Some(pattern) = schema["pattern"].as_str() {
            ensure!(pattern == r"\S", "{path}: unsupported schema pattern");
            ensure!(!text.trim().is_empty(), "{path}: must not be blank");
        }
    }
    if let Some(number) = value.as_f64() {
        ensure!(
            !schema["minimum"].as_f64().is_some_and(|min| number < min)
                && !schema["maximum"].as_f64().is_some_and(|max| number > max),
            "{path}: number out of range"
        );
    }
    if let Some(fields) = value.as_object() {
        for required in schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            ensure!(fields.contains_key(required), "{path}: missing {required}");
        }
        for (name, field) in fields {
            if let Some(property) = schema["properties"].get(name) {
                validate(property, field, spec, &format!("{path}.{name}"))?;
            } else {
                ensure!(
                    schema["additionalProperties"] != false,
                    "{path}: unknown field {name}"
                );
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
        for item in values {
            validate(items, item, spec, path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::kernel_spec::KernelCommandKind;
    use std::sync::Mutex;

    struct FakeKernelPort {
        requests: Mutex<Vec<Value>>,
        answer: Value,
    }

    impl KernelCommandPort for FakeKernelPort {
        fn command(
            &self,
            kind: KernelCommandKind,
            input: Value,
            idempotency_key: Option<String>,
        ) -> Result<Value> {
            assert_eq!(input["type"], kind.name());
            self.requests.lock().unwrap().push(json!({
                "command": input, "idempotency_key": idempotency_key
            }));
            Ok(self.answer.clone())
        }

        fn query(&self, input: Value, wait: bool) -> Result<Value> {
            self.requests
                .lock()
                .unwrap()
                .push(json!({"query": input, "wait": wait}));
            Ok(self.answer.clone())
        }
    }

    fn arguments(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn embedded_description_is_one_json_line_without_a_kernel() {
        let mut output = Vec::new();
        assert_eq!(describe(&arguments(&["--json"]), &mut output).unwrap(), 0);
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            serde_json::from_str::<Value>(SPEC).unwrap()
        );
    }

    #[test]
    fn main_dispatch_keeps_machine_errors_out_of_the_human_error_path() {
        for (words, code) in [
            (vec!["describe", "--json"], 0),
            (vec!["describe"], 1),
            (vec!["kernel", "unknown", "unknown", "--json"], 1),
        ] {
            let mut output = Vec::new();
            assert_eq!(
                super::super::run_with_args(words, &mut output).unwrap(),
                Some(code)
            );
            assert_eq!(
                String::from_utf8(output.clone()).unwrap().lines().count(),
                1
            );
            serde_json::from_slice::<Value>(&output).unwrap();
        }
    }

    #[test]
    fn invalid_arguments_are_one_json_line_with_exit_one() {
        for words in [
            vec![
                "thread",
                "submit_prompt",
                "--thread-id",
                "t",
                "--prompt",
                "  ",
                "--json",
            ],
            vec!["thread", "submit_prompt", "--thread-id", "t", "--json"],
            vec!["planning", "reset_planning", "--target", "invalid"],
            vec!["auto_follow", "set_auto_follow_max_turns", "--value", "-1"],
            vec![
                "auto_follow",
                "set_auto_follow_max_turns",
                "--value",
                "18446744073709551616",
            ],
            vec!["query", "history", "--limit", "4294967296"],
            vec!["query", "history", "--limit", "-1"],
            vec!["session", "load_sessions", "--unknown", "x"],
            vec!["thread", "load_sessions"],
            vec!["operation", "not-an-integer"],
            vec!["history"],
            vec!["query", "sessions", "--wait"],
            vec!["session", "load_sessions", "--json", "--json"],
            vec!["session", "load_sessions", "--type", "stop_all"],
            vec!["session", "load_sessions", "--idempotency-key", " "],
        ] {
            let mut output = Vec::new();
            assert_eq!(
                run(&arguments(&words), &mut output).unwrap(),
                1,
                "{words:?}"
            );
            let text = String::from_utf8(output).unwrap();
            assert_eq!(text.lines().count(), 1, "{words:?}");
            assert!(serde_json::from_str::<Value>(&text).unwrap()["error"]["message"].is_string());
        }
    }

    #[test]
    fn schema_owns_the_command_names_and_scopes() {
        let spec: Value = serde_json::from_str(SPEC).unwrap();
        for entry in spec["commands"].as_array().unwrap() {
            let mut words = vec![
                entry["scope"].as_str().unwrap().to_string(),
                entry["name"].as_str().unwrap().to_string(),
            ];
            for name in entry["input"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| *name != "type")
            {
                let property = resolve(&entry["input"]["properties"][name], &spec).unwrap();
                let value = if let Some(values) = property["enum"].as_array() {
                    values[0].as_str().unwrap().to_string()
                } else if property["type"] == "string" {
                    "test-value".into()
                } else {
                    "1".into()
                };
                words.extend([format!("--{name}"), value]);
            }
            let parsed = parse(&words.iter().map(OsString::from).collect::<Vec<_>>()).unwrap();
            let Request::Command(request) = parsed.request else {
                panic!("expected command");
            };
            assert_eq!(
                serde_json::to_value(request.command).unwrap()["type"],
                entry["name"]
            );
        }
    }

    #[test]
    fn a_failed_command_prints_the_error_code_and_the_key_to_send_again() {
        // The command may have run. The same key makes the second send safe.
        let failing = |_: Invocation| -> Result<Value> {
            Err(KernelCallError {
                code: -32013,
                message: "the kernel did not answer in time".to_string(),
            }
            .into())
        };
        let mut output = Vec::new();
        let words = arguments(&["thread", "stop_all", "--idempotency-key", "caller-key"]);
        assert_eq!(run_with(&words, failing, &mut output).unwrap(), 1);
        let printed: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(printed["error"]["code"], -32013);
        assert_eq!(printed["idempotency_key"], "caller-key");

        // A generated key is printed too. A query has no key, and an error without a code prints none.
        let mut output = Vec::new();
        let words = arguments(&["thread", "stop_all"]);
        assert_eq!(run_with(&words, failing, &mut output).unwrap(), 1);
        let printed: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(printed["idempotency_key"].as_str().unwrap().len(), 32);
        let mut output = Vec::new();
        let lost = |_: Invocation| -> Result<Value> { bail!("connection lost") };
        let words = arguments(&["query", "planning"]);
        assert_eq!(run_with(&words, lost, &mut output).unwrap(), 1);
        let printed: Value = serde_json::from_slice(&output).unwrap();
        assert!(printed.get("idempotency_key").is_none());
        assert!(printed["error"].get("code").is_none());
    }

    #[test]
    fn caller_keys_are_preserved_and_generated_keys_are_per_invocation() {
        let words = arguments(&[
            "session",
            "load_sessions",
            "--idempotency-key",
            "caller-key",
        ]);
        let Request::Command(request) = parse(&words).unwrap().request else {
            panic!("expected command");
        };
        assert_eq!(request.idempotency_key.as_deref(), Some("caller-key"));
        let words = arguments(&["session", "load_sessions"]);
        let Request::Command(first) = parse(&words).unwrap().request else {
            panic!("expected command");
        };
        let Request::Command(second) = parse(&words).unwrap().request else {
            panic!("expected command");
        };
        assert_eq!(first.idempotency_key.as_ref().unwrap().len(), 32);
        assert_ne!(first.idempotency_key, second.idempotency_key);
    }

    #[test]
    fn typed_queries_preserve_cursor_operation_and_wait() {
        let parsed = parse(&arguments(&[
            "history", "--after", "42", "--limit", "5", "--wait", "--json",
        ]))
        .unwrap();
        assert!(parsed.wait);
        let Request::Query(query) = parsed.request else {
            panic!("expected query");
        };
        assert_eq!(query.history_range(), Some((42, 5)));
        let parsed = parse(&arguments(&["operation", "42", "--wait"])).unwrap();
        assert!(parsed.wait);
        let Request::Query(query) = parsed.request else {
            panic!("expected query");
        };
        assert_eq!(query.operation_id(), Some(42));
    }

    #[test]
    fn receipt_rejections_and_unreachable_results_have_distinct_exit_codes() {
        for (value, expected) in [
            (json!({"processed":true}), 0),
            (
                json!({"processed":true,"rejection":{"reason":"busy","next":[{"step":"stop"}]}}),
                2,
            ),
            (json!({"type":"operation","state":"rejected"}), 2),
            (json!({"processed":false}), 1),
        ] {
            let mut output = Vec::new();
            assert_eq!(
                render(Ok(value.clone()), None, &mut output).unwrap(),
                expected
            );
            assert_eq!(serde_json::from_slice::<Value>(&output).unwrap(), value);
        }
    }

    #[test]
    fn commands_use_the_application_port_and_preserve_the_kernel_receipt() {
        for (receipt, expected_code) in [
            (
                json!({"processed":true,"operation":{"op_id":7,"state":"accepted","next":{"query":"history","after":7}}}),
                0,
            ),
            (
                json!({"processed":true,"rejection":{"reason":"busy","message":"busy","next":[{"step":"wait_for","event":"thread_activity"},{"step":"resend"}]}}),
                2,
            ),
        ] {
            let port = FakeKernelPort {
                requests: Mutex::new(Vec::new()),
                answer: receipt.clone(),
            };
            let invocation = parse(&arguments(&[
                "thread",
                "submit_prompt",
                "--thread-id",
                "thread-1",
                "--prompt",
                "a prompt\nwith two lines",
                "--idempotency-key",
                "explicit-key",
                "--json",
            ]))
            .unwrap();
            let mut output = Vec::new();
            assert_eq!(
                render(execute_with_port(invocation, &port), None, &mut output).unwrap(),
                expected_code
            );
            assert_eq!(serde_json::from_slice::<Value>(&output).unwrap(), receipt);
            assert_eq!(String::from_utf8(output).unwrap().lines().count(), 1);
            assert_eq!(
                *port.requests.lock().unwrap(),
                vec![json!({
                    "command":{"type":"submit_prompt","thread_id":"thread-1","prompt":"a prompt\nwith two lines"},
                    "idempotency_key":"explicit-key"
                })]
            );
        }
    }

    #[test]
    fn wait_is_forwarded_to_the_application_port_not_implemented_by_cli_polling() {
        let port = FakeKernelPort {
            requests: Mutex::new(Vec::new()),
            answer: json!({"type":"operation","op_id":7,"state":"done","records":[]}),
        };
        let invocation = parse(&arguments(&["operation", "7", "--wait"])).unwrap();
        let mut output = Vec::new();
        assert_eq!(
            render(execute_with_port(invocation, &port), None, &mut output).unwrap(),
            0
        );
        assert_eq!(
            *port.requests.lock().unwrap(),
            vec![json!({
                "query":{"type":"operation","op_id":7}, "wait":true
            })]
        );
    }
}
