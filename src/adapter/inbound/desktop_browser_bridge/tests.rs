use super::*;
use tokio::io::{AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};

#[cfg(windows)]
type TestClient = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type TestClient = tokio::net::UnixStream;

struct Harness {
    root: PathBuf,
    input: WriteHalf<DuplexStream>,
    output: FrameReader<ReadHalf<DuplexStream>>,
    broker: tokio::task::JoinHandle<Result<()>>,
}

impl Harness {
    async fn start() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir()
            .join(format!("browser-{}", random_id(8).unwrap()));
        let run_dir = root.join("run");
        let (main, broker) = tokio::io::duplex(2 * protocol::MAX_FRAME_BYTES);
        let (input, output) = tokio::io::split(broker);
        let task = tokio::spawn(async move { run_io(&run_dir, input, output).await });
        let (output, input) = tokio::io::split(main);
        let mut harness = Self {
            root,
            input,
            output: FrameReader::new(output),
            broker: task,
        };
        assert_eq!(harness.next().await, json!({"type":"ready"}));
        harness
    }

    async fn send(&mut self, value: Value) {
        write_frame(&mut self.input, &value).await.unwrap();
    }

    async fn next(&mut self) -> Value {
        tokio::time::timeout(IO_DEADLINE, self.output.read())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }

    async fn control(&mut self, value: Value) -> Value {
        let id = value["id"].clone();
        self.send(value).await;
        let response = self.next().await;
        assert_eq!(response["id"], id);
        response
    }

    async fn prepare(&mut self, id: &str) -> Value {
        let response = self.control(json!({"id":id,"command":"prepare"})).await;
        let path = response["result"]["sessionFile"].as_str().unwrap();
        let session: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(
            session["registrationId"],
            response["result"]["registrationId"]
        );
        assert_eq!(session["version"], 1);
        assert_eq!(session["token"].as_str().unwrap().len(), 64);
        assert_eq!(response["result"].as_object().unwrap().len(), 2);
        json!({"sessionFile":path, "session":session})
    }

    async fn bind(&mut self, session: &Value, thread: &str) -> Value {
        self.control(json!({
            "id":"bind","command":"bind","registrationId":session["registrationId"],"threadId":thread
        })).await
    }

    async fn stop(mut self) {
        self.input.shutdown().await.unwrap();
        tokio::time::timeout(IO_DEADLINE, self.broker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            std::fs::read_dir(self.root.join("run"))
                .unwrap()
                .next()
                .is_none()
        );
        std::fs::remove_dir_all(self.root).unwrap();
    }
}

async fn connect(endpoint: &str) -> TestClient {
    #[cfg(windows)]
    {
        tokio::net::windows::named_pipe::ClientOptions::new()
            .open(endpoint)
            .unwrap()
    }
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(endpoint).await.unwrap()
    }
}

async fn request(session: &Value, value: Value) -> FrameReader<TestClient> {
    let mut client = connect(session["endpoint"].as_str().unwrap()).await;
    write_frame(&mut client, &value).await.unwrap();
    FrameReader::new(client)
}

fn call(session: &Value, thread: &str, tool: &str) -> Value {
    json!({
        "kind":"call","registrationId":session["registrationId"],"token":session["token"],
        "threadId":thread,"tool":tool,"arguments":{}
    })
}

async fn response(reader: &mut FrameReader<TestClient>) -> Value {
    tokio::time::timeout(IO_DEADLINE, reader.read())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

async fn assert_closed(reader: &mut FrameReader<TestClient>) {
    match tokio::time::timeout(IO_DEADLINE, reader.read())
        .await
        .unwrap()
    {
        Ok(None) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ) => {}
        result => panic!("expected closed connection, got {result:?}"),
    }
}

#[tokio::test]
async fn authenticates_and_serializes_calls_with_immutable_thread_binding() {
    let mut harness = Harness::start().await;
    let prepared = harness.prepare("prepare").await;
    let session = &prepared["session"];
    let mut unbound = request(session, call(session, "thread-a", "browser_status")).await;
    assert_eq!(
        response(&mut unbound).await["error"]["code"],
        "thread_mismatch"
    );
    let mut missing_thread = call(session, "thread-a", "browser_status");
    missing_thread.as_object_mut().unwrap().remove("threadId");
    let mut missing_thread = request(session, missing_thread).await;
    assert_eq!(
        response(&mut missing_thread).await["error"]["code"],
        "invalid_request"
    );

    let mut invalid = call(session, "thread-a", "browser_status");
    invalid["token"] = json!("supplied-secret-must-not-appear");
    let mut wrong_token = request(session, invalid.clone()).await;
    let unauthorized = response(&mut wrong_token).await;
    assert_eq!(
        unauthorized,
        json!({"error":{"code":"unauthorized","message":"Browser session authentication failed"}})
    );
    invalid["registrationId"] = json!("missing");
    let mut missing_registration = request(session, invalid).await;
    assert_eq!(response(&mut missing_registration).await, unauthorized);
    assert_eq!(harness.bind(session, "thread-a").await["result"], json!({}));
    assert_eq!(harness.bind(session, "thread-a").await["result"], json!({}));
    assert_eq!(
        harness.bind(session, "thread-b").await["error"]["code"],
        "thread_mismatch"
    );

    let second = harness.prepare("second").await;
    assert_eq!(
        harness.bind(&second["session"], "thread-a").await["error"]["code"],
        "thread_owned"
    );
    let mut mismatch = request(session, call(session, "thread-b", "browser_status")).await;
    assert_eq!(
        response(&mut mismatch).await["error"]["code"],
        "thread_mismatch"
    );
    let mut unknown = request(session, call(session, "thread-a", "evaluate")).await;
    assert_eq!(
        response(&mut unknown).await["error"]["code"],
        "invalid_request"
    );

    let mut first = request(session, call(session, "thread-a", "browser_snapshot")).await;
    let forwarded = harness.next().await;
    assert_eq!(forwarded["type"], "call");
    assert_eq!(forwarded["threadId"], "thread-a");
    assert_eq!(forwarded["tool"], "browser_snapshot");
    assert_eq!(forwarded.as_object().unwrap().len(), 6);
    assert!(
        !forwarded
            .to_string()
            .contains(session["token"].as_str().unwrap())
    );
    let mut concurrent = request(session, call(session, "thread-a", "browser_status")).await;
    assert_eq!(response(&mut concurrent).await["error"]["code"], "busy");
    harness
        .send(
            json!({"command":"answer","requestId":forwarded["requestId"],"result":{"content":[]}}),
        )
        .await;
    assert_eq!(response(&mut first).await, json!({"result":{"content":[]}}));
    assert_closed(&mut first).await;

    let mut next = request(session, call(session, "thread-a", "browser_status")).await;
    let forwarded = harness.next().await;
    harness.send(json!({"command":"answer","requestId":forwarded["requestId"],"error":{"code":"paused","message":"Paused"}})).await;
    assert_eq!(
        response(&mut next).await,
        json!({"error":{"code":"paused","message":"Paused"}})
    );
    harness.stop().await;
}

#[tokio::test]
async fn disconnect_and_revoke_cancel_once_and_close_watches() {
    let mut harness = Harness::start().await;
    let prepared = harness.prepare("prepare").await;
    let session = &prepared["session"];
    let mut watch = request(
        session,
        json!({"kind":"watch","registrationId":session["registrationId"],"token":session["token"]}),
    )
    .await;
    assert_eq!(
        response(&mut watch).await,
        json!({"result":{"registrationId":session["registrationId"]}})
    );
    harness.bind(session, "thread").await;
    let client = request(session, call(session, "thread", "browser_status")).await;
    let forwarded = harness.next().await;
    drop(client);
    assert_eq!(
        harness.next().await,
        json!({"type":"cancel","requestId":forwarded["requestId"]})
    );

    let mut pending = request(session, call(session, "thread", "browser_handoff")).await;
    let forwarded = harness.next().await;
    harness
        .send(json!({"id":"revoke","command":"revoke","registrationId":session["registrationId"]}))
        .await;
    assert_eq!(
        harness.next().await,
        json!({"type":"cancel","requestId":forwarded["requestId"]})
    );
    assert_eq!(harness.next().await, json!({"id":"revoke","result":{}}));
    assert_eq!(response(&mut pending).await["error"]["code"], "revoked");
    assert_closed(&mut watch).await;
    assert!(!Path::new(prepared["sessionFile"].as_str().unwrap()).exists());
    harness
        .send(json!({"command":"answer","requestId":forwarded["requestId"],"result":null}))
        .await;
    assert_eq!(
        harness
            .control(
                json!({"id":"again","command":"revoke","registrationId":session["registrationId"]})
            )
            .await,
        json!({"id":"again","result":{}})
    );
    let mut stale = request(session, call(session, "thread", "browser_status")).await;
    assert_eq!(response(&mut stale).await["error"]["code"], "unauthorized");
    harness.stop().await;
}

#[tokio::test]
async fn private_files_native_endpoint_and_parent_eof_cleanup() {
    let mut harness = Harness::start().await;
    let prepared = harness.prepare("prepare").await;
    let path = PathBuf::from(prepared["sessionFile"].as_str().unwrap());
    let session = &prepared["session"];
    #[cfg(windows)]
    {
        use crate::private_fs::*;
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::File::open(&path).unwrap();
        validate_windows_private_owner_and_acl(&path, &file).unwrap();
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(WINDOWS_FILE_FLAG_BACKUP_SEMANTICS | WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path.parent().unwrap())
            .unwrap();
        validate_windows_private_owner_and_acl(path.parent().unwrap(), &directory).unwrap();
        let endpoint = PathBuf::from(session["endpoint"].as_str().unwrap());
        let probe = json!({"kind":"watch","registrationId":session["registrationId"],"token":session["token"]});
        tokio::task::spawn_blocking(move || {
            use std::io::{BufRead, Write};
            let mut pipe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .access_mode(WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL)
                .open(&endpoint)
                .unwrap();
            validate_windows_private_owner_and_acl(&endpoint, &pipe).unwrap();
            // A real watch acknowledgement proves the pipe has been accepted.
            writeln!(pipe, "{probe}").unwrap();
            let mut acknowledged = String::new();
            std::io::BufReader::new(pipe)
                .read_line(&mut acknowledged)
                .unwrap();
            assert!(
                serde_json::from_str::<Value>(&acknowledged)
                    .unwrap()
                    .get("result")
                    .is_some()
            );
        })
        .await
        .unwrap();
        harness.bind(session, "thread").await;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(session["endpoint"].as_str().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        harness.bind(session, "thread").await;
    }
    let mut watch = request(
        session,
        json!({"kind":"watch","registrationId":session["registrationId"],"token":session["token"]}),
    )
    .await;
    response(&mut watch).await;
    let mut pending = request(session, call(session, "thread", "browser_status")).await;
    let forwarded = harness.next().await;
    harness.input.shutdown().await.unwrap();
    assert_eq!(
        harness.next().await,
        json!({"type":"cancel","requestId":forwarded["requestId"]})
    );
    assert_closed(&mut watch).await;
    // EOF may close the transport before a pending cancellation reply is flushed.
    match tokio::time::timeout(IO_DEADLINE, pending.read())
        .await
        .unwrap()
    {
        Ok(Some(reply)) => assert_eq!(reply["error"]["code"], "closed"),
        Ok(None) => {}
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
        other => panic!("unexpected EOF result: {other:?}"),
    }
    tokio::time::timeout(IO_DEADLINE, harness.broker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!path.exists());
    assert!(!path.parent().unwrap().exists());
    assert!(harness.root.join("run").exists());
    std::fs::remove_dir_all(harness.root).unwrap();
}

#[tokio::test]
async fn bounded_frames_reject_oversized_and_incomplete_input_without_echoing_it() {
    let mut oversized = vec![b'x'; protocol::MAX_FRAME_BYTES + 1];
    oversized.push(b'\n');
    let mut reader = FrameReader::new(oversized.as_slice());
    let error = reader.read().await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    let mut incomplete = FrameReader::new(b"{\"token\":\"secret\"}".as_slice());
    assert_eq!(
        incomplete.read().await.unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );

    let mut harness = Harness::start().await;
    let prepared = harness.prepare("prepare").await;
    let session = &prepared["session"];
    let mut client = connect(session["endpoint"].as_str().unwrap()).await;
    // Exactly the bound without newline keeps the reader open until the next
    // byte proves the frame invalid. This avoids racing a write against close.
    client
        .write_all(&vec![b'x'; protocol::MAX_FRAME_BYTES])
        .await
        .unwrap();
    client.write_all(b"\n").await.unwrap();
    let mut client = FrameReader::new(client);
    assert_eq!(
        response(&mut client).await["error"]["code"],
        "invalid_request"
    );
    assert_closed(&mut client).await;
    harness.stop().await;
}

#[tokio::test]
async fn bounds_registrations_and_expires_calls_at_exact_deadlines() {
    let root = crate::test_utils::platform_safe_temp_dir()
        .join(format!("browser-{}", random_id(8).unwrap()));
    let files = SessionFiles::create(&root).unwrap();
    let mut broker = Broker {
        files,
        endpoint: "unused".into(),
        registrations: HashMap::new(),
        pending: HashMap::new(),
    };
    let mut output = Vec::new();
    for index in 0..MAX_REGISTRATIONS {
        broker
            .control(
                json!({"id":index.to_string(),"command":"prepare"}),
                &mut output,
            )
            .await
            .unwrap();
    }
    output.clear();
    broker
        .control(json!({"id":"limit","command":"prepare"}), &mut output)
        .await
        .unwrap();
    let limited: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(limited["error"]["code"], "capacity");
    let registration_id = broker.registrations.keys().next().unwrap().clone();
    assert!(matches!(
        broker.bind(&registration_id, "thread"),
        Reply::Result { .. }
    ));
    for tool in ["browser_status", "browser_handoff"] {
        let request = ClientRequest::Call {
            registration_id: registration_id.clone(),
            token: broker.registrations[&registration_id].token.clone(),
            thread_id: "thread".into(),
            tool: tool.into(),
            arguments: json!({}),
        };
        let before = Instant::now();
        let (admission, _) = broker.admit(request).unwrap();
        let Admission::Call {
            request_id,
            mut reply,
        } = admission
        else {
            panic!("call rejected");
        };
        let deadline = broker.pending[&request_id].deadline;
        let expected = if tool == "browser_handoff" {
            HANDOFF_DEADLINE
        } else {
            CALL_DEADLINE
        };
        assert!(deadline >= before + expected);
        assert!(deadline <= Instant::now() + expected);
        output.clear();
        broker
            .expire(deadline - Duration::from_nanos(1), &mut output)
            .await
            .unwrap();
        assert!(matches!(
            reply.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(output.is_empty());
        broker.expire(deadline, &mut output).await.unwrap();
        let expired = serde_json::to_value(reply.await.unwrap()).unwrap();
        assert_eq!(expired["error"]["code"], "timeout");
        assert_eq!(
            serde_json::from_slice::<Value>(&output).unwrap(),
            json!({"type":"cancel","requestId":request_id})
        );
        assert!(broker.registrations[&registration_id].pending.is_none());
        output.clear();
        broker.expire(deadline, &mut output).await.unwrap();
        assert!(output.is_empty());
    }
    broker.shutdown(&mut output).await.unwrap();
    broker.files.cleanup().unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[tokio::test]
async fn private_listener_rejects_an_existing_endpoint() {
    let root = crate::test_utils::platform_safe_temp_dir()
        .join(format!("browser-{}", random_id(8).unwrap()));
    let mut files = SessionFiles::create(&root).unwrap();
    let mut listener = PrivateIpcListener::bind(&files.endpoint()).unwrap();
    assert!(PrivateIpcListener::bind(&files.endpoint()).is_err());
    let endpoint = listener.endpoint();
    let (client, accepted) = tokio::join!(connect(&endpoint), listener.accept());
    drop(client);
    drop(accepted.unwrap());
    drop(listener);
    files.cleanup().unwrap();
    std::fs::remove_dir(root).unwrap();
}
