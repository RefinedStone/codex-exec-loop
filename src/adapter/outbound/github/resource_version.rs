//! GitHub 문서(PR, issue)의 지금 버전을 `gh` 로 읽는다 (plan 22 2단계). 버전은 그 문서의 `updated_at` 이다.
//! 댓글, 라벨, 상태, PR 의 새 commit 이 이 값을 바꾼다. 모델을 부르지 않고, 문서의 내용은 읽지 않는다.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::external_resource_version_port::ExternalResourceVersionPort;
use crate::domain::work_context::ExternalResourceRef;
use crate::subprocess;

const GITHUB_KIND: &str = "github";
/// 문서 하나를 묻는 최대 시간이다. 넘으면 그 문서는 닿지 않은 것으로 둔다.
const VERSION_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// `updated_at` 은 `2026-11-11T00:00:00Z` 모양이다. 이보다 긴 답은 버전이 아니다.
const MAX_VERSION_CHARS: usize = 40;
/// 사용자의 GitHub 토큰을 찾는 순서이다. 리뷰 확인(`review_poller`)과 같다. 환경을 비운 뒤 처음 찾은 값만
/// `GH_TOKEN` 으로 `gh` 에 넘긴다. 없으면 `gh` 가 저장한 로그인을 쓴다.
const GITHUB_TOKEN_ENVIRONMENT: [&str; 3] = ["AKRA_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];

pub struct GithubResourceVersionAdapter {
    /// `gh` 를 찾고 실행할 때 기준이 되는 폴더이다. 저장소가 아닌 Akra 의 폴더를 준다.
    working_directory: PathBuf,
}

impl GithubResourceVersionAdapter {
    pub fn new(working_directory: PathBuf) -> Self {
        Self { working_directory }
    }

    fn gh_command(&self, target: &GithubIssueTarget) -> Result<Command> {
        let gh = crate::trusted_executable::resolve_native_from_current_path(
            "gh",
            &self.working_directory,
        )
        .context("gh is not available")?;
        let mut command = Command::new(gh);
        crate::trusted_executable::configure_credential_command_environment(
            &mut command,
            &self.working_directory,
            true,
        )?;
        let token = GITHUB_TOKEN_ENVIRONMENT
            .into_iter()
            .find_map(|name| std::env::var_os(name).filter(|token| !token.is_empty()));
        if let Some(token) = token {
            command.env("GH_TOKEN", token);
        }
        command
            .args(["api", &target.api_path(), "--jq", ".updated_at"])
            .current_dir(crate::trusted_executable::neutral_user_config_directory(
                &self.working_directory,
            ))
            .stdin(Stdio::null())
            .env("GH_PROMPT_DISABLED", "1");
        Ok(command)
    }
}

impl ExternalResourceVersionPort for GithubResourceVersionAdapter {
    fn kind(&self) -> &'static str {
        GITHUB_KIND
    }

    fn current_version(&self, resource: &ExternalResourceRef) -> Result<String> {
        let target = GithubIssueTarget::parse(&resource.key)?;
        let mut command = self.gh_command(&target)?;
        let output = subprocess::command_output_with_timeout(
            &mut command,
            "gh api issue",
            VERSION_REQUEST_TIMEOUT,
        )
        .context("gh did not answer")?;
        if !output.status.success() {
            // stderr 에는 저장소 이름과 계정이 들어갈 수 있다. 남기지 않는다.
            bail!("gh could not read the document")
        }
        version_from_output(&output.stdout)
    }
}

/// 장부의 GitHub 키 `owner/repo#number` 이다. PR 도 issue API 로 읽는다.
#[derive(Debug, PartialEq, Eq)]
struct GithubIssueTarget<'a> {
    owner: &'a str,
    repository: &'a str,
    number: &'a str,
}

impl<'a> GithubIssueTarget<'a> {
    /// 키의 각 부분은 `gh` 인자가 된다. GitHub 이름에 쓸 수 있는 글자만 받는다.
    fn parse(key: &'a str) -> Result<Self> {
        let parts = key
            .split_once('#')
            .and_then(|(name, number)| Some((name.split_once('/')?, number)));
        let Some(((owner, repository), number)) = parts else {
            bail!("the GitHub document key is not `owner/repo#number`")
        };
        let name_is_valid = |name: &str, extra: &[char]| {
            !name.is_empty()
                && name.len() <= 100
                && !name.starts_with(['.', '-'])
                && name.chars().all(|character| {
                    character.is_ascii_alphanumeric() || extra.contains(&character)
                })
        };
        let number_is_valid =
            !number.is_empty() && number.len() <= 10 && number.bytes().all(|b| b.is_ascii_digit());
        if !name_is_valid(owner, &['-'])
            || !name_is_valid(repository, &['-', '_', '.'])
            || !number_is_valid
        {
            bail!("the GitHub document key is not `owner/repo#number`")
        }
        Ok(Self {
            owner,
            repository,
            number,
        })
    }

    fn api_path(&self) -> String {
        format!(
            "repos/{}/{}/issues/{}",
            self.owner, self.repository, self.number
        )
    }
}

/// `gh --jq .updated_at` 의 답에서 버전을 읽는다. 시각 모양이 아니면 버전으로 쓰지 않는다.
fn version_from_output(stdout: &[u8]) -> Result<String> {
    let version = String::from_utf8_lossy(stdout).trim().to_string();
    let looks_like_a_time = !version.is_empty()
        && version.chars().count() <= MAX_VERSION_CHARS
        && version
            .chars()
            .all(|character| character.is_ascii_digit() || "TZ:.+-".contains(character));
    if !looks_like_a_time {
        bail!("gh did not return an update time")
    }
    Ok(version)
}

/// 이 제공자가 쓰는 폴더가 저장소 안이 아닌지는 조립이 정한다. 시험은 경로만 본다.
#[cfg(test)]
fn working_directory(adapter: &GithubResourceVersionAdapter) -> &Path {
    &adapter.working_directory
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_keys_become_issue_api_paths() {
        let target = GithubIssueTarget::parse("refinedstone/codex-exec-loop#2230").unwrap();
        assert_eq!(
            target.api_path(),
            "repos/refinedstone/codex-exec-loop/issues/2230"
        );
        assert_eq!(
            GithubIssueTarget::parse("a/b.c_d#1").unwrap().api_path(),
            "repos/a/b.c_d/issues/1"
        );
    }

    #[test]
    fn keys_that_are_not_a_repository_and_a_number_are_rejected() {
        // The parts become `gh` arguments and a URL path. Nothing else may pass.
        for key in [
            "a/b",
            "a#1",
            "/b#1",
            "a/#1",
            "a/b#",
            "a/b#1x",
            "a/b#12345678901",
            "a/b/c#1",
            "a/../b#1",
            "a/.b#1",
            "-a/b#1",
            "a/b?x=1#1",
            "a/b --hostname evil.example#1",
            "a/b#1 --jq .",
            "",
        ] {
            assert!(
                GithubIssueTarget::parse(key).is_err(),
                "{key:?} must be rejected"
            );
        }
    }

    #[test]
    fn only_an_update_time_is_a_version() {
        assert_eq!(
            version_from_output(b"2026-11-11T05:27:29Z\n").unwrap(),
            "2026-11-11T05:27:29Z"
        );
        for output in [
            &b""[..],
            b"null\n",
            b"{\"message\":\"Not Found\"}",
            b"2026-11-11T05:27:29Z and a very long tail that is not a time at all",
        ] {
            assert!(version_from_output(output).is_err());
        }
    }

    #[test]
    fn the_adapter_reports_the_github_kind_and_keeps_its_folder() {
        let adapter = GithubResourceVersionAdapter::new(PathBuf::from("/akra-home"));
        assert_eq!(adapter.kind(), "github");
        assert_eq!(working_directory(&adapter), Path::new("/akra-home"));
    }
}
