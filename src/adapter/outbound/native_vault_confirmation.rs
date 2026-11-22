//! Vault helper가 직접 여는 OS 확인 창이다. 성공한 native child만 승인할 수 있다.
use crate::application::port::outbound::vault_native_confirmation_port::{
    VaultNativeConfirmationOutcome as Outcome, VaultNativeConfirmationPort,
    VaultNativeConfirmationRequest,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
pub(crate) struct NativeVaultConfirmationAdapter;
impl VaultNativeConfirmationPort for NativeVaultConfirmationAdapter {
    fn confirm(
        &self,
        request: &VaultNativeConfirmationRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> Outcome {
        if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Outcome::Cancelled;
        }
        if !request.issue_url.starts_with("https://github.com/")
            || request.issue_url.len() > 2048
            || request.workspace_dir.is_empty()
            || request.workspace_dir.len() > 4096
        {
            return Outcome::Unavailable;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            platform::confirm(request, cancelled, deadline)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = request;
            Outcome::Unsupported
        }
    }
}
#[cfg(any(target_os = "linux", target_os = "macos", test))]
mod platform {
    use super::*;
    use std::ffi::OsString;
    #[cfg(target_os = "macos")]
    use std::io::Write;
    use std::path::Path;
    use std::process::{Command, Stdio};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::time::Duration;

    // 요청 내용은 argv 데이터이다. AppleScript 소스에 문자열을 보간하지 않는다.
    #[cfg(any(target_os = "macos", test))]
    const APPLE_SCRIPT: &str = r#"on run argv
set promptText to item 1 of argv
set answer to display dialog promptText with title "Akra 원문 열람" buttons {"원문 열람", "취소"} default button "취소" cancel button "취소" with icon caution giving up after (item 2 of argv as integer)
if gave up of answer then error number -128
if button returned of answer is not "원문 열람" then error number -128
end run
"#;
    #[derive(Clone, Copy)]
    enum DialogPlatform {
        #[cfg(any(target_os = "macos", test))]
        Mac,
        #[cfg(any(target_os = "linux", test))]
        Linux,
    }
    fn prompt(request: &VaultNativeConfirmationRequest) -> String {
        format!(
            "이 이슈의 가려진 원문을 로컬 열람 세션으로 전달할까요?\n\n{}\n프로젝트: {}\n\n최대 5분 동안 열람할 수 있습니다. 같은 OS 계정의 화면 캡처나 접근성 도구는 원문을 볼 수 있습니다.",
            request.issue_url, request.workspace_dir
        )
    }
    fn command_for(
        platform: DialogPlatform,
        program: &Path,
        request: &VaultNativeConfirmationRequest,
        seconds: u64,
        environment: impl Fn(&str) -> Option<OsString>,
    ) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .current_dir("/")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // ZENITY_OK/CANCEL, loader/plugin 설정과 승인 override는 절대 상속하지 않는다.
        for name in [
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
            "XAUTHORITY",
        ] {
            if let Some(value) = environment(name).filter(|value| {
                value.to_str().is_some_and(|text| {
                    !text.is_empty() && text.len() <= 4096 && !text.chars().any(char::is_control)
                })
            }) {
                command.env(name, value);
            }
        }
        match platform {
            #[cfg(any(target_os = "macos", test))]
            DialogPlatform::Mac => {
                command
                    .arg("-")
                    .arg(prompt(request))
                    .arg(seconds.to_string())
                    .stdin(Stdio::piped());
            }
            #[cfg(any(target_os = "linux", test))]
            DialogPlatform::Linux => {
                command.args([
                    "--question",
                    "--title=Akra 원문 열람",
                    "--default-cancel",
                    "--no-markup",
                    "--ok-label=원문 열람",
                    "--cancel-label=취소",
                ]);
                command
                    .arg(format!("--text={}", prompt(request)))
                    .arg(format!("--timeout={seconds}"))
                    .stdin(Stdio::null());
            }
        }
        command
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn confirm(
        request: &VaultNativeConfirmationRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> Outcome {
        #[cfg(target_os = "macos")]
        let (platform, candidate) = (DialogPlatform::Mac, "/usr/bin/osascript");
        #[cfg(target_os = "linux")]
        let (platform, candidate) = (DialogPlatform::Linux, "/usr/bin/zenity");
        let Ok(cwd) = std::env::current_dir() else {
            return Outcome::Unavailable;
        };
        let Ok(program) = crate::trusted_executable::validate_absolute(Path::new(candidate), &cwd)
            .and_then(|program| {
                crate::trusted_executable::validate_native_executable(&program)?;
                Ok(program)
            })
        else {
            return Outcome::Unsupported;
        };
        let mut command = command_for(
            platform,
            &program,
            request,
            deadline
                .saturating_duration_since(Instant::now())
                .as_secs()
                .clamp(1, 300),
            |name| std::env::var_os(name),
        );
        if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Outcome::Cancelled;
        }
        let Ok(mut child) = crate::subprocess::spawn(&mut command) else {
            return Outcome::Unavailable;
        };
        #[cfg(target_os = "macos")]
        {
            let sent = child
                .take_stdin()
                .is_some_and(|mut input| input.write_all(APPLE_SCRIPT.as_bytes()).is_ok());
            if !sent {
                let _ = child.terminate_and_wait();
                return Outcome::Unavailable;
            }
        }
        loop {
            if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
                let _ = child.terminate_and_wait();
                return Outcome::Cancelled;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
                        return Outcome::Cancelled;
                    }
                    return if status.success() {
                        Outcome::Confirmed
                    } else {
                        Outcome::Cancelled
                    };
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(_) => {
                    let _ = child.terminate_and_wait();
                    return Outcome::Unavailable;
                }
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        fn request() -> VaultNativeConfirmationRequest {
            VaultNativeConfirmationRequest {
                issue_url: "https://github.com/acme/project/issues/1".into(),
                workspace_dir: "/work/project".into(),
            }
        }
        #[test]
        fn linux_prompt_defaults_to_cancel_and_never_inherits_approval_or_loader_variables() {
            let command = command_for(
                DialogPlatform::Linux,
                Path::new("/usr/bin/zenity"),
                &request(),
                300,
                |name| {
                    Some(OsString::from(if name == "LANG" {
                        "ko_KR.UTF-8"
                    } else {
                        "test"
                    }))
                },
            );
            let args = command
                .get_args()
                .map(|value| value.to_string_lossy())
                .collect::<Vec<_>>();
            assert!(args.iter().any(|value| value == "--default-cancel"));
            assert!(args.iter().any(|value| value == "--no-markup"));
            let names = command
                .get_envs()
                .map(|(name, _)| name.to_string_lossy())
                .collect::<Vec<_>>();
            for forbidden in [
                "ZENITY_OK",
                "ZENITY_CANCEL",
                "LD_PRELOAD",
                "DYLD_INSERT_LIBRARIES",
                "GTK_MODULES",
                "AKRA_VAULT_APPROVED",
            ] {
                assert!(!names.iter().any(|name| name == forbidden));
            }
            assert!(command.get_envs().any(|(name, _)| name == "LANG"));
        }
        #[test]
        fn mac_script_is_static_and_target_is_a_separate_argument() {
            let mut target = request();
            target.workspace_dir = "\" & do shell script \"untrusted\" & \"".into();
            let command = command_for(
                DialogPlatform::Mac,
                Path::new("/usr/bin/osascript"),
                &target,
                300,
                |_| None,
            );
            let args = command
                .get_args()
                .map(|value| value.to_string_lossy())
                .collect::<Vec<_>>();
            assert_eq!(args[0], "-");
            assert!(args[1].contains(&target.workspace_dir));
            assert!(!APPLE_SCRIPT.contains(&target.workspace_dir));
            assert!(APPLE_SCRIPT.contains("default button \"취소\" cancel button \"취소\""));
        }
    }
}
