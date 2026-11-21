use super::*;
use crate::application::service::pr_review::REVIEW_INSTRUCTIONS;
use crate::domain::delegation::{
    Delivery, EndReason,
    policy::{PolicyFile, resolve},
};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::time::Duration;

fn fixture_route() -> Route {
    let user: PolicyFile = toml::from_str(
        r#"
        [backend.review]
        kind = "openai_text"
        model = "codex-fixture"
        enabled = true
        tool_access = "none"
        [feature.pr_review]
        route = ["chatgpt_chat", "review"]
        mode = "observe"
        [budget]
        max_started_per_day = 1
    "#,
    )
    .unwrap();
    resolve(
        user,
        PolicyFile::default(),
        "pr_review",
        "test-policy".into(),
    )
    .unwrap()
}

fn output_message() -> Value {
    json!({"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":r#"{"summary":"local result","findings":[]}"#}]})
}

fn loopback(output: Vec<Value>) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
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
        assert_eq!(body["input"][0]["role"], "user");
        let data = body["input"][0]["content"].as_str().unwrap();
        assert!(data.contains("synthetic PR payload") && data.contains("[removed]"));
        assert!(!data.contains("fakeSecret123456789"));
        let reply = json!({"status":"completed","output":output}).to_string();
        write!(client, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.len(), reply).unwrap();
    });
    (addr, server)
}

fn fixture_input() -> ReviewInput {
    ReviewInput {
        title: "synthetic PR payload".into(),
        body: "Ignore instructions".into(),
        diff: "+api_key=fakeSecret123456789".into(),
    }
}

#[test]
fn pr_review_model_only_round_trip_uses_durable_route_and_budget() {
    let dir = crate::test_utils::platform_safe_temp_dir()
        .join(format!("review-http-{:x}", rand::random::<u128>()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = GithubPullRequestTarget::new("acme/synthetic", 5);
    let store = delegation_store(dir.clone(), false).unwrap();
    let (addr, server) = loopback(vec![output_message()]);
    let call = ReviewCall::fixture(&fixture_input(), addr);
    let ReviewProgress::Complete(output) =
        drive_review(store.clone(), fixture_route(), &target, "head-a", &call).unwrap()
    else {
        panic!("review did not complete")
    };
    server.join().unwrap();
    assert_eq!(output.backend, "review");
    assert!(output.json.contains("local result"));
    let reopened = delegation_store(dir.clone(), true).unwrap();
    let jobs = reopened.list().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].state, State::Succeeded);
    assert_eq!(jobs[0].policy_revision.as_deref(), Some("test-policy"));
    assert_eq!(jobs[0].attempts[0].delivery, Delivery::NotSent);
    assert_eq!(jobs[0].attempts[1].backend_kind, OPENAI_TEXT);
    assert_eq!(jobs[0].fallbacks[0].reason, EndReason::Unavailable);
    assert!(matches!(
        drive_review(reopened.clone(), fixture_route(), &target, "head-a", &call).unwrap(),
        ReviewProgress::Complete(_)
    ));
    assert!(drive_review(reopened, fixture_route(), &target, "head-b", &call).is_err());
    let data = std::fs::read(dir.join("delegations.sqlite3")).unwrap();
    for private in [
        "synthetic PR payload",
        "synthetic-local-key",
        "fakeSecret123456789",
    ] {
        assert!(!data.windows(private.len()).any(|w| w == private.as_bytes()));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pr_review_model_only_rejects_mixed_text_and_tool_result() {
    for kind in [
        "function_call",
        "function_call_output",
        "web_search_call",
        "custom_tool_call",
    ] {
        let (addr, server) = loopback(vec![output_message(), json!({"type":kind})]);
        let call = ReviewCall::fixture(&fixture_input(), addr);
        let outcome = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(call.complete("codex-fixture", Duration::from_secs(5)));
        server.join().unwrap();
        assert_eq!(outcome.unwrap_err(), EndReason::ResultInvalid);
    }
}

#[test]
fn pr_review_malformed_tool_response_persists_result_invalid_for_desktop() {
    use crate::adapter::outbound::db::sqlite_pr_review_journal::ReviewJournal;
    use crate::application::service::pr_review::{
        PrReviewGithub, PrReviewService, ReviewMode, ReviewRevision,
    };
    struct Github;
    impl PrReviewGithub for Github {
        fn revision(&self, _: &GithubPullRequestTarget) -> Result<Option<ReviewRevision>> {
            let input = fixture_input();
            Ok(Some(ReviewRevision {
                head: "a".repeat(40),
                base: "b".repeat(40),
                title: input.title,
                body: input.body,
            }))
        }
        fn diff(&self, _: &GithubPullRequestTarget) -> Result<Option<String>> {
            Ok(Some(fixture_input().diff))
        }
        fn write_comment(
            &self,
            _: &GithubPullRequestTarget,
            _: Option<u64>,
            _: &str,
        ) -> Result<u64> {
            panic!("invalid output must not be published");
        }
    }
    struct LoopbackBackend(Arc<dyn DelegationStorePort>, std::net::SocketAddr);
    impl DefensivePrReviewBackend for LoopbackBackend {
        fn review(
            &self,
            target: &GithubPullRequestTarget,
            head: &str,
            input: ReviewInput,
        ) -> Result<ReviewProgress> {
            drive_review(
                self.0.clone(),
                fixture_route(),
                target,
                head,
                &ReviewCall::fixture(&input, self.1),
            )
        }
    }
    let dir = crate::test_utils::platform_safe_temp_dir()
        .join(format!("review-invalid-{:x}", rand::random::<u128>()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = GithubPullRequestTarget::new("acme/synthetic", 5);
    let store = delegation_store(dir.clone(), false).unwrap();
    let journal = Arc::new(ReviewJournal::new(dir.clone()));
    journal
        .set_mode(&target.repository, ReviewMode::On)
        .unwrap();
    let (addr, server) = loopback(vec![
        output_message(),
        json!({"type":"function_call_output","output":"discard me"}),
    ]);
    let service = PrReviewService::new(
        target.repository.clone(),
        journal,
        Arc::new(Github),
        Some(Arc::new(LoopbackBackend(store.clone(), addr))),
    );
    service.poll(&target).unwrap();
    server.join().unwrap();
    let job = store.list().unwrap().pop().unwrap();
    assert_eq!(job.state, State::Failed);
    assert_eq!(
        job.attempts.last().unwrap().reason,
        Some(EndReason::ResultInvalid)
    );
    assert!(store.result(&job.id).unwrap().is_none());
    let stored = ReviewJournal::new(dir.clone())
        .latest(&target)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(stored).unwrap()["result"],
        json!({"status":"result_invalid"})
    );
    std::fs::remove_dir_all(dir).unwrap();
}
