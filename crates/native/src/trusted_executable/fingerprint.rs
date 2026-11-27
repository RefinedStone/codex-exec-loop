use super::*;
use sha2::{Digest, Sha256};

const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;

/// A trusted native executable whose format, digest, and identity were observed
/// through the same opened handle. It is revalidated before and after execution.
/// Callers cannot construct trusted evidence from unverified fields.
///
/// ```compile_fail
/// use akra_native::trusted_executable::PinnedNativeExecutable;
/// let _ = PinnedNativeExecutable { path: Default::default(), context: Default::default(), file: todo!(), identity: todo!(), sha256: String::new() };
/// ```
pub struct PinnedNativeExecutable {
    path: PathBuf,
    context: PathBuf,
    file: fs::File,
    identity: InspectedFileIdentity,
    sha256: String,
}

impl PinnedNativeExecutable {
    pub fn inspect(path: &Path, context: &Path) -> Result<Self> {
        let path = validate_absolute(path, context)?;
        let (mut file, identity) = open_validated_file(&path)?;
        let length = file.metadata()?.len();
        if length == 0 || length > MAX_EXECUTABLE_BYTES {
            bail!("native executable exceeds its bounded fingerprint contract");
        }
        // Do not validate a separate open: the exact pinned bytes are both parsed
        // and hashed, then compared with the path's identity again.
        validate_native_executable_format(&mut file, length)?;
        let sha256 = hash_file(&mut file, length)?;
        finish_validated_file_read(&path, &file, identity)?;
        Ok(Self {
            path,
            context: context.into(),
            file,
            identity,
            sha256,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn verify_identity(&self) -> Result<()> {
        if validate_absolute(&self.path, &self.context)? != self.path {
            bail!("pinned native executable path changed");
        }
        finish_validated_file_read(&self.path, &self.file, self.identity)
    }

    pub fn verify(&self) -> Result<()> {
        self.verify_identity()?;
        let mut file = self.file.try_clone()?;
        if hash_file(&mut file, self.file.metadata()?.len())? != self.sha256 {
            bail!("pinned native executable bytes changed");
        }
        finish_validated_file_read(&self.path, &self.file, self.identity)
    }
}

fn hash_file(file: &mut fs::File, expected: u64) -> Result<String> {
    if expected == 0 || expected > MAX_EXECUTABLE_BYTES {
        bail!("native executable exceeds its bounded fingerprint contract");
    }
    file.seek(SeekFrom::Start(0))?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .context("executable size overflow")?;
        if total > expected {
            bail!("native executable changed size while hashing");
        }
        digest.update(&buffer[..read]);
    }
    if total != expected {
        bail!("native executable changed size while hashing");
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests;
