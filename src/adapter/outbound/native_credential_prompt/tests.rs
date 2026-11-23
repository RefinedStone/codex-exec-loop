use super::*;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
fn request() -> ConnectRequest {
    ConnectRequest {
        operation_id: "synthetic-prompt".into(),
        profile_id: "a".repeat(64),
        provider: Provider::Linear,
        expected_revision: 0,
        configuration: ConnectionConfiguration::default(),
    }
}
#[test]
fn prompt_command_has_one_fixed_argument_and_no_secret_environment() {
    let command = child::prompt_command(Path::new("C:/synthetic/akra.exe"), |name| {
        if name == "API_TOKEN" || name == "LD_PRELOAD" {
            Some("synthetic-canary-secret".into())
        } else if name == "SystemRoot" {
            Some("C:/Windows".into())
        } else {
            None
        }
    });
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![std::ffi::OsStr::new("provider-credential-prompt")]
    );
    assert!(command.get_envs().all(|(_, value)| {
        value.is_none_or(|value| !value.to_string_lossy().contains("synthetic-canary-secret"))
    }));
}
#[test]
fn secret_reply_must_match_operation_and_have_no_trailing_bytes() {
    let wire = PromptRequest::Token {
        version: 1,
        request: request(),
    };
    let mut bytes = Vec::new();
    frames::write_header(
        &mut bytes,
        &PromptResponse::Submitted {
            version: 1,
            operation_id: "synthetic-prompt".into(),
        },
    )
    .unwrap();
    let token =
        SecretToken::from_bytes(Provider::Linear, b"synthetic-canary-secret".to_vec()).unwrap();
    frames::write_secret(&mut bytes, &token).unwrap();
    match decode_response(&wire, &bytes).unwrap() {
        PromptOutcome::Token(actual) => assert_eq!(actual.expose(), token.expose()),
        _ => panic!("expected a private token frame"),
    }
    bytes.push(0);
    assert!(decode_response(&wire, &bytes).is_err());
    let other = PromptRequest::Token {
        version: 1,
        request: ConnectRequest {
            operation_id: "other".into(),
            ..request()
        },
    };
    assert!(decode_response(&other, &bytes[..bytes.len() - 1]).is_err());
}
#[test]
fn cancellation_prevents_launch_and_noncanonical_metadata_is_rejected() {
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        NativeCredentialPromptAdapter
            .prompt(
                &request(),
                &cancelled,
                Instant::now() + Duration::from_secs(1)
            )
            .err(),
        Some(ProviderCredentialError::Cancelled)
    );
    let mut invalid = request();
    invalid.configuration.site_url = Some("https://not-used.example".into());
    assert_eq!(
        validate(&invalid),
        Err(ProviderCredentialError::InvalidRequest)
    );
}
#[test]
fn policy_names_background_access_and_requires_organization_permission() {
    let text = native::policy_text(&request(), true);
    assert!(text.contains("회사·조직의 데이터 정책"));
    assert!(text.contains("Desktop이 꺼져 있어도"));
    assert!(native::token_text(&request()).contains("API 토큰"));
}

fn echo_command() -> std::process::Command {
    #[cfg(windows)]
    let mut command = {
        let program = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = std::process::Command::new(program);
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::OpenStandardInput().CopyTo([Console]::OpenStandardOutput())",
        ]);
        command
    };
    #[cfg(unix)]
    let mut command = std::process::Command::new("/bin/cat");
    child::safe_environment(&mut command, |name| std::env::var_os(name));
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    command
}
// Error-path fixtures keep their leader alive after stdin closes or cat gets SIGPIPE.
// This makes the test exercise forced cleanup rather than racing natural process exit.
fn live_owner_echo_command() -> std::process::Command {
    #[cfg(unix)]
    {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "/bin/cat; exec /bin/sleep 30"]);
        child::safe_environment(&mut command, |name| std::env::var_os(name));
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        command
    }
    #[cfg(windows)]
    {
        echo_command()
    }
}

#[test]
fn synthetic_child_echo_is_private_bounded_and_requires_normal_exit() {
    let cancelled = AtomicBool::new(false);
    let input = b"synthetic-child-pipe-canary";
    let mut command = echo_command();
    assert!(command.get_args().all(|arg| {
        !arg.to_string_lossy()
            .contains("synthetic-child-pipe-canary")
    }));
    assert!(
        command
            .get_envs()
            .all(|(_, value)| value.is_none_or(|value| !value
                .to_string_lossy()
                .contains("synthetic-child-pipe-canary")))
    );
    let output = child::capture(
        &mut command,
        input.to_vec(),
        false,
        &cancelled,
        Instant::now() + Duration::from_secs(10),
        128,
    )
    .unwrap();
    assert_eq!(output.as_slice(), input);
    let mut command = live_owner_echo_command();
    assert_eq!(
        child::capture(
            &mut command,
            vec![b'x'; 1024],
            false,
            &cancelled,
            Instant::now() + Duration::from_secs(10),
            16
        )
        .err(),
        Some(ProviderCredentialError::TooLarge)
    );
}
#[test]
fn synthetic_child_timeout_kills_and_reaps_an_open_input_owner() {
    let cancelled = AtomicBool::new(false);
    let mut command = live_owner_echo_command();
    assert_eq!(
        child::capture(
            &mut command,
            Vec::new(),
            true,
            &cancelled,
            Instant::now() + Duration::from_millis(100),
            16
        )
        .err(),
        Some(ProviderCredentialError::Cancelled)
    );
}
