use std::path::Path;

/*
 * Drag-and-drop and some terminals deliver image references as plain pasted
 * paths. Detecting them keeps one mental model: pasting anything that names an
 * existing image attaches it instead of typing the path into the prompt.
 *
 * This module is pure text/filesystem inspection — no process execution — so
 * the inbound adapter may call it directly. Scanning is line-based because
 * terminals paste dragged files as one path per line, often quoted or
 * backslash-escaped when the file name contains spaces.
 */

const IMAGE_FILE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PastedImageScan {
    pub(super) attached_paths: Vec<String>,
    // Paste text with consumed image-path lines removed; inserting this into
    // the composer keeps prose while never re-typing an attached path.
    pub(super) remaining_text: String,
}

pub(super) fn scan_pasted_text(pasted_text: &str) -> PastedImageScan {
    let mut attached_paths = Vec::new();
    let mut remaining_lines = Vec::new();
    for raw_line in pasted_text.lines() {
        match single_existing_image_path(raw_line) {
            Some(path) => {
                if !attached_paths.contains(&path) {
                    attached_paths.push(path);
                }
            }
            None => remaining_lines.push(raw_line),
        }
    }

    PastedImageScan {
        attached_paths,
        remaining_text: remaining_lines.join("\n"),
    }
}

fn single_existing_image_path(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    path_candidates(trimmed)
        .into_iter()
        .find(|candidate| is_existing_image_file_path(candidate))
}

fn path_candidates(trimmed_line: &str) -> Vec<String> {
    let mut candidates = vec![trimmed_line.to_string()];
    let unquoted = strip_symmetric_quotes(trimmed_line);
    if unquoted != trimmed_line {
        candidates.push(unquoted.clone());
    }
    let unescaped = unescape_spaces(&unquoted);
    if unescaped != unquoted {
        candidates.push(unescaped);
    }
    candidates
}

fn strip_symmetric_quotes(value: &str) -> String {
    let quote_pairs: [(&str, &str); 3] = [("\"", "\""), ("'", "'"), ("\u{201c}", "\u{201d}")];
    for (opening, closing) in quote_pairs {
        if let Some(inner) = value
            .strip_prefix(opening)
            .and_then(|inner| inner.strip_suffix(closing))
        {
            return inner.to_string();
        }
    }
    value.to_string()
}

fn unescape_spaces(value: &str) -> String {
    value.replace("\\ ", " ")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const MINIMAL_PNG_BYTES: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
    ];

    fn unique_test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("akra-image-{label}-{}", std::process::id()))
    }

    #[test]
    fn scan_consumes_image_path_lines_and_keeps_prose() {
        let parent = unique_test_directory("detect");
        std::fs::create_dir_all(&parent).expect("temp parent must exist");
        let image_path = parent.join("shot.png");
        std::fs::write(&image_path, MINIMAL_PNG_BYTES).expect("fixture must exist");
        let missing_path = parent.join("missing.png");
        let text_path = parent.join("notes.txt");
        std::fs::write(&text_path, "text").expect("fixture must exist");

        let pasted = format!(
            "\"{}\"\n{}\n'{}'\nplain words about /relative/only.png",
            image_path.display(),
            text_path.display(),
            missing_path.display(),
        );
        let scan = scan_pasted_text(&pasted);

        assert_eq!(scan.attached_paths, vec![image_path.display().to_string()]);
        assert_eq!(
            scan.remaining_text,
            format!(
                "{}\n'{}'\nplain words about /relative/only.png",
                text_path.display(),
                missing_path.display()
            )
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn scan_supports_quoted_and_escaped_paths_with_spaces() {
        let parent = unique_test_directory("spaces");
        std::fs::create_dir_all(&parent).expect("temp parent must exist");
        let spaced_quoted = parent.join("first shot.png");
        std::fs::write(&spaced_quoted, MINIMAL_PNG_BYTES).expect("fixture must exist");
        let spaced_escaped_name = parent.join("second shot.jpeg");
        std::fs::write(&spaced_escaped_name, MINIMAL_PNG_BYTES).expect("fixture must exist");

        let quoted = format!("\"{}\"", spaced_quoted.display());
        let escaped = spaced_escaped_name
            .display()
            .to_string()
            .replace(' ', "\\ ");
        let scan = scan_pasted_text(&format!("{quoted}\n{escaped}"));

        assert_eq!(scan.attached_paths.len(), 2);
        assert_eq!(scan.remaining_text, "");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn scan_deduplicates_repeated_lines() {
        let parent = unique_test_directory("dup");
        std::fs::create_dir_all(&parent).expect("temp parent must exist");
        let image_path = parent.join("dup.jpg");
        std::fs::write(&image_path, MINIMAL_PNG_BYTES).expect("fixture must exist");
        let token = image_path.display().to_string();

        let scan = scan_pasted_text(&format!("{token}\n{token}"));
        assert_eq!(scan.attached_paths, vec![token]);
        assert_eq!(scan.remaining_text, "");
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn scan_without_images_returns_the_original_text() {
        let scan = scan_pasted_text("just\nplain words");
        assert!(scan.attached_paths.is_empty());
        assert_eq!(scan.remaining_text, "just\nplain words");
    }
}
