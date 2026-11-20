use super::*;
use crate::adapter::inbound::desktop_confirmation_bridge::{run_io, storage::random_id};
use crate::application::port::inbound::stored_command_port::issue_confirmation::IssueConfirmationTarget;
use crate::domain::issue_automation::collection::IssueCollectionMode;
use serde_json::json;
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

struct Harness {
    root: PathBuf,
    input: WriteHalf<DuplexStream>,
    output: FrameReader<ReadHalf<DuplexStream>>,
    task: tokio::task::JoinHandle<Result<()>>,
}
impl Harness {
    async fn start() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir()
            .join(format!("hc-{}", random_id(5).unwrap()));
        let run = root.clone();
        let (main, helper) = tokio::io::duplex(128 * 1024);
        let (input, output) = tokio::io::split(helper);
        let task = tokio::spawn(async move { run_io(&run, input, output).await });
        let (output, input) = tokio::io::split(main);
        let mut harness = Self {
            root,
            input,
            output: FrameReader::new(output),
            task,
        };
        assert_eq!(harness.next().await, json!({"type":"ready"}));
        harness
    }
    async fn next(&mut self) -> Value {
        tokio::time::timeout(IO_DEADLINE, self.output.read())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }
    async fn control(&mut self, value: Value) -> Value {
        write_frame(&mut self.input, &value).await.unwrap();
        self.next().await
    }
    async fn prepare(&mut self) -> (PathBuf, IssueConfirmationBinding) {
        let reply = self
            .control(json!({"id":"prepare","command":"prepare"}))
            .await;
        let path = PathBuf::from(reply["result"]["session_file"].as_str().unwrap());
        let binding = IssueConfirmationBinding {
            registration_id: reply["result"]["registration_id"]
                .as_str()
                .unwrap()
                .to_owned(),
            connection_id: "a".repeat(32),
            attachment_id: "b".repeat(32),
            client_id: "main-client".into(),
            workspace: "/workspace".into(),
        };
        (path, binding)
    }
    async fn stop(mut self) {
        self.input.shutdown().await.unwrap();
        tokio::time::timeout(IO_DEADLINE, self.task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(std::fs::read_dir(&self.root).unwrap().next().is_none());
        std::fs::remove_dir_all(self.root).unwrap();
    }
}
fn intent(binding: IssueConfirmationBinding) -> IssueConfirmationIntent {
    IssueConfirmationIntent {
        target: IssueConfirmationTarget::Mode {
            workspace: binding.workspace.clone(),
            from: None,
            epoch: None,
            revision: None,
            to: IssueCollectionMode::Observe,
        },
        binding,
        key: "key-1".into(),
        fingerprint: "c".repeat(32),
    }
}

#[tokio::test]
async fn real_private_ipc_binds_one_attachment_retries_exactly_and_revokes() {
    let mut harness = Harness::start().await;
    let (path, binding) = harness.prepare().await;
    let registered = ConfirmationRegistration::register_at(binding.clone(), &path, &harness.root)
        .await
        .unwrap();
    let mut foreign = binding.clone();
    foreign.client_id = "other".into();
    assert!(
        ConfirmationRegistration::register_at(foreign, &path, &harness.root)
            .await
            .is_err()
    );
    let expected = intent(binding.clone());
    let approved = harness.control(json!({"id":"approve","command":"approve","registration_id":binding.registration_id,"intent":expected})).await;
    let proof = approved["result"]["proof_id"].as_str().unwrap();
    let first = registered.consume(proof, &expected).await.unwrap();
    let retry = registered.consume(proof, &expected).await.unwrap();
    assert_eq!(first, retry);
    assert_eq!(first.target(), &expected.target);
    let mut changed = expected.clone();
    changed.key = "other-key".into();
    assert!(registered.consume(proof, &changed).await.is_err());
    harness
        .control(
            json!({"id":"revoke","command":"revoke","registration_id":binding.registration_id}),
        )
        .await;
    assert!(!path.exists());
    assert!(registered.consume(proof, &expected).await.is_err());
    drop(registered);
    harness.stop().await;
}

#[tokio::test]
async fn watch_disconnect_removes_registration_and_main_eof_removes_remaining_files() {
    let mut harness = Harness::start().await;
    let (path, binding) = harness.prepare().await;
    let registered = ConfirmationRegistration::register_at(binding, &path, &harness.root)
        .await
        .unwrap();
    drop(registered);
    tokio::time::timeout(IO_DEADLINE, async {
        while path.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    harness.prepare().await;
    harness.stop().await;
}

#[tokio::test]
async fn credential_replacement_is_rejected_before_any_consumption() {
    let mut harness = Harness::start().await;
    let (path, binding) = harness.prepare().await;
    let registered = ConfirmationRegistration::register_at(binding.clone(), &path, &harness.root)
        .await
        .unwrap();
    let expected = intent(binding.clone());
    let approved = harness.control(json!({"id":"approve","command":"approve","registration_id":binding.registration_id,"intent":expected})).await;
    let proof = approved["result"]["proof_id"].as_str().unwrap();
    let mut file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    file["token"] = json!("e".repeat(64));
    std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
    assert!(registered.consume(proof, &expected).await.is_err());
    drop(registered);
    harness.stop().await;
}
