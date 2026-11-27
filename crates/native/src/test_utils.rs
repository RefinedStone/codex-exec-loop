use std::path::PathBuf;

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
