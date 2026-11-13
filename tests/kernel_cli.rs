use std::process::Command;

#[test]
fn describe_prints_json_without_creating_configuration() {
    let home =
        std::env::temp_dir().join(format!("akra-kernel-cli-describe-{}", std::process::id()));
    assert!(!home.exists());
    let output = Command::new(env!("CARGO_BIN_EXE_akra"))
        .args(["describe", "--json"])
        .env("AKRA_HOME", &home)
        .output()
        .expect("run CLI");
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    let spec: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(spec["commands"].is_array());
    assert!(!home.exists(), "describe must not initialize configuration");
}

#[test]
fn invalid_kernel_input_is_json_before_startup_configuration() {
    let home = std::env::temp_dir().join(format!("akra-kernel-cli-invalid-{}", std::process::id()));
    assert!(!home.exists());
    let output = Command::new(env!("CARGO_BIN_EXE_akra"))
        .args(["kernel", "missing-scope", "missing-command", "--json"])
        .env("AKRA_HOME", &home)
        .output()
        .expect("run CLI");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    let error: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(error["error"]["message"].is_string());
    assert!(
        !home.exists(),
        "invalid input must not initialize configuration"
    );
}

/*
 * plan 18 K4-9a: the `reset` entrance does not run the reset. It sends `reset_planning` to the repository kernel.
 * No Gateway runs under this home, so the command-only kernel runs it under the repository lock and records it.
 * The user sees the same text as before. No app-server starts and no model turn runs.
 */
#[test]
fn the_reset_entrance_sends_a_kernel_command_and_shows_the_cause_of_a_failure() {
    let root = std::env::temp_dir().join(format!("akra-kernel-cli-reset-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("test folder");
    // The temporary folder is behind a link on macOS (`/var` -> `/private/var`). With the link path the planning
    // store fails to open its database, so the test uses the real path.
    let root = root.canonicalize().expect("real path of the test folder");
    let home = root.join("home");
    // The folder has no planning workspace. The reset fails, and that failure is the thing to observe.
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace folder");
    let akra = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_akra"))
            .args(arguments)
            .env("AKRA_HOME", &home)
            .output()
            .expect("run CLI")
    };
    let workspace_text = workspace.to_str().expect("utf-8 path");

    let reset = akra(&["reset", "queue", workspace_text]);
    let rendered = String::from_utf8(reset.stdout).unwrap();
    assert_eq!(reset.status.code(), Some(1), "{rendered}");
    assert!(rendered.contains("command: reset"), "{rendered}");
    assert!(rendered.contains("target: queue"), "{rendered}");
    assert!(
        rendered.contains("issue: planning workspace is unavailable; initialize planning first"),
        "{rendered}"
    );

    let history = akra(&[
        "kernel",
        "history",
        "--after",
        "0",
        "--workspace",
        workspace_text,
        "--json",
    ]);
    assert!(history.status.success(), "{history:?}");
    let history: serde_json::Value = serde_json::from_slice(&history.stdout).unwrap();
    let records = history["records"].as_array().expect("records");
    assert_eq!(records.len(), 2, "{history}");
    let received = &records[0]["event"];
    assert_eq!(received["kind"], "command_received");
    assert_eq!(received["command"], "reset_planning");
    assert_eq!(received["caller"], "akra-cli");
    let completed = &records[1]["event"];
    assert_eq!(completed["kind"], "command_completed");
    assert_eq!(completed["op_id"], received["op_id"]);
    assert_eq!(completed["result"], "failed");
    assert_eq!(
        completed["detail"],
        "planning workspace is unavailable; initialize planning first"
    );

    // An unconfirmed destructive reset is refused at the entrance. Nothing reaches the kernel.
    let refused = akra(&["reset", "all", workspace_text]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        String::from_utf8(refused.stdout)
            .unwrap()
            .contains("Rerun with --confirm")
    );
    let after = akra(&[
        "kernel",
        "history",
        "--after",
        "0",
        "--workspace",
        workspace_text,
        "--json",
    ]);
    let after: serde_json::Value = serde_json::from_slice(&after.stdout).unwrap();
    assert_eq!(after["records"].as_array().expect("records").len(), 2);

    let _ = std::fs::remove_dir_all(&root);
}
