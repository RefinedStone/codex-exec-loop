use crate::composition::production::git_operator as git;
use anyhow::{Context, Result, bail, ensure};
use std::ffi::OsString;
use std::io::{IsTerminal, Write};
mod approval;

pub(crate) const USAGE: &str = "akra git protect <list|review|add PATTERN|remove PATTERN> [--remote NAME] | akra git force-push <REMOTE> <BRANCH>";
pub(crate) fn run(args: &[OsString], stdout: &mut impl Write, desktop: bool) -> Result<i32> {
    let args = args
        .iter()
        .map(|a| a.to_str().context("invalid argument encoding"))
        .collect::<Result<Vec<_>>>()?;
    let cwd = std::env::current_dir()?;
    let cwd = cwd.to_str().context("non-unicode workspace")?;
    ensure!(![crate::application::port::planning_task_tool_contract::PLANNING_TOOL_PARENT_THREAD_ID_ENV,crate::application::port::planning_task_tool_contract::PLANNING_TOOL_PARENT_TURN_ID_ENV,"CODEX_THREAD_ID"].iter().any(|k|std::env::var_os(k).is_some()),"Git protection cannot be changed by an agent turn");
    let interactive =
        desktop || (std::io::stdin().is_terminal() && std::io::stderr().is_terminal());
    match args.as_slice() {
        ["protect", rest @ ..] => {
            let (rest, remote) = match rest {
                [a @ .., "--remote", remote] => (a, *remote),
                _ => (rest, "origin"),
            };
            let (verb, rule) = match rest {
                [v @ ("list" | "review")] => (*v, None),
                [v @ ("add" | "remove"), r] => (*v, Some(*r)),
                _ => bail!(USAGE),
            };
            ensure!(
                interactive || verb == "list",
                "policy changes require an interactive operator"
            );
            let dest = git::destination(cwd, remote)?;
            let shown = serde_json::to_string_pretty(&dest)?;
            if verb != "list" {
                let expected = rule.unwrap_or(remote);
                approval::confirm(
                    &format!("Git 보호 목록: {verb} {}\n{shown}", rule.unwrap_or("")),
                    expected,
                )?;
                git::edit(cwd, &dest, verb, rule)?;
                writeln!(stdout, "보호 설정을 저장했습니다.")?;
            } else {
                writeln!(stdout, "{shown}")?;
            }
        }
        ["force-push", remote, branch] => {
            ensure!(
                interactive,
                "a reviewed lease push requires an interactive operator"
            );
            let plan = git::plan(cwd, remote, branch)?;
            let lost = plan
                .lost
                .lines()
                .map(|l| {
                    l.chars()
                        .filter(|c| !c.is_control())
                        .take(140)
                        .collect::<String>()
                })
                .take(20)
                .collect::<Vec<_>>()
                .join("\n");
            approval::confirm(
                &format!(
                    "강제 push: {}\n목적지: {}\n검토한 원격 SHA: {}\n로컬 SHA: {}\n원격에서 빠지는 커밋 (최대 20):\n{}",
                    plan.destination.url, branch, plan.remote_sha, plan.local_sha, lost
                ),
                branch,
            )?;
            git::execute(cwd, &plan)?;
            writeln!(stdout, "완료")?;
        }
        _ => bail!(USAGE),
    }
    Ok(0)
}
