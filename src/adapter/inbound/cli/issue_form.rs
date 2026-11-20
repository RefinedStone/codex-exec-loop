use std::ffi::{OsStr, OsString};
use std::io::Write;

use anyhow::{Result, bail};

use crate::application::port::inbound::issue_form_install_port::{
    IssueFormInstallRequest, IssueFormInstallResponse, IssueFormInstallStatus,
};
use crate::composition::production;

pub(super) const USAGE: &str =
    "Usage: akra issue-form install --cwd <workspace> --base <branch> [--json]";

enum Invocation {
    Help,
    Install {
        request: IssueFormInstallRequest,
        json: bool,
    },
}

fn parse(args: &[OsString]) -> Result<Invocation> {
    if args.is_empty() || (args.len() == 1 && matches!(args[0].to_str(), Some("--help" | "-h"))) {
        return Ok(Invocation::Help);
    }
    if args[0] != OsStr::new("install") {
        bail!("{USAGE}");
    }
    if args.len() == 2 && matches!(args[1].to_str(), Some("--help" | "-h")) {
        return Ok(Invocation::Help);
    }
    let mut workspace_dir = None;
    let mut base_branch = None;
    let mut json = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].to_str() {
            Some("--json") if !json => {
                json = true;
                index += 1;
            }
            Some(flag @ ("--cwd" | "--base")) => {
                let Some(value) = args.get(index + 1).and_then(|value| value.to_str()) else {
                    bail!("{USAGE}");
                };
                if value.is_empty() || value.starts_with("--") {
                    bail!("{USAGE}");
                }
                let destination = if flag == "--cwd" {
                    &mut workspace_dir
                } else {
                    &mut base_branch
                };
                if destination.replace(value.to_owned()).is_some() {
                    bail!("{USAGE}");
                }
                index += 2;
            }
            _ => bail!("{USAGE}"),
        }
    }
    let (Some(workspace_dir), Some(base_branch)) = (workspace_dir, base_branch) else {
        bail!("{USAGE}");
    };
    Ok(Invocation::Install {
        request: IssueFormInstallRequest {
            workspace_dir,
            base_branch,
        },
        json,
    })
}

pub(super) fn run(args: &[OsString], stdout: &mut impl Write) -> Result<i32> {
    run_with(args, stdout, |request| {
        production::build_issue_form_install().install(request)
    })
}

fn run_with(
    args: &[OsString],
    stdout: &mut impl Write,
    install: impl FnOnce(&IssueFormInstallRequest) -> Result<IssueFormInstallResponse>,
) -> Result<i32> {
    let Invocation::Install { request, json } = parse(args)? else {
        writeln!(stdout, "{USAGE}")?;
        return Ok(0);
    };
    let response = install(&request)?;
    if json {
        super::reports::render_json_line(stdout, &response)?;
    } else {
        let summary = match response.status {
            IssueFormInstallStatus::AlreadyInstalled => {
                "기준 브랜치에 같은 이슈 양식이 이미 있습니다."
            }
            IssueFormInstallStatus::PullRequestCreated => "이슈 양식 설치 PR을 만들었습니다.",
            IssueFormInstallStatus::PullRequestFound => "기존 이슈 양식 설치 PR을 확인했습니다.",
            IssueFormInstallStatus::NeedsConfirmation => {
                "이전 원격 요청의 결과를 확인해야 합니다. 자동으로 다시 보내지 않았습니다."
            }
            IssueFormInstallStatus::Conflict => "기존 양식이나 브랜치가 달라 설치를 보류했습니다.",
        };
        writeln!(stdout, "{summary}")?;
        if let Some(pull_request) = &response.pull_request {
            writeln!(stdout, "{}", pull_request.url)?;
        }
    }
    Ok(match response.status {
        IssueFormInstallStatus::AlreadyInstalled
        | IssueFormInstallStatus::PullRequestCreated
        | IssueFormInstallStatus::PullRequestFound => 0,
        IssueFormInstallStatus::NeedsConfirmation => 2,
        IssueFormInstallStatus::Conflict => 3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(|value| OsString::from(*value)).collect()
    }

    #[test]
    fn help_and_invalid_invocations_never_initialize_or_call_the_installer() {
        for values in [vec![], vec!["--help"], vec!["install", "--help"]] {
            let mut output = Vec::new();
            assert_eq!(
                run_with(&args(&values), &mut output, |_| panic!(
                    "no installation for help"
                ))
                .unwrap(),
                0
            );
            assert!(String::from_utf8(output).unwrap().contains(USAGE));
        }
        for values in [
            vec!["install"],
            vec!["install", "--cwd", "/repo", "--base"],
            vec![
                "install", "--cwd", "/repo", "--base", "main", "--cwd", "/other",
            ],
            vec!["install", "--cwd", "/repo", "--base", "main", "--unknown"],
            vec!["delete", "--cwd", "/repo", "--base", "main"],
        ] {
            assert!(
                run_with(&args(&values), &mut Vec::new(), |_| panic!(
                    "invalid command must not install"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn explicit_target_is_forwarded_once_and_uncertain_outcomes_have_a_nonzero_json_result() {
        let mut output = Vec::new();
        let exit = run_with(
            &args(&[
                "install",
                "--cwd",
                "/work/my repository",
                "--base",
                "release/next",
                "--json",
            ]),
            &mut output,
            |request| {
                assert_eq!(request.workspace_dir, "/work/my repository");
                assert_eq!(request.base_branch, "release/next");
                Ok(IssueFormInstallResponse {
                    status: IssueFormInstallStatus::NeedsConfirmation,
                    path: ".github/ISSUE_TEMPLATE/akra-work.yml".into(),
                    base_branch: request.base_branch.clone(),
                    head_branch: Some("akra/issue-form/attempt".into()),
                    base_commit_sha: None,
                    commit_sha: None,
                    pull_request: None,
                    reason: None,
                })
            },
        )
        .unwrap();
        assert_eq!(exit, 2);
        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(value["status"], "needs_confirmation");
        assert_eq!(value["base_branch"], "release/next");
        assert!(output.ends_with(b"\n"));
    }
}
