//! The declaration is a frozen blob. Its two literals cannot select the binary,
//! Git options, a script, a worktree file, or an inherited environment.
use super::context::PrivateGitContext;
use super::environment::digest;
use super::{Reason, TaskVerificationExecutionManifest};
use akra_native::trusted_executable::PinnedNativeExecutable;
use serde::Deserialize;
use std::process::Command;
use std::time::Instant;

mod consent;
pub(super) use consent::confirm;
pub(crate) use consent::confirm_text;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Grep {
    path: String,
    literal: String,
}

impl Grep {
    pub(super) fn read(
        context: &mut PrivateGitContext,
        binary: &PinnedNativeExecutable,
        source: &str,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, Reason> {
        let file = format!("{source}:.akra/verify-grep.json");
        let output = context
            .output(
                binary,
                &["cat-file", "blob", &file],
                2048,
                deadline,
                cancelled,
            )
            .map_err(|_| Reason::InvalidInput)?;
        let grep: Self =
            serde_json::from_slice(&output.stdout).map_err(|_| Reason::InvalidInput)?;
        if grep.path.len() > 256
            || grep.path.split('/').any(|part| {
                matches!(part, "" | "." | "..")
                    || !part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            })
            || grep.literal.trim().is_empty()
            || grep.literal.len() > 256
            || !grep
                .literal
                .bytes()
                .all(|b| b.is_ascii_graphic() || b == b' ')
        {
            return Err(Reason::InvalidInput);
        }
        let entry = context
            .output(
                binary,
                &["ls-tree", "-z", source, "--", &grep.path],
                512,
                deadline,
                cancelled,
            )
            .map_err(|_| Reason::InvalidInput)?;
        if !entry.stdout.starts_with(b"100644 blob ") && !entry.stdout.starts_with(b"100755 blob ")
        {
            return Err(Reason::InvalidInput);
        }
        let size = context
            .output(
                binary,
                &["cat-file", "-s", &format!("{source}:{}", grep.path)],
                128,
                deadline,
                cancelled,
            )
            .map_err(|_| Reason::InvalidInput)?;
        let length = super::context::text_line(&size.stdout)
            .map_err(|_| Reason::InvalidInput)?
            .parse::<usize>()
            .map_err(|_| Reason::InvalidInput)?;
        if length > super::environment::MAX_BLOB_BYTES {
            return Err(Reason::InvalidInput);
        }
        Ok(grep)
    }

    pub(super) fn arguments<'a>(&'a self, source: &'a str) -> [&'a str; 9] {
        [
            "grep",
            "--quiet",
            "--no-textconv",
            "--fixed-strings",
            "-e",
            &self.literal,
            source,
            "--",
            &self.path,
        ]
    }

    pub(super) fn fingerprint(
        &self,
        binary: &PinnedNativeExecutable,
        source: &str,
    ) -> Result<String, Reason> {
        let bytes =
            serde_json::to_vec(&("repo-git-grep-v1", binary.sha256(), self.arguments(source)))
                .map_err(|_| Reason::InvalidInput)?;
        Ok(digest(&bytes))
    }
}
