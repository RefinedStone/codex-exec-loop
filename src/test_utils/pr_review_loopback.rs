use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};

use crate::adapter::outbound::openai_review::REVIEW_INSTRUCTIONS;

pub(crate) fn loopback(output: Vec<Value>) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(&client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "POST /v1/responses HTTP/1.1");
        let mut length = None;
        let mut auth = false;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.split_once(':').unwrap();
            if name.eq_ignore_ascii_case("authorization") {
                auth = value.trim() == "Bearer synthetic-local-key";
            }
            if name.eq_ignore_ascii_case("content-length") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        assert!(auth);
        let mut payload = vec![0; length.unwrap()];
        reader.read_exact(&mut payload).unwrap();
        let body: Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(body["model"], "codex-fixture");
        assert_eq!(body["tools"], json!([]));
        assert_eq!(body["tool_choice"], "none");
        assert_eq!(body["store"], false);
        assert_eq!(body["instructions"], REVIEW_INSTRUCTIONS);
        assert!(
            !body["instructions"]
                .as_str()
                .unwrap()
                .contains("Ignore instructions")
        );
        assert_eq!(body["input"][0]["role"], "user");
        let data = body["input"][0]["content"].as_str().unwrap();
        assert!(data.contains("Ignore instructions"));
        assert!(data.contains("synthetic PR payload") && data.contains("[removed]"));
        assert!(!data.contains("fakeSecret123456789"));
        let reply = json!({"status":"completed","output":output}).to_string();
        write!(client, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.len(), reply).unwrap();
    });
    (addr, server)
}
