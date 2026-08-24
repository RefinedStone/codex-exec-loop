use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/*
 * clipboard_image implements the operator-facing image attachment flow that
 * competitor TUIs expose: an image-only clipboard paste (or an explicit Ctrl+V)
 * probes the OS clipboard outside the terminal's bracketed-paste text channel,
 * stages PNG bytes on disk, and lets the turn carry a `localImage` item.
 *
 * The terminal never transports image bytes, so this module shells out to the
 * same proven per-platform tools that upstream clients use (osascript,
 * PowerShell, wl-paste, xclip) and validates the payload before staging.
 */

pub(super) const MAX_IMAGE_ATTACHMENTS: usize = 5;
const MAX_CLIPBOARD_IMAGE_BYTES: usize = 25 * 1024 * 1024;
const IMAGE_FILE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const STAGING_DIRECTORY_NAME: &str = "akra-image-paste";
const STAGING_FILE_PREFIX: &str = "akra-paste";

static STAGING_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StagedClipboardImage {
    pub(super) path: PathBuf,
    pub(super) display_name: String,
}

impl StagedClipboardImage {
    fn new(path: PathBuf) -> Self {
        let display_name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "image".to_string());
        Self { path, display_name }
    }
}

/*
 * Probe result contract: `Ok(None)` means the clipboard simply holds no image
 * (the composer stays untouched), while `Err` carries operator-facing copy for
 * a probe that could not be completed.
 */
pub(super) type ClipboardImageProbeResult = Result<Option<StagedClipboardImage>, String>;

pub(super) fn read_clipboard_image_from_staging_directory(
    staging_directory: &Path,
) -> ClipboardImageProbeResult {
    let Some(png_bytes) = read_platform_clipboard_png_bytes()? else {
        return Ok(None);
    };
    validate_png_bytes(&png_bytes)?;
    let staged = stage_png_bytes(staging_directory, &png_bytes)?;
    Ok(Some(staged))
}

fn read_platform_clipboard_png_bytes() -> Result<Option<Vec<u8>>, String> {
    #[cfg(target_os = "macos")]
    {
        read_macos_clipboard_png_bytes()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if runs_inside_wsl() {
            read_windows_clipboard_png_bytes()
        } else {
            read_linux_clipboard_png_bytes()
        }
    }
    #[cfg(windows)]
    {
        read_windows_clipboard_png_bytes()
    }
}

#[cfg(target_os = "macos")]
fn read_macos_clipboard_png_bytes() -> Result<Option<Vec<u8>>, String> {
    /*
     * macOS exposes screenshot bytes through the legacy `PNGf` pasteboard flavor.
     * osascript cannot stream binary to stdout reliably, so it writes a scratch
     * file that this process reads back and deletes immediately.
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
        .map_err(|error| format!("clipboard probe failed to launch osascript: {error}"))?;
    if !output.status.success() {
        // A clipboard without PNG data makes osascript fail; that is a normal
        // "no image" outcome rather than an operator-facing error.
        return Ok(None);
    }
    let read_result = std::fs::read(&scratch_path);
    let _ = std::fs::remove_file(&scratch_path);
    match read_result {
        Ok(bytes) if !bytes.is_empty() => Ok(Some(bytes)),
        _ => Ok(None),
    }
}

#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
fn read_windows_clipboard_png_bytes() -> Result<Option<Vec<u8>>, String> {
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
        .map_err(|error| format!("clipboard probe failed to launch powershell.exe: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let base64_text = String::from_utf8_lossy(&output.stdout);
    let trimmed = base64_text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match decode_base64(trimmed) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(_) => Err("clipboard image data was not valid base64".to_string()),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_linux_clipboard_png_bytes() -> Result<Option<Vec<u8>>, String> {
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
            return Ok(Some(output.stdout));
        }
    }
    Ok(None)
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

#[cfg(test)]
fn max_clipboard_image_bytes_for_test() -> usize {
    MAX_CLIPBOARD_IMAGE_BYTES
}

fn stage_png_bytes(
    staging_directory: &Path,
    png_bytes: &[u8],
) -> Result<StagedClipboardImage, String> {
    std::fs::create_dir_all(staging_directory)
        .map_err(|error| format!("could not create image staging directory: {error}"))?;
    let staged_path = staging_directory.join(format!(
        "{STAGING_FILE_PREFIX}-{}-{}.png",
        unix_millis(),
        STAGING_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&staged_path, png_bytes)
        .map_err(|error| format!("could not stage clipboard image: {error}"))?;
    Ok(StagedClipboardImage::new(staged_path))
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

pub(super) fn image_attachment_staging_directory() -> PathBuf {
    std::env::temp_dir().join(STAGING_DIRECTORY_NAME)
}

/*
 * Drag-and-drop and some terminals deliver image references as plain pasted
 * paths. Detecting them keeps one mental model: pasting anything that names an
 * existing image attaches it instead of typing the path into the prompt.
 */
pub(super) fn extract_existing_image_paths(pasted_text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for token in pasted_text.split_whitespace() {
        let cleaned = token.trim_matches(|character| {
            matches!(character, '"' | '\'' | ',' | '\u{201c}' | '\u{201d}')
        });
        if !is_existing_image_file_path(cleaned) {
            continue;
        }
        if !paths.iter().any(|existing| existing == cleaned) {
            paths.push(cleaned.to_string());
        }
    }
    paths
}

fn is_existing_image_file_path(token: &str) -> bool {
    if !(token.starts_with('/') || is_windows_absolute_path(token)) {
        return false;
    }
    let extension = Path::new(token)
        .extension()
        .map(|extension| extension.to_ascii_lowercase().to_string_lossy().to_string());
    let Some(extension) = extension else {
        return false;
    };
    IMAGE_FILE_EXTENSIONS.contains(&extension.as_str()) && Path::new(token).is_file()
}

fn is_windows_absolute_path(token: &str) -> bool {
    let mut characters = token.chars();
    matches!(
        (characters.next(), characters.next(), characters.next()),
        (Some(drive), Some(':'), Some('\\') | Some('/')) if drive.is_ascii_alphabetic()
    )
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

    fn unique_test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("akra-image-{label}-{}", std::process::id()))
    }

    #[test]
    fn staging_uses_shared_temp_directory_and_png_extension() {
        let directory = image_attachment_staging_directory();
        assert!(directory.ends_with(STAGING_DIRECTORY_NAME));
    }

    #[test]
    fn staged_clipboard_image_writes_bytes_and_derives_display_name() {
        let parent = unique_test_directory("stage");
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
        let oversized = vec![0u8; max_clipboard_image_bytes_for_test() + 1];
        assert_eq!(
            validate_png_bytes(&oversized),
            Err("clipboard image exceeds the 25 MB limit".to_string())
        );
    }

    #[test]
    fn extract_existing_image_paths_finds_only_real_image_files() {
        let parent = unique_test_directory("detect");
        std::fs::create_dir_all(&parent).expect("temp parent must exist");
        let image_path = parent.join("shot.png");
        std::fs::write(&image_path, MINIMAL_PNG_BYTES).expect("fixture must exist");
        let missing_path = parent.join("missing.png");
        let text_path = parent.join("notes.txt");
        std::fs::write(&text_path, "text").expect("fixture must exist");

        let pasted = format!(
            "\"{}\"  {}\n'{}' {} plain words",
            image_path.display(),
            text_path.display(),
            missing_path.display(),
            "/relative/only.png",
        );
        let detected = extract_existing_image_paths(&pasted);

        assert_eq!(detected, vec![image_path.display().to_string()]);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn extract_existing_image_paths_deduplicates_repeated_tokens() {
        let parent = unique_test_directory("dup");
        std::fs::create_dir_all(&parent).expect("temp parent must exist");
        let image_path = parent.join("dup.jpg");
        std::fs::write(&image_path, MINIMAL_PNG_BYTES).expect("fixture must exist");
        let token = image_path.display().to_string();

        let detected = extract_existing_image_paths(&format!("{token} {token}\n{token}"));
        assert_eq!(detected, vec![token]);
        let _ = std::fs::remove_dir_all(&parent);
    }

    /*
     * Manual smoke check against the live OS clipboard:
     *   1. copy an image (e.g. `osascript -e 'set the clipboard to (read \
     *      (POSIX file "shot.png") as «class PNGf»)'`)
     *   2. `cargo test --lib clipboard_image::tests::live_clipboard_probe -- --ignored --nocapture`
     */
    #[test]
    #[ignore = "requires a live OS clipboard holding image data"]
    fn live_clipboard_probe() {
        let staged =
            read_clipboard_image_from_staging_directory(&image_attachment_staging_directory())
                .expect("probe should not fail on a live clipboard");
        match staged {
            Some(staged) => println!("staged: {}", staged.path.display()),
            None => println!("clipboard holds no image"),
        }
    }
}
