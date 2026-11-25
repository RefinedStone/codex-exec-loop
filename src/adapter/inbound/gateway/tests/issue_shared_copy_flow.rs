//! 기존 이슈 공유본 게시의 실제 Gateway/production journal 조립 회귀 (#2660).
//!
//! 실제 private runtime, 정책·템플릿·코드 검증기 어댑터와 feedback 저장소를 사용한다.
//! GitHub와 키/Vault API만 합성이며 사용자 키, OS 승인, 모델 턴, 외부 게시를 호출하지 않는다.
//! 서비스 재조립은 같은 시험 프로세스 안에서 Gateway와 서비스를 닫고 다시 만드는 것이다.
//! 이 fixture에는 과거 pending Vault intent가 없다. API 호출 수와 생성자의 디렉터리 준비는 별개다.

mod fixtures;
mod isolation;

use fixtures::{ApiCalls, World, public_text};
use isolation::run_isolated;
use serde_json::json;

#[test]
fn default_policy_shared_copy_is_confirmed_and_survives_reassembly() {
    run_isolated(
        "default_policy_shared_copy_is_confirmed_and_survives_reassembly",
        async {
            let world = World::new();
            let mut session = world.start().await;

            let opened = session.call("akra/issue/open", world.target()).await;
            assert!(
                opened["result"].is_object(),
                "default-policy issue open must succeed; actual Gateway response: {opened}"
            );
            assert_eq!(
                opened["result"]["template"]["status"], "not_configured",
                "{opened}"
            );
            assert_eq!(opened["result"]["requiredSections"], json!([]), "{opened}");
            assert_eq!(world.counts(), ApiCalls::default());

            let text = public_text();
            let preview = session
                .call(
                    "akra/issue/preview",
                    json!({ "target": world.target(), "text": text }),
                )
                .await;
            assert_eq!(preview["result"]["publishable"], true, "{preview}");
            assert_eq!(preview["result"]["errors"], json!([]), "{preview}");

            // 화면에서 확인한 공유본의 승인과 open의 편집 기준을 그대로 제출한다.
            let published = session
                .call(
                    "akra/issue/publish",
                    json!({
                        "target": world.target(),
                        "text": text,
                        "approval": preview["result"]["approval"],
                        "editBasis": opened["result"]["editBasis"],
                    }),
                )
                .await;
            assert_eq!(published["result"]["status"], "confirmed", "{published}");
            let intent_id = published["result"]["intent"]["intentId"].clone();
            assert!(intent_id.is_string(), "{published}");
            assert_eq!(world.writes(), 1);
            assert_eq!(world.remote_copy().title, text["title"].as_str().unwrap());
            assert_eq!(world.remote_copy().body, text["body"].as_str().unwrap());
            assert_eq!(world.counts(), ApiCalls::default());
            assert!(world.runtime().is_dir());
            session.close().await;

            let mut reassembled = world.start().await;
            let reopened = reassembled.call("akra/issue/open", world.target()).await;
            assert_eq!(
                reopened["result"]["intent"]["state"], "confirmed",
                "{reopened}"
            );
            assert_eq!(
                reopened["result"]["intent"]["intentId"], intent_id,
                "{reopened}"
            );
            let rechecked = reassembled.call("akra/issue/recheck", world.target()).await;
            assert_eq!(
                rechecked["result"]["intent"]["state"], "confirmed",
                "{rechecked}"
            );
            assert_eq!(
                rechecked["result"]["intent"]["intentId"], intent_id,
                "{rechecked}"
            );
            assert_eq!(
                world.writes(),
                1,
                "reassembly and recheck must not send another update"
            );
            assert_eq!(world.counts(), ApiCalls::default());
            reassembled.close().await;
        },
    );
}

async fn publish_request(
    world: &World,
    session: &mut fixtures::Session,
    text: serde_json::Value,
    shared_copy_only: bool,
) -> serde_json::Value {
    let opened = session.call("akra/issue/open", world.target()).await;
    let preview = session
        .call(
            "akra/issue/preview",
            json!({ "target": world.target(), "text": text }),
        )
        .await;
    json!({
        "target": world.target(),
        "text": text,
        "approval": preview["result"]["approval"],
        "editBasis": opened["result"]["editBasis"],
        "sharedCopyOnly": shared_copy_only,
    })
}

fn restorable_text() -> serde_json::Value {
    let body = "synthetic private log";
    json!({
        "title": "Synthetic issue with a restorable log",
        "body": body,
        "spans": [{ "field": "body", "start": 0, "end": body.len(), "class": "log" }],
    })
}

#[test]
fn explicit_shared_copy_skips_sensitive_apis_before_and_after_held() {
    run_isolated(
        "explicit_shared_copy_skips_sensitive_apis_before_and_after_held",
        async {
            let world = World::new();
            let mut session = world.start().await;
            let request = publish_request(&world, &mut session, restorable_text(), false).await;
            let held = session.call("akra/issue/publish", request).await;
            assert_eq!(held["result"]["status"], "held", "{held}");
            assert_eq!(world.writes(), 0);
            let held_calls = world.counts();
            assert!(held_calls.key_get_or_create > 0);
            let request = publish_request(&world, &mut session, restorable_text(), true).await;
            let sent = session.call("akra/issue/publish", request).await;
            assert_eq!(sent["result"]["status"], "confirmed", "{sent}");
            assert_eq!(
                world.counts(),
                held_calls,
                "the explicit transition must not retry key or Vault APIs"
            );
            assert_eq!(world.writes(), 1);
            session.close().await;
        },
    );
}

#[test]
fn first_explicit_shared_copy_request_never_touches_sensitive_apis() {
    run_isolated(
        "first_explicit_shared_copy_request_never_touches_sensitive_apis",
        async {
            let world = World::new();
            let mut session = world.start().await;
            let request = publish_request(&world, &mut session, restorable_text(), true).await;
            let sent = session.call("akra/issue/publish", request).await;
            assert_eq!(sent["result"]["status"], "confirmed", "{sent}");
            assert_eq!(world.writes(), 1);
            assert_eq!(world.counts(), ApiCalls::default());
            session.close().await;
        },
    );
}

#[test]
fn lost_response_and_failed_reads_never_resend_after_reassembly() {
    run_isolated(
        "lost_response_and_failed_reads_never_resend_after_reassembly",
        async {
            let world = World::new();
            world.set_behavior(fixtures::WriteBehavior::LostResponse {
                applied: false,
                fail_reads: true,
            });
            let mut session = world.start().await;
            let request = publish_request(&world, &mut session, public_text(), false).await;
            let pending = session.call("akra/issue/publish", request.clone()).await;
            assert_eq!(
                pending["result"]["status"], "needs_confirmation",
                "{pending}"
            );
            let intent = pending["result"]["intent"]["intentId"].clone();
            assert_eq!(world.writes(), 1);
            session.close().await;

            let mut session = world.start().await;
            let failed_read = session.call("akra/issue/recheck", world.target()).await;
            assert!(failed_read["error"].is_object(), "{failed_read}");
            let retained = crate::composition::production::build_issue_publish_journal()
                .load(world.workspace.to_str().unwrap())
                .unwrap();
            assert_eq!(
                retained[0].state,
                crate::domain::issue_publish_intent::IssuePublishIntentState::NeedsConfirmation
            );
            world.recover_reads(false);
            let other_body = session.call("akra/issue/recheck", world.target()).await;
            assert_eq!(
                other_body["result"]["intent"]["state"], "needs_confirmation",
                "{other_body}"
            );
            let repeated = session.call("akra/issue/publish", request).await;
            assert_eq!(
                repeated["result"]["intent"]["intentId"], intent,
                "{repeated}"
            );
            assert_eq!(world.writes(), 1);
            world.recover_reads(true);
            let confirmed = session.call("akra/issue/recheck", world.target()).await;
            assert_eq!(
                confirmed["result"]["intent"]["state"], "confirmed",
                "{confirmed}"
            );
            assert_eq!(confirmed["result"]["intent"]["intentId"], intent);
            assert_eq!(world.writes(), 1);
            assert_eq!(world.counts(), ApiCalls::default());
            session.close().await;
        },
    );
}

#[test]
fn shared_copy_option_cannot_bypass_required_sections_or_title_validation() {
    run_isolated(
        "shared_copy_option_cannot_bypass_required_sections_or_title_validation",
        async {
            let world = World::new();
            world.policy("[convention]\nrequired_sections = [\"Impact\"]\n");
            let mut session = world.start().await;
            for text in [
                public_text(),
                json!({ "title": "", "body": "## Impact\nSynthetic details", "spans": [] }),
            ] {
                let request = publish_request(&world, &mut session, text, true).await;
                let blocked = session.call("akra/issue/publish", request).await;
                assert_eq!(blocked["result"]["status"], "blocked", "{blocked}");
            }
            assert_eq!(world.writes(), 0);
            assert_eq!(world.counts(), ApiCalls::default());
            session.close().await;
        },
    );
}

#[cfg(windows)]
#[test]
fn configured_templates_and_required_external_validators_remain_blocked() {
    run_isolated(
        "configured_templates_and_required_external_validators_remain_blocked",
        async {
            let world = World::new();
            world.policy("[convention]\ntemplate = \".github/ISSUE_TEMPLATE/bug.md\"\n");
            let mut session = world.start().await;
            let opened = session.call("akra/issue/open", world.target()).await;
            assert!(
                opened["error"].is_object()
                    || opened["result"]["template"]["status"] != "not_configured",
                "{opened}"
            );
            let request = publish_request(&world, &mut session, public_text(), true).await;
            let blocked = session.call("akra/issue/publish", request).await;
            assert_ne!(blocked["result"]["status"], "confirmed", "{blocked}");
            assert_eq!(world.writes(), 0);
            session.close().await;

            world.policy("[[code_validators]]\nid = \"mandatory\"\nrequired = true\ntargets = [\"publish\"]\n");
            let user_policy = format!(
                "[approved_code_validators.mandatory]\ncommand = \"C:/fixture/never-run.exe\"\nsha256 = \"{}\"\n",
                "a".repeat(64),
            );
            use std::io::Write;
            use std::os::windows::fs::OpenOptionsExt;
            let mut policy_file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .access_mode(
                    crate::private_fs::WINDOWS_GENERIC_READ
                        | crate::private_fs::WINDOWS_GENERIC_WRITE
                        | crate::private_fs::WINDOWS_READ_CONTROL
                        | crate::private_fs::WINDOWS_WRITE_DAC,
                )
                .open(isolation::root().join("user/issue-policy.toml"))
                .unwrap();
            crate::private_fs::set_windows_private_acl(&policy_file, false).unwrap();
            policy_file.write_all(user_policy.as_bytes()).unwrap();
            drop(policy_file);
            let mut session = world.start().await;
            let validator_open = session.call("akra/issue/open", world.target()).await;
            assert!(
                validator_open["result"].is_object(),
                "validator policy open failed: {validator_open}"
            );
            let validator_preview = session
                .call(
                    "akra/issue/preview",
                    json!({"target": world.target(), "text": public_text()}),
                )
                .await;
            assert!(
                validator_preview["result"]["approval"].is_object(),
                "validator preview failed: {validator_preview}"
            );
            let request = publish_request(&world, &mut session, public_text(), true).await;
            let blocked = session.call("akra/issue/publish", request).await;
            assert_eq!(blocked["result"]["status"], "blocked", "{blocked}");
            assert_eq!(world.writes(), 0);
            assert_eq!(world.counts(), ApiCalls::default());
            session.close().await;
        },
    );
}

#[cfg(windows)]
mod commit_failures;

#[cfg(windows)]
mod delayed_workers;

#[test]
fn handoff_uses_confirmed_journal_and_holds_unconfirmed_or_edited_copies() {
    run_isolated(
        "handoff_uses_confirmed_journal_and_holds_unconfirmed_or_edited_copies",
        async {
            let world = World::new();
            let mut session = world.start().await;
            let ordinary = session.call("akra/issue/handoffCopy", world.target()).await;
            assert_eq!(
                ordinary["result"]["sharedCopy"]["title"], "Old synthetic title",
                "{ordinary}"
            );
            let request = publish_request(&world, &mut session, public_text(), true).await;
            let confirmed = session.call("akra/issue/publish", request).await;
            assert_eq!(confirmed["result"]["status"], "confirmed", "{confirmed}");
            session.close().await;
            let mut session = world.start().await;
            let handoff = session.call("akra/issue/handoffCopy", world.target()).await;
            assert_eq!(
                handoff["result"]["sharedCopy"]["body"],
                public_text()["body"],
                "{handoff}"
            );
            let private = "synthetic local span reinserted by an external editor";
            world.edit_remote_body(private);
            let withheld = session.call("akra/issue/handoffCopy", world.target()).await;
            assert!(withheld["error"].is_object(), "{withheld}");
            assert!(!withheld.to_string().contains(private));
            world.edit_remote_body(public_text()["body"].as_str().unwrap());
            let next =
                json!({"title":"Next approved title", "body":"Next approved body", "spans":[]});
            let request = publish_request(&world, &mut session, next, true).await;
            world.set_behavior(fixtures::WriteBehavior::LostResponse {
                applied: false,
                fail_reads: true,
            });
            let pending = session.call("akra/issue/publish", request).await;
            assert_eq!(
                pending["result"]["status"], "needs_confirmation",
                "{pending}"
            );
            let failed = session.call("akra/issue/handoffCopy", world.target()).await;
            assert!(failed["error"].is_object(), "{failed}");
            world.recover_reads(false);
            let pending = session.call("akra/issue/handoffCopy", world.target()).await;
            assert!(pending["error"].is_object(), "{pending}");
            assert_eq!(world.writes(), 2);
            assert_eq!(world.counts(), ApiCalls::default());
            session.close().await;
        },
    );
}

#[test]
fn independently_opened_editors_cannot_overwrite_a_newer_confirmed_copy() {
    run_isolated(
        "independently_opened_editors_cannot_overwrite_a_newer_confirmed_copy",
        async {
            let world = World::new();
            let mut first = world.start().await;
            let mut second = world.start().await;
            let first_text = public_text();
            let second_text =
                json!({"title":"Second synthetic title","body":"Second synthetic body"});
            let request_a = publish_request(&world, &mut first, first_text.clone(), false).await;
            let request_b = publish_request(&world, &mut second, second_text.clone(), false).await;
            assert_eq!(request_a["editBasis"], request_b["editBasis"]);
            let (a, b) = tokio::join!(
                first.call("akra/issue/publish", request_a),
                second.call("akra/issue/publish", request_b)
            );
            assert_eq!(world.writes(), 1, "{a} / {b}");
            let (confirmed, stale, winner) = if a["result"]["status"] == "confirmed" {
                (&a, &b, &first_text)
            } else {
                (&b, &a, &second_text)
            };
            assert_eq!(confirmed["result"]["status"], "confirmed", "{confirmed}");
            assert_eq!(stale["result"]["status"], "conflict", "{stale}");
            assert_eq!(world.remote_copy().title, winner["title"].as_str().unwrap());
            assert_eq!(world.remote_copy().body, winner["body"].as_str().unwrap());
            assert_eq!(world.counts(), ApiCalls::default());
            first.close().await;
            second.close().await;
            let mut reopened = world.start().await;
            let recovered = reopened.call("akra/issue/recheck", world.target()).await;
            assert!(recovered["result"].is_object(), "{recovered}");
            assert_eq!(world.writes(), 1);
            reopened.close().await;
        },
    );
}

#[test]
fn two_editors_with_an_uncertain_publish_share_one_durable_intent() {
    run_isolated(
        "two_editors_with_an_uncertain_publish_share_one_durable_intent",
        async {
            let world = World::new();
            world.set_behavior(fixtures::WriteBehavior::LostResponse {
                applied: false,
                fail_reads: false,
            });
            let mut first = world.start().await;
            let mut second = world.start().await;
            let request_a = publish_request(&world, &mut first, public_text(), false).await;
            let request_b = publish_request(&world, &mut second, public_text(), false).await;
            let (a, b) = tokio::join!(
                first.call("akra/issue/publish", request_a.clone()),
                second.call("akra/issue/publish", request_b)
            );
            assert_eq!(a["result"]["status"], "needs_confirmation", "{a}");
            assert_eq!(b["result"]["status"], "needs_confirmation", "{b}");
            let intent = a["result"]["intent"]["intentId"].clone();
            assert!(intent.is_string());
            assert_eq!(b["result"]["intent"]["intentId"], intent);
            assert_eq!(world.writes(), 1);
            first.close().await;
            second.close().await;
            let mut reopened = world.start().await;
            let repeated = reopened.call("akra/issue/publish", request_a).await;
            assert_eq!(repeated["result"]["intent"]["intentId"], intent);
            assert_eq!(world.writes(), 1);
            world.recover_reads(true);
            let confirmed = reopened.call("akra/issue/recheck", world.target()).await;
            assert_eq!(
                confirmed["result"]["intent"]["state"], "confirmed",
                "{confirmed}"
            );
            assert_eq!(confirmed["result"]["intent"]["intentId"], intent);
            assert_eq!(world.writes(), 1);
            assert_eq!(world.counts(), ApiCalls::default());
            reopened.close().await;
        },
    );
}
