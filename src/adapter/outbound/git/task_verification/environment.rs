use std::ffi::{OsStr, OsString};
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::trusted_executable::PinnedNativeExecutable;

pub(super) const RECIPE_VERSION: &str = "git-diff-check-v1";
pub(super) const HEAD: &[u8] = b"ref: refs/heads/akra-verification-unborn\n";
pub(super) const ATTRIBUTES: &[u8] =
    b"* diff whitespace=blank-at-eol,blank-at-eof,space-before-tab conflict-marker-size=7\n";
pub(super) const MAX_PATHS: usize = 4096;
pub(super) const MAX_BLOB_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const DEADLINE_SECONDS: u64 = 30;

const FIXED_ENVIRONMENT: &[(&str, &str)] = &[
    ("LANG", "C"),
    ("LC_ALL", "C"),
    ("TZ", "UTC"),
    ("PATH", ""),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_NO_REPLACE_OBJECTS", "1"),
    ("GIT_NO_LAZY_FETCH", "1"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_ATTR_NOSYSTEM", "1"),
    ("GIT_CONFIG_COUNT", "0"),
    ("GIT_LITERAL_PATHSPECS", "1"),
    ("GIT_ALLOW_PROTOCOL", ""),
    ("GIT_PROTOCOL_FROM_USER", "0"),
    ("GIT_EDITOR", ":"),
    ("GIT_SEQUENCE_EDITOR", ":"),
];

pub(super) struct HostEnvironment {
    platform_variables: Vec<(String, OsString)>,
}

impl HostEnvironment {
    pub(super) fn read() -> Result<Self> {
        let mut platform_variables = Vec::new();
        #[cfg(windows)]
        {
            let path =
                std::env::var_os("SystemRoot").context("Windows SystemRoot is unavailable")?;
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                bail!("Windows SystemRoot must be absolute");
            }
            let path = std::fs::canonicalize(path)?;
            let pin = crate::private_sqlite::PinnedPrivateDirectory::pin_ancestors(&path)?;
            pin.verify()?;
            platform_variables.push(("SystemRoot".into(), git_path(&path)?));
        }
        #[cfg(not(windows))]
        let _ = &mut platform_variables;
        Ok(Self { platform_variables })
    }

    pub(super) fn apply(&self, command: &mut Command, home: &Path, temp: &Path) -> Result<()> {
        command.env_clear().envs(FIXED_ENVIRONMENT.iter().copied());
        command
            .env("GIT_CONFIG_GLOBAL", null_device())
            .env("GIT_CONFIG_SYSTEM", null_device());
        for name in ["HOME", "USERPROFILE", "XDG_CONFIG_HOME"] {
            command.env(name, git_path(home)?);
        }
        for name in ["TMP", "TEMP", "TMPDIR"] {
            command.env(name, git_path(temp)?);
        }
        for (name, value) in &self.platform_variables {
            command.env(name, value);
        }
        Ok(())
    }

    pub(super) fn fingerprint(
        &self,
        binary: &PinnedNativeExecutable,
        oid_length: usize,
    ) -> Result<String> {
        let variables = self
            .platform_variables
            .iter()
            .map(|(name, value)| {
                Ok((
                    name,
                    value.to_str().context("non-UTF-8 platform environment")?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let executable = git_path(binary.path())?;
        Ok(digest(&serde_json::to_vec(&(
            RECIPE_VERSION,
            std::env::consts::OS,
            std::env::consts::ARCH,
            executable.to_str().context("non-UTF-8 Git executable")?,
            binary.sha256(),
            oid_length,
            FIXED_ENVIRONMENT,
            variables,
            null_device(),
            "HOME/USERPROFILE/XDG_CONFIG_HOME=<private-home>; TMP/TEMP/TMPDIR=<private-temp>",
            configuration_fingerprint(oid_length)?,
            command_fingerprint(binary, "<base-tree>", "<source-tree>")?,
            (
                MAX_PATHS,
                MAX_BLOB_BYTES,
                MAX_TOTAL_BYTES,
                DEADLINE_SECONDS,
                crate::domain::planning::verification::MAX_TASK_VERIFICATION_STREAM_BYTES,
            ),
        ))?))
    }
}

pub(super) fn resolve_binary(context: &Path) -> Result<PinnedNativeExecutable> {
    let path = crate::trusted_executable::resolve_native_from_current_path("git", context)?;
    PinnedNativeExecutable::inspect(&path, context)
}

pub(super) fn configuration(oid_length: usize) -> Result<String> {
    let format = match oid_length {
        40 => "[core]\n\trepositoryformatversion = 0\n",
        64 => {
            "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n[core]\n"
        }
        _ => bail!("unsupported Git object format"),
    };
    Ok(format!(
        "{format}\tbare = false\n\tfilemode = false\n\tlogallrefupdates = false\n\tautocrlf = false\n\tsafecrlf = false\n\twhitespace = blank-at-eol,blank-at-eof,space-before-tab\n"
    ))
}

pub(super) fn configuration_fingerprint(oid_length: usize) -> Result<String> {
    Ok(digest(&serde_json::to_vec(&(
        RECIPE_VERSION,
        configuration(oid_length)?,
        HEAD,
        ATTRIBUTES,
        "empty-index;unborn-head;verified-source-alternate;no-preexisting-alternates",
    ))?))
}

pub(super) fn base_command(
    binary: &PinnedNativeExecutable,
    git_dir: &Path,
    worktree: &Path,
) -> Result<Command> {
    let mut command = crate::git_subprocess::command_with_program(
        binary.path().as_os_str(),
        std::iter::empty::<&str>(),
    );
    append_base_arguments(&mut command, &git_path(git_dir)?, &git_path(worktree)?);
    command.stdin(Stdio::null());
    Ok(command)
}

fn append_base_arguments(command: &mut Command, git_dir: &OsStr, worktree: &OsStr) {
    command
        .arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(worktree)
        .args([
            "-c",
            "core.whitespace=blank-at-eol,blank-at-eof,space-before-tab",
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.safecrlf=false",
        ])
        .stdin(Stdio::null());
}

pub(super) fn append_diff_arguments(command: &mut Command, base: &str, source: &str) {
    command.args([
        "diff",
        "--check",
        "--text",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--no-color",
        "--ignore-submodules=none",
        "--diff-algorithm=myers",
        "--no-indent-heuristic",
        base,
        source,
        "--",
    ]);
}

pub(super) fn command_fingerprint(
    binary: &PinnedNativeExecutable,
    base: &str,
    source: &str,
) -> Result<String> {
    // Placeholders are data for hashing only; this command is never spawned.
    let mut command = crate::git_subprocess::command_with_program(
        binary.path().as_os_str(),
        std::iter::empty::<&str>(),
    );
    append_base_arguments(
        &mut command,
        OsStr::new("<metadata>"),
        OsStr::new("<worktree>"),
    );
    append_diff_arguments(&mut command, base, source);
    let arguments = command
        .get_args()
        .map(|value| value.to_str().context("non-UTF-8 fixed Git argument"))
        .collect::<Result<Vec<_>>>()?;
    Ok(digest(&serde_json::to_vec(&(RECIPE_VERSION, arguments))?))
}

pub(super) fn git_path(path: &Path) -> Result<OsString> {
    #[cfg(windows)]
    {
        crate::trusted_executable::windows_process_argument_path(path)
    }
    #[cfg(not(windows))]
    {
        Ok(path.as_os_str().to_owned())
    }
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn null_device() -> &'static str {
    #[cfg(windows)]
    {
        "NUL"
    }
    #[cfg(not(windows))]
    {
        "/dev/null"
    }
}
