#[cfg(debug_assertions)]
pub(super) fn debug_executable_allows_default_diagnostics(
    executable_path: Option<&std::path::Path>,
) -> bool {
    let Some(executable_path) = executable_path else {
        return false;
    };
    if executable_is_target_debug_deps_harness(executable_path) {
        return false;
    }

    let Some(file_name) = executable_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
    else {
        return false;
    };
    let binary_name = file_name.strip_suffix(".exe").unwrap_or(file_name);
    matches!(
        binary_name,
        "codex-exec-loop-native" | "akra" | "akra-admin" | "akra-telegram"
    )
}

#[cfg(debug_assertions)]
fn executable_is_target_debug_deps_harness(executable_path: &std::path::Path) -> bool {
    let components = executable_path
        .components()
        .map(|component| component.as_os_str())
        .collect::<Vec<_>>();
    components.windows(3).any(|window| {
        window[0] == std::ffi::OsStr::new("target")
            && window[1] == std::ffi::OsStr::new("debug")
            && window[2] == std::ffi::OsStr::new("deps")
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::debug_executable_allows_default_diagnostics;

    #[test]
    fn debug_default_diagnostics_are_disabled_for_test_harness_binaries() {
        assert!(!debug_executable_allows_default_diagnostics(Some(
            Path::new("/repo/target/debug/deps/integration_test-abc123",)
        )));
        assert!(!debug_executable_allows_default_diagnostics(Some(
            Path::new("/repo/target/debug/deps/akra-abc123",)
        )));
    }

    #[test]
    fn debug_default_diagnostics_are_enabled_for_cargo_run_binaries() {
        assert!(debug_executable_allows_default_diagnostics(Some(
            Path::new("/repo/target/debug/codex-exec-loop-native",)
        )));
        assert!(debug_executable_allows_default_diagnostics(Some(
            Path::new("/repo/target/debug/akra",)
        )));
        assert!(debug_executable_allows_default_diagnostics(Some(
            Path::new("/repo/target/debug/akra-admin",)
        )));
        assert!(debug_executable_allows_default_diagnostics(Some(
            Path::new("/home/user/deps/repo/target/debug/akra",)
        )));
    }
}
