use serde_json::Value;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/*
 * 시험의 git 명령이다. 시험을 돌린 셸의 `GIT_*` 를 물려받지 않는다. git hook 이나 일부 에이전트 환경은 `GIT_DIR`
 * 같은 값을 내보낸다. 그 값이 남으면 `git -C <시험 폴더>` 도 시험 폴더가 아니라 그 저장소를 가리켜서, 시험의
 * `init`, `config user.*`, `commit`, `reset` 이 실제 작업 저장소에 실행된다. 이 저장소의 커밋 123 개가 시험 신원
 * (`Akra Test <akra@example.invalid>`)으로 나간 원인이다. production 은 `git_subprocess` 가 같은 규칙으로 막는다.
 *
 * 파일 감시(`core.fsmonitor`)도 끈다. 개발자의 전역 git 설정에 그 값이 켜져 있으면 시험이 만드는 임시 저장소마다
 * `git fsmonitor--daemon` 이 뜨고, 임시 폴더를 지운 뒤에도 남는다. 한 개발 기기에 데몬이 약 970개 쌓여 git 명령이
 * 멈췄다. production 의 git 도 같은 값을 끈다 (`git_subprocess`).
 */
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

pub(crate) fn process_environment_mutex() -> &'static Mutex<()> {
    static PROCESS_ENVIRONMENT_MUTEX: Mutex<()> = Mutex::new(());
    &PROCESS_ENVIRONMENT_MUTEX
}

pub(crate) fn json_payload_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value.contains(needle),
        Value::Array(values) => values
            .iter()
            .any(|value| json_payload_contains(value, needle)),
        Value::Object(values) => values
            .values()
            .any(|value| json_payload_contains(value, needle)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_commands_drop_inherited_repository_routing() {
        let inherited = [
            ("GIT_DIR", "/real/work/repository/.git"),
            ("Git_Work_Tree", "/real/work/repository"),
            ("GIT_INDEX_FILE", "/real/work/repository/.git/index"),
            ("GIT_CONFIG_GLOBAL", "/real/home/.gitconfig"),
            ("HOME", "/home/operator"),
        ]
        .map(|(name, value)| (OsString::from(name), OsString::from(value)));

        let command = git_command_with_inherited(inherited);
        let removed = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        for routing in [
            "GIT_DIR",
            "Git_Work_Tree",
            "GIT_INDEX_FILE",
            "GIT_CONFIG_GLOBAL",
        ] {
            assert!(
                removed.contains(&routing.to_string()),
                "{routing} must not reach test git"
            );
        }
        assert!(
            !removed.contains(&"HOME".to_string()),
            "test git keeps the user's home"
        );
    }

    #[test]
    fn test_git_turns_the_file_monitor_off_even_when_the_shell_configures_git() {
        // A developer's global `core.fsmonitor=true` starts one daemon per fixture repository. The test
        // git sets the value for its own process, also when the shell passes its own GIT_CONFIG_* values.
        let inherited = [
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "core.fsmonitor"),
            ("GIT_CONFIG_VALUE_0", "true"),
        ]
        .map(|(name, value)| (OsString::from(name), OsString::from(value)));

        let command = git_command_with_inherited(inherited);
        let set = command
            .get_envs()
            .filter_map(|(name, value)| {
                Some((
                    name.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            set,
            [
                ("GIT_CONFIG_COUNT".to_string(), "1".to_string()),
                ("GIT_CONFIG_KEY_0".to_string(), "core.fsmonitor".to_string()),
                ("GIT_CONFIG_VALUE_0".to_string(), "false".to_string()),
            ]
        );
    }
}
