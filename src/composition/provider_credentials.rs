//! One authority-profile identity is shared by Gateway storage and native helpers.
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct ProviderProfile {
    pub(crate) id: String,
    pub(crate) runtime: PathBuf,
}
/// Resolves without creating a directory, database, or key-store entry.
pub(crate) fn profile() -> Result<ProviderProfile> {
    profile_at(SqlitePlanningAuthorityAdapter::authority_home_root()?)
}
fn profile_at(home: PathBuf) -> Result<ProviderProfile> {
    if !home.is_absolute()
        || home.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        bail!("invalid provider authority profile");
    }
    let identity = existing_identity(&home)?;
    let id = identity_hash(&identity);
    Ok(ProviderProfile {
        id,
        runtime: home.join("provider-credentials").join("runtime"),
    })
}
fn identity_hash(identity: &Path) -> String {
    let mut hash = Sha256::new();
    hash.update(b"akra-provider-authority-profile-v1\0");
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // canonicalize resolves existing case aliases. Preserve the original UTF-16:
        // Unicode lowercase and lossy decoding can merge different Windows names.
        for word in identity.as_os_str().encode_wide() {
            hash.update(word.to_le_bytes());
        }
    }
    #[cfg(not(windows))]
    hash.update(identity.as_os_str().as_encoded_bytes());
    format!("{:x}", hash.finalize())
}

fn existing_identity(path: &Path) -> Result<PathBuf> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    bail!("provider profile ancestor is not a directory");
                }
                let mut result = std::fs::canonicalize(ancestor)
                    .context("provider profile cannot be resolved")?;
                for component in missing.into_iter().rev() {
                    result.push(component);
                }
                return Ok(result);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(
                    ancestor
                        .file_name()
                        .context("invalid provider profile component")?
                        .to_owned(),
                );
                ancestor = ancestor
                    .parent()
                    .context("provider profile has no existing ancestor")?;
            }
            Err(_) => bail!("provider profile is unavailable"),
        }
    }
}

const OWNER_FILES: crate::private_sqlite::DatabaseFiles = crate::private_sqlite::DatabaseFiles::new(
    "owner.db",
    "owner.db-journal",
    "owner.db-wal",
    "owner.db-shm",
);
/// A separate SQLite exclusive transaction is the profile owner lock. It contains no credentials.
/// Its private anchors also fence path replacement while the Gateway serves this profile.
pub(crate) struct ProviderProfileOwner {
    opened: std::sync::Mutex<crate::private_sqlite::PrivateConnection>,
    runtime: PathBuf,
}
pub(crate) fn acquire_owner(
    profile: &ProviderProfile,
) -> Result<std::sync::Arc<ProviderProfileOwner>> {
    let mut opened = crate::private_sqlite::open_or_create_strict_with_busy_timeout(
        &profile.runtime,
        OWNER_FILES,
        crate::private_sqlite::Synchronous::Extra,
        std::time::Duration::ZERO,
    )
    .map_err(|_| anyhow::anyhow!("provider profile owner is unavailable"))?;
    let (connection, guard) = opened.parts();
    guard
        .verify_identity()
        .map_err(|_| anyhow::anyhow!("provider profile identity changed"))?;
    connection
        .execute_batch("BEGIN EXCLUSIVE")
        .map_err(|_| anyhow::anyhow!("another Gateway owns this provider profile"))?;
    guard
        .verify_identity()
        .map_err(|_| anyhow::anyhow!("provider profile identity changed"))?;
    Ok(std::sync::Arc::new(ProviderProfileOwner {
        opened: std::sync::Mutex::new(opened),
        runtime: profile.runtime.clone(),
    }))
}
impl ProviderProfileOwner {
    fn runtime(&self) -> Result<PathBuf> {
        let mut opened = self
            .opened
            .lock()
            .map_err(|_| anyhow::anyhow!("provider profile owner is unavailable"))?;
        let (_, guard) = opened.parts();
        guard
            .verify_identity()
            .map_err(|_| anyhow::anyhow!("provider profile identity changed"))?;
        Ok(self.runtime.clone())
    }
    /// Only the exclusive profile owner may remove an abandoned Unix socket. A live or replaced
    /// endpoint is an error; no caller-supplied path and no blind unlink is accepted.
    pub(crate) async fn prepare_endpoint(&self) -> Result<()> {
        let runtime = self.runtime()?;
        #[cfg(windows)]
        {
            let _ = runtime;
            Ok(())
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{FileTypeExt, MetadataExt};
            let path = runtime.join("migration.sock");
            let before = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => bail!("provider migration endpoint is unavailable"),
            };
            // SAFETY: geteuid has no preconditions and retains no pointers.
            let uid = unsafe { libc::geteuid() };
            if !before.file_type().is_socket() || before.uid() != uid || before.mode() & 0o077 != 0
            {
                bail!("provider migration endpoint has unsafe ownership or type");
            }
            let stale = match tokio::time::timeout(
                std::time::Duration::from_millis(250),
                tokio::net::UnixStream::connect(&path),
            )
            .await
            {
                Ok(Err(error)) => matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ),
                _ => false,
            };
            if !stale {
                bail!("provider migration endpoint is active or unavailable");
            }
            self.runtime()?;
            let after = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(_) => bail!("provider migration endpoint identity changed"),
            };
            if after.dev() != before.dev()
                || after.ino() != before.ino()
                || after.uid() != uid
                || !after.file_type().is_socket()
            {
                bail!("provider migration endpoint identity changed");
            }
            std::fs::remove_file(&path)
                .map_err(|_| anyhow::anyhow!("provider migration endpoint cleanup failed"))?;
            self.runtime()?;
            Ok(())
        }
    }
}

#[derive(Clone)]
pub(crate) struct ProviderServices {
    pub(crate) profile: ProviderProfile,
    pub(crate) owner: std::sync::Arc<ProviderProfileOwner>,
    pub(crate) credentials: std::sync::Arc<
        crate::application::service::provider_credentials::ProviderCredentialService,
    >,
    pub(crate) work:
        std::sync::Arc<crate::application::service::provider_work::ProviderWorkService>,
}
/// Construction does not query a key store, prompt a user, or make a network request.
pub(crate) fn build() -> Result<ProviderServices> {
    use crate::adapter::outbound::db::SqliteProviderCredentialLedgerAdapter;
    use crate::adapter::outbound::{
        native_credential_prompt::NativeCredentialPromptAdapter,
        provider_http::ProviderHttpAdapter, provider_key_store::OsProviderKeyStoreAdapter,
    };
    use crate::application::service::{
        provider_credentials::ProviderCredentialService, provider_work::ProviderWorkService,
    };
    use std::sync::Arc;
    let profile = profile()?;
    let owner = acquire_owner(&profile)?;
    let resolving = owner.clone();
    let ledger = Arc::new(SqliteProviderCredentialLedgerAdapter::new(
        profile.id.clone(),
        move || resolving.runtime(),
    )?);
    let api = Arc::new(ProviderHttpAdapter::default());
    let credentials = Arc::new(ProviderCredentialService::new(
        profile.id.clone(),
        ledger,
        Arc::new(OsProviderKeyStoreAdapter::new()?),
        Arc::new(NativeCredentialPromptAdapter),
        api.clone(),
    )?);
    let work = Arc::new(ProviderWorkService::new(credentials.clone(), api));
    Ok(ProviderServices {
        profile,
        owner,
        credentials,
        work,
    })
}

pub(crate) fn native_input() -> std::sync::Arc<
    dyn crate::application::port::outbound::provider_credentials_port::NativePromptPort,
> {
    std::sync::Arc::new(
        crate::adapter::outbound::native_credential_prompt::NativeCredentialInputAdapter,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_namespace_is_stable_across_creation_and_different_for_other_homes() {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-provider-profile-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let home = root.join("profile");
        let before = profile_at(home.clone()).unwrap();
        assert!(!root.exists());
        std::fs::create_dir_all(&home).unwrap();
        let after = profile_at(home).unwrap();
        assert_eq!(before.id, after.id);
        assert_ne!(after.id, profile_at(root.join("other")).unwrap().id);
        assert!(crate::domain::provider_credentials::valid_profile_id(
            &after.id
        ));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn profile_owner_is_exclusive_even_when_gateway_run_directories_differ() {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-provider-owner-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let profile = profile_at(root.clone()).unwrap();
        let first = acquire_owner(&profile).unwrap();
        // Failed competitors must not release the first connection's POSIX locks by closing anchors.
        assert!(acquire_owner(&profile).is_err());
        assert!(acquire_owner(&profile).is_err());
        assert_eq!(first.runtime().unwrap(), profile.runtime);
        drop(first);
        let restarted = acquire_owner(&profile).unwrap();
        assert_eq!(restarted.runtime().unwrap(), profile.runtime);
        drop(restarted);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn exclusive_owner_recovers_stale_socket_but_preserves_live_or_foreign_entries() {
        use std::os::unix::fs::PermissionsExt;
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "pc-owner-{}-{:x}",
            std::process::id(),
            rand::random::<u32>()
        ));
        let profile = profile_at(root.clone()).unwrap();
        let owner = acquire_owner(&profile).unwrap();
        let socket = profile.runtime.join("migration.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(owner.prepare_endpoint().await.is_err());
        assert!(socket.exists());
        drop(listener);
        owner.prepare_endpoint().await.unwrap();
        assert!(!socket.exists());
        std::fs::write(&socket, b"not a socket").unwrap();
        assert!(owner.prepare_endpoint().await.is_err());
        assert_eq!(std::fs::read(&socket).unwrap(), b"not a socket");
        drop(owner);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn canonical_existing_case_aliases_share_a_profile() {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-provider-case-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let home = root.join("MixedProfileName");
        std::fs::create_dir_all(&home).unwrap();
        let alias = PathBuf::from(home.to_str().unwrap().to_ascii_uppercase());
        assert_eq!(profile_at(home).unwrap().id, profile_at(alias).unwrap().id);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn unicode_case_expansion_and_invalid_utf16_do_not_merge_profile_names() {
        use std::os::windows::ffi::OsStringExt;
        assert_ne!(
            identity_hash(Path::new("C:/profiles/İ")),
            identity_hash(Path::new("C:/profiles/i\u{307}"))
        );
        let first = PathBuf::from(std::ffi::OsString::from_wide(&[67, 58, 92, 0xd800]));
        let second = PathBuf::from(std::ffi::OsString::from_wide(&[67, 58, 92, 0xd801]));
        assert_ne!(identity_hash(&first), identity_hash(&second));
    }
}
