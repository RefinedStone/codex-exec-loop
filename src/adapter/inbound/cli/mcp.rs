//! `akra mcp` 명령이다. 사람과 에이전트가 같은 명령으로 MCP hub 를 쓴다. 결과는 모두 JSON 이다.
//! 로그인 주소는 stdout JSON 이 아니라 stderr 에 먼저 보여서, 사용자가 기다리는 동안 열 수 있게 한다.

use std::ffi::OsString;
use std::io::{Read, Write};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::application::port::inbound::mcp_hub_port::McpHubPort;
use crate::domain::mcp_hub::{McpHubServerSpec, McpHubSharedSkillAction};

use super::usage::MCP_USAGE;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum McpCliCommand {
    Servers,
    Add(McpHubServerSpec),
    Remove(String),
    Login(String),
    Search(String),
    Describe {
        server: String,
        tool: String,
    },
    Call {
        server: String,
        tool: String,
        arguments: CallArguments,
    },
    Skill(McpHubSharedSkillAction),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum CallArguments {
    Empty,
    Inline(String),
    Stdin,
}

pub(super) fn parse(args: &[OsString]) -> Result<McpCliCommand> {
    let args = args
        .iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_string)
                .context("akra mcp arguments must be valid UTF-8")
        })
        .collect::<Result<Vec<_>>>()?;
    let words = args.iter().map(String::as_str).collect::<Vec<_>>();
    let invalid = |detail: String| anyhow::Error::msg(detail);
    Ok(match words.as_slice() {
        ["servers"] => McpCliCommand::Servers,
        ["add", name, "--url", url] => {
            McpCliCommand::Add(McpHubServerSpec::http(*name, *url).map_err(invalid)?)
        }
        ["add", name, "--", command, rest @ ..] => McpCliCommand::Add(
            McpHubServerSpec::stdio(
                *name,
                *command,
                rest.iter().map(|arg| arg.to_string()).collect(),
            )
            .map_err(invalid)?,
        ),
        ["remove", name] => McpCliCommand::Remove(name.to_string()),
        ["login", name] => McpCliCommand::Login(name.to_string()),
        ["search", query @ ..] => McpCliCommand::Search(query.join(" ")),
        ["describe", server, tool] => McpCliCommand::Describe {
            server: server.to_string(),
            tool: tool.to_string(),
        },
        ["call", server, tool] => call(server, tool, CallArguments::Empty),
        ["call", server, tool, "-"] => call(server, tool, CallArguments::Stdin),
        ["call", server, tool, json] => call(server, tool, CallArguments::Inline(json.to_string())),
        ["skill", "status"] => McpCliCommand::Skill(McpHubSharedSkillAction::Status),
        ["skill", "install"] => McpCliCommand::Skill(McpHubSharedSkillAction::Install),
        ["skill", "remove"] => McpCliCommand::Skill(McpHubSharedSkillAction::Remove),
        _ => bail!("{MCP_USAGE}"),
    })
}

fn call(server: &str, tool: &str, arguments: CallArguments) -> McpCliCommand {
    McpCliCommand::Call {
        server: server.to_string(),
        tool: tool.to_string(),
        arguments,
    }
}

pub(super) fn run(
    command: McpCliCommand,
    hub: &dyn McpHubPort,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<i32> {
    let output = match command {
        McpCliCommand::Servers => json!({ "servers": hub.servers()? }),
        McpCliCommand::Add(spec) => {
            let name = spec.name.clone();
            hub.add_server(spec)?;
            json!({ "added": name, "next": format!("akra mcp login {name}") })
        }
        McpCliCommand::Remove(name) => {
            hub.remove_server(&name)?;
            json!({ "removed": name })
        }
        McpCliCommand::Login(name) => {
            hub.login(&name, &mut |url| {
                // 쓰기 실패는 로그인을 막지 않는다. 주소는 JSON 결과에도 남지 않으므로 여기서만 보인다.
                let _ = writeln!(stderr, "Open this URL to log in to `{name}`:\n{url}");
                let _ = stderr.flush();
            })?;
            json!({ "loggedIn": name })
        }
        McpCliCommand::Search(query) => json!({ "tools": hub.search(&query)? }),
        McpCliCommand::Describe { server, tool } => {
            serde_json::to_value(hub.describe(&server, &tool)?)?
        }
        McpCliCommand::Call {
            server,
            tool,
            arguments,
        } => hub.call(&server, &tool, parse_arguments(arguments, stdin)?)?,
        McpCliCommand::Skill(action) => serde_json::to_value(hub.shared_skill(action)?)?,
    };
    writeln!(stdout, "{}", serde_json::to_string_pretty(&output)?)?;
    let failed_call = output.get("isError").and_then(Value::as_bool) == Some(true);
    Ok(if failed_call { 1 } else { 0 })
}

fn parse_arguments(arguments: CallArguments, stdin: &mut dyn Read) -> Result<Value> {
    let text = match arguments {
        CallArguments::Empty => return Ok(json!({})),
        CallArguments::Inline(text) => text,
        CallArguments::Stdin => {
            let mut text = String::new();
            stdin
                .read_to_string(&mut text)
                .context("failed to read tool arguments from stdin")?;
            text
        }
    };
    // Windows PowerShell 5.1 은 파이프 입력 앞에 BOM 을 붙인다.
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .context("tool arguments must be a JSON object")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::mcp_hub::{
        McpHubServerSummary, McpHubSharedSkill, McpHubToolDetail, McpHubToolSummary,
    };
    use std::sync::Mutex;

    fn parse_words(words: &[&str]) -> Result<McpCliCommand> {
        parse(&words.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn parses_every_subcommand() {
        assert_eq!(parse_words(&["servers"]).unwrap(), McpCliCommand::Servers);
        assert_eq!(
            parse_words(&["add", "notion", "--url", "https://mcp.notion.com/mcp"]).unwrap(),
            McpCliCommand::Add(
                McpHubServerSpec::http("notion", "https://mcp.notion.com/mcp").unwrap()
            )
        );
        assert_eq!(
            parse_words(&["add", "local", "--", "node", "server.js", "--flag"]).unwrap(),
            McpCliCommand::Add(
                McpHubServerSpec::stdio("local", "node", vec!["server.js".into(), "--flag".into()])
                    .unwrap()
            )
        );
        assert_eq!(
            parse_words(&["search", "notion", "page"]).unwrap(),
            McpCliCommand::Search("notion page".into())
        );
        assert_eq!(
            parse_words(&["search"]).unwrap(),
            McpCliCommand::Search(String::new())
        );
        assert_eq!(
            parse_words(&["call", "notion", "search", "-"]).unwrap(),
            call("notion", "search", CallArguments::Stdin)
        );
        assert!(parse_words(&["add", "x", "--url", "http://example.com"]).is_err());
        assert!(parse_words(&["call", "notion"]).is_err());
        assert!(parse_words(&[]).is_err());
    }

    #[derive(Default)]
    struct FakeHub {
        calls: Mutex<Vec<Value>>,
    }

    impl McpHubPort for FakeHub {
        fn servers(&self) -> Result<Vec<McpHubServerSummary>> {
            Ok(vec![])
        }
        fn add_server(&self, _spec: McpHubServerSpec) -> Result<()> {
            Ok(())
        }
        fn remove_server(&self, _name: &str) -> Result<()> {
            Ok(())
        }
        fn login(&self, _name: &str, on_url: &mut dyn FnMut(&str)) -> Result<()> {
            on_url("https://auth.example/authorize");
            Ok(())
        }
        fn search(&self, _query: &str) -> Result<Vec<McpHubToolSummary>> {
            Ok(vec![])
        }
        fn describe(&self, _server: &str, _tool: &str) -> Result<McpHubToolDetail> {
            bail!("unused")
        }
        fn call(&self, _server: &str, _tool: &str, arguments: Value) -> Result<Value> {
            self.calls.lock().unwrap().push(arguments.clone());
            Ok(json!({ "content": [], "isError": arguments.get("fail").is_some() }))
        }
        fn shared_skill(&self, action: McpHubSharedSkillAction) -> Result<McpHubSharedSkill> {
            Ok(McpHubSharedSkill {
                installed: action == McpHubSharedSkillAction::Install,
                path: "/codex/skills/akra-mcp-hub/SKILL.md".into(),
            })
        }
    }

    #[test]
    fn skill_subcommands_report_the_shared_skill_as_json() {
        let hub = FakeHub::default();
        let (code, stdout, _) = run_words(&hub, &["skill", "install"], "");
        let output: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(code, 0);
        assert_eq!(output["installed"], true);
        assert_eq!(output["path"], "/codex/skills/akra-mcp-hub/SKILL.md");
        let (_, stdout, _) = run_words(&hub, &["skill", "remove"], "");
        assert_eq!(
            serde_json::from_str::<Value>(&stdout).unwrap()["installed"],
            false
        );
        assert!(parse_words(&["skill"]).is_err());
    }

    fn run_words(hub: &FakeHub, words: &[&str], input: &str) -> (i32, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(
            parse_words(words).unwrap(),
            hub,
            &mut input.as_bytes(),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        (
            code,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn call_reads_arguments_inline_or_from_stdin_and_reports_tool_errors() {
        let hub = FakeHub::default();
        assert_eq!(run_words(&hub, &["call", "s", "t", r#"{"a":1}"#], "").0, 0);
        assert_eq!(run_words(&hub, &["call", "s", "t", "-"], r#"{"b":2}"#).0, 0);
        assert_eq!(run_words(&hub, &["call", "s", "t"], "").0, 0);
        assert_eq!(
            run_words(&hub, &["call", "s", "t", "-"], "\u{feff}{\"c\":3}").0,
            0
        );
        assert_eq!(
            run_words(&hub, &["call", "s", "t", r#"{"fail":true}"#], "").0,
            1
        );
        let calls = hub.calls.lock().unwrap();
        assert_eq!(calls[0]["a"], 1);
        assert_eq!(calls[1]["b"], 2);
        assert_eq!(calls[2], json!({}));
        assert_eq!(calls[3]["c"], 3);
    }

    #[test]
    fn login_shows_the_url_on_stderr_and_json_on_stdout() {
        let (code, stdout, stderr) = run_words(&FakeHub::default(), &["login", "notion"], "");
        assert_eq!(code, 0);
        assert!(stderr.contains("https://auth.example/authorize"));
        let output: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(output["loggedIn"], "notion");
    }

    #[test]
    fn invalid_json_arguments_fail_before_the_call() {
        let hub = FakeHub::default();
        let result = run(
            parse_words(&["call", "s", "t", "{oops"]).unwrap(),
            &hub,
            &mut "".as_bytes(),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        assert!(result.is_err());
        assert!(hub.calls.lock().unwrap().is_empty());
    }
}
