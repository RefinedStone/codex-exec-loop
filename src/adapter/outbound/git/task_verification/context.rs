use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Instant;

use anyhow::{Context, Result, bail};

use super::environment::{self, HostEnvironment, git_path};
use crate::adapter::outbound::filesystem::task_evidence_artifact::files::{
    StableFile, TemporaryFile,
};
use crate::private_sqlite::PinnedPrivateDirectory;
use crate::subprocess::controlled::{ControlledTermination, wait_with_output_controlled};
use crate::trusted_executable::PinnedNativeExecutable;

const HOME: usize = 0;
const TEMP: usize = 1;
const WORKTREE: usize = 2;
const METADATA: usize = 3;
const INFO: usize = 4;
const OBJECT_INFO: usize = 6;
const MAX_METADATA_BYTES: usize = 256 * 1024;

pub(super) struct PrivateGitContext {
    pub(super) source_head: String,
    root: PathBuf,
    pins: Vec<PinnedPrivateDirectory>,
    files: Vec<(usize, StableFile, Vec<u8>)>,
    source_objects: Option<PathBuf>,
    environment: HostEnvironment,
}

impl PrivateGitContext {
    pub(super) fn new(
        workspace: &Path,
        oid_length: usize,
        binary: &PinnedNativeExecutable,
        environment: HostEnvironment,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self> {
        let root = crate::adapter::outbound::github::automation::create_private_temp_directory(
            "akra-task-verification",
        );
        #[cfg(test)]
        if let Err(error) = &root {
            super::diagnostics::report("allocate_private_context", error);
        }
        let root = root?;
        let root = fs::canonicalize(root)?;
        let pin = PinnedPrivateDirectory::open(&root, false)?;
        let home = pin.child("home", true)?;
        let temp = pin.child("tmp", true)?;
        let worktree = pin.child("worktree", true)?;
        let metadata = pin.child("metadata", true)?;
        let info = metadata.child("info", true)?;
        let objects = metadata.child("objects", true)?;
        let object_info = objects.child("info", true)?;
        let refs = metadata.child("refs", true)?.child("heads", true)?;
        let mut context = Self {
            source_head: String::new(),
            root,
            pins: vec![
                home,
                temp,
                worktree,
                metadata,
                info,
                objects,
                object_info,
                refs,
                pin,
            ],
            files: Vec::new(),
            source_objects: None,
            environment,
        };
        context.install_file(
            METADATA,
            "config",
            environment::configuration(oid_length)?.as_bytes(),
        )?;
        context.install_file(METADATA, "HEAD", environment::HEAD)?;
        context.install_file(INFO, "attributes", environment::ATTRIBUTES)?;

        // Only metadata/config inspection uses the selected source repository. The
        // recipe itself receives the fixed private config, unborn HEAD and empty index.
        let audit = context.source_output(
            binary,
            workspace,
            &[
                "config",
                "--includes",
                "--null",
                "--name-only",
                "--get-regexp",
                crate::git_execution_guard::EXECUTABLE_CONFIG_SECTION_REGEX,
            ],
            deadline,
            cancelled,
        )?;
        if !audit.status.success() && audit.status.code() != Some(1) {
            bail!("source Git configuration audit failed");
        }
        crate::git_execution_guard::validate_host_git_execution_config_keys(&audit.stdout)?;
        let top = context.source_text(
            binary,
            workspace,
            &["rev-parse", "--show-toplevel"],
            deadline,
            cancelled,
        )?;
        if fs::canonicalize(top)? != fs::canonicalize(workspace)? {
            bail!("verification source is not an exact worktree root");
        }
        context.source_head = context.source_text(
            binary,
            workspace,
            &["rev-parse", "--verify", "HEAD"],
            deadline,
            cancelled,
        )?;
        let format = context.source_text(
            binary,
            workspace,
            &["rev-parse", "--show-object-format"],
            deadline,
            cancelled,
        )?;
        if format != if oid_length == 40 { "sha1" } else { "sha256" } {
            bail!("source Git object format changed");
        }
        let common = context.source_text(
            binary,
            workspace,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            deadline,
            cancelled,
        )?;
        let common = PathBuf::from(common);
        if !common.is_absolute() || fs::symlink_metadata(&common)?.file_type().is_symlink() {
            bail!("source common Git directory is not canonical metadata");
        }
        let common = fs::canonicalize(common)?;
        let objects = common.join("objects");
        validate_source_objects(&objects)?;
        context
            .pins
            .push(PinnedPrivateDirectory::pin_ancestors(&fs::canonicalize(
                workspace,
            )?)?);
        context
            .pins
            .push(PinnedPrivateDirectory::pin_ancestors(&common)?);
        context
            .pins
            .push(PinnedPrivateDirectory::pin_ancestors(&objects)?);
        context.pins.push(PinnedPrivateDirectory::pin_ancestors(
            &objects.join("info"),
        )?);
        let label = git_path(&objects)?;
        let label = label
            .to_str()
            .context("non-UTF-8 source object directory")?;
        if label.chars().any(char::is_control) {
            bail!("invalid source object directory");
        }
        context.source_objects = Some(objects);
        context.install_file(OBJECT_INFO, "alternates", format!("{label}\n").as_bytes())?;
        context.verify()?;
        Ok(context)
    }

    fn install_file(&mut self, directory: usize, name: &str, bytes: &[u8]) -> Result<()> {
        let pin = &self.pins[directory];
        {
            let mut temporary = TemporaryFile::create(pin, "prepare")?;
            temporary.write_all(bytes)?;
            temporary.publish_noreplace(name)?;
        }
        let mut file = StableFile::open(pin, name)?;
        if file.read_bounded(pin, MAX_METADATA_BYTES as u64)? != bytes {
            bail!("private Git metadata bytes changed");
        }
        self.files.push((directory, file, bytes.to_vec()));
        Ok(())
    }

    pub(super) fn verify(&mut self) -> Result<()> {
        for pin in &self.pins {
            pin.verify()?;
        }
        for (directory, file, expected) in &mut self.files {
            if file.read_bounded(&self.pins[*directory], MAX_METADATA_BYTES as u64)? != *expected {
                bail!("private Git execution configuration changed");
            }
        }
        // An absent index is Git's empty index for this unborn repository. No
        // command writes it, and no source index or worktree is consulted.
        require_missing(&self.pins[METADATA].path().join("index"))?;
        if fs::read_dir(self.pins[WORKTREE].path())?.next().is_some() {
            bail!("private verification worktree is no longer empty");
        }
        if let Some(objects) = &self.source_objects {
            validate_source_objects(objects)?;
        }
        Ok(())
    }

    pub(super) fn command(&self, binary: &PinnedNativeExecutable) -> Result<Command> {
        let mut command = environment::base_command(
            binary,
            self.pins[METADATA].path(),
            self.pins[WORKTREE].path(),
        )?;
        self.configure(&mut command)?;
        command.env(
            "GIT_INDEX_FILE",
            git_path(&self.pins[METADATA].path().join("index"))?,
        );
        Ok(command)
    }

    fn configure(&self, command: &mut Command) -> Result<()> {
        self.environment
            .apply(command, self.pins[HOME].path(), self.pins[TEMP].path())?;
        command.current_dir(self.pins[WORKTREE].path());
        Ok(())
    }

    pub(super) fn environment_fingerprint(
        &self,
        binary: &PinnedNativeExecutable,
        oid_length: usize,
    ) -> Result<String> {
        self.environment.fingerprint(binary, oid_length)
    }

    pub(super) fn output(
        &mut self,
        binary: &PinnedNativeExecutable,
        arguments: &[&str],
        limit: usize,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Output> {
        self.verify()?;
        let mut command = self.command(binary)?;
        command.args(arguments);
        let output = checked_output(binary, &mut command, limit, deadline, cancelled)?;
        self.verify()?;
        if !output.status.success() {
            bail!("fixed verification Git inspection failed");
        }
        Ok(output)
    }

    fn source_output(
        &self,
        binary: &PinnedNativeExecutable,
        workspace: &Path,
        arguments: &[&str],
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Output> {
        let mut command = crate::git_subprocess::command_with_program(
            binary.path().as_os_str(),
            std::iter::empty::<&str>(),
        );
        command.arg("-C").arg(git_path(workspace)?).args(arguments);
        self.configure(&mut command)?;
        checked_output(
            binary,
            &mut command,
            MAX_METADATA_BYTES,
            deadline,
            cancelled,
        )
    }

    fn source_text(
        &self,
        binary: &PinnedNativeExecutable,
        workspace: &Path,
        arguments: &[&str],
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String> {
        let output = self.source_output(binary, workspace, arguments, deadline, cancelled)?;
        if !output.status.success() {
            bail!("source Git metadata inspection failed");
        }
        text_line(&output.stdout)
    }
}

impl Drop for PrivateGitContext {
    fn drop(&mut self) {
        let safe = self.pins.last().is_some_and(|pin| pin.verify().is_ok())
            && self
                .pins
                .get(METADATA)
                .is_some_and(|pin| pin.verify().is_ok());
        self.files.clear();
        self.pins.clear();
        if safe {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn checked_output(
    binary: &PinnedNativeExecutable,
    command: &mut Command,
    limit: usize,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<Output> {
    if cancelled() || Instant::now() >= deadline {
        bail!("verification preparation stopped");
    }
    binary.verify_identity()?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = wait_with_output_controlled(
        crate::subprocess::spawn(command)?,
        deadline,
        cancelled,
        limit,
        64 * 1024,
    );
    binary.verify_identity()?;
    let ControlledTermination::Exited(status) = output.termination else {
        bail!("verification inspection exceeded its process contract");
    };
    Ok(Output {
        status,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

pub(super) fn text_line(bytes: &[u8]) -> Result<String> {
    let value = std::str::from_utf8(bytes)?.trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.chars().any(char::is_control) {
        bail!("invalid Git metadata output");
    }
    Ok(value.into())
}

fn require_missing(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(_) => bail!("verification rejects preexisting routing or index metadata"),
    }
}

fn validate_source_objects(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("source objects must be a real directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } {
            bail!("source objects are not owned by this user");
        }
    }
    #[cfg(windows)]
    {
        use crate::private_fs::*;
        use std::os::windows::fs::OpenOptionsExt;
        let file = fs::OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?;
        validate_windows_path_identity(path, &file, true)?;
    }
    for file in ["alternates", "http-alternates"] {
        require_missing(&path.join("info").join(file))?;
    }
    Ok(())
}
