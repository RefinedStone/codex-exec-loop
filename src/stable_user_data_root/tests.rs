use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use super::stable_user_data_root;

const CHILD_TEST: &str = "stable_user_data_root::tests::stable_root_subprocess_probe";
const EXPECTED_ROOT: &str = "AKRA_TEST_STABLE_USER_DATA_ROOT";

#[test]
fn stable_root_ignores_workspace_and_environment_overrides() {
    let expected = stable_user_data_root().unwrap();
    assert!(expected.is_absolute());
    let fixture = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-stable-root-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>(),
    ));
    std::fs::create_dir(&fixture).unwrap();
    for name in ["first-workspace", "second-workspace"] {
        let workspace = fixture.join(name);
        std::fs::create_dir(&workspace).unwrap();
        let override_root = workspace.join("redirected-user-data");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", CHILD_TEST, "--nocapture"])
            .current_dir(&workspace)
            .env(EXPECTED_ROOT, &expected)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for variable in [
            "AKRA_HOME",
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
            "XDG_DATA_HOME",
        ] {
            command.env(variable, &override_root);
        }
        let child = crate::subprocess::spawn(&mut command).unwrap();
        let output = crate::subprocess::wait_with_output_timeout(
            child,
            "stable OS data root probe",
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "stable root probe failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("stable user root: OS account confirmed")
        );
        assert_eq!(std::fs::read_dir(&workspace).unwrap().count(), 0);
    }
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn stable_root_subprocess_probe() {
    let Some(expected) = std::env::var_os(EXPECTED_ROOT) else {
        return;
    };
    assert_eq!(stable_user_data_root().unwrap(), PathBuf::from(expected));
    println!("stable user root: OS account confirmed");
}

#[cfg(windows)]
#[test]
fn windows_selector_balances_com_and_preserves_existing_apartments() {
    use windows_sys::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
    };

    for (initial, other) in [
        (COINIT_MULTITHREADED, COINIT_APARTMENTTHREADED),
        (COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED),
    ] {
        std::thread::spawn(move || {
            // SAFETY: this is a fresh thread; these calls only manage its COM apartment.
            let initialized = unsafe { CoInitializeEx(std::ptr::null(), initial as u32) };
            assert!(initialized >= 0);
            let selected = stable_user_data_root();
            // The helper must leave our original apartment initialized.
            let preserved = unsafe { CoInitializeEx(std::ptr::null(), other as u32) };
            if preserved >= 0 {
                // Balance an unexpected successful switch before reporting the failure.
                unsafe { CoUninitialize() };
            }
            assert_eq!(preserved, RPC_E_CHANGED_MODE);
            // Release our initialization. A leaked helper reference would prevent switching.
            unsafe { CoUninitialize() };
            let switched = unsafe { CoInitializeEx(std::ptr::null(), other as u32) };
            if switched >= 0 {
                unsafe { CoUninitialize() };
            }
            assert!(
                switched >= 0,
                "the helper must balance its own COM reference"
            );
            assert!(selected.unwrap().is_absolute());
        })
        .join()
        .unwrap();
    }
}
