use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn make_records_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let temp_dir = std::env::temp_dir();

    for attempt in 0..100 {
        let dir = temp_dir.join(format!(
            "codex-exec-loop-validation-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return dir,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("validation temp dir should be created: {error}"),
        }
    }

    panic!("unique validation temp dir should be allocated");
}

fn write_record(dir: &Path, file_name: &str, body: &str) {
    fs::write(dir.join(file_name), body).expect("validation record should be written");
}

fn summarize(records_dir: &Path, args: &[&str]) -> String {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("bash")
        .arg(repo_root.join("scripts/summarize_native_validation.sh"))
        .arg("--records-dir")
        .arg(records_dir)
        .args(args)
        .output()
        .expect("validation summary script should run");

    assert!(
        output.status.success(),
        "summary script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).expect("summary output should be utf8")
}

fn run_release_version_check(tag: &str, manifest_body: &str) -> std::process::Output {
    let dir = make_records_dir();
    let manifest_path = dir.join("Cargo.toml");
    fs::write(&manifest_path, manifest_body).expect("manifest fixture should be written");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("bash")
        .arg(repo_root.join("scripts/validate_native_release_version.sh"))
        .arg("--tag")
        .arg(tag)
        .arg("--manifest")
        .arg(&manifest_path)
        .output()
        .expect("release version validation script should run");
    fs::remove_dir_all(dir).expect("validation temp dir should be removed");
    output
}

#[test]
fn release_version_check_accepts_matching_v_tag() {
    let output = run_release_version_check(
        "v1.3.4",
        r#"[package]
name = "codex-exec-loop-native"
version = "1.3.4"
"#,
    );

    assert!(
        output.status.success(),
        "release version check failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("release_version=1.3.4"));
    assert!(stdout.contains("crate_version=1.3.4"));
}

#[test]
fn release_version_check_rejects_mismatched_tag() {
    let output = run_release_version_check(
        "v1.3.4",
        r#"[package]
name = "codex-exec-loop-native"
version = "1.3.3"
"#,
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("release tag and Cargo.toml version do not match"));
    assert!(stderr.contains("tag version: 1.3.4"));
    assert!(stderr.contains("Cargo.toml version: 1.3.3"));
}

#[test]
fn prompt_input_delay_profile_counts_tmux_detached_pty_row() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "prompt-tmux.txt",
        r#"date: 2026-05-09
commit: abc123
os: Ubuntu 24.04.2 LTS / WSL2 6.6.114.1-microsoft-standard-WSL2
terminal: tmux 3.4 detached PTY
shell: bash
frontend: inline
term: tmux-256color
check_profile: prompt-input-delay-pty
checks:
- launch inline TUI in tmux PTY
result: pass
notes: prompt echo under budget
"#,
    );
    write_record(
        &records_dir,
        "baseline-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
check_profile: terminal-baseline
checks:
- launch and exit
result: pass
notes: baseline only
"#,
    );

    let output = summarize(&records_dir, &["--check-profile", "prompt-input-delay-pty"]);

    assert!(output.contains("check profile: prompt-input-delay-pty"));
    assert!(output.contains("required pass: 1/5"));
    assert!(output.contains("Linux / tmux detached PTY / bash / inline"));
    assert!(output.contains("prompt-tmux.txt"));
    assert!(!output.contains("baseline-terminal-app.txt"));
    assert!(!output.contains("Unmatched Records"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}

#[test]
fn default_summary_filters_out_prompt_input_delay_records() {
    let records_dir = make_records_dir();
    write_record(
        &records_dir,
        "prompt-tmux.txt",
        r#"date: 2026-05-09
commit: abc123
os: Ubuntu 24.04.2 LTS / WSL2 6.6.114.1-microsoft-standard-WSL2
terminal: tmux 3.4 detached PTY
shell: bash
frontend: inline
term: tmux-256color
check_profile: prompt-input-delay-pty
checks:
- launch inline TUI in tmux PTY
result: pass
notes: prompt echo under budget
"#,
    );
    write_record(
        &records_dir,
        "baseline-terminal-app.txt",
        r#"date: 2026-05-09
commit: abc123
os: macOS 14.5
terminal: Terminal.app
shell: zsh
frontend: inline
term: xterm-256color
check_profile: terminal-baseline
checks:
- launch and exit
result: pass
notes: baseline only
"#,
    );

    let output = summarize(&records_dir, &[]);

    assert!(output.contains("check profile: terminal-baseline"));
    assert!(output.contains("required pass: 1/8"));
    assert!(output.contains("baseline-terminal-app.txt"));
    assert!(!output.contains("prompt-tmux.txt"));
    assert!(!output.contains("Unmatched Records"));

    fs::remove_dir_all(records_dir).expect("validation temp dir should be removed");
}
