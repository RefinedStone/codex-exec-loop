use std::fs::{self, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::super::{directory, files::ObjectIdentity, lease::PrivateLease};
use super::Fixture;
use crate::adapter::outbound::filesystem::vault_fs::{
    compare_and_swap_file, read_optional_file, write_file_atomic,
};
use crate::private_fs::{WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL};
use crate::subprocess::ManagedChild;

const WORKER: &str =
    "adapter::outbound::filesystem::vault_fs::platform::tests::processes::vault_fs_process_worker";
const WAIT_LIMIT: Duration = Duration::from_secs(10);

fn spawn_worker(fixture: &Fixture, mode: &str, slot: &str) -> ManagedChild {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", WORKER, "--nocapture"])
        .env("AKRA_VAULT_FS_TEST_MODE", mode)
        .env("AKRA_VAULT_FS_TEST_ROOT", &fixture.root)
        .env("AKRA_VAULT_FS_TEST_CONTROL", &fixture.base)
        .env("AKRA_VAULT_FS_TEST_SLOT", slot)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::subprocess::spawn(&mut command).unwrap()
}

fn wait_for(path: &std::path::Path, expected: &[u8]) {
    let started = Instant::now();
    loop {
        if fs::read(path).is_ok_and(|body| body == expected) {
            return;
        }
        assert!(
            started.elapsed() < WAIT_LIMIT,
            "the vault worker did not reach its checkpoint"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn finish_worker(child: &mut ManagedChild) {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "the isolated vault worker failed");
            return;
        }
        if started.elapsed() >= WAIT_LIMIT {
            child.terminate_and_wait().unwrap();
            panic!("the isolated vault worker did not exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn two_processes_cannot_both_commit_the_same_ciphertext_cas() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    write_file_atomic(&fixture.root, &relative, b"seed ciphertext").unwrap();
    let mut first = spawn_worker(&fixture, "cas", "first");
    let mut second = spawn_worker(&fixture, "cas", "second");
    wait_for(&fixture.base.join("ready-first"), b"ready");
    wait_for(&fixture.base.join("ready-second"), b"ready");
    fs::write(fixture.base.join("release"), b"release").unwrap();
    finish_worker(&mut first);
    finish_worker(&mut second);
    let first_won = fs::read(fixture.base.join("result-first")).unwrap() == b"committed";
    let second_won = fs::read(fixture.base.join("result-second")).unwrap() == b"committed";
    assert_ne!(
        first_won, second_won,
        "exactly one process must win the original CAS"
    );
    let winner = if first_won { "first" } else { "second" };
    assert_eq!(
        read_optional_file(&fixture.root, &relative)
            .unwrap()
            .as_deref(),
        Some(winner)
    );
    assert_eq!(
        fs::read_dir(fixture.root.join("records")).unwrap().count(),
        1
    );
}

#[test]
fn killed_process_releases_the_same_root_mutation_lease() {
    let fixture = Fixture::new();
    let relative = Fixture::relative();
    write_file_atomic(&fixture.root, &relative, b"seed ciphertext").unwrap();
    let lock_path = fixture.root.join("evidence-vault-mutation.lock");
    let retained = OpenOptions::new()
        .read(true)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&lock_path)
        .unwrap();
    let before = ObjectIdentity::of(&retained).unwrap();
    let mut child = spawn_worker(&fixture, "hold-root", "owner");
    wait_for(&fixture.base.join("ready-owner"), b"ready");
    let root = directory::open_root(&fixture.root).unwrap().unwrap();
    assert!(PrivateLease::mutation_with_timeout(root, Duration::from_millis(50)).is_err());
    child.terminate_and_wait().unwrap();
    assert!(
        compare_and_swap_file(
            &fixture.root,
            &relative,
            Some("seed ciphertext"),
            "after process death",
        )
        .unwrap()
    );
    let current = OpenOptions::new()
        .read(true)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(lock_path)
        .unwrap();
    assert_eq!(ObjectIdentity::of(&current).unwrap(), before);
    assert_eq!(ObjectIdentity::of(&retained).unwrap(), before);
    assert_eq!(current.metadata().unwrap().len(), 0);
    assert_eq!(
        read_optional_file(&fixture.root, &relative)
            .unwrap()
            .as_deref(),
        Some("after process death"),
    );
}

#[test]
fn vault_fs_process_worker() {
    let Ok(mode) = std::env::var("AKRA_VAULT_FS_TEST_MODE") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("AKRA_VAULT_FS_TEST_ROOT").unwrap());
    let control = PathBuf::from(std::env::var_os("AKRA_VAULT_FS_TEST_CONTROL").unwrap());
    let slot = std::env::var("AKRA_VAULT_FS_TEST_SLOT").unwrap();
    assert!(matches!(slot.as_str(), "first" | "second" | "owner"));
    match mode.as_str() {
        "cas" => {
            fs::write(control.join(format!("ready-{slot}")), b"ready").unwrap();
            wait_for(&control.join("release"), b"release");
            let committed =
                compare_and_swap_file(&root, &Fixture::relative(), Some("seed ciphertext"), &slot)
                    .unwrap();
            fs::write(
                control.join(format!("result-{slot}")),
                if committed {
                    b"committed".as_slice()
                } else {
                    b"conflict".as_slice()
                },
            )
            .unwrap();
        }
        "hold-root" => {
            let root = directory::open_root(&root).unwrap().unwrap();
            let lease = PrivateLease::mutation(root).unwrap();
            lease.verify_identity().unwrap();
            fs::write(control.join(format!("ready-{slot}")), b"ready").unwrap();
            // The parent kills and reaps this owner; the bounded fallback prevents
            // an orphan if the test harness is stopped outside ManagedChild.
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("the parent did not terminate the isolated vault lease owner");
        }
        _ => panic!("unknown synthetic vault worker mode"),
    }
}
