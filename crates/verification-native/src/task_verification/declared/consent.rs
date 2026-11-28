//! A host-owned, default-cancel dialog for one already constructed command.
//! The prompt and repository values are never interpolated into program text.
use super::{Command, Instant, PinnedNativeExecutable, Reason, TaskVerificationExecutionManifest};
use akra_native::subprocess::{
    self,
    controlled::{ControlledTermination, wait_with_output_controlled},
};
use std::path::PathBuf;
use std::process::Stdio;

pub(in crate::task_verification) fn confirm(
    recipe: &Command,
    manifest: &TaskVerificationExecutionManifest,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Reason> {
    let cwd = recipe.get_current_dir().ok_or(Reason::InvalidInput)?;
    let argv = std::iter::once(recipe.get_program())
        .chain(recipe.get_args())
        .map(|arg| arg.to_str().ok_or(Reason::InvalidInput))
        .collect::<Result<Vec<_>, _>>()?;
    let details = serde_json::to_string_pretty(&serde_json::json!({
        "argv": argv, "cwd": cwd, "source": manifest.source, "base_tree": manifest.base_tree_sha,
        "executable_sha256": manifest.git_binary_sha256, "environment_sha256": manifest.environment_sha256,
    })).map_err(|_| Reason::InvalidInput)?;
    let text = format!("Run this repository check once?\n\n{details}");
    confirm_text(cwd, &text, deadline, cancelled)
}

pub(crate) fn confirm_text(
    cwd: &std::path::Path,
    text: &str,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), Reason> {
    let candidate = dialog_path()?;
    let pin = PinnedNativeExecutable::inspect(&candidate, cwd).map_err(|_| Reason::Unavailable)?;
    let mut dialog = Command::new(pin.path());
    dialog
        .env_clear()
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure(&mut dialog, text)?;
    if cancelled() || Instant::now() >= deadline {
        return Err(Reason::Cancelled);
    }
    pin.verify().map_err(|_| Reason::ConfigurationChanged)?;
    let child = subprocess::spawn(&mut dialog).map_err(|_| Reason::Unavailable)?;
    let result = wait_with_output_controlled(child, deadline, cancelled, 1024, 1024);
    pin.verify().map_err(|_| Reason::ConfigurationChanged)?;
    match result.termination {
        ControlledTermination::Exited(status)
            if status.code() == Some(APPROVED_EXIT)
                && result.stdout_complete
                && result.stderr_complete =>
        {
            Ok(())
        }
        ControlledTermination::Exited(_)
        | ControlledTermination::Cancelled
        | ControlledTermination::TimedOut => Err(Reason::Cancelled),
        ControlledTermination::OutputLimit | ControlledTermination::IoFailure => {
            Err(Reason::Unavailable)
        }
    }
}

#[cfg(windows)]
const APPROVED_EXIT: i32 = 73;
#[cfg(not(windows))]
const APPROVED_EXIT: i32 = 0;

#[cfg(windows)]
fn dialog_path() -> Result<PathBuf, Reason> {
    Ok(windows_root()?.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
}

#[cfg(windows)]
fn windows_root() -> Result<PathBuf, Reason> {
    let root = windows_registry::LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|key| key.get_string("SystemRoot"))
        .map_err(|_| Reason::Unavailable)?;
    let root = PathBuf::from(root);
    if !root.is_absolute() {
        return Err(Reason::Unavailable);
    }
    Ok(root)
}

#[cfg(windows)]
fn configure(dialog: &mut Command, text: &str) -> Result<(), Reason> {
    // Load only the signed OS assembly. No user profile, module discovery,
    // command lookup, stdin or interpolated PowerShell source participates.
    const PROMPT: &str = r#"$ErrorActionPreference='Stop';$PSModuleAutoLoadingPreference='None';[void][Reflection.Assembly]::Load('System.Windows.Forms, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089');if ([Windows.Forms.MessageBox]::Show($env:AKRA_VERIFY_DISPLAY,'Akra verification','OKCancel','Warning','Button2') -eq 'OK') {exit 73};exit 1"#;
    dialog
        .env("SystemRoot", windows_root()?)
        .env("AKRA_VERIFY_DISPLAY", text)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-Command",
            PROMPT,
        ]);
    Ok(())
}

#[cfg(target_os = "linux")]
fn dialog_path() -> Result<PathBuf, Reason> {
    Ok("/usr/bin/zenity".into())
}
#[cfg(target_os = "linux")]
fn configure(dialog: &mut Command, text: &str) -> Result<(), Reason> {
    for name in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "XAUTHORITY",
    ] {
        if let Some(value) = std::env::var_os(name) {
            dialog.env(name, value);
        }
    }
    dialog
        .args([
            "--question",
            "--default-cancel",
            "--no-markup",
            "--title=Akra verification",
            "--ok-label=Run once",
            "--cancel-label=Cancel",
        ])
        .arg(format!("--text={text}"));
    Ok(())
}

#[cfg(target_os = "macos")]
fn dialog_path() -> Result<PathBuf, Reason> {
    Ok("/usr/bin/osascript".into())
}
#[cfg(target_os = "macos")]
fn configure(dialog: &mut Command, text: &str) -> Result<(), Reason> {
    dialog.args(["-e", r#"on run argv
display dialog (item 1 of argv) with title "Akra verification" buttons {"Run once", "Cancel"} default button "Cancel" cancel button "Cancel"
end run"#, "--", text]);
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn dialog_path() -> Result<PathBuf, Reason> {
    Err(Reason::Unavailable)
}
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn configure(_: &mut Command, _: &str) -> Result<(), Reason> {
    Err(Reason::Unavailable)
}
