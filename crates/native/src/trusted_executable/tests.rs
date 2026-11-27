use super::*;
#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink};
#[cfg(windows)]
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn native_executable_validation_accepts_platform_binary_and_rejects_invalid_formats() {
    let root = fixture_root("native-format-validation");
    #[cfg(unix)]
    make_safe_directory_chain(&root);
    let native = root.join(if cfg!(windows) {
        "native.exe"
    } else {
        "native"
    });
    copy_native_executable_fixture(&native).expect("native executable fixture should copy");
    validate_native_executable(&native).expect("current-platform executable should validate");

    let malformed = root.join(if cfg!(windows) {
        "malformed.exe"
    } else {
        "malformed"
    });
    fs::copy(&native, &malformed).expect("malformed native fixture should copy");
    corrupt_native_structure(&malformed);
    let malformed_error = validate_native_executable(&malformed)
        .expect_err("malformed native structure must fail closed");
    assert!(malformed_error.chain().map(ToString::to_string).any(
        |detail| detail.contains("table")
            || detail.contains("boundary")
            || detail.contains("security limit")
    ));

    let invalid_entry = root.join(if cfg!(windows) {
        "invalid-entry.exe"
    } else {
        "invalid-entry"
    });
    fs::copy(&native, &invalid_entry).expect("invalid entry fixture should copy");
    corrupt_native_entry_relationship(&invalid_entry);
    let entry_error = validate_native_executable(&invalid_entry)
        .expect_err("native entry outside executable code must fail closed");
    assert!(
        entry_error
            .chain()
            .map(ToString::to_string)
            .any(|detail| detail.contains("entry") || detail.contains("subtype"))
    );

    let truncated = root.join(if cfg!(windows) {
        "truncated.exe"
    } else {
        "truncated"
    });
    #[cfg(windows)]
    write_executable_bytes(&truncated, b"MZ");
    #[cfg(target_os = "macos")]
    write_executable_bytes(&truncated, b"\xcf\xfa\xed\xfe");
    #[cfg(all(unix, not(target_os = "macos")))]
    write_executable_bytes(&truncated, b"\x7fELF");
    let truncated_error =
        validate_native_executable(&truncated).expect_err("magic-only executable must fail closed");
    assert!(
        truncated_error
            .chain()
            .map(ToString::to_string)
            .any(|detail| detail.contains("truncated"))
    );

    let foreign = root.join(if cfg!(windows) {
        "foreign.exe"
    } else {
        "foreign"
    });
    let mut foreign_header = vec![0_u8; 64];
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    foreign_header[..4].copy_from_slice(b"\xcf\xfa\xed\xfe");
    #[cfg(target_os = "macos")]
    foreign_header[..4].copy_from_slice(b"\x7fELF");
    write_executable_bytes(&foreign, &foreign_header);
    let foreign_error = validate_native_executable(&foreign)
        .expect_err("foreign executable format must fail closed");
    assert!(
        foreign_error
            .chain()
            .map(ToString::to_string)
            .any(|detail| detail.contains("not a"))
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unix_executable_mode_requires_at_least_one_execute_bit() {
    assert!(!super::unix_executable_mode_is_set(0o600));
    assert!(super::unix_executable_mode_is_set(0o100));
    assert!(super::unix_executable_mode_is_set(0o010));
    assert!(super::unix_executable_mode_is_set(0o001));
}

#[cfg(unix)]
#[test]
fn native_unix_mode_policy_limits_group_writes_to_privileged_macos_directories() {
    assert!(validate_unix_mode(20, 0o755, true, false).is_ok());
    assert!(validate_unix_mode(20, 0o755, false, true).is_ok());
    assert!(validate_unix_mode(20, 0o777, true, false).is_err());
    assert!(validate_unix_mode(20, 0o775, false, true).is_err());
    assert!(validate_unix_mode(20, 0o644, false, true).is_err());

    #[cfg(target_os = "macos")]
    {
        assert!(validate_unix_mode(super::MACOS_ADMIN_GROUP_ID, 0o775, true, false).is_ok());
        assert!(validate_unix_mode(super::MACOS_ADMIN_GROUP_ID, 0o775, false, true).is_err());
    }
    #[cfg(not(target_os = "macos"))]
    assert!(validate_unix_mode(80, 0o775, true, false).is_err());
}

#[test]
fn native_host_git_and_node_formats_are_accepted() {
    let root = fixture_root("native-host-tools");
    #[cfg(unix)]
    make_safe_directory_chain(&root);
    for (index, program) in ["git", "node"].into_iter().enumerate() {
        #[cfg(windows)]
        let source = resolve_native_from_current_path(
            program,
            &std::env::current_dir().expect("test current directory should resolve"),
        )
        .unwrap_or_else(|error| {
            panic!("installed native {program} should resolve safely: {error:#}")
        });
        #[cfg(not(windows))]
        let source = find_host_tool(program)
            .unwrap_or_else(|| panic!("test host should provide native {program}"));
        #[cfg(windows)]
        assert!(
            Command::new(&source)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success()),
            "resolved native {program} should execute"
        );
        let destination = root.join(if cfg!(windows) {
            format!("tool-{index}.exe")
        } else {
            format!("tool-{index}")
        });
        fs::copy(&source, &destination).unwrap_or_else(|error| {
            panic!(
                "host tool {} should copy to fixture: {error}",
                source.display()
            )
        });
        #[cfg(unix)]
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))
            .expect("host tool fixture should become executable");
        validate_native_executable(&destination)
            .unwrap_or_else(|error| panic!("native {program} should validate: {error:#}"));
        #[cfg(windows)]
        {
            let hardlink = root.join(format!("tool-{index}-hardlink.exe"));
            fs::hard_link(&destination, &hardlink)
                .expect("trusted executable hardlink fixture should create");
            validate_native_executable(&hardlink).unwrap_or_else(|error| {
                panic!("hard-linked native {program} should validate: {error:#}")
            });
        }
    }
    let _ = fs::remove_dir_all(root);
}

#[cfg(windows)]
#[test]
fn windows_process_argument_path_normalizes_only_drive_and_unc_extended_forms() {
    assert_eq!(
        super::windows_process_argument_path(Path::new(r"\\?\C:\tools\codex.js"))
            .expect("extended drive path should normalize"),
        std::ffi::OsString::from(r"C:\tools\codex.js")
    );
    assert_eq!(
        super::windows_process_argument_path(Path::new(r"\\?\UNC\server\share\codex.js"))
            .expect("extended UNC path should normalize"),
        std::ffi::OsString::from(r"\\server\share\codex.js")
    );
    assert!(
        super::windows_process_argument_path(Path::new(
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\codex.js"
        ))
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn resolves_safe_user_local_and_canonical_symlink_installations() {
    let workspace_root = fixture_root("trusted-executable-workspace");
    let install_root = fixture_root("trusted-executable-install");
    let workspace = workspace_root.join("workspace");
    let install = install_root.join("user-local");
    fs::create_dir_all(&workspace).expect("workspace should be created");
    fs::create_dir_all(&install).expect("user install should be created");
    make_safe_directory_chain(&workspace_root);
    make_safe_directory_chain(&install_root);
    let target = install.join("codex-real");
    write_executable(&target, "#!/bin/sh\nexit 0\n");
    let link = install.join("codex");
    symlink(&target, &link).expect("npm-style executable symlink should be created");

    let resolved = resolve_from_path("codex", install.as_os_str(), &workspace)
        .expect("safe user-local symlink should resolve to its canonical target");
    assert_eq!(
        resolved,
        fs::canonicalize(target).expect("target should canonicalize")
    );
    assert_eq!(
        validate_absolute(&resolved, &workspace)
            .expect("a pinned canonical executable should remain valid"),
        resolved
    );
}

#[cfg(unix)]
#[test]
fn rejects_relative_repo_and_writable_path_candidates_without_falling_through() {
    let root = fixture_root("trusted-executable-reject");
    let safe_root = fixture_root("trusted-executable-safe-fallback");
    let workspace = root.join("workspace");
    let repo_bin = workspace.join("bin");
    let safe_bin = safe_root.join("safe-bin");
    fs::create_dir_all(&repo_bin).expect("repo bin should be created");
    fs::create_dir_all(&safe_bin).expect("safe bin should be created");
    make_safe_directory_chain(&root);
    make_safe_directory_chain(&safe_root);
    write_executable(&repo_bin.join("codex"), "#!/bin/sh\nexit 91\n");
    write_executable(&safe_bin.join("codex"), "#!/bin/sh\nexit 0\n");

    let relative = std::env::join_paths([PathBuf::from("."), safe_bin.clone()])
        .expect("relative PATH fixture should join");
    assert!(
        resolve_from_path("codex", &relative, &workspace)
            .expect_err("relative PATH entry must fail closed")
            .to_string()
            .contains("relative")
    );

    let repo_first =
        std::env::join_paths([repo_bin, safe_bin.clone()]).expect("repo-first PATH should join");
    let repo_error = resolve_from_path("codex", &repo_first, &workspace)
        .expect_err("repo executable must not fall through to a later safe binary");
    assert!(
        repo_error
            .chain()
            .map(ToString::to_string)
            .any(|detail| detail.contains("repository"))
    );

    let mut permissions = fs::metadata(&safe_bin)
        .expect("safe bin metadata should exist")
        .permissions();
    permissions.set_mode(0o777);
    fs::set_permissions(&safe_bin, permissions).expect("unsafe mode should be applied");
    assert!(resolve_from_path("codex", safe_bin.as_os_str(), &workspace).is_err());
    assert!(
        sanitized_path(safe_bin.as_os_str(), &workspace)
            .expect_err("writable PATH directory should be removed")
            .to_string()
            .contains("no trusted")
    );
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("executable fixture should write");
    let mut permissions = fs::metadata(path)
        .expect("fixture metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("fixture should become executable");
}

fn write_executable_bytes(path: &Path, body: &[u8]) {
    fs::write(path, body).expect("executable byte fixture should write");
    #[cfg(unix)]
    {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("executable byte fixture should become executable");
    }
}

#[cfg(not(windows))]
fn find_host_tool(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|directory| {
        super::executable_names(program)
            .into_iter()
            .map(|name| directory.join(name))
            .find(|candidate| candidate.is_file())
            .and_then(|candidate| fs::canonicalize(candidate).ok())
    })
}

fn corrupt_native_structure(path: &Path) {
    let mut bytes = fs::read(path).expect("native fixture should remain readable");
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let invalid_offset = u64::try_from(bytes.len()).expect("fixture length should fit u64");
        let encoded = if cfg!(target_endian = "little") {
            invalid_offset.to_le_bytes()
        } else {
            invalid_offset.to_be_bytes()
        };
        bytes[32..40].copy_from_slice(&encoded);
    }
    #[cfg(target_os = "macos")]
    match &bytes[..4] {
        [0xcf, 0xfa, 0xed, 0xfe] => bytes[20..24].copy_from_slice(&u32::MAX.to_le_bytes()),
        [0xfe, 0xed, 0xfa, 0xcf] => bytes[20..24].copy_from_slice(&u32::MAX.to_be_bytes()),
        [0xca, 0xfe, 0xba, 0xbe] | [0xca, 0xfe, 0xba, 0xbf] => {
            bytes[4..8].copy_from_slice(&65_u32.to_be_bytes());
        }
        [0xbe, 0xba, 0xfe, 0xca] | [0xbf, 0xba, 0xfe, 0xca] => {
            bytes[4..8].copy_from_slice(&65_u32.to_le_bytes());
        }
        magic => panic!("unexpected native Mach-O magic: {magic:?}"),
    }
    #[cfg(windows)]
    bytes[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
    write_executable_bytes(path, &bytes);
}

fn corrupt_native_entry_relationship(path: &Path) {
    let mut bytes = fs::read(path).expect("native fixture should remain readable");
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let encoded = if cfg!(target_endian = "little") {
            u64::MAX.to_le_bytes()
        } else {
            u64::MAX.to_be_bytes()
        };
        bytes[24..32].copy_from_slice(&encoded);
    }
    #[cfg(target_os = "macos")]
    corrupt_macho_entry_or_subtype(&mut bytes);
    #[cfg(windows)]
    {
        let pe_offset = usize::try_from(u32::from_le_bytes(
            bytes[0x3c..0x40]
                .try_into()
                .expect("PE header offset bytes should exist"),
        ))
        .expect("PE header offset should fit usize");
        let entry_offset = pe_offset + 4 + 20 + 16;
        bytes[entry_offset..entry_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    }
    write_executable_bytes(path, &bytes);
}

#[cfg(target_os = "macos")]
fn corrupt_macho_entry_or_subtype(bytes: &mut [u8]) {
    let magic: [u8; 4] = bytes[..4].try_into().expect("Mach-O magic should exist");
    match magic {
        [0xcf, 0xfa, 0xed, 0xfe] => corrupt_thin_macho_main_entry(bytes, 0, true),
        [0xfe, 0xed, 0xfa, 0xcf] => corrupt_thin_macho_main_entry(bytes, 0, false),
        [0xca, 0xfe, 0xba, 0xbe]
        | [0xca, 0xfe, 0xba, 0xbf]
        | [0xbe, 0xba, 0xfe, 0xca]
        | [0xbf, 0xba, 0xfe, 0xca] => {
            let little = matches!(magic, [0xbe, 0xba, 0xfe, 0xca] | [0xbf, 0xba, 0xfe, 0xca]);
            let fat64 = matches!(magic, [0xca, 0xfe, 0xba, 0xbf] | [0xbf, 0xba, 0xfe, 0xca]);
            let count = read_test_u32(bytes, 4, little);
            let entry_size = if fat64 { 32 } else { 20 };
            let expected_cpu = if cfg!(target_arch = "aarch64") {
                0x0100_000c
            } else {
                0x0100_0007
            };
            let mut changed = false;
            for index in 0..usize::try_from(count).expect("fat count should fit usize") {
                let entry = 8 + index * entry_size;
                if read_test_u32(bytes, entry, little) != expected_cpu {
                    continue;
                }
                let subtype = read_test_u32(bytes, entry + 4, little);
                let offset = if fat64 {
                    read_test_u64(bytes, entry + 8, little)
                } else {
                    u64::from(read_test_u32(bytes, entry + 8, little))
                };
                let offset = usize::try_from(offset).expect("fat slice offset should fit usize");
                let inner_little = match &bytes[offset..offset + 4] {
                    [0xcf, 0xfa, 0xed, 0xfe] => true,
                    [0xfe, 0xed, 0xfa, 0xcf] => false,
                    value => panic!("unexpected thin Mach-O magic: {value:?}"),
                };
                write_test_u32(bytes, offset + 8, subtype ^ 1, inner_little);
                changed = true;
            }
            assert!(changed, "fat Mach-O fixture should contain a native slice");
        }
        value => panic!("unexpected native Mach-O magic: {value:?}"),
    }
}

#[cfg(target_os = "macos")]
fn corrupt_thin_macho_main_entry(bytes: &mut [u8], base: usize, little: bool) {
    let command_count = read_test_u32(bytes, base + 16, little);
    let mut cursor = base + 32;
    for _ in 0..command_count {
        let command = read_test_u32(bytes, cursor, little);
        let command_size = usize::try_from(read_test_u32(bytes, cursor + 4, little))
            .expect("Mach-O command size should fit usize");
        if command == 0x8000_0028 {
            let encoded = if little {
                u64::MAX.to_le_bytes()
            } else {
                u64::MAX.to_be_bytes()
            };
            bytes[cursor + 8..cursor + 16].copy_from_slice(&encoded);
            return;
        }
        cursor += command_size;
    }
    panic!("thin Mach-O fixture should contain LC_MAIN");
}

#[cfg(target_os = "macos")]
fn read_test_u32(bytes: &[u8], offset: usize, little: bool) -> u32 {
    let value = bytes[offset..offset + 4]
        .try_into()
        .expect("test u32 bytes should exist");
    if little {
        u32::from_le_bytes(value)
    } else {
        u32::from_be_bytes(value)
    }
}

#[cfg(target_os = "macos")]
fn read_test_u64(bytes: &[u8], offset: usize, little: bool) -> u64 {
    let value = bytes[offset..offset + 8]
        .try_into()
        .expect("test u64 bytes should exist");
    if little {
        u64::from_le_bytes(value)
    } else {
        u64::from_be_bytes(value)
    }
}

#[cfg(target_os = "macos")]
fn write_test_u32(bytes: &mut [u8], offset: usize, value: u32, little: bool) {
    let encoded = if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    bytes[offset..offset + 4].copy_from_slice(&encoded);
}

#[cfg(unix)]
fn make_safe_directory_chain(root: &Path) {
    let mut root_permissions = fs::metadata(root)
        .expect("fixture root metadata should exist")
        .permissions();
    root_permissions.set_mode(0o755);
    fs::set_permissions(root, root_permissions).expect("fixture root should be safe");
    for child in [
        root.join("workspace"),
        root.join("user-local"),
        root.join("safe-bin"),
    ] {
        if child.exists() {
            let mut permissions = fs::metadata(&child)
                .expect("child metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(child, permissions).expect("child should be safe");
        }
    }
}

#[cfg(unix)]
fn fixture_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow Unix epoch")
        .as_nanos();
    let root = PathBuf::from(std::env::var_os("HOME").expect("HOME should be available"))
        .join(".cache")
        .join(format!("akra-{label}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).expect("fixture root should be created");
    root
}

#[cfg(windows)]
fn fixture_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow Unix epoch")
        .as_nanos();
    let root = crate::private_fs::windows_local_app_data_path()
        .expect("Windows LocalAppData should resolve")
        .join("Akra")
        .join("tests")
        .join(format!("{label}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).expect("fixture root should be created");
    root
}

#[test]
fn workspace_source_root_remains_untrusted_after_crate_extraction() {
    let context = fixture_root("native-source-boundary");
    let workspace = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    assert!(path_is_repository_controlled(
        &workspace.join("src"),
        &context
    ));
    assert!(path_is_repository_controlled(
        &workspace.join("crates").join("issue-core"),
        &context
    ));
    fs::remove_dir_all(context).unwrap();
}
