//! The built Rust helper crosses OS stdio/private IPC; the Gateway uses its existing test kernel.
//! This is a test-owned approval response, not a native-dialog or real-model smoke test.
use super::*;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Instant;
use tokio::process::{ChildStdin, ChildStdout};

struct ChildGuard(Child);

impl ChildGuard {
    async fn wait(&mut self) -> ExitStatus {
        tokio::time::timeout(TEST_TIMEOUT, async {
            loop {
                if let Some(status) = self.0.try_wait().expect("helper exit should be readable") {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("helper should exit after stdin EOF")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(Some(_))) {
            return;
        }
        // An assertion failure must not leave a live helper or an unreaped exited child.
        let _ = self.0.kill();
        let deadline = Instant::now() + TEST_TIMEOUT;
        loop {
            match self.0.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => {
                    eprintln!("could not reap the native confirmation helper within the deadline");
                    return;
                }
            }
        }
    }
}

struct NativeBroker {
    input: Option<ChildStdin>,
    output: FrameReader<ChildStdout>,
    child: ChildGuard,
}

impl NativeBroker {
    async fn start() -> Self {
        let binary = PathBuf::from(
            std::env::var_os("AKRA_CONFIRMATION_HELPER_TEST_BIN").expect(
                "build akra-gateway and set AKRA_CONFIRMATION_HELPER_TEST_BIN to its absolute path",
            ),
        );
        let home = PathBuf::from(
            std::env::var_os("AKRA_HOME")
                .expect("set AKRA_HOME to an isolated temporary directory for this explicit test"),
        );
        assert!(
            binary.is_absolute() && binary.is_file(),
            "a built helper is required"
        );
        assert!(
            home.is_absolute() && home.is_dir(),
            "an existing absolute AKRA_HOME is required"
        );
        let mut child = ChildGuard(
            Command::new(binary)
                .arg("desktop-confirmation-bridge")
                .env("AKRA_HOME", home)
                .env_remove("AKRA_TRACE")
                .env_remove("AKRA_TRACE_FILE")
                .env_remove("RUST_LOG")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("the built confirmation helper should start"),
        );
        let input = ChildStdin::from_std(child.0.stdin.take().unwrap()).unwrap();
        let output = ChildStdout::from_std(child.0.stdout.take().unwrap()).unwrap();
        let mut broker = Self {
            input: Some(input),
            output: FrameReader::new(output),
            child,
        };
        assert_eq!(broker.next().await, json!({"type":"ready"}));
        broker
    }

    async fn next(&mut self) -> Value {
        tokio::time::timeout(TEST_TIMEOUT, self.output.read())
            .await
            .expect("helper response should arrive in time")
            .expect("helper frame should be valid")
            .expect("helper stdout should remain open")
    }

    async fn request(&mut self, value: Value) -> Value {
        tokio::time::timeout(
            TEST_TIMEOUT,
            write_frame(self.input.as_mut().unwrap(), &value),
        )
        .await
        .expect("helper stdin should accept a frame in time")
        .expect("helper stdin should remain writable");
        let reply = self.next().await;
        assert_eq!(reply["id"], value["id"]);
        assert!(
            reply["error"].is_null(),
            "helper control request was rejected"
        );
        reply["result"].clone()
    }

    async fn close(&mut self) {
        let mut input = self.input.take().unwrap();
        tokio::time::timeout(TEST_TIMEOUT, input.shutdown())
            .await
            .expect("helper stdin should close in time")
            .unwrap();
        drop(input);
        assert!(
            self.child.wait().await.success(),
            "helper should exit cleanly"
        );
        assert!(
            tokio::time::timeout(TEST_TIMEOUT, self.output.read())
                .await
                .unwrap()
                .unwrap()
                .is_none(),
            "helper stdout should end after EOF"
        );
    }
}

async fn prepare_envelope(
    client: &mut WebSocketStream<DuplexStream>,
    helper: &mut NativeBroker,
    registration: &Value,
    key: &str,
) -> Value {
    let request = json!({
        "command":{"type":"set_issue_automation_mode","mode":"observe"},
        "idempotency_key":key
    });
    let prepared = kernel_request(
        client,
        "prepare",
        "akra/kernel/confirmation/prepare",
        json!({"request":request}),
    )
    .await;
    let intent = prepared["result"]["intent"].clone();
    assert_eq!(intent["target"]["to"], "observe", "{prepared}");
    let approved = helper
        .request(json!({"id":"approve","command":"approve",
            "registration_id":registration["registration_id"],"intent":intent}))
        .await;
    assert!(approved["proof_id"].is_string());
    json!({"request":request,"confirmation":{
        "registration_id":registration["registration_id"],
        "proof_id":approved["proof_id"],"intent":intent
    }})
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires built akra-gateway, AKRA_CONFIRMATION_HELPER_TEST_BIN, and isolated AKRA_HOME"]
async fn native_helper_process_confirms_replays_and_revokes_on_eof() {
    tokio::time::timeout(Duration::from_secs(60), native_process_round_trip())
        .await
        .expect("native confirmation round trip should finish in time");
}

async fn native_process_round_trip() {
    let folder = KernelTestFolder::new("native-confirmation");
    let kernels = Arc::new(RepositoryKernelRegistry::default());
    let mut helper = NativeBroker::start().await;
    let mut client = connect_kernel_client(kernels.clone()).await;
    named_attach(&mut client, "attach", &folder.workspace()).await;
    let off = |key| json!({"command":{"type":"set_issue_automation_mode","mode":"off"},"idempotency_key":key});
    let initial = kernel_request(
        &mut client,
        "initial",
        "akra/kernel/command",
        off("initial"),
    )
    .await;
    assert_eq!(initial["result"]["result"]["mode"], "off", "{initial}");

    let registration = helper
        .request(json!({"id":"register","command":"prepare"}))
        .await;
    let session_file = PathBuf::from(registration["session_file"].as_str().unwrap());
    let session_directory = session_file.parent().unwrap().to_owned();
    assert!(session_file.is_file());
    let registered = kernel_request(
        &mut client,
        "register",
        "akra/kernel/confirmation/register",
        registration.clone(),
    )
    .await;
    assert!(registered["result"]["binding"].is_object(), "{registered}");

    let envelope = prepare_envelope(&mut client, &mut helper, &registration, "confirmed").await;
    let accepted = kernel_request(
        &mut client,
        "accept",
        "akra/kernel/command_confirmed",
        envelope.clone(),
    )
    .await;
    assert_eq!(accepted["result"]["processed"], true, "{accepted}");
    assert_eq!(
        accepted["result"]["result"]["mode"], "observe",
        "{accepted}"
    );
    assert!(accepted["result"]["failure"].is_null(), "{accepted}");
    let operation = accepted["result"]["operation"]["op_id"].clone();
    assert!(operation.is_i64());
    let retried = kernel_request(
        &mut client,
        "retry",
        "akra/kernel/command_confirmed",
        envelope.clone(),
    )
    .await;
    assert_eq!(
        retried["result"]["operation"]["replayed"], true,
        "{retried}"
    );
    assert_eq!(retried["result"]["operation"]["op_id"], operation);

    let lowered = kernel_request(&mut client, "lower", "akra/kernel/command", off("lower")).await;
    assert_eq!(lowered["result"]["result"]["mode"], "off", "{lowered}");
    let unused = prepare_envelope(&mut client, &mut helper, &registration, "unused").await;
    // Keep the same attach, revision, and valid intent: only the owning helper ends.
    helper.close().await;
    assert!(
        !session_file.exists(),
        "EOF should remove the registration file"
    );
    assert!(
        !session_directory.exists(),
        "EOF should remove the private IPC directory"
    );
    let denied = kernel_request(
        &mut client,
        "after-eof",
        "akra/kernel/command_confirmed",
        unused,
    )
    .await;
    assert_eq!(denied["error"]["code"], -32602, "{denied}");
    let replay = kernel_request(
        &mut client,
        "receipt",
        "akra/kernel/command",
        envelope["request"].clone(),
    )
    .await;
    assert_eq!(replay["result"]["operation"]["replayed"], true, "{replay}");
    assert_eq!(replay["result"]["operation"]["op_id"], operation);
    let state = kernel_request(
        &mut client,
        "state",
        "akra/kernel/query",
        json!({"query":{"type":"issue_automation"}}),
    )
    .await;
    assert_eq!(state["result"]["mode"], "off", "{state}");
    detach(&mut client).await;
    kernels.shutdown();
}
