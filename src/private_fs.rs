//! Compatibility path for the native Windows file ownership boundary.
pub(crate) use akra_native::private_fs::{
    WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT, WINDOWS_FILE_FLAG_BACKUP_SEMANTICS,
    WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
    WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC, WindowsFileIdentitySnapshot,
    WindowsOwnerOnlySecurityDescriptor, set_windows_private_acl,
    validate_windows_ancestor_directory_identity, validate_windows_identity_snapshot,
    validate_windows_owner, validate_windows_path_identity, validate_windows_path_identity_only,
    validate_windows_private_owner_and_acl, validate_windows_shared_private_directory,
    validate_windows_trusted_executable_acl, validate_windows_trusted_executable_path_identity,
    windows_current_user_sid_string, windows_file_identity_key, windows_file_identity_snapshot,
    windows_file_link_count, windows_local_app_data_path,
};
