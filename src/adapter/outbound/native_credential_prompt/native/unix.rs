#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::*;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use zeroize::Zeroizing;

#[cfg(any(target_os = "macos", test))]
const MAC_CONFIRM: &str = r#"on run argv
set answer to display dialog (item 1 of argv) with title "Akra 회사 데이터 정책 확인" buttons {"동의하고 계속", "취소"} default button "취소" cancel button "취소" with icon caution giving up after (item 2 of argv as integer)
if gave up of answer then error number -128
if button returned of answer is not "동의하고 계속" then error number -128
return ""
end run
"#;
#[cfg(any(target_os = "macos", test))]
const MAC_INPUT: &str = r#"on run argv
set answer to display dialog (item 1 of argv) with title "Akra API 토큰 입력" default answer "" with hidden answer buttons {"연결", "취소"} default button "취소" cancel button "취소" giving up after (item 2 of argv as integer)
if gave up of answer then error number -128
if button returned of answer is not "연결" then error number -128
return text returned of answer
end run
"#;
#[derive(Clone, Copy)]
enum DialogPlatform {
    #[cfg(any(target_os = "macos", test))]
    Mac,
    #[cfg(any(target_os = "linux", test))]
    Linux,
}
fn command_for(
    platform: DialogPlatform,
    program: &Path,
    text: &str,
    seconds: u64,
    input: bool,
    environment: impl Fn(&str) -> Option<OsString>,
) -> (Command, Vec<u8>) {
    let mut command = Command::new(program);
    super::super::child::safe_environment(&mut command, environment);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir("/");
    let script = match platform {
        #[cfg(any(target_os = "macos", test))]
        DialogPlatform::Mac => {
            command.arg("-").arg(text).arg(seconds.to_string());
            if input {
                MAC_INPUT.as_bytes().to_vec()
            } else {
                MAC_CONFIRM.as_bytes().to_vec()
            }
        }
        #[cfg(any(target_os = "linux", test))]
        DialogPlatform::Linux => {
            command.arg(if input { "--entry" } else { "--question" });
            if input {
                command.arg("--hide-text");
            }
            command
                .args(["--default-cancel", "--no-markup", "--cancel-label=취소"])
                .arg(if input {
                    "--title=Akra API 토큰 입력"
                } else {
                    "--title=Akra 회사 데이터 정책 확인"
                })
                .arg(if input {
                    "--ok-label=연결"
                } else {
                    "--ok-label=동의하고 계속"
                })
                .arg(format!("--text={text}"))
                .arg(format!("--timeout={seconds}"));
            Vec::new()
        }
    };
    (command, script)
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn dialog(
    text: &str,
    input: bool,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> CredentialResult<Zeroizing<Vec<u8>>> {
    #[cfg(target_os = "macos")]
    let (platform, candidate) = (DialogPlatform::Mac, "/usr/bin/osascript");
    #[cfg(target_os = "linux")]
    let (platform, candidate) = (DialogPlatform::Linux, "/usr/bin/zenity");
    let cwd = std::env::current_dir().map_err(|_| ProviderCredentialError::Unavailable)?;
    let program = crate::trusted_executable::validate_absolute(Path::new(candidate), &cwd)
        .and_then(|program| {
            crate::trusted_executable::validate_native_executable(&program)?;
            Ok(program)
        })
        .map_err(|_| ProviderCredentialError::Unsupported)?;
    let (mut command, script) = command_for(
        platform,
        &program,
        text,
        deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
            .clamp(1, 300),
        input,
        |name| std::env::var_os(name),
    );
    super::super::child::capture(
        &mut command,
        script,
        false,
        cancelled,
        deadline,
        if input {
            crate::private_ipc::credential_frames::MAX_SECRET_BYTES + 2
        } else {
            1024
        },
    )
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn confirm(
    text: &str,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> CredentialResult<bool> {
    match dialog(text, false, cancelled, deadline) {
        Ok(_) => Ok(true),
        Err(ProviderCredentialError::Cancelled) => Ok(false),
        Err(error) => Err(error),
    }
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn prompt(
    provider: Provider,
    text: &str,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> CredentialResult<SecretToken> {
    let mut bytes = dialog(text, true, cancelled, deadline)?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    SecretToken::from_bytes(provider, std::mem::take(&mut *bytes))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_scripts_are_static_hidden_and_commands_do_not_inherit_secret_variables() {
        for platform in [DialogPlatform::Mac, DialogPlatform::Linux] {
            let (command, script) = command_for(
                platform,
                Path::new("/usr/bin/example"),
                "public target",
                30,
                true,
                |name| {
                    if name == "LANG" {
                        Some("ko_KR.UTF-8".into())
                    } else {
                        None
                    }
                },
            );
            let args = command
                .get_args()
                .map(|v| v.to_string_lossy())
                .collect::<Vec<_>>();
            assert!(!args.iter().any(|v| v.contains("synthetic-canary-secret")));
            assert!(
                command.get_envs().all(|(key, _)| key != "API_TOKEN"
                    && key != "LD_PRELOAD"
                    && key != "ZENITY_OK")
            );
            if matches!(platform, DialogPlatform::Mac) {
                assert!(
                    String::from_utf8(script)
                        .unwrap()
                        .contains("with hidden answer")
                );
                assert!(!MAC_INPUT.contains("public target"));
            } else {
                assert!(args.iter().any(|v| v == "--hide-text"));
                assert!(args.iter().any(|v| v == "--default-cancel"));
            }
        }
    }
}
