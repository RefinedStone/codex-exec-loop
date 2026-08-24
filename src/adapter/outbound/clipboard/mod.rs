use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

    fn staging_directory(&self) -> PathBuf {
        std::env::temp_dir().join(STAGING_DIRECTORY_NAME)
    }
}

impl ClipboardImageProbePort for PlatformClipboardImageAdapter {
    fn probe_clipboard_image(&self) -> ClipboardImageProbeOutcome {
        let staging_directory = self.staging_directory();
        let Some(png_bytes) = self.read_platform_clipboard_png_bytes() else {
            return ClipboardImageProbeOutcome::NoImage;
        };
        if let Err(reason) = validate_png_bytes(&png_bytes) {
            return ClipboardImageProbeOutcome::Failed(reason);
        }
        match stage_png_bytes(&staging_directory, &png_bytes) {
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
            read_windows_clipboard_png_bytes()
        } else {
            read_linux_clipboard_png_bytes()
        }
    }

    #[cfg(windows)]
    fn read_platform_clipboard_png_bytes(&self) -> Option<Vec<u8>> {
        read_windows_clipboard_png_bytes()
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
    let scratch_path = std::env::temp_dir().join(format!(
        "{STAGING_FILE_PREFIX}-scratch-{}-{}.png",
        unix_millis(),
        STAGING_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let scratch_display = scratch_path.display().to_string();
    let script = format!(
        "set imageData to the clipboard as \u{00ab}class PNGf\u{00bb}\n\
         set fileRef to open for access POSIX file \"{scratch_display}\" with write permission\n\
         set eof fileRef to 0\n\
         write imageData to fileRef\n\
         close access fileRef"
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let read_result = std::fs::read(&scratch_path);
    let _ = std::fs::remove_file(&scratch_path);
    match read_result {
        Ok(bytes) if !bytes.is_empty() => Some(bytes),
        _ => None,
    }
}

#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn read_windows_clipboard_png_bytes() -> Option<Vec<u8>> {
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
    let output = std::process::Command::new("powershell.exe")
        .args(["-NonInteractive", "-NoProfile", "-Command", script])
        .output()
        .ok()?;
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
        let Ok(output) = std::process::Command::new(program).args(arguments).output() else {
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

fn stage_png_bytes(
    staging_directory: &Path,
    png_bytes: &[u8],
) -> Result<ClipboardImageAttachment, String> {
    std::fs::create_dir_all(staging_directory)
        .map_err(|error| format!("could not create image staging directory: {error}"))?;
    let staged_path = staging_directory.join(format!(
        "{STAGING_FILE_PREFIX}-{}-{}.png",
        unix_millis(),
        STAGING_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&staged_path, png_bytes)
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
        let staged = stage_png_bytes(&parent, MINIMAL_PNG_BYTES).expect("staging must succeed");

        assert_eq!(
            staged.display_name,
            staged.path.file_name().unwrap().to_string_lossy()
        );
        assert_eq!(staged.path.extension().unwrap(), "png");
        let written = std::fs::read(&staged.path).expect("staged bytes must exist");
        assert_eq!(written, MINIMAL_PNG_BYTES);
        let _ = std::fs::remove_dir_all(&parent);
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
