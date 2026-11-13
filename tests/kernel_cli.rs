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
 *
 * Unix only: the Gateway socket is under `AKRA_HOME`, so the temporary home has no Gateway. The Windows pipe name
 * belongs to the user, and this test must not reach the Gateway of a developer.
 */
#[cfg(unix)]
#[test]
fn the_reset_entrance_sends_a_kernel_command_and_shows_the_cause_of_a_failure() {
    let root = std::env::temp_dir().join(format!("akra-kernel-cli-reset-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("test folder");
    // The temporary folder is behind a link on macOS (`/var` -> `/private/var`). The planning store opens its
    // database without following links, so the test uses the real path.
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

/*
 * plan 18 K4-9d: a stored task command runs without a Gateway and without a kernel. The command-only kernel runs it
 * under the repository lock, answers with the result, and records it. The planning query reads the store.
 * Unix only, for the same reason as the reset test above.
 */
#[cfg(unix)]
#[test]
fn a_stored_task_command_runs_without_a_gateway_and_a_stale_revision_is_rejected() {
    let root = std::env::temp_dir().join(format!("akra-kernel-cli-task-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("test folder");
    let root = root.canonicalize().expect("real path of the test folder");
    let home = root.join("home");
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace folder");
    let workspace_text = workspace.to_str().expect("utf-8 path").to_string();
    let akra = |arguments: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_akra"))
            .args(arguments)
            .args(["--workspace", &workspace_text, "--json"])
            .env("AKRA_HOME", &home)
            .output()
            .expect("run CLI");
        let answer: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("one JSON line");
        (output.status.code(), answer)
    };

    let (code, created) = akra(&[
        "kernel",
        "planning",
        "create_task",
        "--task",
        r#"{"title":"Write the test"}"#,
    ]);
    assert_eq!(code, Some(0), "{created}");
    assert_eq!(created["result"]["type"], "task_created");
    let task_id = created["result"]["task_id"].as_str().expect("task id");
    let revision = created["result"]["planning_revision"]
        .as_i64()
        .expect("revision");

    // The planning query needs no kernel. It answers the revision that the next change must send.
    let (code, planning) = akra(&["kernel", "query", "planning"]);
    assert_eq!(code, Some(0), "{planning}");
    assert_eq!(planning["revision"], revision);
    assert_eq!(planning["active_tasks"][0]["task_id"], task_id);

    // A stale revision: exit 2, nothing changed, and the next steps are data.
    let stale = (revision - 1).to_string();
    let (code, rejected) = akra(&[
        "kernel",
        "planning",
        "update_task",
        "--task-id",
        task_id,
        "--task",
        r#"{"title":"Changed"}"#,
        "--expected-planning-revision",
        &stale,
    ]);
    assert_eq!(code, Some(2), "{rejected}");
    assert_eq!(rejected["rejection"]["reason"], "planning_revision_stale");
    assert_eq!(rejected["rejection"]["next"][0]["step"], "query");
    assert_eq!(rejected["rejection"]["next"][0]["name"], "planning");

    // A delete needs the confirmation and the current revision.
    let current = revision.to_string();
    let (code, unconfirmed) = akra(&[
        "kernel",
        "planning",
        "delete_task",
        "--task-id",
        task_id,
        "--expected-planning-revision",
        &current,
    ]);
    assert_eq!(code, Some(1), "{unconfirmed}");
    assert_eq!(unconfirmed["error"]["code"], -32010);
    let (code, deleted) = akra(&[
        "kernel",
        "planning",
        "delete_task",
        "--task-id",
        task_id,
        "--expected-planning-revision",
        &current,
        "--confirm",
        "true",
    ]);
    assert_eq!(code, Some(0), "{deleted}");
    assert_eq!(deleted["result"]["type"], "task_deleted");
    let (_, planning) = akra(&["kernel", "query", "planning"]);
    assert!(
        planning["active_tasks"]
            .as_array()
            .is_none_or(|tasks| tasks.is_empty()),
        "{planning}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
