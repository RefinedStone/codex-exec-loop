//! Only native UI can approve. No argv, RPC field or environment variable supplies the response.
#[cfg(windows)]
use anyhow::Context;
use anyhow::{Result, ensure};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub(super) fn confirm(message: &str, expected: &str) -> Result<()> {
    ensure!(
        !expected.is_empty() && expected.len() <= 512 && !expected.chars().any(char::is_control),
        "invalid confirmation target"
    );
    let prompt = format!(
        "{}\n\n계속하려면 다음 글자를 정확히 입력하십시오: {}",
        message, expected
    );
    #[cfg(windows)]
    let (path, args) = {
        let system = std::env::var_os("SystemRoot").context("missing Windows system directory")?;
        let script = "Add-Type -AssemblyName Microsoft.VisualBasic; $s=[Microsoft.VisualBasic.Interaction]::InputBox($env:AKRA_GIT_PROMPT, 'Akra Git 확인', ''); [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false); [Console]::Write($s)";
        (
            PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            vec![
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-STA",
                "-Command",
                script,
            ],
        )
    };
    #[cfg(target_os = "macos")]
    let (path, args) = (
        PathBuf::from("/usr/bin/osascript"),
        vec![
            "-e",
            "on run argv\nset a to display dialog (item 1 of argv) default answer \"\" with title \"Akra Git\" buttons {\"취소\", \"확인\"} default button \"취소\" cancel button \"취소\" giving up after 90\nif gave up of a then error number -128\nreturn text returned of a\nend run",
            prompt.as_str(),
        ],
    );
    #[cfg(target_os = "linux")]
    let (path, args) = (
        PathBuf::from("/usr/bin/zenity"),
        vec![
            "--entry",
            "--no-markup",
            "--title=Akra Git",
            "--timeout=90",
            "--text",
            prompt.as_str(),
        ],
    );
    let path = crate::trusted_executable::validate_absolute(&path, &std::env::current_dir()?)?;
    crate::trusted_executable::validate_native_executable(&path)?;
    let mut command = Command::new(path);
    command
        .env_clear()
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "SystemRoot",
        "WINDIR",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "XAUTHORITY",
        "LANG",
    ] {
        if let Some(v) = std::env::var_os(key) {
            command.env(key, v);
        }
    }
    #[cfg(windows)]
    command.env("AKRA_GIT_PROMPT", &prompt);
    let child = crate::subprocess::spawn(&mut command)?;
    let result = crate::subprocess::wait_with_output_timeout_and_limits(
        child,
        "operator Git confirmation",
        std::time::Duration::from_secs(90),
        2048,
        2048,
    )?;
    ensure!(
        result.status.success()
            && std::str::from_utf8(&result.stdout)?.trim_end_matches(['\r', '\n']) == expected,
        "operation cancelled or native confirmation unavailable"
    );
    Ok(())
}
