use std::fs::{self, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use super::{HookStage, READ_HOOK, read_template};

const TEMPLATE: &str = ".github/ISSUE_TEMPLATE/bug.md";
const LIMIT: usize = 1024 * 1024;

struct Fixture {
    container: PathBuf,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let container = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-windows-template-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        crate::private_directory::ensure_private_directory(&container).unwrap();
        let workspace = container.join("workspace");
        fs::create_dir_all(workspace.join(".github/ISSUE_TEMPLATE")).unwrap();
        Self {
            container,
            workspace,
        }
    }

    fn write(&self, relative: &str, contents: &[u8]) {
        let path = self.workspace.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn read(&self, relative: &str) -> anyhow::Result<Option<String>> {
        read_template(&self.workspace, Path::new(relative), LIMIT)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.container);
    }
}

struct HookGuard;

impl Drop for HookGuard {
    fn drop(&mut self) {
        READ_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

fn with_hook<T>(
    stage: HookStage,
    hook: impl FnOnce() + 'static,
    operation: impl FnOnce() -> T,
) -> T {
    READ_HOOK.with(|slot| {
        assert!(slot.borrow_mut().replace((stage, Box::new(hook))).is_none());
    });
    let _guard = HookGuard;
    operation()
}

fn run_command(command: &mut Command, label: &'static str) -> std::process::Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(command).unwrap(),
        label,
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{label}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn junction(link: &Path, target: &Path) {
    // Path::join preserves forward slashes inside template-relative strings,
    // but cmd's mklink parses them as options. Normalize only the fixture CLI
    // spelling; production paths and the reparse-point assertions stay intact.
    let link = link.to_string_lossy().replace('/', "\\");
    let target = target.to_string_lossy().replace('/', "\\");
    let mut command = Command::new("cmd");
    command.args([
        "/C",
        "mklink",
        "/J",
        link.strip_prefix(r"\\?\").unwrap_or(&link),
        target.strip_prefix(r"\\?\").unwrap_or(&target),
    ]);
    run_command(&mut command, "issue template junction fixture");
}

fn grant_everyone(path: &Path, access: &str) {
    let mut command = Command::new("icacls.exe");
    command
        .arg(path)
        .arg("/grant")
        .arg(format!("*S-1-1-0:{access}"));
    run_command(&mut command, "issue template ACL fixture");
}

fn acl_description(path: &Path) -> Vec<u8> {
    let mut command = Command::new("icacls.exe");
    command.arg(path);
    run_command(&mut command, "issue template ACL inspection").stdout
}

#[test]
fn windows_reader_reads_unicode_markdown_and_yaml_without_creating_absent_paths() {
    let fixture = Fixture::new();
    for (name, contents) in [
        (".github/ISSUE_TEMPLATE/고장.md", "## 증상\n설명\n"),
        (".github/ISSUE_TEMPLATE/bug.yml", "name: 버그\nbody: []\n"),
        (".github/ISSUE_TEMPLATE/bug.yaml", "name: 버그\nbody: []\n"),
    ] {
        fixture.write(name, contents.as_bytes());
        assert_eq!(fixture.read(name).unwrap().as_deref(), Some(contents));
    }
    assert_eq!(fixture.read(TEMPLATE).unwrap(), None);
    fs::remove_dir_all(fixture.workspace.join(".github")).unwrap();
    assert_eq!(fixture.read(TEMPLATE).unwrap(), None);
    assert!(!fixture.workspace.join(".github").exists());
}

#[test]
fn windows_reader_missing_workspace_is_an_error() {
    let fixture = Fixture::new();
    let missing = fixture.container.join("absent-workspace");
    assert!(read_template(&missing, Path::new(TEMPLATE), LIMIT).is_err());
    assert!(!missing.exists());
}

#[test]
fn windows_reader_rejects_unc_devices_and_non_absolute_workspace_roots() {
    for root in [
        "",
        ".",
        r"C:relative",
        r"\rooted-without-drive",
        r"\\server\share\workspace",
        r"\\?\UNC\server\share\workspace",
        r"\\.\pipe\akra-template",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\workspace",
    ] {
        let error = read_template(Path::new(root), Path::new(TEMPLATE), LIMIT).unwrap_err();
        assert!(
            error.to_string().contains("absolute local drive"),
            "{root:?}: {error:#}"
        );
    }
}

#[test]
fn windows_reader_rejects_path_aliases_before_inspecting_the_workspace() {
    let fixture = Fixture::new();
    let missing = fixture.container.join("missing-root");
    for relative in [
        "",
        ".",
        "..",
        "../bug.md",
        r"..\bug.md",
        "./bug.md",
        "folder//bug.md",
        r"C:\bug.md",
        r"C:bug.md",
        r"\\server\share\bug.md",
        r"\\?\C:\bug.md",
        "bug.md:secret",
        "folder/bug.md:secret",
        "bug.md.",
        "bug.md ",
        "bug\n.md",
        "bug\0.md",
        "bad<name>.md",
        "NUL.md",
        "NUL .md",
        "con.md",
        "PRN.yaml",
        "AUX.md",
        "COM0.md",
        "COM1.md",
        "COM¹.md",
        "COM².yaml",
        "COM³.yml",
        "LPT9.md",
        "LPT¹.md",
        "LPT².yaml",
        "LPT³.yml",
        "CONIN$.md",
        "CONOUT$.md",
        "COM²/bug.md",
    ] {
        let error = read_template(&missing, Path::new(relative), LIMIT).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("Windows path alias") || message.contains("Windows device"),
            "{relative:?}: {error:#}"
        );
    }
}

#[test]
fn windows_reader_rejects_root_and_parent_junctions_including_absent_descendants() {
    for relative in [None, Some(".github"), Some(".github/ISSUE_TEMPLATE")] {
        for missing_target in [false, true] {
            let fixture = Fixture::new();
            let target = fixture.container.join("outside");
            if !missing_target {
                fs::create_dir_all(target.join(".github/ISSUE_TEMPLATE")).unwrap();
                fs::write(target.join("sentinel"), b"unchanged").unwrap();
            }
            let link = relative
                .map(|relative| fixture.workspace.join(relative))
                .unwrap_or_else(|| fixture.container.join("selected-workspace"));
            if relative.is_some() {
                fs::remove_dir_all(&link).unwrap();
            }
            junction(&link, &target);
            let root = if relative.is_none() {
                &link
            } else {
                &fixture.workspace
            };
            let result = read_template(root, Path::new(TEMPLATE), LIMIT);
            assert!(
                result.is_err(),
                "{relative:?}, missing target {missing_target}: {result:?}"
            );
            if !missing_target {
                assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"unchanged");
            }
            fs::remove_dir(&link).unwrap();
        }
    }
}

#[test]
fn windows_reader_rejects_leaf_directories_junctions_and_hardlinks() {
    let fixture = Fixture::new();
    let path = fixture.workspace.join(TEMPLATE);
    fs::create_dir(&path).unwrap();
    assert!(fixture.read(TEMPLATE).is_err());
    fs::remove_dir(&path).unwrap();

    let outside = fixture.container.join("outside");
    fs::create_dir(&outside).unwrap();
    junction(&path, &outside);
    assert!(fixture.read(TEMPLATE).is_err());
    fs::remove_dir(&path).unwrap();

    let secret = outside.join("source.md");
    fs::write(&secret, b"outside body").unwrap();
    fs::hard_link(&secret, &path).unwrap();
    assert!(fixture.read(TEMPLATE).is_err());
    assert_eq!(fs::read(secret).unwrap(), b"outside body");
}

#[test]
fn windows_reader_preserves_the_size_limit_and_requires_utf8() {
    let fixture = Fixture::new();
    fixture.write(TEMPLATE, &vec![b'x'; LIMIT]);
    assert_eq!(fixture.read(TEMPLATE).unwrap().unwrap().len(), LIMIT);
    fixture.write(TEMPLATE, &vec![b'x'; LIMIT + 1]);
    assert!(fixture.read(TEMPLATE).is_err());
    fixture.write(TEMPLATE, &[0xff, 0xfe, 0x80]);
    assert!(fixture.read(TEMPLATE).is_err());
    fixture.write(TEMPLATE, b"");
    assert_eq!(fixture.read(TEMPLATE).unwrap(), Some(String::new()));
    for limit in [0, LIMIT + 1, usize::MAX] {
        assert!(read_template(&fixture.workspace, Path::new(TEMPLATE), limit).is_err());
    }
}

#[test]
fn windows_reader_accepts_long_local_and_verbatim_unicode_workspaces() {
    let fixture = Fixture::new();
    let mut root = fixture.workspace.clone();
    while root.to_string_lossy().encode_utf16().count() < 300 {
        root.push("긴경로-issue-template-component");
    }
    fs::create_dir_all(root.join(".github/ISSUE_TEMPLATE")).unwrap();
    fs::write(root.join(TEMPLATE), "## 긴 경로\n내용\n").unwrap();
    let canonical = fs::canonicalize(&root).unwrap();
    for root in [root, canonical] {
        assert_eq!(
            read_template(&root, Path::new(TEMPLATE), LIMIT)
                .unwrap()
                .as_deref(),
            Some("## 긴 경로\n내용\n")
        );
    }
}

#[test]
fn windows_reader_pins_parents_and_leaf_until_the_read_finishes() {
    let fixture = Fixture::new();
    fixture.write(TEMPLATE, b"## Summary\nunchanged\n");
    let workspace = fixture.workspace.clone();
    let path = workspace.join(TEMPLATE);
    let pinned_path = path.clone();
    let result = with_hook(
        HookStage::FileOpened,
        move || {
            for directory in [
                workspace.clone(),
                workspace.join(".github"),
                workspace.join(".github/ISSUE_TEMPLATE"),
            ] {
                assert!(fs::rename(&directory, directory.with_extension("moved")).is_err());
            }
            assert!(fs::rename(&pinned_path, pinned_path.with_extension("moved")).is_err());
            assert!(fs::write(&pinned_path, b"replacement").is_err());
            assert!(fs::remove_file(&pinned_path).is_err());
        },
        || fixture.read(TEMPLATE),
    );
    assert_eq!(result.unwrap().as_deref(), Some("## Summary\nunchanged\n"));
    let moved = path.with_extension("moved");
    fs::rename(&path, &moved).unwrap();
    fs::write(&moved, b"after read").unwrap();
    fs::rename(&moved, &path).unwrap();
    let moved_workspace = fixture.container.join("moved-workspace");
    fs::rename(&fixture.workspace, &moved_workspace).unwrap();
    fs::rename(&moved_workspace, &fixture.workspace).unwrap();
}

#[test]
fn windows_reader_refuses_existing_writers_but_allows_another_reader() {
    let fixture = Fixture::new();
    fixture.write(TEMPLATE, b"body");
    let writer = OpenOptions::new()
        .write(true)
        .share_mode(crate::private_fs::WINDOWS_FILE_SHARE_ALL)
        .open(fixture.workspace.join(TEMPLATE))
        .unwrap();
    assert!(fixture.read(TEMPLATE).is_err());
    drop(writer);

    let workspace = fixture.workspace.clone();
    let result = with_hook(
        HookStage::FileOpened,
        move || {
            assert_eq!(
                read_template(&workspace, Path::new(TEMPLATE), LIMIT).unwrap(),
                Some("body".into())
            );
        },
        || fixture.read(TEMPLATE),
    );
    assert_eq!(result.unwrap(), Some("body".into()));
}

#[test]
fn windows_reader_accepts_inherited_read_access_without_changing_acls() {
    let fixture = Fixture::new();
    fixture.write(TEMPLATE, b"body");
    grant_everyone(&fixture.workspace, "(OI)(CI)(R)");
    let path = fixture.workspace.join(TEMPLATE);
    let before = acl_description(&path);
    assert_eq!(fixture.read(TEMPLATE).unwrap(), Some("body".into()));
    assert_eq!(acl_description(&path), before);
}

#[test]
fn windows_reader_rejects_untrusted_writable_root_parents_and_leaf_without_repair() {
    for relative in ["", ".github", ".github/ISSUE_TEMPLATE", TEMPLATE] {
        let fixture = Fixture::new();
        fixture.write(TEMPLATE, b"body");
        let path = fixture.workspace.join(relative);
        grant_everyone(&path, "(M)");
        let before = acl_description(&path);
        assert!(fixture.read(TEMPLATE).is_err(), "{relative}");
        assert_eq!(acl_description(&path), before, "{relative}");
        assert_eq!(fs::read(fixture.workspace.join(TEMPLATE)).unwrap(), b"body");
    }
}

#[test]
fn windows_reader_rechecks_mutation_trust_after_reading_bytes() {
    for relative in ["", ".github/ISSUE_TEMPLATE", TEMPLATE] {
        let fixture = Fixture::new();
        fixture.write(TEMPLATE, b"body");
        let path = fixture.workspace.join(relative);
        let result = with_hook(
            HookStage::BytesRead,
            move || grant_everyone(&path, "(M)"),
            || fixture.read(TEMPLATE),
        );
        assert!(result.is_err(), "{relative}");
    }
}

#[test]
fn windows_reader_does_not_treat_a_raced_entry_as_missing() {
    for missing_parent in [false, true] {
        let fixture = Fixture::new();
        let path = if missing_parent {
            fs::remove_dir_all(fixture.workspace.join(".github")).unwrap();
            fixture.workspace.join(".github")
        } else {
            fixture.workspace.join(TEMPLATE)
        };
        let result = with_hook(
            HookStage::MissingObserved,
            move || {
                if missing_parent {
                    fs::create_dir(&path).unwrap();
                } else {
                    fs::write(&path, b"appeared").unwrap();
                }
            },
            || fixture.read(TEMPLATE),
        );
        assert!(result.is_err(), "missing parent {missing_parent}");
    }
}

#[test]
fn windows_reader_preserves_private_runtime_exclusion_case_insensitively() {
    let fixture = Fixture::new();
    fixture.write(".akra/runtime/private.md", b"## Private\nnot a template\n");
    fixture.write(".akra/other.md", b"## Allowed\n");
    for relative in [
        ".akra",
        ".akra/runtime",
        ".akra/runtime/private.md",
        ".AKRA/RUNTIME/private.md",
        r".AkRa\RuNtImE\private.md",
    ] {
        let error = fixture.read(relative).unwrap_err();
        assert!(
            error.to_string().contains("private planning runtime"),
            "{relative}: {error:#}"
        );
    }
    assert_eq!(
        fixture.read(".akra/other.md").unwrap().as_deref(),
        Some("## Allowed\n")
    );
}

#[test]
fn windows_reader_rejects_available_dos_short_names_for_private_runtime() {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    fn short_path(path: &Path) -> PathBuf {
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // SAFETY: the input is terminated and this call requests the output size.
        let length = unsafe { GetShortPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0) };
        assert!(length > 0, "{}", std::io::Error::last_os_error());
        let mut buffer = vec![0u16; length as usize];
        // SAFETY: the live output buffer has exactly the requested capacity.
        let written = unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), length) };
        assert!(
            written > 0 && written < length,
            "{}",
            std::io::Error::last_os_error()
        );
        buffer.truncate(written as usize);
        PathBuf::from(OsString::from_wide(&buffer))
    }

    let fixture = Fixture::new();
    let relative = ".akra/runtime/private.md";
    fixture.write(relative, b"## Private\nnot a template\n");
    let short_root = short_path(&fixture.workspace);
    let short_file = short_path(&fixture.workspace.join(relative));
    let short_relative = short_file.strip_prefix(&short_root).unwrap();
    let error = read_template(&short_root, short_relative, LIMIT).unwrap_err();
    assert!(
        error.to_string().contains("private planning runtime"),
        "{}: {error:#}",
        short_relative.display()
    );
    if short_relative == Path::new(relative) {
        eprintln!("The volume does not expose a distinct DOS short name for the runtime fixture.");
    } else {
        eprintln!("Rejected the runtime through its distinct DOS short name.");
    }
}
