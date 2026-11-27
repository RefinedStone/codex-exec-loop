use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

pub(crate) fn git_command() -> Command {
    git_command_with_inherited(std::env::vars_os())
}

fn git_command_with_inherited(
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
) -> Command {
    let mut command = Command::new("git");
    for (name, _) in inherited {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(name);
        }
    }
    for (name, value) in TEST_GIT_CONFIG_ENVIRONMENT {
        command.env(name, value);
    }
    command
}

/// 시험의 git 에 주는 설정이다. 저장소의 설정 파일을 바꾸지 않고 이 명령에만 적용된다.
const TEST_GIT_CONFIG_ENVIRONMENT: [(&str, &str); 3] = [
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "core.fsmonitor"),
    ("GIT_CONFIG_VALUE_0", "false"),
];

pub(crate) fn platform_safe_temp_dir() -> PathBuf {
    let temp_dir = std::env::temp_dir();
    #[cfg(target_os = "macos")]
    {
        // macOS exposes /var as a symlink to /private/var; NOFOLLOW fixtures need
        // the physical path. Other platforms must retain their native spelling:
        // Windows canonicalization adds a \\?\ prefix that Git for Windows rejects.
        std::fs::canonicalize(&temp_dir).unwrap_or(temp_dir)
    }
    #[cfg(not(target_os = "macos"))]
    temp_dir
}
