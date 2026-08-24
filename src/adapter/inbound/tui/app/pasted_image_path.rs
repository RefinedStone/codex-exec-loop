use std::path::Path;

/*
 * Drag-and-drop and some terminals deliver image references as plain pasted
 * paths. Detecting them keeps one mental model: pasting anything that names an
 * existing image attaches it instead of typing the path into the prompt. This
 * module is pure text/filesystem inspection — no process execution — so the
 * inbound adapter may call it directly.
 */

const IMAGE_FILE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

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
}
