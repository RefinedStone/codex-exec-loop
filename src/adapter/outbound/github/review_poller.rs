/*
GitHub review poller outbound adapter다.

application service는 "현재 branch의 PR을 찾고, 해당 PR의 review 활동을 시간순 domain snapshot으로
받는다"는 port 계약만 안다. 이 파일은 그 계약을 local git repository identity, local GitHub credential,
GitHub REST endpoint, curl 실행, 응답 DTO mapping으로 풀어낸다. GitHub API JSON 구조는 private response
타입에 가두고, 바깥에는 `GithubPullRequestActivitySnapshot`만 노출한다.
*/
use crate::application::port::outbound::github_automation_port::{
    AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, AKRA_GITHUB_PUSH_REMOTE_ENV_VAR,
    parse_github_repository_identity, resolve_github_push_remote_name_strict,
};
use crate::application::port::outbound::github_pr_validation_port::{
    GithubPrValidationError, GithubPrValidationErrorClass, GithubValidationProviderMetadata,
};
use crate::application::port::outbound::github_review_poller_port::GithubReviewPollerPort;
use crate::domain::github_review::{
    GithubPullRequestActivityEvent, GithubPullRequestActivityKind,
    GithubPullRequestActivitySnapshot, GithubPullRequestTarget,
};
use crate::git_subprocess;
use crate::subprocess;
use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
#[cfg(all(test, unix))]
use std::ffi::OsStr;
#[cfg(all(test, unix))]
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
const GITHUB_API_BASE_URL: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const PER_PAGE: usize = 100;
const MAX_PAGINATED_PAGES: usize = 20;
const MAX_PAGINATED_ITEMS: usize = PER_PAGE * MAX_PAGINATED_PAGES;
const MAX_GITHUB_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PAGINATED_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
const ACTIVITY_LOAD_TIMEOUT: Duration = Duration::from_secs(90);
const CURL_CONNECT_TIMEOUT_SECONDS: &str = "10";
const CURL_MAX_TIME_SECONDS: &str = "30";
const CURL_SPAWN_ATTEMPTS: usize = 3;
const CURL_SPAWN_RETRY_DELAY: Duration = Duration::from_millis(10);
const LEGACY_CREDENTIAL_SCAN_ENV: &str = "AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN";
const VALIDATION_HTTP_STATUS_MARKER: &str = "\nAKRA_VALIDATION_HTTP_STATUS:";

#[cfg(unix)]
fn unresolved_curl_executable_path() -> PathBuf {
    PathBuf::from("/__akra_unresolved_curl_executable__")
}

#[cfg(windows)]
fn unresolved_curl_executable_path() -> PathBuf {
    PathBuf::from(r"C:\__akra_unresolved_curl_executable__.exe")
}

#[cfg(unix)]
fn unresolved_gh_executable_path() -> PathBuf {
    PathBuf::from("/__akra_unresolved_gh_executable__")
}

#[cfg(windows)]
fn unresolved_gh_executable_path() -> PathBuf {
    PathBuf::from(r"C:\__akra_unresolved_gh_executable__.exe")
}
// GitHub `head=owner:branch` 같은 query value는 branch slash, colon, and shell-sensitive 문자를 포함할 수 있다.
// endpoint path는 직접 조립하지만 query value는 이 set으로 percent-encode해 GitHub search 조건이 깨지지 않게 한다.
const GITHUB_QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'/')
    .add(b':')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');
pub struct GithubReviewPollerAdapter {
    // 테스트와 production이 같은 request builder를 쓰되, 테스트는 curl path/base URL을 바꿀 수 있게 값으로 둔다.
    curl_path: String,
    curl_resolution_error: Option<String>,
    api_base_url: String,
    user_agent: String,
    // local GitHub credential에서 추출한 token이다. raw credential line은 이 adapter 밖으로 보존하지 않는다.
    token: String,
    subprocess_timeout: Duration,
}

#[derive(Clone, Copy)]
struct GithubActivityLoadBudget {
    deadline: Instant,
}

impl GithubActivityLoadBudget {
    fn new() -> Self {
        Self {
            deadline: Instant::now()
                .checked_add(ACTIVITY_LOAD_TIMEOUT)
                .unwrap_or_else(Instant::now),
        }
    }

    fn remaining(self, operation: &str) -> Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                anyhow!(
                    "GitHub review activity load exceeded its {:?} aggregate deadline before {operation}",
                    ACTIVITY_LOAD_TIMEOUT
                )
            })
    }
}

impl GithubReviewPollerAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new_for_workspace(token, &cwd)
    }

    fn new_for_workspace(token: impl Into<String>, workspace: &Path) -> Self {
        Self::new_for_workspace_with_curl_resolution(
            token,
            crate::trusted_executable::resolve_native_from_current_path("curl", workspace),
        )
    }

    #[cfg(all(test, unix))]
    fn new_for_workspace_with_path(
        token: impl Into<String>,
        workspace: &Path,
        path: &OsStr,
    ) -> Self {
        Self::new_for_workspace_with_curl_resolution(
            token,
            crate::trusted_executable::resolve_native_from_path("curl", path, workspace),
        )
    }

    fn new_for_workspace_with_curl_resolution(
        token: impl Into<String>,
        resolution: Result<PathBuf>,
    ) -> Self {
        let (curl_path, curl_resolution_error) = match resolution {
            Ok(path) => (path.display().to_string(), None),
            Err(error) => (
                unresolved_curl_executable_path().display().to_string(),
                Some(error.to_string()),
            ),
        };
        Self {
            curl_path,
            curl_resolution_error,
            api_base_url: GITHUB_API_BASE_URL.to_string(),
            user_agent: format!("codex-exec-loop-native/{}", env!("CARGO_PKG_VERSION")),
            token: token.into(),
            subprocess_timeout: subprocess::configured_subprocess_timeout(),
        }
    }
    pub fn from_local_github_credentials(repo_root: &Path) -> Result<Self> {
        /*
        poller는 repository automation과 같은 local GitHub credential contract를 쓴다.
        discovery는 환경변수와 신뢰된 gh auth token으로 제한한다. repository local credential.helper는 임의
        명령을 실행할 수 있고 credential file은 별도 owner/symlink 검증 없이는 신뢰할 수 없으므로 둘 다 사용하지 않는다. 찾은 token은
        즉시 bearer token으로 변환하며 이후 HTTP code는 token source나 raw credential URL을 알지 못한다.
        */
        Ok(Self::new_for_workspace(
            Self::read_local_github_token(repo_root)?,
            repo_root,
        ))
    }

    /*
    현재 git branch에서 열린 PR을 찾는 discovery entrypoint다.

    repository full name은 delivery와 같은 configured push remote에서, head branch는 현재
    checkout에서 읽는다. detached HEAD, 빈 branch, base branch 자체는 review 대상 PR을 특정할 수
    없으므로 `None`으로 접어 service가 polling을 건너뛰게 한다.
    */
    pub fn find_open_pull_request_for_current_branch(
        &self,
        repo_root: &Path,
        base_branch: &str,
    ) -> Result<Option<GithubPullRequestTarget>> {
        let repository = Self::resolve_repository_full_name(repo_root)?;
        let head_branch = Self::resolve_current_branch_name(repo_root)?;
        if head_branch == "HEAD" || head_branch.trim().is_empty() || head_branch == base_branch {
            return Ok(None);
        }

        self.find_open_pull_request_for_branch(&repository, &head_branch, base_branch)
    }
    fn find_open_pull_request_for_branch(
        &self,
        repository: &str,
        head_branch: &str,
        base_branch: &str,
    ) -> Result<Option<GithubPullRequestTarget>> {
        // GitHub pull request list API의 `head` filter는 `owner:branch` 형식이라 repo owner가 필요하다.
        // fork가 아닌 현재 repository branch만 찾는 정책이므로 repository full name의 owner를 그대로 사용한다.
        let owner = repository
            .split_once('/')
            .map(|(owner, _)| owner)
            .ok_or_else(|| anyhow!("failed to parse repository owner from {repository}"))?;
        let head = Self::encode_query_value(&format!("{owner}:{head_branch}"));
        let base = Self::encode_query_value(base_branch);
        let endpoint =
            format!("/repos/{repository}/pulls?state=open&head={head}&base={base}&per_page=1");
        let matches: Vec<PullRequestLocatorResponse> = self.fetch_object(&endpoint)?;
        Ok(matches
            .into_iter()
            .next()
            .map(|pull_request| GithubPullRequestTarget::new(repository, pull_request.number)))
    }
    #[cfg(all(test, unix))]
    fn resolve_git_dir(repo_root: &Path) -> Result<PathBuf> {
        Self::resolve_git_path(repo_root, "--git-dir", "git dir")
    }
    #[cfg(all(test, unix))]
    fn resolve_git_common_dir(repo_root: &Path) -> Result<PathBuf> {
        Self::resolve_git_path(repo_root, "--git-common-dir", "git common dir")
    }
    #[cfg(all(test, unix))]
    fn resolve_git_path(repo_root: &Path, flag: &str, label: &str) -> Result<PathBuf> {
        /*
        linked worktree에서는 `.git`이 directory가 아니라 pointer file일 수 있다.
        직접 path를 추측하면 credential lookup이 clone/worktree 형태마다 깨진다.
        `git rev-parse --path-format=absolute`로 git에게 canonical git dir 또는 common dir을 묻기 때문에
        일반 clone과 linked worktree 모두 같은 discovery path를 쓴다.
        */
        let path =
            Self::run_git_command(repo_root, &["rev-parse", "--path-format=absolute", flag])?;
        if path.is_empty() {
            bail!("resolved empty {label} from {}", repo_root.display());
        }
        Ok(PathBuf::from(path))
    }
    fn resolve_repository_full_name(repo_root: &Path) -> Result<String> {
        /*
        repository identity는 delivery와 같은 push remote의 push URL에서 얻는다.
        GitHub pull request API는 repository-scoped라 owner/repo path를 먼저 알아야 한다.
        이 lookup을 local git에 묶으면 어떤 installation을 검색해야 할지 모르는 상태에서 GitHub에 broad search를 하지 않아도 된다.
        */
        let push_remote = Self::resolve_push_remote_name(repo_root)?;
        let remote_url = Self::run_git_command(
            repo_root,
            &["remote", "get-url", "--push", push_remote.as_str()],
        )
        .map_err(|_| {
            anyhow!(
                "configured GitHub push remote `{push_remote}` is not available; implicit fallback is disabled"
            )
        })?;
        Self::parse_repository_full_name(&remote_url)
    }
    fn resolve_push_remote_name(repo_root: &Path) -> Result<String> {
        let env_value = std::env::var(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR).ok();
        let config_value =
            Self::read_optional_repo_config(repo_root, AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY)?;
        resolve_github_push_remote_name_strict(env_value.as_deref(), config_value.as_deref())
            .map_err(|message| anyhow!("{message}; implicit fallback is disabled"))
    }
    fn read_optional_repo_config(repo_root: &Path, key: &str) -> Result<Option<String>> {
        let mut command = git_subprocess::command(std::iter::empty::<&str>());
        command
            .arg("-C")
            .arg(repo_root)
            .args(["config", "--get", key])
            .stdin(Stdio::null());
        let output = subprocess::command_output_with_timeout(
            &mut command,
            &format!("git config --get {key}"),
            subprocess::configured_subprocess_timeout(),
        )
        .with_context(|| format!("failed to read repository configuration `{key}`"))?;
        if output.status.success() {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            return Ok((!value.is_empty()).then_some(value));
        }
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        bail!("failed to read repository configuration `{key}`")
    }
    fn resolve_current_branch_name(repo_root: &Path) -> Result<String> {
        /*
        current branch는 사용자의 review lane이다.
        detached HEAD는 `HEAD`를 반환하며, discovery entrypoint는 이를 non-pollable로 접는다.
        stable `owner:branch` head filter를 만들 수 없으면 어떤 PR activity를 가져와야 하는지 특정할 수 없다.
        */
        Self::run_git_command(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])
    }
    fn run_git_command(repo_root: &Path, args: &[&str]) -> Result<String> {
        Self::run_git_command_with_program(
            repo_root,
            args,
            Path::new("git"),
            subprocess::configured_subprocess_timeout(),
        )
    }

    fn run_git_command_with_program(
        repo_root: &Path,
        args: &[&str],
        program: &Path,
        timeout: Duration,
    ) -> Result<String> {
        /*
        git은 non-interactive command로 실행하고 stdout은 helper boundary에서 trim한다.
        이 helper 위쪽 caller는 repository/branch/credential 같은 domain-specific parse error를 붙이고,
        command 자체가 실패하면 stderr를 포함해 remote 설정이나 worktree 상태 문제를 진단할 수 있게 한다.
        */
        let command_label = format!("git {}", args.join(" "));
        let mut command =
            git_subprocess::command_with_program(program.as_os_str(), std::iter::empty::<&str>());
        command
            .arg("-C")
            .arg(repo_root)
            .args(args)
            .stdin(Stdio::null());
        let output = subprocess::command_output_with_timeout(&mut command, &command_label, timeout)
            .with_context(|| {
                format!(
                    "failed to run git {} from {}",
                    args.join(" "),
                    repo_root.display()
                )
            })?;
        if !output.status.success() {
            bail!(
                "git {} failed from {}: {}",
                args.join(" "),
                repo_root.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
    fn parse_repository_full_name(remote_url: &str) -> Result<String> {
        // poller는 GitHub REST path의 `{owner}/{repo}`만 필요하므로 SSH/HTTPS remote를 같은 identity로 접는다.
        // 다른 hosting provider URL은 GitHub API로 안전하게 변환할 수 없어 명시적으로 거부한다.
        parse_github_repository_identity(remote_url).ok_or_else(|| {
            if remote_url.contains("github.com") {
                anyhow!("failed to parse GitHub repository identity")
            } else {
                anyhow!("unsupported GitHub remote URL")
            }
        })
    }
    fn read_local_github_token(repo_root: &Path) -> Result<String> {
        let gh_program =
            crate::trusted_executable::resolve_native_from_current_path("gh", repo_root)
                .unwrap_or_else(|_| unresolved_gh_executable_path());
        Self::read_local_github_token_with_gh_program(repo_root, &gh_program)
    }

    fn read_local_github_token_with_gh_program(
        repo_root: &Path,
        gh_program: &Path,
    ) -> Result<String> {
        if std::env::var_os(LEGACY_CREDENTIAL_SCAN_ENV).is_some() {
            bail!(
                "AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN is no longer supported; use an explicit token environment variable or trusted gh auth token"
            );
        }
        if let Some(token) = Self::read_token_from_environment() {
            return Ok(token);
        }
        if let Some(token) = Self::read_gh_auth_token_with_program(
            repo_root,
            gh_program,
            subprocess::configured_subprocess_timeout(),
        )? {
            return Ok(token);
        }
        bail!(
            "no GitHub token found in AKRA_GITHUB_TOKEN, GH_TOKEN, or GITHUB_TOKEN; trusted gh auth was unavailable; repository credential helpers and direct credential-file scanning are not used"
        );
    }

    fn read_token_from_environment() -> Option<String> {
        ["AKRA_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"]
            .into_iter()
            .find_map(|key| {
                std::env::var(key)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
    }

    fn read_gh_auth_token_with_program(
        repo_root: &Path,
        program: &Path,
        timeout: Duration,
    ) -> Result<Option<String>> {
        #[cfg(not(test))]
        if !program.is_absolute() {
            bail!("gh credential discovery requires a pinned absolute executable")
        }
        let mut command = Command::new(program);
        crate::trusted_executable::configure_credential_command_environment(
            &mut command,
            repo_root,
            true,
        )?;
        command
            .arg("auth")
            .arg("token")
            .current_dir(crate::trusted_executable::neutral_user_config_directory(
                repo_root,
            ))
            .stdin(Stdio::null())
            .env("GIT_TERMINAL_PROMPT", "0");
        let output =
            subprocess::command_output_with_timeout(&mut command, "gh auth token", timeout);
        let Ok(output) = output else {
            return Ok(None);
        };
        if !output.status.success() {
            return Ok(None);
        }
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!token.is_empty()).then_some(token))
    }

    #[cfg(all(test, unix))]
    fn read_named_github_credential_token(repo_root: &Path) -> Result<Option<String>> {
        /*
        linked worktree에는 개별 git dir과 common git dir이 나뉠 수 있으므로 먼저 worktree-local credential을
        확인하고, 없으면 common dir을 확인한다. `refinedstone-credentials`는 기존 checkout을 깨지 않기 위한
        legacy fallback일 뿐 새 이름은 `akra-github-credentials`다.
        */
        let git_dir = Self::resolve_git_dir(repo_root)
            .map_err(|_| anyhow!("failed to resolve a repo-local legacy credential directory"))?;
        let common_dir = Self::resolve_git_common_dir(repo_root)
            .map_err(|_| anyhow!("failed to resolve a shared legacy credential directory"))?;
        for root in [git_dir.as_path(), common_dir.as_path()] {
            for file_name in [
                "akra-github-credentials",
                "github-credentials",
                "refinedstone-credentials",
            ] {
                let credential_path = root.join(file_name);
                if !credential_path.is_file() {
                    continue;
                }
                let line = Self::read_first_non_empty_line(&credential_path)
                    .context("failed to read a repo-local legacy GitHub credential file")?;
                if line.starts_with("https://") {
                    return Ok(Some(Self::parse_github_credential_token(&line)?));
                }
                return Ok(Some(line));
            }
        }
        Ok(None)
    }

    #[cfg(all(test, unix))]
    fn read_git_credential_file_token_for_root(users_root: &Path) -> Result<Option<String>> {
        Self::read_git_credential_file_token_from_candidates(
            Self::git_credential_file_candidates_for_root(users_root)?,
        )
    }

    #[cfg(all(test, unix))]
    fn read_git_credential_file_token_from_candidates(
        candidates: Vec<PathBuf>,
    ) -> Result<Option<String>> {
        for path in candidates {
            if !path.is_file() {
                continue;
            }
            let contents =
                fs::read_to_string(&path).context("failed to read a legacy Git credential file")?;
            for line in contents.lines().map(str::trim) {
                if !line.starts_with("https://") || !line.contains("@github.com") {
                    continue;
                }
                if let Ok(token) = Self::parse_github_credential_token(line) {
                    return Ok(Some(token));
                }
            }
        }
        Ok(None)
    }

    #[cfg(all(test, unix))]
    fn git_credential_file_candidates_for_root(users_root: &Path) -> Result<Vec<PathBuf>> {
        let mut candidates = Vec::new();
        if let Some(path) =
            Self::resolve_windows_credential_path_for_current_user_in_root(users_root)?
        {
            Self::push_unique_path(&mut candidates, path);
        }
        if let Some(home) = std::env::var_os("HOME") {
            Self::push_unique_path(
                &mut candidates,
                PathBuf::from(home).join(".git-credentials"),
            );
        }
        if let Some(userprofile) = std::env::var_os("USERPROFILE") {
            Self::push_unique_path(
                &mut candidates,
                PathBuf::from(userprofile).join(".git-credentials"),
            );
        }
        Ok(candidates)
    }

    #[cfg(all(test, unix))]
    fn push_unique_path(candidates: &mut Vec<PathBuf>, path: PathBuf) {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    }

    #[cfg(all(test, unix))]
    fn read_first_non_empty_line(path: &Path) -> Result<String> {
        /*
        credential file은 이 adapter 밖의 git/helper script가 관리하므로 trailing newline이나 빈 줄이 있을 수 있다.
        poller는 첫 non-empty line만 credential로 인정한다.
        */
        let contents = fs::read_to_string(path)?;
        contents
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(ToString::to_string)
            .ok_or_else(|| anyhow!("legacy GitHub credential file has no usable token line"))
    }

    #[cfg(all(test, unix))]
    fn find_windows_github_credential_line_in_root(users_root: &Path) -> Result<Option<String>> {
        let Some(credential_path) =
            Self::resolve_windows_credential_path_for_current_user_in_root(users_root)?
        else {
            return Ok(None);
        };
        let contents = match fs::read_to_string(&credential_path) {
            Ok(contents) => contents,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                ) =>
            {
                return Ok(None);
            }
            Err(error) => {
                return Err(error).context("failed to read a legacy Windows credential file");
            }
        };
        // Windows credential file에는 여러 host credential이 섞일 수 있으므로 GitHub line만 선택한다.
        Ok(contents.lines().map(str::trim).find_map(|line| {
            (line.starts_with("https://") && line.contains("@github.com")).then(|| line.to_string())
        }))
    }

    #[cfg(all(test, unix))]
    fn resolve_windows_credential_path_for_current_user_in_root(
        users_root: &Path,
    ) -> Result<Option<PathBuf>> {
        if !users_root.exists() {
            return Ok(None);
        }
        for current_user in Self::current_user_names() {
            if let Some(user_home) =
                Self::resolve_current_user_windows_home(users_root, current_user.as_str())?
            {
                return Ok(Some(user_home.join(".git-credentials")));
            }
        }
        Ok(None)
    }

    #[cfg(all(test, unix))]
    fn current_user_names() -> Vec<String> {
        let mut names = Vec::new();
        for key in ["USERNAME", "USER"] {
            if let Some(value) = std::env::var(key)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                && !names
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&value))
            {
                names.push(value);
            }
        }
        names
    }
    #[cfg(all(test, unix))]
    fn resolve_current_user_windows_home(
        users_root: &Path,
        current_user: &str,
    ) -> Result<Option<PathBuf>> {
        /*
        WSL `$USER`는 보통 Windows profile directory 이름과 같지만 casing이 다를 수 있다.
        또한 `/mnt/c/Users` scan이 permission 때문에 막힐 수 있다.
        그래서 먼저 direct path 존재 여부를 기록하고, scan은 더 tolerant한 casing fallback으로만 사용한다.
        scan이 막혀도 direct match가 있으면 credential fallback을 계속 살릴 수 있다.
        */
        let direct_match = users_root.join(current_user);
        let direct_match_exists = direct_match.is_dir();
        let entries = match fs::read_dir(users_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                return Ok(direct_match_exists.then_some(direct_match));
            }
            Err(error) => {
                return Err(error).context("failed to inspect legacy Windows credential profiles");
            }
        };
        // Windows user directory casing은 WSL `$USER`와 다를 수 있으므로 case-insensitive scan으로 fallback을 보존한다.
        for entry in entries.flatten() {
            let entry_name = entry.file_name();
            if entry_name
                .to_string_lossy()
                .eq_ignore_ascii_case(current_user)
            {
                let path = entry.path();
                if path.is_dir() {
                    return Ok(Some(path));
                }
            }
        }
        if direct_match_exists {
            return Ok(Some(direct_match));
        }
        Ok(None)
    }
    #[cfg(all(test, unix))]
    fn parse_github_credential_token(line: &str) -> Result<String> {
        // credential line은 `https://<username>:<token>@github.com` 형태다. bearer token으로 쓰는 값은 password slot뿐이다.
        let credential = line
            .strip_prefix("https://")
            .and_then(|value| {
                value
                    .split_once("@github.com")
                    .map(|(credential, _)| credential)
            })
            .ok_or_else(|| anyhow!("failed to parse GitHub credential token"))?;
        let token = credential
            .split_once(':')
            .map(|(_, token)| token)
            .ok_or_else(|| anyhow!("failed to parse GitHub credential token"))?;
        if token.trim().is_empty() {
            bail!("failed to parse GitHub credential token");
        }
        Ok(token.to_string())
    }
    fn encode_query_value(value: &str) -> String {
        /*
        GitHub의 head/base filter는 path가 아니라 query string에 들어간다.
        branch 이름에는 slash나 reserved character가 들어갈 수 있으므로 여기서만 percent-encode한다.
        endpoint path 자체는 이미 별도로 조립되므로 double-encoding하지 않는 것이 중요하다.
        */
        utf8_percent_encode(value, GITHUB_QUERY_ENCODE_SET).to_string()
    }
    fn fetch_pull_request_details(
        &self,
        target: &GithubPullRequestTarget,
        budget: GithubActivityLoadBudget,
    ) -> Result<PullRequestResponse> {
        self.fetch_object_with_budget(
            &format!("/repos/{}/pulls/{}", target.repository, target.number),
            Some(budget),
        )
    }
    fn fetch_pull_request_reviews(
        &self,
        target: &GithubPullRequestTarget,
        budget: GithubActivityLoadBudget,
    ) -> Result<Vec<PullRequestReviewResponse>> {
        self.fetch_paginated_array_with_budget(
            &format!(
                "/repos/{}/pulls/{}/reviews",
                target.repository, target.number
            ),
            Some(budget),
        )
    }
    fn fetch_review_comments(
        &self,
        target: &GithubPullRequestTarget,
        budget: GithubActivityLoadBudget,
    ) -> Result<Vec<PullRequestReviewCommentResponse>> {
        self.fetch_paginated_array_with_budget(
            &format!(
                "/repos/{}/pulls/{}/comments",
                target.repository, target.number
            ),
            Some(budget),
        )
    }
    fn fetch_issue_comments(
        &self,
        target: &GithubPullRequestTarget,
        budget: GithubActivityLoadBudget,
    ) -> Result<Vec<IssueCommentResponse>> {
        self.fetch_paginated_array_with_budget(
            &format!(
                "/repos/{}/issues/{}/comments",
                target.repository, target.number
            ),
            Some(budget),
        )
    }
    fn fetch_object<T>(&self, endpoint: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        /*
        single-object endpoint도 fetch_json + parse_json을 그대로 통과한다.
        curl failure, HTTP status failure, serde shape error가 paginated array endpoint와 같은 endpoint context를 갖게 하려는
        의도다. 호출자가 object인지 array인지에 따라 진단 품질이 달라지면 polling failure를 해석하기 어렵다.
        */
        self.fetch_object_with_budget(endpoint, None)
    }

    fn fetch_object_with_budget<T>(
        &self,
        endpoint: &str,
        budget: Option<GithubActivityLoadBudget>,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let body = self.fetch_json_with_budget(endpoint, budget)?;
        Self::parse_json(&body, endpoint)
    }
    #[cfg(all(test, unix))]
    fn fetch_paginated_array<T>(&self, endpoint: &str) -> Result<Vec<T>>
    where
        T: DeserializeOwned,
    {
        /*
        GitHub REST pagination은 한 page가 `PER_PAGE`보다 적은 item을 반환하면 끝난 것으로 본다.

        Link header parsing에 의존하지 않고 curl output parsing을 JSON body로 제한하기 위한 선택이다.
        tradeoff는 page마다 request를 하나씩 더 보내는 것이지만, PR review activity volume에서는 충분히 작고
        fixture JSON으로 테스트하기도 단순하다.
        */
        self.fetch_paginated_array_with_budget(endpoint, None)
    }

    fn fetch_paginated_array_with_budget<T>(
        &self,
        endpoint: &str,
        budget: Option<GithubActivityLoadBudget>,
    ) -> Result<Vec<T>>
    where
        T: DeserializeOwned,
    {
        let mut items = Vec::new();
        let mut aggregate_bytes = 0_usize;
        for page in 1..=MAX_PAGINATED_PAGES {
            let page_endpoint = format!("{endpoint}?per_page={PER_PAGE}&page={page}");
            let body = self.fetch_json_with_budget(&page_endpoint, budget)?;
            aggregate_bytes = aggregate_bytes
                .checked_add(body.len())
                .filter(|total| *total <= MAX_PAGINATED_RESPONSE_BYTES)
                .ok_or_else(|| {
                    anyhow!(
                        "GitHub pagination response exceeded the aggregate {} byte limit for {endpoint}",
                        MAX_PAGINATED_RESPONSE_BYTES
                    )
                })?;
            let page_items: Vec<T> = Self::parse_json(&body, &page_endpoint)?;
            let count = page_items.len();
            if count > PER_PAGE {
                bail!(
                    "GitHub pagination returned {count} items for a {PER_PAGE}-item page at {page_endpoint}"
                );
            }
            let item_count = items
                .len()
                .checked_add(count)
                .filter(|total| *total <= MAX_PAGINATED_ITEMS)
                .ok_or_else(|| {
                    anyhow!(
                        "GitHub pagination exceeded the {MAX_PAGINATED_ITEMS}-item limit for {endpoint}"
                    )
                })?;
            items.reserve(item_count - items.len());
            items.extend(page_items);
            if count < PER_PAGE {
                return Ok(items);
            }
        }
        bail!(
            "GitHub pagination remained full after {MAX_PAGINATED_PAGES} pages for {endpoint}; refusing an incomplete activity snapshot"
        )
    }
    #[cfg(all(test, unix))]
    fn fetch_json(&self, endpoint: &str) -> Result<String> {
        self.fetch_json_with_budget(endpoint, None)
    }

    pub(super) fn fetch_validation_response(
        &self,
        endpoint: &str,
    ) -> std::result::Result<
        super::pr_validation::GithubValidationApiResponse,
        GithubPrValidationError,
    > {
        if self.curl_resolution_error.is_some() {
            return Err(GithubPrValidationError::integrity_failed(
                "trusted curl executable could not be pinned for GitHub validation",
            ));
        }
        let url = format!("{}{}", self.api_base_url, endpoint);
        let authorization = format!("Authorization: Bearer {}", self.token);
        let user_agent = format!("User-Agent: {}", self.user_agent);
        let api_version = format!("X-GitHub-Api-Version: {}", GITHUB_API_VERSION);
        let output = self
            .run_validation_curl_process(
                &url,
                &api_version,
                &authorization,
                &user_agent,
                self.subprocess_timeout,
            )
            .map_err(|_| {
                GithubPrValidationError::retryable("GitHub validation request failed or timed out")
            })?;
        if !output.status.success() {
            return Err(GithubPrValidationError::retryable(
                "GitHub validation transport failed",
            ));
        }
        parse_validation_http_response(&output.stdout, Utc::now())
    }

    fn fetch_json_with_budget(
        &self,
        endpoint: &str,
        budget: Option<GithubActivityLoadBudget>,
    ) -> Result<String> {
        /*
        persistent HTTP client 대신 curl을 사용해 adapter dependency를 가볍게 유지하고 repository shell automation과 실행 방식을 맞춘다.
        timeout flag는 TUI contract의 일부다.
        review polling은 app-server loop를 무기한 막지 않고, 진단 가능한 command failure로 돌아와야 한다.
        bearer token과 API version/user-agent header는 이 outbound boundary에서만 조립한다.
        */
        if let Some(error) = &self.curl_resolution_error {
            bail!("trusted curl executable could not be pinned: {error}")
        }
        let url = format!("{}{}", self.api_base_url, endpoint);
        let authorization = format!("Authorization: Bearer {}", self.token);
        let user_agent = format!("User-Agent: {}", self.user_agent);
        let api_version = format!("X-GitHub-Api-Version: {}", GITHUB_API_VERSION);
        let request_timeout = match budget {
            Some(budget) => self
                .subprocess_timeout
                .min(budget.remaining(&format!("requesting {endpoint}"))?),
            None => self.subprocess_timeout,
        };
        let output = self
            .run_curl_process(
                &url,
                &api_version,
                &authorization,
                &user_agent,
                request_timeout,
            )
            .with_context(|| format!("failed to run {} for {url}", self.curl_path))?;
        if !output.status.success() {
            bail!(
                "github api request failed for {url}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        if output.stdout.len() > MAX_GITHUB_RESPONSE_BYTES {
            bail!(
                "github api response exceeded the {} byte limit for {url}",
                MAX_GITHUB_RESPONSE_BYTES
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
    fn run_curl_process(
        &self,
        url: &str,
        api_version: &str,
        authorization: &str,
        user_agent: &str,
        request_timeout: Duration,
    ) -> io::Result<Output> {
        let config = build_curl_stdin_config(api_version, authorization, user_agent);
        let command_label = format!("{} {url}", self.curl_path);
        for attempt in 1..=CURL_SPAWN_ATTEMPTS {
            let mut command = Command::new(&self.curl_path);
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            crate::trusted_executable::configure_credential_command_environment(
                &mut command,
                &cwd,
                false,
            )
            .map_err(io::Error::other)?;
            command
                .arg("-q")
                .args([
                    // curl forwards custom headers across redirects. Never let a 3xx move the
                    // bearer token to another HTTPS host; repository moves must be reconciled
                    // through the frozen GitHub remote identity instead.
                    "-sSf",
                    "--proto",
                    "=https",
                    "--connect-timeout",
                    CURL_CONNECT_TIMEOUT_SECONDS,
                    "--max-time",
                    CURL_MAX_TIME_SECONDS,
                    "--max-filesize",
                    &MAX_GITHUB_RESPONSE_BYTES.to_string(),
                    "--config",
                    "-",
                ])
                .arg(url);
            let output = match subprocess::command_output_with_input_and_timeout(
                &mut command,
                &command_label,
                config.as_bytes(),
                request_timeout,
            ) {
                Ok(output) => output,
                Err(error)
                    if attempt < CURL_SPAWN_ATTEMPTS && is_transient_curl_spawn_error(&error) =>
                {
                    thread::sleep(CURL_SPAWN_RETRY_DELAY);
                    continue;
                }
                Err(error) => return Err(error),
            };
            return Ok(output);
        }

        unreachable!("curl spawn retry loop should return from every attempt")
    }

    fn run_validation_curl_process(
        &self,
        url: &str,
        api_version: &str,
        authorization: &str,
        user_agent: &str,
        request_timeout: Duration,
    ) -> io::Result<Output> {
        let config = build_curl_stdin_config(api_version, authorization, user_agent);
        let command_label = format!("{} {url}", self.curl_path);
        for attempt in 1..=CURL_SPAWN_ATTEMPTS {
            let mut command = Command::new(&self.curl_path);
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            crate::trusted_executable::configure_credential_command_environment(
                &mut command,
                &cwd,
                false,
            )
            .map_err(io::Error::other)?;
            command
                .arg("-q")
                .args([
                    "-sS",
                    "--proto",
                    "=https",
                    "--connect-timeout",
                    CURL_CONNECT_TIMEOUT_SECONDS,
                    "--max-time",
                    CURL_MAX_TIME_SECONDS,
                    "--max-filesize",
                    &MAX_GITHUB_RESPONSE_BYTES.to_string(),
                    "--dump-header",
                    "-",
                    "--write-out",
                    &format!("{VALIDATION_HTTP_STATUS_MARKER}%{{http_code}}"),
                    "--config",
                    "-",
                ])
                .arg(url);
            let output = match subprocess::command_output_with_input_and_timeout(
                &mut command,
                &command_label,
                config.as_bytes(),
                request_timeout,
            ) {
                Ok(output) => output,
                Err(error)
                    if attempt < CURL_SPAWN_ATTEMPTS && is_transient_curl_spawn_error(&error) =>
                {
                    thread::sleep(CURL_SPAWN_RETRY_DELAY);
                    continue;
                }
                Err(error) => return Err(error),
            };
            return Ok(output);
        }
        unreachable!("validation curl spawn retry loop should return from every attempt")
    }

    fn parse_json<T>(body: &str, endpoint: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        /*
        JSON parsing은 endpoint context와 함께 실패해야 한다.
        GitHub response DTO는 private type이므로, shape drift가 발생하면 어떤 REST endpoint의 payload가 contract와 달라졌는지
        바로 드러나야 한다.
        */
        serde_json::from_str(body)
            .with_context(|| format!("failed to parse GitHub response for {endpoint}"))
    }
    fn to_snapshot(
        target: &GithubPullRequestTarget,
        pull_request: PullRequestResponse,
        reviews: Vec<PullRequestReviewResponse>,
        review_comments: Vec<PullRequestReviewCommentResponse>,
        issue_comments: Vec<IssueCommentResponse>,
    ) -> GithubPullRequestActivitySnapshot {
        /*
        GitHub는 PR activity를 review submission, inline review comment, issue comment endpoint로 나눠 제공한다.
        domain service는 이전 snapshot과 비교할 하나의 activity stream을 원하므로, adapter가 endpoint-specific DTO를 먼저
        domain event shape로 합친다. sort는 모든 event kind가 같은 domain shape가 된 뒤에만 수행해 timestamp 기준 ordering을
        endpoint 종류와 분리한다.
        */
        let mut events = reviews
            .into_iter()
            .filter_map(Self::to_review_event)
            .collect::<Vec<_>>();
        events.extend(
            review_comments
                .into_iter()
                .map(Self::to_review_comment_event)
                .collect::<Vec<_>>(),
        );
        events.extend(
            issue_comments
                .into_iter()
                .map(Self::to_issue_comment_event)
                .collect::<Vec<_>>(),
        );
        let mut snapshot = GithubPullRequestActivitySnapshot {
            target: target.clone(),
            title: pull_request.title,
            url: pull_request.html_url,
            head_branch: pull_request.head.ref_name,
            base_branch: pull_request.base.ref_name,
            events,
        };
        snapshot.sort_events();
        snapshot
    }
    fn to_review_event(
        review: PullRequestReviewResponse,
    ) -> Option<GithubPullRequestActivityEvent> {
        /*
        pending review는 submitted timestamp가 없고 아직 public review activity가 아니다.
        이를 건너뛰면 PR timeline 자체가 operator에게 보여주지 않는 draft review state를 poller가 새 활동처럼 알리는 일을 막는다.
        submitted_at이 있는 review만 domain event가 된다.
        */
        let submitted_at = review.submitted_at?;

        Some(GithubPullRequestActivityEvent {
            id: review.id,
            kind: GithubPullRequestActivityKind::Review,
            submitted_at,
            author_login: review
                .user
                .map(|user| user.login)
                .unwrap_or_else(|| "github".to_string()),
            body: review.body.unwrap_or_default(),
            state: Some(review.state),
            url: review.html_url,
            path: None,
        })
    }
    fn to_review_comment_event(
        comment: PullRequestReviewCommentResponse,
    ) -> GithubPullRequestActivityEvent {
        GithubPullRequestActivityEvent {
            id: comment.id,
            kind: GithubPullRequestActivityKind::ReviewComment,
            submitted_at: comment.updated_at,
            author_login: comment.user.login,
            body: comment.body,
            state: None,
            url: comment.html_url,
            path: Some(comment.path),
        }
    }
    fn to_issue_comment_event(comment: IssueCommentResponse) -> GithubPullRequestActivityEvent {
        GithubPullRequestActivityEvent {
            id: comment.id,
            kind: GithubPullRequestActivityKind::IssueComment,
            submitted_at: comment.updated_at,
            author_login: comment.user.login,
            body: comment.body,
            state: None,
            url: comment.html_url,
            path: None,
        }
    }
}

fn parse_validation_http_response(
    output: &[u8],
    observed_at: DateTime<Utc>,
) -> std::result::Result<super::pr_validation::GithubValidationApiResponse, GithubPrValidationError>
{
    if output.len() > MAX_GITHUB_RESPONSE_BYTES + 64 * 1024 {
        return Err(GithubPrValidationError::integrity_failed(
            "GitHub validation response exceeded its bounded envelope",
        ));
    }
    let output = String::from_utf8(output.to_vec()).map_err(|_| {
        GithubPrValidationError::integrity_failed("GitHub validation response was not valid UTF-8")
    })?;
    let marker_index = output.rfind(VALIDATION_HTTP_STATUS_MARKER).ok_or_else(|| {
        GithubPrValidationError::integrity_failed(
            "GitHub validation response omitted its HTTP status",
        )
    })?;
    let status = output[marker_index + VALIDATION_HTTP_STATUS_MARKER.len()..]
        .trim()
        .parse::<u16>()
        .map_err(|_| {
            GithubPrValidationError::integrity_failed(
                "GitHub validation response contained an invalid HTTP status",
            )
        })?;
    let (headers, body) = split_validation_http_envelope(&output[..marker_index])?;
    let metadata = GithubValidationProviderMetadata {
        rate_limit_remaining: validation_header(headers, "x-ratelimit-remaining")
            .and_then(|value| value.parse::<u64>().ok()),
        rate_limit_reset_at: validation_header(headers, "x-ratelimit-reset")
            .and_then(|value| value.parse::<i64>().ok())
            .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single()),
    };
    if (200..300).contains(&status) {
        return Ok(super::pr_validation::GithubValidationApiResponse {
            body: body.to_string(),
            metadata,
        });
    }
    let retry_after_at = validation_header(headers, "retry-after").and_then(|value| {
        value
            .parse::<i64>()
            .ok()
            .and_then(TimeDelta::try_seconds)
            .map(|delay| observed_at + delay)
            .or_else(|| {
                DateTime::parse_from_rfc2822(value)
                    .ok()
                    .map(|value| value.with_timezone(&Utc))
            })
    });
    let class = match status {
        403 if metadata.rate_limit_remaining == Some(0) || retry_after_at.is_some() => {
            GithubPrValidationErrorClass::RetryableProvider
        }
        401 | 403 => GithubPrValidationErrorClass::AuthenticationBlocked,
        408 | 425 | 429 | 500..=599 => GithubPrValidationErrorClass::RetryableProvider,
        404 | 409 | 422 => GithubPrValidationErrorClass::IdentityFailed,
        _ => GithubPrValidationErrorClass::IntegrityFailed,
    };
    Err(GithubPrValidationError::new(
        class,
        format!("GitHub validation request returned HTTP {status}"),
    )
    .with_retry_after_at(retry_after_at)
    .with_provider_metadata(metadata))
}

fn split_validation_http_envelope(
    mut payload: &str,
) -> std::result::Result<(&str, &str), GithubPrValidationError> {
    let mut last_headers = None;
    while payload.starts_with("HTTP/") {
        let (headers, body) = split_validation_header_block(payload).ok_or_else(|| {
            GithubPrValidationError::integrity_failed(
                "GitHub validation response contained an incomplete HTTP header block",
            )
        })?;
        last_headers = Some(headers);
        payload = body;
        if !payload.starts_with("HTTP/") {
            break;
        }
    }
    let headers = last_headers.ok_or_else(|| {
        GithubPrValidationError::integrity_failed(
            "GitHub validation response omitted its HTTP headers",
        )
    })?;
    Ok((headers, payload))
}

fn split_validation_header_block(value: &str) -> Option<(&str, &str)> {
    value
        .find("\r\n\r\n")
        .map(|index| (&value[..index], &value[index + 4..]))
        .or_else(|| {
            value
                .find("\n\n")
                .map(|index| (&value[..index], &value[index + 2..]))
        })
}

fn validation_header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then_some(value.trim())
    })
}

fn build_curl_stdin_config(api_version: &str, authorization: &str, user_agent: &str) -> String {
    let mut config = String::new();
    config.push_str("header = ");
    config.push_str(&curl_config_string_literal(
        "Accept: application/vnd.github+json",
    ));
    config.push('\n');
    config.push_str("header = ");
    config.push_str(&curl_config_string_literal(api_version));
    config.push('\n');
    config.push_str("header = ");
    config.push_str(&curl_config_string_literal(authorization));
    config.push('\n');
    config.push_str("header = ");
    config.push_str(&curl_config_string_literal(user_agent));
    config.push('\n');
    config
}

fn curl_config_string_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn is_transient_curl_spawn_error(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(26))
}

impl GithubReviewPollerPort for GithubReviewPollerAdapter {
    fn load_pull_request_activity(
        &self,
        target: &GithubPullRequestTarget,
    ) -> Result<GithubPullRequestActivitySnapshot> {
        /*
        activity load는 의도적으로 full refresh다.
        이전 snapshot과의 diffing은 application service가 맡고, adapter는 각 poll을 stateless하게 유지한다.
        매번 GitHub에서 PR header와 세 activity endpoint를 다시 읽어 authoritative PR timeline snapshot을 재구성한다.
        */
        let budget = GithubActivityLoadBudget::new();
        let pull_request = self.fetch_pull_request_details(target, budget)?;
        let reviews = self.fetch_pull_request_reviews(target, budget)?;
        let review_comments = self.fetch_review_comments(target, budget)?;
        let issue_comments = self.fetch_issue_comments(target, budget)?;
        Ok(Self::to_snapshot(
            target,
            pull_request,
            reviews,
            review_comments,
            issue_comments,
        ))
    }
}

/*
private GitHub REST response DTO들이다.

이 struct들은 domain snapshot을 만들기 위해 필요한 field만 모델링한다.
private로 유지하면 GitHub JSON field name, nullable detail, endpoint별 payload 차이가 application-layer contract로 새지 않는다.
*/
#[derive(Debug, Clone, Deserialize)]
struct PullRequestResponse {
    title: String,
    html_url: String,
    head: PullRequestBranchRef,
    base: PullRequestBranchRef,
}
#[derive(Debug, Clone, Deserialize)]
struct PullRequestLocatorResponse {
    number: u64,
}
#[derive(Debug, Clone, Deserialize)]
struct PullRequestBranchRef {
    #[serde(rename = "ref")]
    ref_name: String,
}
#[derive(Debug, Clone, Deserialize)]
struct PullRequestReviewResponse {
    id: u64,
    body: Option<String>,
    state: String,
    submitted_at: Option<String>,
    html_url: String,
    user: Option<GitHubUser>,
}
#[derive(Debug, Clone, Deserialize)]
struct PullRequestReviewCommentResponse {
    id: u64,
    body: String,
    updated_at: String,
    html_url: String,
    path: String,
    user: GitHubUser,
}
#[derive(Debug, Clone, Deserialize)]
struct IssueCommentResponse {
    id: u64,
    body: String,
    updated_at: String,
    html_url: String,
    user: GitHubUser,
}
#[derive(Debug, Clone, Deserialize)]
struct GitHubUser {
    login: String,
}

#[cfg(test)]
mod validation_http_tests {
    use chrono::{TimeDelta, TimeZone, Utc};

    use super::{VALIDATION_HTTP_STATUS_MARKER, parse_validation_http_response};
    use crate::application::port::outbound::github_pr_validation_port::GithubPrValidationErrorClass;

    fn response(status: u16, headers: &str, body: &str) -> Vec<u8> {
        format!("HTTP/2 {status}\r\n{headers}\r\n\r\n{body}{VALIDATION_HTTP_STATUS_MARKER}{status}")
            .into_bytes()
    }

    #[test]
    fn validation_http_success_preserves_body_and_rate_metadata() {
        let observed_at = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        let parsed = parse_validation_http_response(
            &response(
                200,
                "x-ratelimit-remaining: 4998\r\nx-ratelimit-reset: 1786320600",
                "{\"ok\":true}",
            ),
            observed_at,
        )
        .unwrap();
        assert_eq!(parsed.body, "{\"ok\":true}");
        assert_eq!(parsed.metadata.rate_limit_remaining, Some(4_998));
        assert_eq!(
            parsed.metadata.rate_limit_reset_at,
            Utc.timestamp_opt(1_786_320_600, 0).single()
        );
    }

    #[test]
    fn validation_http_429_and_rate_limited_403_are_retryable_with_retry_after() {
        let observed_at = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        for status in [429, 403] {
            let error = parse_validation_http_response(
                &response(
                    status,
                    "Retry-After: 75\r\nX-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: 1786320600",
                    "{\"message\":\"redacted\"}",
                ),
                observed_at,
            )
            .unwrap_err();
            assert_eq!(error.class, GithubPrValidationErrorClass::RetryableProvider);
            assert_eq!(
                error.retry_after_at,
                Some(observed_at + TimeDelta::seconds(75))
            );
            assert_eq!(error.provider_metadata.rate_limit_remaining, Some(0));
            assert!(!error.message.contains("redacted"));
        }
    }

    #[test]
    fn validation_http_5xx_retries_but_plain_403_blocks_authentication() {
        let observed_at = Utc.with_ymd_and_hms(2026, 8, 10, 0, 0, 0).unwrap();
        let unavailable =
            parse_validation_http_response(&response(503, "", "provider detail"), observed_at)
                .unwrap_err();
        assert_eq!(
            unavailable.class,
            GithubPrValidationErrorClass::RetryableProvider
        );
        let forbidden =
            parse_validation_http_response(&response(403, "", "secret detail"), observed_at)
                .unwrap_err();
        assert_eq!(
            forbidden.class,
            GithubPrValidationErrorClass::AuthenticationBlocked
        );
        assert!(!forbidden.message.contains("secret detail"));
    }
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, unix))]
mod timeout_policy_tests {
    use super::GithubReviewPollerAdapter;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn gh_auth_token_timeout_degrades_to_none() {
        let root = unique_temp_dir("review-poller-gh-auth-timeout");
        fs::create_dir_all(&root).expect("fixture root should be created");
        let program = write_executable_script(
            &root,
            "gh",
            r#"#!/bin/sh
set -eu
sleep 2
"#,
        );
        let token = GithubReviewPollerAdapter::read_gh_auth_token_with_program(
            &root,
            &program,
            Duration::from_millis(50),
        )
        .expect("timed out gh auth token should degrade to none");

        assert_eq!(token, None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_git_command_uses_shared_timeout_policy() {
        let root = unique_temp_dir("review-poller-run-git-timeout");
        fs::create_dir_all(&root).expect("fixture root should be created");
        let program = write_executable_script(
            &root,
            "git",
            r#"#!/bin/sh
set -eu
sleep 2
"#,
        );
        let error = GithubReviewPollerAdapter::run_git_command_with_program(
            &root,
            &["rev-parse", "HEAD"],
            &program,
            Duration::from_millis(50),
        )
        .expect_err("timed out git command should surface timeout");
        let error_chain = error
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" | ");

        assert!(error_chain.contains("timed out after"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fetch_json_uses_shared_timeout_policy_for_curl() {
        let root = unique_temp_dir("review-poller-fetch-json-timeout");
        fs::create_dir_all(&root).expect("fixture root should be created");
        let script = write_executable_script(
            &root,
            "fake-curl",
            r#"#!/bin/sh
set -eu
sleep 2
"#,
        );
        let adapter = GithubReviewPollerAdapter {
            curl_path: script.display().to_string(),
            curl_resolution_error: None,
            api_base_url: "https://api.test".to_string(),
            user_agent: "akra-test".to_string(),
            token: "secret-token".to_string(),
            subprocess_timeout: std::time::Duration::from_millis(50),
        };

        let error = adapter
            .fetch_json("/repos/acme/widgets/pulls/42")
            .expect_err("timed out curl should surface timeout context");
        let error_chain = error
            .chain()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" | ");

        assert!(error_chain.contains("timed out after"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fetch_json_sends_authorization_via_curl_stdin_config() {
        let root = unique_temp_dir("review-poller-curl-stdin-config");
        fs::create_dir_all(&root).expect("fixture root should be created");
        let argv_path = root.join("curl-argv.txt");
        let stdin_path = root.join("curl-stdin.txt");
        let script = write_executable_script(
            &root,
            "fake-curl",
            &format!(
                r#"#!/bin/sh
set -eu
printf '%s\n' "$@" > "{}"
cat > "{}"
printf '{{"ok":true}}'
"#,
                argv_path.display(),
                stdin_path.display(),
            ),
        );
        let adapter = GithubReviewPollerAdapter {
            curl_path: script.display().to_string(),
            curl_resolution_error: None,
            api_base_url: "https://api.test".to_string(),
            user_agent: "akra-test".to_string(),
            token: "secret-token".to_string(),
            subprocess_timeout: std::time::Duration::from_secs(1),
        };

        let body = adapter
            .fetch_json("/repos/acme/widgets/pulls/42")
            .expect("fake curl should return JSON body");
        let argv = fs::read_to_string(&argv_path).expect("curl argv capture should be readable");
        let stdin = fs::read_to_string(&stdin_path).expect("curl stdin capture should be readable");

        assert_eq!(body, "{\"ok\":true}");
        assert_eq!(
            argv,
            "-q\n-sSf\n--proto\n=https\n--connect-timeout\n10\n--max-time\n30\n--max-filesize\n8388608\n--config\n-\nhttps://api.test/repos/acme/widgets/pulls/42\n"
        );
        assert!(
            stdin.contains("header = \"Authorization: Bearer secret-token\""),
            "curl stdin config should carry authorization header: {stdin}"
        );
        assert!(!argv.contains("secret-token"));
        assert!(!stdin.contains("url = \"https://api.test/repos/acme/widgets/pulls/42\""));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fetch_json_ignores_poisoned_home_curl_config_before_sending_bearer_token() {
        let _guard = crate::test_utils::process_environment_mutex()
            .lock()
            .expect("environment fixture lock should not be poisoned");
        let root = unique_temp_dir("review-poller-poisoned-curlrc");
        let home = root.join("home");
        fs::create_dir_all(&home).expect("fixture home should be created");
        let trace_path = root.join("curl-trace.log");
        let output_path = root.join("curl-output.log");
        let proxy_contact_path = root.join("proxy-contact.log");
        fs::write(
            home.join(".curlrc"),
            format!(
                "trace = \"{}\"\noutput = \"{}\"\nproxy = \"http://127.0.0.1:9\"\n",
                trace_path.display(),
                output_path.display(),
            ),
        )
        .expect("poisoned curlrc should be written");
        let script = write_executable_script(
            &root,
            "fake-curl",
            &format!(
                r#"#!/bin/sh
set -eu
if [ "${{1-}}" != "-q" ] && [ -f "$HOME/.curlrc" ]; then
  cat > "{trace_path}"
  printf 'poisoned output\n' > "{output_path}"
  printf 'poisoned proxy contacted\n' > "{proxy_contact_path}"
  exit 65
fi
cat >/dev/null
printf '{{"ok":true}}'
"#,
                trace_path = trace_path.display(),
                output_path = output_path.display(),
                proxy_contact_path = proxy_contact_path.display(),
            ),
        );
        let adapter = GithubReviewPollerAdapter {
            curl_path: script.display().to_string(),
            curl_resolution_error: None,
            api_base_url: "https://api.test".to_string(),
            user_agent: "akra-test".to_string(),
            token: "bearer-token-must-not-leak".to_string(),
            subprocess_timeout: std::time::Duration::from_secs(1),
        };
        let previous_home = std::env::var_os("HOME");
        unsafe { std::env::set_var("HOME", &home) };

        let result = adapter.fetch_json("/repos/acme/widgets/pulls/42");

        unsafe {
            match previous_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
        assert_eq!(
            result.expect("fake curl should return JSON"),
            "{\"ok\":true}"
        );
        for leak_path in [&trace_path, &output_path, &proxy_contact_path] {
            assert!(
                !leak_path.exists(),
                "curl user configuration must not create {}",
                leak_path.display()
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"))
    }

    fn write_executable_script(root: &Path, name: &str, body: &str) -> PathBuf {
        let script_path = root.join(name);
        fs::write(&script_path, body).expect("script fixture should be written");
        let mut permissions = fs::metadata(&script_path)
            .expect("script metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions)
            .expect("script fixture should be executable");
        script_path
    }
}
