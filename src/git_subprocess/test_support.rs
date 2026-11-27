//! Root libtest preserves its existing executable fixture selection and timing hooks.
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(not(test))]
use std::sync::OnceLock;

use akra_verification_native::git_command::append_hardening_arguments;

pub(crate) fn command<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    command_with_program(OsStr::new("git"), args)
}

pub(crate) fn command_with_program<I, S>(program: &OsStr, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    #[cfg(test)]
    let _timer = crate::test_utils::NativeTestTimer::new("git_prepare");
    let resolved_program = if is_git_program(program) {
        resolve_git_program(program)
    } else {
        Ok(PathBuf::from(program))
    };
    // Keep trust validation canonical; Git for Windows needs ordinary drive/UNC spelling when
    // locating its own installation configuration from the executable path.
    #[cfg(windows)]
    let resolved_program = resolved_program.and_then(|program| {
        crate::trusted_executable::windows_process_argument_path(&program).map(PathBuf::from)
    });
    let mut command = match resolved_program {
        Ok(program) => Command::new(program),
        Err(error) => {
            tracing::error!(error = %error, "trusted host Git executable resolution failed");
            Command::new(fail_closed_git_program())
        }
    };
    harden_command(&mut command, std::env::vars_os());
    append_hardening_arguments(&mut command);
    command.args(args).stdin(Stdio::null());
    command
}

fn resolve_git_program(program: &OsStr) -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    if Path::new(program).is_absolute() {
        #[cfg(test)]
        return Ok(PathBuf::from(program));
        #[cfg(not(test))]
        return crate::trusted_executable::validate_absolute(Path::new(program), &cwd);
    }
    trusted_git_from_environment(&cwd)
}

#[cfg(test)]
fn trusted_git_from_environment(cwd: &Path) -> anyhow::Result<PathBuf> {
    crate::trusted_executable::resolve_native_from_current_path("git", cwd)
}

#[cfg(not(test))]
fn trusted_git_from_environment(cwd: &Path) -> anyhow::Result<PathBuf> {
    static PINNED_GIT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    PINNED_GIT
        .get_or_init(|| {
            crate::trusted_executable::resolve_native_from_current_path("git", cwd)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

#[cfg(unix)]
fn fail_closed_git_program() -> &'static Path {
    Path::new("/__akra_untrusted_git_executable__")
}

#[cfg(windows)]
fn fail_closed_git_program() -> &'static Path {
    Path::new(r"C:\__akra_untrusted_git_executable__.exe")
}

pub(crate) fn command_for_program<I, S>(program: &str, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    if is_git_program(OsStr::new(program)) {
        command_with_program(OsStr::new(program), args)
    } else {
        #[cfg(not(test))]
        let program = {
            let path = Path::new(program);
            std::env::current_dir()
                .ok()
                .filter(|_| path.is_absolute())
                .and_then(|cwd| crate::trusted_executable::validate_absolute(path, &cwd).ok())
                .filter(|path| crate::trusted_executable::validate_native_executable(path).is_ok())
                .unwrap_or_else(|| fail_closed_generic_program().to_path_buf())
        };
        let mut command = Command::new(program);
        command.args(args);
        command
    }
}

#[cfg(all(not(test), unix))]
fn fail_closed_generic_program() -> &'static Path {
    Path::new("/__akra_untrusted_host_executable__")
}

#[cfg(all(not(test), windows))]
fn fail_closed_generic_program() -> &'static Path {
    Path::new(r"C:\__akra_untrusted_host_executable__.exe")
}

pub(crate) fn is_git_program(program: &OsStr) -> bool {
    let Some(file_name) = Path::new(program).file_name().and_then(OsStr::to_str) else {
        return false;
    };
    file_name.eq_ignore_ascii_case("git") || file_name.eq_ignore_ascii_case("git.exe")
}

fn harden_command(
    command: &mut Command,
    inherited_environment: impl IntoIterator<Item = (OsString, OsString)>,
) {
    let _timer = crate::test_utils::NativeTestTimer::new("git_harden");
    akra_verification_native::git_command::harden_command(command, inherited_environment);
}
