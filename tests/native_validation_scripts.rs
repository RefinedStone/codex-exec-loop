use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn make_records_dir() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "codex-exec-loop-validation-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("validation temp dir should be created");
    dir
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
