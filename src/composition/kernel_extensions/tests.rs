use super::*;
use serde_json::{Value, json};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const CRASH_ROOT: &str = "AKRA_TEST_EXTENSION_CRASH_ROOT";

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn call(host: &Arc<ExtensionService>, action: &str, input: Value) -> Value {
    host.call(
        host.context("native-recovery-test".into(), None),
        "akra.example",
        1,
        action,
        input,
        None,
    )
    .unwrap()
}

#[test]
fn native_host_crash_child() {
    let Some(root) = std::env::var_os(CRASH_ROOT) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let host = build(&root, ExtensionScope::LocalUser, None, None).unwrap();
    call(
        &host,
        "configure",
        json!({"enabled":true,"expected_revision":0}),
    );
    let started = call(&host, "count", json!({"steps":100,"delay_ms":25}));
    let id = started["id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let job = call(&host, "job", json!({"job_id":id}));
        if job["state"] == "running" {
            std::fs::write(
                root.join("ready.json"),
                json!({
                    "job_id":id,"host_id":host.host_id(),"generation":host.generation()
                })
                .to_string(),
            )
            .unwrap();
            // The parent terminates this disposable host while the durable job is running.
            loop {
                std::thread::park();
            }
        }
        assert!(Instant::now() < deadline, "worker did not start");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn native_forced_host_exit_recovers_interrupted_work_without_rerunning() {
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-extension-crash-{:032x}",
        rand::random::<u128>()
    ));
    crate::private_directory::ensure_private_directory(&root).unwrap();
    let mut child = ChildGuard(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "composition::kernel_extensions::tests::native_host_crash_child",
                "--nocapture",
            ])
            .env(CRASH_ROOT, &root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let ready = loop {
        if let Ok(bytes) = std::fs::read(root.join("ready.json"))
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
        {
            break value;
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before publishing a running job"
        );
        assert!(Instant::now() < deadline, "child host did not become ready");
        std::thread::sleep(Duration::from_millis(5));
    };
    child.0.kill().unwrap();
    assert!(!child.0.wait().unwrap().success());
    let restored = build(&root, ExtensionScope::LocalUser, None, None).unwrap();
    assert_eq!(restored.host_id(), ready["host_id"].as_str().unwrap());
    assert_ne!(restored.generation(), ready["generation"].as_str().unwrap());
    assert!(restored.catalog()[0].enabled);
    let job = call(&restored, "job", json!({"job_id":ready["job_id"]}));
    assert_eq!(job["state"], "interrupted");
    assert_eq!(job["cleanup"], "complete");
    assert_eq!(job["generation"], ready["generation"]);
    assert!(job["result"].is_null() && job["progress"].is_null());
    let jobs = call(&restored, "jobs", json!({}));
    assert_eq!(jobs["jobs"].as_array().unwrap().len(), 1);
    assert_eq!(jobs["jobs"][0]["state"], "interrupted");
    drop(restored);
    std::fs::remove_dir_all(root).unwrap();
}
