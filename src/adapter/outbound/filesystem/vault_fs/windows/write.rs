use std::fmt;
use std::fs::File;
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_DISPOSITION_INFO, FILE_RENAME_INFO, FileDispositionInfo, FileRenameInfo,
    SetFileInformationByHandle,
};

use crate::private_sqlite::PinnedPrivateDirectory;

use super::directory;
use super::files::{self, NewFilePurpose, ObjectIdentity, RecordSnapshot};
use super::lease::PrivateLease;

#[derive(Debug)]
pub(super) struct WriteOutcomeUnknown;

impl fmt::Display for WriteOutcomeUnknown {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the vault installation outcome could not be verified")
    }
}

impl std::error::Error for WriteOutcomeUnknown {}

pub(super) fn write_file_atomic(root: &Path, name: &str, body: &[u8]) -> Result<()> {
    std::str::from_utf8(body).context("the encrypted vault record is not UTF-8")?;
    let lease = writer_lease(root)?;
    let records = directory::open_records(lease.directory(), true)?
        .context("the vault record directory could not be prepared")?;
    let original = files::read_snapshot(&records, name)?;
    if !install(&lease, &records, name, original.as_ref(), body)? {
        bail!("the vault destination changed before installation");
    }
    Ok(())
}

pub(super) fn compare_and_swap_file(
    root: &Path,
    name: &str,
    observed: Option<&str>,
    replacement: &str,
) -> Result<bool> {
    let lease = writer_lease(root)?;
    let records = directory::open_records(lease.directory(), true)?
        .context("the vault record directory could not be prepared")?;
    let original = files::read_snapshot(&records, name)?;
    if original.as_ref().map(|record| record.body.as_str()) != observed {
        lease.verify_identity()?;
        return Ok(false);
    }
    if observed == Some(replacement) {
        lease.verify_identity()?;
        return Ok(true);
    }
    install(
        &lease,
        &records,
        name,
        original.as_ref(),
        replacement.as_bytes(),
    )
}

fn writer_lease(root: &Path) -> Result<PrivateLease> {
    let root =
        directory::open_root(root)?.context("the vault writer root has not been prepared")?;
    PrivateLease::mutation(root)
}

pub(super) fn remove_observed_file(root: &Path, name: &str, observed: &str) -> Result<bool> {
    let lease = writer_lease(root)?;
    let Some(records) = directory::open_records(lease.directory(), false)? else {
        return Ok(false);
    };
    let Some(original) = files::read_snapshot(&records, name)? else {
        return Ok(false);
    };
    if original.body != observed {
        return Ok(false);
    }
    let file = files::open_for_removal(&records, name)?;
    if ObjectIdentity::of(&file)? != original.object
        || files::read_snapshot(&records, name)?.as_ref() != Some(&original)
    {
        return Ok(false);
    }
    lease.verify_identity()?;
    disposition(&file)?;
    drop(file);
    lease
        .verify_identity()
        .map_err(|error| error.context(WriteOutcomeUnknown))?;
    // NTFS can hold a delete-pending name while another reader has a handle.
    // Do not acknowledge cleanup unless this name is absent.
    match files::read_snapshot(&records, name) {
        Ok(None) => Ok(true),
        Ok(Some(_)) => Err(anyhow::Error::new(WriteOutcomeUnknown)),
        Err(error) => Err(error.context(WriteOutcomeUnknown)),
    }
}

fn install(
    lease: &PrivateLease,
    parent: &PinnedPrivateDirectory,
    name: &str,
    original: Option<&RecordSnapshot>,
    body: &[u8],
) -> Result<bool> {
    let mut temporary = TemporaryFile::create(parent)?;
    temporary.write_all(body)?;
    #[cfg(test)]
    super::test_hooks::before_replace();
    lease.verify_identity()?;
    if files::read_snapshot(parent, name)?.as_ref() != original {
        return Ok(false);
    }
    temporary.verify_source()?;

    // A rename request has now crossed the mutation boundary. On failure the
    // original state must be proven; an unclassified failure is never retried.
    temporary.preserve = true;
    if let Err(error) = rename_handle(&temporary.file, parent, name, original.is_some()) {
        return classify_failed_rename(lease, &mut temporary, name, original, error);
    }
    let installed = (|| -> Result<()> {
        #[cfg(test)]
        super::test_hooks::after_replace()?;
        // The handle was opened WRITE_THROUGH. NTFS flushes associated rename
        // metadata; also require an explicit file flush before acknowledging.
        temporary
            .file
            .sync_all()
            .context("the installed vault record could not be flushed")?;
        files::verify_named_file(parent, name, &temporary.file)?;
        if ObjectIdentity::of(&temporary.file)? != temporary.identity {
            bail!("the installed vault source handle changed identity");
        }
        let record = files::read_snapshot(parent, name)?
            .context("the installed vault record disappeared")?;
        if record.object != temporary.identity || record.body.as_bytes() != body {
            bail!("the installed vault record did not match its source handle and ciphertext");
        }
        lease.verify_identity()
    })();
    installed.map_err(|error| error.context(WriteOutcomeUnknown))?;
    Ok(true)
}

fn classify_failed_rename(
    lease: &PrivateLease,
    temporary: &mut TemporaryFile<'_>,
    name: &str,
    original: Option<&RecordSnapshot>,
    error: anyhow::Error,
) -> Result<bool> {
    let current = (|| {
        temporary.verify_source()?;
        lease.verify_identity()?;
        files::read_snapshot(temporary.parent, name)
    })();
    match current {
        Ok(current) if current.as_ref() == original => {
            // The old destination and the private source are both still unchanged.
            temporary.preserve = false;
            Err(error)
        }
        Ok(Some(current))
            if original.is_none()
                && files::is_already_exists(&error)
                && current.object != temporary.identity =>
        {
            // Native no-replace refused an entry that appeared after our last check.
            temporary.preserve = false;
            Ok(false)
        }
        _ => Err(error.context(WriteOutcomeUnknown)),
    }
}

struct TemporaryFile<'a> {
    parent: &'a PinnedPrivateDirectory,
    name: String,
    file: File,
    identity: ObjectIdentity,
    preserve: bool,
}

impl<'a> TemporaryFile<'a> {
    fn create(parent: &'a PinnedPrivateDirectory) -> Result<Self> {
        for _ in 0..8 {
            let name = format!(".vault-{:032x}.tmp", rand::random::<u128>());
            match files::create_private_file(parent, &name, NewFilePurpose::Record) {
                Ok(file) => {
                    let identity = ObjectIdentity::of(&file)?;
                    return Ok(Self {
                        parent,
                        name,
                        file,
                        identity,
                        preserve: false,
                    });
                }
                Err(error) if files::is_already_exists(&error) => {}
                Err(error) => return Err(error),
            }
        }
        bail!("a private vault temporary file could not be allocated")
    }

    fn write_all(&mut self, body: &[u8]) -> Result<()> {
        self.file
            .write_all(body)
            .context("the encrypted vault record could not be written")?;
        self.file
            .sync_all()
            .context("the encrypted vault record could not be flushed")?;
        if self.file.metadata()?.len() != body.len() as u64 {
            bail!("the private vault temporary file changed size");
        }
        self.verify_source()
    }

    fn verify_source(&self) -> Result<()> {
        files::verify_named_file(self.parent, &self.name, &self.file)?;
        if ObjectIdentity::of(&self.file)? != self.identity {
            bail!("the private vault temporary handle changed identity");
        }
        Ok(())
    }
}

impl Drop for TemporaryFile<'_> {
    fn drop(&mut self) {
        if !self.preserve && self.verify_source().is_ok() {
            let _cleanup = disposition(&self.file);
        }
    }
}

fn disposition(file: &File) -> Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the handle remains open for the call, and the initialized input
    // structure has the exact layout and size required by FileDispositionInfo.
    let size = u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO>())?;
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&raw const disposition).cast(),
            size,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error())
            .context("the vault record could not be removed");
    }
    Ok(())
}

pub(super) fn rename_handle(
    source: &File,
    parent: &PinnedPrivateDirectory,
    destination: &str,
    replace: bool,
) -> Result<()> {
    let destination = directory::absolute_destination(parent, destination)?;
    let name: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = offset + name.len() * std::mem::size_of::<u16>();
    // A usize allocation provides the pointer alignment required by FILE_RENAME_INFO.
    let mut storage = vec![0_usize; size.div_ceil(std::mem::size_of::<usize>())];
    let information = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the aligned, zeroed allocation contains the complete fixed header
    // and absolute UTF-16 filename obtained from the pinned parent's handle.
    // The source and no-delete parent/ancestor handles remain open. Use a NULL
    // RootDirectory with this absolute path for Win32 compatibility.
    let success = unsafe {
        (*information).Anonymous.ReplaceIfExists = replace;
        (*information).RootDirectory = std::ptr::null_mut();
        (*information).FileNameLength = ((name.len() - 1) * std::mem::size_of::<u16>()) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            storage.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
            name.len(),
        );
        SetFileInformationByHandle(
            source.as_raw_handle(),
            FileRenameInfo,
            information.cast(),
            size as u32,
        )
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error())
            .context("the vault record could not be installed by its source handle");
    }
    Ok(())
}
