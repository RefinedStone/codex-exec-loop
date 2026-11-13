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
