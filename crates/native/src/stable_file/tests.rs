use super::*;
use crate::private_path::{DirectoryChain, PinnedPrivateDirectory};
use std::path::PathBuf;

struct OwnedRoot(PathBuf);

impl OwnedRoot {
    fn new() -> Self {
        Self(
            crate::private_temp::create_private_temp_directory("akra-native-file-contract")
                .unwrap(),
        )
    }
}

impl Drop for OwnedRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn native_publication_never_replaces_an_existing_private_destination() {
    let root = OwnedRoot::new();
    let directory = PinnedPrivateDirectory::open(&root.0, false).unwrap();
    {
        let mut first = TemporaryFile::create(&directory, "capture.tmp").unwrap();
        first.write_all(b"first").unwrap();
        first.publish_noreplace("sealed").unwrap();
    }
    {
        let mut second = TemporaryFile::create(&directory, "capture.tmp").unwrap();
        second.write_all(b"second").unwrap();
        second.publish_noreplace("sealed").unwrap();
    }
    let mut sealed = StableFile::open(&directory, "sealed").unwrap();
    assert_eq!(sealed.read_bounded(&directory, 5).unwrap(), b"first");
    assert!(!root.0.join("capture.tmp").exists());
}

#[test]
fn native_bounded_read_rechecks_the_pinned_file_identity() {
    let root = OwnedRoot::new();
    let directory = PinnedPrivateDirectory::open(&root.0, false).unwrap();
    {
        let mut capture = TemporaryFile::create(&directory, "capture.tmp").unwrap();
        capture.write_all(b"bytes").unwrap();
        capture.publish_noreplace("sealed").unwrap();
    }
    let mut sealed = StableFile::open(&directory, "sealed").unwrap();
    assert!(sealed.read_bounded(&directory, 4).is_err());
    assert_eq!(sealed.read_bounded(&directory, 5).unwrap(), b"bytes");
    std::fs::write(root.0.join("sealed"), b"changed bytes").unwrap();
    assert!(sealed.read_bounded(&directory, 100).is_err());
}

#[test]
fn native_existing_directory_probe_and_abandoned_capture_do_not_create_or_publish_state() {
    let root = OwnedRoot::new();
    let missing = root.0.join("missing").join("nested");
    assert!(DirectoryChain::open_existing(&missing).unwrap().is_none());
    assert!(!root.0.join("missing").exists());
    let directory = PinnedPrivateDirectory::open(&root.0, false).unwrap();
    {
        let mut capture = TemporaryFile::create(&directory, "capture.tmp").unwrap();
        capture.write_all(b"incomplete").unwrap();
    }
    assert!(!root.0.join("capture.tmp").exists());
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
}
