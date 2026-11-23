//! Test-only classifications. Never print command arguments, environment values, paths, or bytes.
use anyhow::Error;

pub(super) fn report(stage: &'static str, error: &Error) {
    eprintln!(
        "host-verification preparation failed: stage={stage}; causes={:?}",
        classifications(error)
    );
}

pub(super) fn classifications(error: &Error) -> Vec<String> {
    let mut classes = Vec::new();
    for cause in error.chain().take(8) {
        let class = if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            format!("io(kind={:?},os={:?})", error.kind(), error.raw_os_error())
        } else {
            let message = cause.to_string();
            let labels = [
                (
                    "private Windows handles do not identify",
                    "windows_identity_changed",
                ),
                (
                    "private Windows ACL is not owner-bound",
                    "windows_owner_acl",
                ),
                (
                    "private Windows ACL grants access beyond",
                    "windows_extra_acl_principal",
                ),
                (
                    "failed to secure private Windows path",
                    "secure_private_path",
                ),
                (
                    "failed to validate private directory",
                    "validate_private_directory",
                ),
                (
                    "failed to create trusted temp base",
                    "create_trusted_temp_base",
                ),
                (
                    "failed to create private temporary root",
                    "create_private_temp_root",
                ),
                ("stable non-reparse", "windows_path_identity"),
                (
                    "source Git configuration audit failed",
                    "source_configuration_exit",
                ),
                (
                    "source Git metadata inspection failed",
                    "source_metadata_exit",
                ),
                (
                    "fixed verification Git inspection failed",
                    "fixed_git_inspection_exit",
                ),
                (
                    "verification inspection exceeded its process contract",
                    "inspection_process_contract",
                ),
                ("verification preparation stopped", "preparation_stopped"),
                (
                    "private Git metadata bytes changed",
                    "private_metadata_changed",
                ),
                (
                    "private Git execution configuration changed",
                    "private_configuration_changed",
                ),
                ("native executable", "native_executable_validation"),
            ];
            labels
                .iter()
                .find_map(|(text, label)| message.contains(text).then_some(*label))
                .unwrap_or("opaque_error")
                .to_owned()
        };
        if !classes.contains(&class) {
            classes.push(class);
        }
    }
    classes
}
