//! origin 설정만 읽는 로컬 Git 경계다. push remote 선택과 네트워크 요청을 수행하지 않는다.
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};

use crate::application::port::outbound::github_automation_port::{
    credential_redacted_canonical_github_push_url, parse_github_repository_identity,
};
use crate::application::port::outbound::issue_collection_origin_port::{
    IssueCollectionOrigin, IssueCollectionOriginPort,
};

const MAX_ORIGIN_OUTPUT_BYTES: usize = 8 * 1024;
type OriginReader = dyn Fn(&str) -> Result<Vec<u8>> + Send + Sync;

pub(crate) struct GitIssueCollectionOriginAdapter {
    read: Box<OriginReader>,
}

impl GitIssueCollectionOriginAdapter {
    pub(crate) fn new() -> Self {
        Self {
            read: Box::new(read_origin),
        }
    }
}

impl IssueCollectionOriginPort for GitIssueCollectionOriginAdapter {
    fn github_origin(&self, workspace_dir: &str) -> Result<IssueCollectionOrigin> {
        if !Path::new(workspace_dir).is_absolute() {
            bail!("the issue source workspace is invalid");
        }
        // 출력·stderr·전송 오류에는 URL의 인증 정보가 있을 수 있으므로 오류 원문을 보존하지 않는다.
        let bytes =
            (self.read)(workspace_dir).map_err(|_| anyhow!("the GitHub origin cannot be read"))?;
        parse_origin(&bytes).ok_or_else(|| anyhow!("the GitHub origin is unavailable or ambiguous"))
    }
}

fn read_origin(workspace_dir: &str) -> Result<Vec<u8>> {
    let mut command = crate::git_subprocess::command([
        "config",
        "--local",
        "--null",
        "--get-all",
        "remote.origin.url",
    ]);
    command.current_dir(workspace_dir).stderr(Stdio::null());
    let output = crate::subprocess::command_output_with_timeout(
        &mut command,
        "git issue source origin",
        Duration::from_secs(5),
    )
    .map_err(|_| anyhow!("the GitHub origin cannot be read"))?;
    if !output.status.success() {
        bail!("the GitHub origin cannot be read");
    }
    Ok(output.stdout)
}

fn parse_origin(bytes: &[u8]) -> Option<IssueCollectionOrigin> {
    if bytes.len() > MAX_ORIGIN_OUTPUT_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?.strip_suffix('\0')?;
    let mut identity = None;
    for value in text.split('\0') {
        // 기존 canonical URL 해석은 HTTPS와 GitHub SSH만 허용하고 자격 증명을 버린다.
        let canonical = credential_redacted_canonical_github_push_url(value)?;
        let parsed = parse_github_repository_identity(&canonical)?;
        if identity.as_ref().is_some_and(|current| current != &parsed) {
            return None;
        }
        identity = Some(parsed);
    }
    let identity = identity?;
    let (owner, name) = identity.split_once('/')?;
    Some(IssueCollectionOrigin {
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_identity_accepts_https_and_ssh_without_returning_credentials() {
        for url in [
            "https://github.com/example/project.git\0",
            "git@github.com:example/project.git\0",
            "ssh://git@github.com/example/project.git\0",
            "https://synthetic-user:synthetic-secret@github.com/example/project.git\0",
        ] {
            let value = parse_origin(url.as_bytes()).unwrap();
            assert_eq!(
                value,
                IssueCollectionOrigin {
                    owner: "example".to_string(),
                    name: "project".to_string(),
                }
            );
            assert!(!format!("{value:?}").contains("synthetic-secret"));
        }
        assert!(
            parse_origin(
                b"https://github.com/example/project.git\0git@github.com:example/project.git\0"
            )
            .is_some()
        );
    }

    #[test]
    fn origin_identity_rejects_multiple_repositories_and_untrusted_url_shapes() {
        for output in [
            b"https://github.com/example/project.git\0https://github.com/other/project.git\0"
                .as_slice(),
            b"https://github.com.attacker.invalid/example/project.git\0",
            b"https://github.com/example/project.git?token=synthetic\0",
            b"file:///workspace/repository\0",
            b"https://github.com/example/project.git",
            b"\0",
            b"\xff\0",
        ] {
            assert!(parse_origin(output).is_none());
        }
        assert!(parse_origin(&vec![b'a'; MAX_ORIGIN_OUTPUT_BYTES + 1]).is_none());
    }

    #[test]
    fn provider_errors_do_not_expose_the_original_remote_value() {
        let adapter = GitIssueCollectionOriginAdapter {
            read: Box::new(|_| bail!("synthetic-private-origin-and-token")),
        };
        let workspace = std::env::current_dir().unwrap();
        let error = adapter
            .github_origin(workspace.to_str().unwrap())
            .unwrap_err();
        assert_eq!(error.to_string(), "the GitHub origin cannot be read");
        assert!(!format!("{error:#}").contains("synthetic-private"));
    }

    #[test]
    fn reads_only_the_origin_of_a_local_fixture() {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-origin-{}-{:08x}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec![
                "config",
                "remote.origin.url",
                "https://github.com/example/source.git",
            ],
            vec![
                "config",
                "remote.other.url",
                "https://github.com/example/push.git",
            ],
            vec!["config", "akra.githubPushRemote", "other"],
        ] {
            let status = crate::test_utils::git_command()
                .args(args)
                .current_dir(&root)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        }
        let value = GitIssueCollectionOriginAdapter::new()
            .github_origin(root.to_str().unwrap())
            .unwrap();
        assert_eq!(value.name, "source");
        std::fs::remove_dir_all(root).unwrap();
    }
}
