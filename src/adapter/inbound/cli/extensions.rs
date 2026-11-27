//! Thin machine entrance: server catalog schemas validate the same request sent to Desktop's host.
use crate::domain::kernel_extension::ExtensionCallRejection;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io::Write;

pub(super) const USAGE: &str = "Usage: akra extensions list [--scope local_user|repository] [--workspace <path>] --json\n       akra extensions call <extension> <action> --version <n> --input <json> [--idempotency-key <key>] [--thread-id <verified-thread>] [--scope local_user|repository] --json";
struct Invocation {
    scope: String,
    workspace: String,
    call: Option<Value>,
    thread_id: Option<String>,
}
pub(super) fn run(args: &[OsString], stdout: &mut impl Write) -> Result<i32> {
    let result = parse(args).and_then(|i| {
        crate::composition::kernel_extensions::cli_request(
            &i.scope,
            &i.workspace,
            i.call,
            i.thread_id.as_deref(),
        )
    });
    let (code, value) = match result {
        Ok(value) => (0, value),
        Err(error) => (1, rejection_json(&error)),
    };
    serde_json::to_writer(&mut *stdout, &value)?;
    writeln!(stdout)?;
    Ok(code)
}
fn rejection_json(error: &anyhow::Error) -> Value {
    match error.downcast_ref::<ExtensionCallRejection>() {
        Some(rejection) => json!({"error": rejection}),
        None => json!({"error":{"message":format!("{error:#}")}}),
    }
}
fn parse(args: &[OsString]) -> Result<Invocation> {
    let args = args
        .iter()
        .map(|a| a.to_str().context("extension arguments must be UTF-8"))
        .collect::<Result<Vec<_>>>()?;
    let mut positional = Vec::new();
    let mut flags = std::collections::BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let word = args[index];
        if let Some(flag) = word.strip_prefix("--") {
            ensure!(!flags.contains_key(flag), "duplicate extension option");
            if flag == "json" {
                flags.insert(flag, "true");
            } else {
                ensure!(
                    matches!(
                        flag,
                        "scope"
                            | "workspace"
                            | "version"
                            | "input"
                            | "idempotency-key"
                            | "thread-id"
                    ),
                    "unknown extension option"
                );
                index += 1;
                let value = *args.get(index).context("extension option needs a value")?;
                flags.insert(flag, value);
            }
        } else {
            positional.push(word);
        }
        index += 1;
    }
    let scope = flags
        .get("scope")
        .copied()
        .unwrap_or("local_user")
        .to_string();
    ensure!(
        matches!(scope.as_str(), "local_user" | "repository"),
        "invalid extension scope"
    );
    let workspace = flags
        .get("workspace")
        .map(|p| (*p).to_owned())
        .unwrap_or(std::env::current_dir()?.to_string_lossy().into_owned());
    let thread_id = flags.get("thread-id").map(|value| (*value).to_owned());
    if let Some(thread) = &thread_id {
        ensure!(
            !thread.is_empty() && thread.len() <= 256,
            "invalid thread identifier"
        );
    }
    let call = match positional.as_slice() {
        ["list"] => {
            ensure!(
                !flags.contains_key("input")
                    && !flags.contains_key("version")
                    && !flags.contains_key("idempotency-key")
                    && !flags.contains_key("thread-id"),
                "call options cannot be used with list"
            );
            None
        }
        ["call", extension, action] => {
            let version = flags
                .get("version")
                .copied()
                .unwrap_or("1")
                .parse::<u32>()?;
            let input: Value =
                serde_json::from_str(flags.get("input").context("--input is required")?)?;
            Some(
                json!({"extension":extension,"version":version,"action":action,"input":input,
                "idempotency_key":flags.get("idempotency-key")}),
            )
        }
        _ => bail!("{USAGE}"),
    };
    Ok(Invocation {
        scope,
        workspace,
        call,
        thread_id,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn words(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }
    #[test]
    fn extension_cli_preserves_authoritative_rejection_code_message_and_data() {
        let rejection = ExtensionCallRejection {
            code: -32003,
            message: "extension is disabled".into(),
            data: Some(json!({"reason":"disabled","revision":2})),
        };
        let error = anyhow::Error::new(rejection.clone()).context("transport context");
        assert_eq!(rejection_json(&error), json!({"error":rejection}));
    }
    #[test]
    fn extension_cli_preserves_json_and_rejects_unknown_flags() {
        let parsed = parse(&words(&[
            "call",
            "akra.example",
            "echo",
            "--input",
            "{\"text\":\"hello\"}",
            "--json",
        ]))
        .unwrap();
        assert_eq!(parsed.call.unwrap()["input"], json!({"text":"hello"}));
        assert!(parse(&words(&["list", "--host", "another-user"])).is_err());
        assert!(
            parse(&words(&[
                "list",
                "--scope",
                "local_user",
                "--scope",
                "repository"
            ]))
            .is_err()
        );
    }

    #[test]
    fn extension_cli_keeps_explicit_blog_version_three_in_the_request_body() {
        for (action, input) in [
            ("generate", json!({"commits":["a".repeat(40)]})),
            ("seen", json!({"id":"article"})),
            (
                "revise",
                json!({"id":"article","revision":2,"instruction":"근거를 설명해 주세요"}),
            ),
            ("review", json!({"id":"article","revision":2})),
            ("restore", json!({"id":"article","revision":2})),
            ("export", json!({"id":"article","revision":2})),
        ] {
            let encoded = input.to_string();
            let parsed = parse(&words(&[
                "call",
                "akra.tech-blog",
                action,
                "--version",
                "3",
                "--scope",
                "repository",
                "--workspace",
                "fixture",
                "--input",
                &encoded,
                "--json",
            ]))
            .unwrap();
            assert_eq!(parsed.scope, "repository");
            assert_eq!(parsed.workspace, "fixture");
            assert_eq!(
                parsed.call.unwrap(),
                json!({
                    "extension":"akra.tech-blog","version":3,"action":action,
                    "input":input,"idempotency_key":null
                })
            );
        }
    }

    #[test]
    fn extension_cli_keeps_version_one_default_for_other_extensions() {
        for (extension, action, input) in [
            ("akra.example", "echo", json!({"text":"hello"})),
            ("akra.storage-analysis", "jobs", json!({})),
        ] {
            let encoded = input.to_string();
            let parsed = parse(&words(&[
                "call", extension, action, "--input", &encoded, "--json",
            ]))
            .unwrap();
            assert_eq!(parsed.scope, "local_user");
            assert_eq!(
                parsed.call.unwrap(),
                json!({
                    "extension":extension,"version":1,"action":action,
                    "input":input,"idempotency_key":null
                })
            );
        }
    }
}
