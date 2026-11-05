use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::adapter::outbound::github::automation::{
    create_private_temp_directory, trusted_temp_directory_base,
};

use crate::application::port::outbound::clipboard_image_probe_port::ClipboardImageProbePort;
use crate::domain::clipboard_image::{ClipboardImageAttachment, ClipboardImageProbeOutcome};

/*
 * PlatformClipboardImageAdapter implements the operator-facing image attachment
 * flow that competitor TUIs expose: an image-only clipboard paste (or an
 * explicit Ctrl+V) probes the OS clipboard outside the terminal's
 * bracketed-paste text channel and stages validated PNG bytes on disk so the
 * turn can carry a `localImage` item.
 *
 * The terminal never transports image bytes, so this adapter shells out to the
 * same proven per-platform tools that upstream clients use (osascript,
 * PowerShell, wl-paste, xclip) and validates the payload before staging.
 */

const MAX_CLIPBOARD_IMAGE_BYTES: usize = 25 * 1024 * 1024;
const STAGING_DIRECTORY_NAME: &str = "akra-image-paste";
const STAGING_FILE_PREFIX: &str = "akra-paste";
// The turn reads the staged file after the probe returns, so only old staging dirs are swept.
const STALE_STAGING_AGE: Duration = Duration::from_secs(24 * 60 * 60);
// PowerShell + System.Windows.Forms start alone measured ~3.7 s on a warm Windows host.
const CLIPBOARD_TOOL_TIMEOUT: Duration = Duration::from_secs(15);
// Windows prints base64 (4 bytes per 3); the slack covers trailing newlines.
const MAX_CLIPBOARD_TOOL_STDOUT_BYTES: usize = MAX_CLIPBOARD_IMAGE_BYTES.div_ceil(3) * 4 + 1024;
const MAX_CLIPBOARD_TOOL_STDERR_BYTES: usize = 64 * 1024;
// Clipboard tools need only session/display variables; tokens and loader overrides are dropped.
const CLIPBOARD_TOOL_ENVIRONMENT: [&str; 18] = [
    "SystemRoot",
    "windir",
    "SystemDrive",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "LOCALAPPDATA",
    "APPDATA",
    "HOME",
    "USER",
    "TMPDIR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "XAUTHORITY",
    "WSL_INTEROP",
    "WSL_DISTRO_NAME",
    "WSLENV",
];
#[cfg(all(unix, not(target_os = "macos")))]
const WSL_WINDOWS_POWERSHELL: &str =
    "/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe";

static STAGING_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) struct PlatformClipboardImageAdapter;

impl Default for PlatformClipboardImageAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformClipboardImageAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl ClipboardImageProbePort for PlatformClipboardImageAdapter {
    fn probe_clipboard_image(&self) -> ClipboardImageProbeOutcome {
        let Some(png_bytes) = self.read_platform_clipboard_png_bytes() else {
            return ClipboardImageProbeOutcome::NoImage;
        };
        if let Err(reason) = validate_png_bytes(&png_bytes) {
            return ClipboardImageProbeOutcome::Failed(reason);
        }
        match stage_png_bytes(&png_bytes) {
            Ok(staged) => ClipboardImageProbeOutcome::Attached(staged),
            Err(reason) => ClipboardImageProbeOutcome::Failed(reason),
        }
    }
}

impl PlatformClipboardImageAdapter {
    #[cfg(target_os = "macos")]
    fn read_platform_clipboard_png_bytes(&self) -> Option<Vec<u8>> {
        read_macos_clipboard_png_bytes()
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn read_platform_clipboard_png_bytes(&self) -> Option<Vec<u8>> {
        if runs_inside_wsl() {
            // Windows binaries fail native ELF validation, so WSL uses the fixed System32 path.
            read_windows_clipboard_png_bytes(PathBuf::from(WSL_WINDOWS_POWERSHELL))
        } else {
            read_linux_clipboard_png_bytes()
        }
    }

    #[cfg(windows)]
    fn read_platform_clipboard_png_bytes(&self) -> Option<Vec<u8>> {
        read_windows_clipboard_png_bytes(resolve_clipboard_tool("powershell")?)
    }
}

#[cfg(target_os = "macos")]
fn read_macos_clipboard_png_bytes() -> Option<Vec<u8>> {
    /*
     * macOS exposes screenshot bytes through the legacy `PNGf` pasteboard flavor.
     * osascript cannot stream binary to stdout reliably, so it writes a scratch
     * file that this process reads back and deletes immediately. A clipboard
     * without PNG data makes osascript fail; that is a normal "no image"
     * outcome rather than an error.
     */
    let scratch_directory = create_private_temp_directory(STAGING_DIRECTORY_NAME).ok()?;
    let scratch_path = scratch_directory.join(format!("{STAGING_FILE_PREFIX}-scratch.png"));
    let scratch_display = scratch_path.display().to_string();
    let script = format!(
        "set imageData to the clipboard as \u{00ab}class PNGf\u{00bb}\n\
         set fileRef to open for access POSIX file \"{scratch_display}\" with write permission\n\
         set eof fileRef to 0\n\
         write imageData to fileRef\n\
         close access fileRef"
    );
    let succeeded = resolve_clipboard_tool("osascript")
        .and_then(|osascript| run_clipboard_tool(&osascript, &["-e", script.as_str()]))
        .is_some_and(|output| output.status.success());
    let read_result = if succeeded {
        std::fs::read(&scratch_path)
    } else {
        Err(std::io::ErrorKind::NotFound.into())
    };
    let _ = std::fs::remove_dir_all(&scratch_directory);
    match read_result {
        Ok(bytes) if !bytes.is_empty() => Some(bytes),
        _ => None,
    }
}

#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn read_windows_clipboard_png_bytes(powershell: PathBuf) -> Option<Vec<u8>> {
    /*
     * Windows keeps decoded bitmap data rather than encoded PNG bytes, so
     * System.Drawing re-encodes the clipboard image and prints base64 on stdout.
     * Piping base64 instead of a file also keeps WSL2 working across the
     * Windows/Linux filesystem boundary.
     */
    let script = "Add-Type -AssemblyName System.Windows.Forms; \
                  $img = [System.Windows.Forms.Clipboard]::GetImage(); \
                  if ($img) { \
                  $ms = New-Object System.IO.MemoryStream; \
                  $img.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); \
                  [Convert]::ToBase64String($ms.ToArray()) }";
    let output = run_clipboard_tool(
        &powershell,
        &["-NonInteractive", "-NoProfile", "-Command", script],
    )?;
    if !output.status.success() {
        return None;
    }
    let base64_text = String::from_utf8_lossy(&output.stdout);
    let trimmed = base64_text.trim();
    if trimmed.is_empty() {
        return None;
    }
    decode_base64(trimmed).ok()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_linux_clipboard_png_bytes() -> Option<Vec<u8>> {
    let candidates: [(&str, &[&str]); 2] = [
        ("wl-paste", &["-t", "image/png"]),
        (
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
        ),
    ];
    for (program, arguments) in candidates {
        let Some(output) =
            resolve_clipboard_tool(program).and_then(|tool| run_clipboard_tool(&tool, arguments))
        else {
            // Missing clipboard tooling is a normal Linux configuration; the
            // next candidate (or the no-image outcome) handles it.
            continue;
        };
        if output.status.success() && !output.stdout.is_empty() {
            return Some(output.stdout);
        }
    }
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
fn runs_inside_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|version| version.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

fn resolve_clipboard_tool(program: &str) -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    crate::trusted_executable::resolve_native_from_current_path(program, &cwd).ok()
}

fn run_clipboard_tool(executable: &Path, arguments: &[&str]) -> Option<Output> {
    let mut command = Command::new(executable);
    command.args(arguments).env_clear();
    for name in CLIPBOARD_TOOL_ENVIRONMENT {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    // Timeout or oversized output kills the child and reads as "no image".
    crate::subprocess::command_output_with_input_timeout_and_limits(
        &mut command,
        "clipboard image probe",
        &[],
        CLIPBOARD_TOOL_TIMEOUT,
        MAX_CLIPBOARD_TOOL_STDOUT_BYTES,
        MAX_CLIPBOARD_TOOL_STDERR_BYTES,
    )
    .ok()
}

fn validate_png_bytes(bytes: &[u8]) -> Result<(), String> {
    const PNG_MAGIC: [u8; 4] = [0x89, b'P', b'N', b'G'];
    if bytes.len() > MAX_CLIPBOARD_IMAGE_BYTES {
        return Err(format!(
            "clipboard image exceeds the {} MB limit",
            MAX_CLIPBOARD_IMAGE_BYTES / (1024 * 1024)
        ));
    }
    if bytes.len() < PNG_MAGIC.len() || bytes[..PNG_MAGIC.len()] != PNG_MAGIC {
        return Err("clipboard image was not PNG data".to_string());
    }
    Ok(())
}

fn stage_png_bytes(png_bytes: &[u8]) -> Result<ClipboardImageAttachment, String> {
    if let Ok(base) = trusted_temp_directory_base() {
        remove_stale_staging_directories(&base, SystemTime::now());
    }
    let staging_directory = create_private_temp_directory(STAGING_DIRECTORY_NAME)
        .map_err(|error| format!("could not create image staging directory: {error:#}"))?;
    write_staged_png(&staging_directory, png_bytes)
}

fn write_staged_png(
    staging_directory: &Path,
    png_bytes: &[u8],
) -> Result<ClipboardImageAttachment, String> {
    use std::io::Write;

    let staged_path = staging_directory.join(format!(
        "{STAGING_FILE_PREFIX}-{}-{}.png",
        unix_millis(),
        STAGING_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(&staged_path)
        .and_then(|mut file| file.write_all(png_bytes))
        .map_err(|error| format!("could not stage clipboard image: {error}"))?;
    let display_name = staged_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "image".to_string());
    Ok(ClipboardImageAttachment {
        path: staged_path,
        display_name,
    })
}

fn remove_stale_staging_directories(base: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let is_staging = entry.file_name().to_str().is_some_and(|name| {
            name.strip_prefix(STAGING_DIRECTORY_NAME)
                .is_some_and(|rest| rest.starts_with('-'))
        });
        // DirEntry metadata does not follow symlinks, so a planted link is never traversed.
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STALE_STAGING_AGE);
        if is_staging && metadata.is_dir() && stale && owned_by_current_user(&metadata) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(unix)]
fn owned_by_current_user(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no preconditions and does not retain pointers.
    metadata.uid() == unsafe { libc::geteuid() }
}

// The Windows trusted temp base already lives under the current user's LocalAppData.
#[cfg(windows)]
fn owned_by_current_user(_metadata: &std::fs::Metadata) -> bool {
    true
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|_| "invalid base64 payload".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_PNG_BYTES: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
    ];

    #[test]
    fn validation_rejects_non_png_payloads() {
        assert!(validate_png_bytes(b"GIF89a....").is_err());
        assert!(validate_png_bytes(&[]).is_err());
        assert_eq!(
            validate_png_bytes(&MINIMAL_PNG_BYTES[1..]),
            Err("clipboard image was not PNG data".to_string())
        );
        assert!(validate_png_bytes(MINIMAL_PNG_BYTES).is_ok());
    }

    #[test]
    fn validation_reports_oversized_payloads_with_limit_copy() {
        let oversized = vec![0u8; MAX_CLIPBOARD_IMAGE_BYTES + 1];
        assert_eq!(
            validate_png_bytes(&oversized),
            Err("clipboard image exceeds the 25 MB limit".to_string())
        );
    }

    #[test]
    fn staged_attachments_write_bytes_and_derive_display_name() {
        let parent =
            std::env::temp_dir().join(format!("akra-clipboard-stage-{}", std::process::id()));
        std::fs::create_dir_all(&parent).expect("test staging dir must exist");
        let staged = write_staged_png(&parent, MINIMAL_PNG_BYTES).expect("staging must succeed");

        assert_eq!(
            staged.display_name,
            staged.path.file_name().unwrap().to_string_lossy()
        );
        assert_eq!(staged.path.extension().unwrap(), "png");
        let written = std::fs::read(&staged.path).expect("staged bytes must exist");
        assert_eq!(written, MINIMAL_PNG_BYTES);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn stale_staging_sweep_removes_only_old_staging_directories() {
        let base =
            std::env::temp_dir().join(format!("akra-clipboard-sweep-{}", std::process::id()));
        let staging = base.join(format!("{STAGING_DIRECTORY_NAME}-0123abcd"));
        let unrelated = base.join("unrelated-dir");
        std::fs::create_dir_all(&staging).expect("staging dir must be created");
        std::fs::create_dir_all(&unrelated).expect("unrelated dir must be created");
        std::fs::write(staging.join("akra-paste-1.png"), MINIMAL_PNG_BYTES).unwrap();

        remove_stale_staging_directories(&base, SystemTime::now());
        assert!(staging.exists(), "fresh staging dirs must survive");

        remove_stale_staging_directories(
            &base,
            SystemTime::now() + STALE_STAGING_AGE + Duration::from_secs(60),
        );
        assert!(!staging.exists(), "stale staging dirs must be removed");
        assert!(unrelated.exists(), "unrelated dirs must never be removed");
        let _ = std::fs::remove_dir_all(&base);
    }

    /*
     * Manual smoke check against a live OS clipboard:
     *   1. copy an image to the clipboard
     *   2. `cargo test --lib clipboard -- --ignored --nocapture`
     */
    #[test]
    #[ignore = "requires a live OS clipboard holding image data"]
    fn live_clipboard_probe() {
        let outcome = PlatformClipboardImageAdapter::new().probe_clipboard_image();
        match outcome {
            ClipboardImageProbeOutcome::Attached(attachment) => {
                println!("staged: {}", attachment.path.display());
            }
            ClipboardImageProbeOutcome::NoImage => println!("clipboard holds no image"),
            ClipboardImageProbeOutcome::Failed(reason) => println!("probe failed: {reason}"),
        }
    }
}
