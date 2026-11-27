//! Read-only agent selection CLI; explicit operator preference changes are separate.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

pub(super) const USAGE: &str = "Usage: akra context <list|show|changes> [--workspace <path>] [--task-id <id>] [--path <relative-path>]... [--kind <kind> --key <key>] --json\n       akra context refresh [--workspace <path>] [--task-id <id>|--path <relative-path>] --json\n       akra context preference <pin|exclude|automatic> --url <url> [--workspace <path>] --confirm true --json";

#[derive(Debug)]
struct Invocation {
    method: &'static str,
    params: Value,
    output: Output,
}
#[derive(Debug)]
enum Output {
    List,
    Show(String, String),
    Changes,
    Raw,
}

pub(super) fn run(args: &[OsString], stdout: &mut impl Write) -> Result<i32> {
    run_with(
        args,
        crate::composition::production::work_context_cli_request,
        stdout,
    )
}

fn run_with(
    args: &[OsString],
    request: impl FnOnce(&str, Value) -> Result<Value>,
    stdout: &mut impl Write,
) -> Result<i32> {
    let result = parse(args).and_then(|invocation| {
        let answer = request(invocation.method, invocation.params)?;
        project(answer, invocation.output)
    });
    let (code, answer) = match result {
        Ok(answer) => (0, answer),
        Err(error) => (
            1,
            json!({"error":{"code":"context_request_failed","message":format!("{error:#}")}}),
        ),
    };
    serde_json::to_writer(&mut *stdout, &answer)?;
    writeln!(stdout)?;
    Ok(code)
}

fn parse(args: &[OsString]) -> Result<Invocation> {
    let args = args
        .iter()
        .map(|a| a.to_str().context("context arguments must be UTF-8"))
        .collect::<Result<Vec<_>>>()?;
    let action = *args.first().context(USAGE)?;
    let mut offset = 1;
    let preference = if action == "preference" {
        offset += 1;
        let choice = *args.get(1).context(USAGE)?;
        ensure!(
            matches!(choice, "pin" | "exclude" | "automatic"),
            "unknown context preference"
        );
        Some(choice)
    } else {
        None
    };
    let mut options = std::collections::BTreeMap::new();
    let mut paths = Vec::new();
    while offset < args.len() {
        let flag = args[offset];
        offset += 1;
        if flag == "--json" {
            ensure!(
                options.insert(flag, "true").is_none(),
                "duplicate context option"
            );
            continue;
        }
        ensure!(
            matches!(
                flag,
                "--workspace" | "--task-id" | "--path" | "--kind" | "--key" | "--url" | "--confirm"
            ),
            "unknown context option"
        );
        let value = *args
            .get(offset)
            .context("context option requires a value")?;
        ensure!(
            !value.is_empty() && !value.starts_with("--"),
            "context option requires a value"
        );
        offset += 1;
        if flag == "--path" {
            paths.push(value);
        } else {
            ensure!(
                options.insert(flag, value).is_none(),
                "duplicate context option"
            );
        }
    }
    ensure!(
        options.remove("--json").is_some(),
        "context commands require --json"
    );
    let mut cwd = std::env::current_dir()?;
    if let Some(workspace) = options.remove("--workspace") {
        cwd = cwd.join(PathBuf::from(workspace));
    }
    let cwd = cwd.to_str().context("context workspace must be UTF-8")?;
    let task_id = options.remove("--task-id");
    let (method, params, output) = match action {
        "list" | "changes" | "show" => {
            let output = match action {
                "list" => Output::List,
                "changes" => Output::Changes,
                _ => Output::Show(
                    options
                        .remove("--kind")
                        .context("show requires --kind")?
                        .to_owned(),
                    options
                        .remove("--key")
                        .context("show requires --key")?
                        .to_owned(),
                ),
            };
            (
                "akra/context/select",
                json!({"cwd":cwd,"taskId":task_id,"paths":paths}),
                output,
            )
        }
        "refresh" => {
            ensure!(
                paths.len() <= 1 && (paths.is_empty() || task_id.is_none()),
                "refresh accepts one exact scope"
            );
            let scope = if let Some(task_id) = task_id {
                json!({"kind":"task","taskId":task_id})
            } else if let Some(path) = paths.first() {
                json!({"kind":"path","path":path})
            } else {
                json!({"kind":"repository"})
            };
            (
                "akra/context/refreshScope",
                json!({"cwd":cwd,"scope":scope}),
                Output::Raw,
            )
        }
        "preference" => {
            ensure!(
                task_id.is_none() && paths.is_empty(),
                "preferences are repository-wide"
            );
            ensure!(
                options.remove("--confirm") == Some("true"),
                "preferences require --confirm true"
            );
            let url = options
                .remove("--url")
                .context("preference requires --url")?;
            (
                "akra/context/setPreference",
                json!({"cwd":cwd,"url":url,"preference":preference}),
                Output::Raw,
            )
        }
        _ => bail!("{USAGE}"),
    };
    ensure!(
        options.is_empty(),
        "context option is not valid for this command"
    );
    Ok(Invocation {
        method,
        params,
        output,
    })
}

fn project(mut answer: Value, output: Output) -> Result<Value> {
    if matches!(output, Output::Raw) {
        return Ok(answer);
    }
    let resources = answer
        .get_mut("resources")
        .and_then(Value::as_array_mut)
        .context("invalid context Gateway response")?;
    ensure!(
        resources.iter().all(|r| r["kind"].is_string()
            && r["key"].is_string()
            && r["url"].is_string()
            && r["reason"].is_string()
            && r["selected"].is_boolean()
            && r["changed"].is_boolean()),
        "invalid context selection response"
    );
    match output {
        Output::List => Ok(answer),
        Output::Changes => {
            resources.retain(|r| r["selected"] == true && r["changed"] == true);
            Ok(answer)
        }
        Output::Show(kind, key) => {
            let mut matching = resources
                .iter()
                .filter(|r| r["kind"] == kind && r["key"] == key);
            let resource = matching
                .next()
                .context("document not in this context selection")?;
            ensure!(matching.next().is_none(), "ambiguous context document");
            Ok(json!({"resource":resource}))
        }
        Output::Raw => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(|word| OsString::from(*word)).collect()
    }
    #[test]
    fn context_cli_parses_workspace_and_filters_without_configuration() {
        let invocation = parse(&args(&[
            "list",
            "--workspace",
            ".",
            "--task-id",
            "T-1",
            "--path",
            "src/lib.rs",
            "--path",
            "docs/a.md",
            "--json",
        ]))
        .unwrap();
        assert_eq!(invocation.method, "akra/context/select");
        assert_eq!(invocation.params["taskId"], "T-1");
        assert_eq!(
            invocation.params["paths"],
            json!(["src/lib.rs", "docs/a.md"])
        );
        assert!(PathBuf::from(invocation.params["cwd"].as_str().unwrap()).is_absolute());
        for invalid in [
            vec!["list", "--task-id", "T-1", "--task-id", "T-2", "--json"],
            vec!["show", "--kind", "jira", "--json"],
            vec![
                "preference",
                "pin",
                "--url",
                "https://example.test",
                "--json",
            ],
            vec!["refresh", "--task-id", "T-1", "--path", "src", "--json"],
        ] {
            assert!(parse(&args(&invalid)).is_err());
        }
    }
    #[test]
    fn context_cli_projects_only_selected_changed_documents_and_reports_gateway_failure() {
        let a = json!({"kind":"github","key":"a/b#1","url":"https://github.com/a/b/pull/1","selected":true,"reason":"task_link","changed":true});
        let b = json!({"kind":"github","key":"a/b#2","url":"https://github.com/a/b/pull/2","selected":false,"reason":"user_exclude","changed":true});
        let answer = json!({"resources":[a.clone(),b.clone()]});
        assert_eq!(
            project(answer.clone(), Output::Changes).unwrap(),
            json!({"resources":[a]})
        );
        assert_eq!(
            project(answer, Output::Show("github".into(), "a/b#2".into())).unwrap(),
            json!({"resource":b})
        );
        let mut stdout = Vec::new();
        assert_eq!(
            run_with(
                &args(&["list", "--json"]),
                |_, _| bail!("context Gateway unavailable"),
                &mut stdout
            )
            .unwrap(),
            1
        );
        let error: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(error["error"]["code"], "context_request_failed");
    }
}
