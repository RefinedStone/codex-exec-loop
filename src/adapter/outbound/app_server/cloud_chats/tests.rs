use std::time::SystemTime;

use super::http::split_http_response;
use super::payload::MAX_CONVERSATION_MESSAGES;
use super::*;

#[cfg(unix)]
mod port;

#[test]
fn reads_retry_after_from_the_final_response_headers() {
    let stdout = "HTTP/1.1 103 Early Hints\r\nRetry-After: 99\r\n\r\nHTTP/2 429\r\nrEtRy-AfTeR: 15\r\n\r\n{\"a\":1}\nline2\n429";
    let parsed = split_http_response(stdout, SystemTime::UNIX_EPOCH).expect("parses");
    assert_eq!(parsed.status, 429);
    assert_eq!(parsed.retry_after_seconds, Some(15));
    assert_eq!(parsed.body, "{\"a\":1}\nline2");
}

#[test]
fn splits_when_retry_after_header_is_absent() {
    let stdout = "HTTP/2 200\n\n{}\n200";
    let parsed = split_http_response(stdout, SystemTime::UNIX_EPOCH).expect("parses");
    assert_eq!(parsed.status, 200);
    assert_eq!(parsed.retry_after_seconds, None);
    assert_eq!(parsed.body, "{}");
}

#[test]
fn converts_a_retry_after_http_date_to_remaining_seconds() {
    let stdout = "HTTP/2 429\r\nRetry-After: Thu, 01 Jan 1970 00:00:15 GMT\r\n\r\n{}\n429";
    let now = SystemTime::UNIX_EPOCH + Duration::from_millis(3_500);
    let parsed = split_http_response(stdout, now).expect("parses");
    assert_eq!(parsed.retry_after_seconds, Some(12));
}

#[test]
fn parses_the_conversation_list() {
    let body = r#"{"items":[{"id":"c-1","title":"첫 대화"},{"id":"c-2","title":""},{"id":"c-3"}]}"#;
    let chats = parse_chat_list(body).expect("parses");
    assert_eq!(chats.len(), 3);
    assert_eq!(chats[0].id, "c-1");
    assert_eq!(chats[0].title, "첫 대화");
    assert_eq!(chats[1].title, "(제목 없음)");
    assert_eq!(chats[2].title, "(제목 없음)");
}

#[test]
fn walks_the_mapping_oldest_first_and_keeps_user_and_assistant_text() {
    let body = r#"{
        "title":"T",
        "current_node":"n3",
        "mapping":{
            "root":{"parent":null,"message":null},
            "n1":{"parent":"root","message":{"id":"m1","author":{"role":"user"},"content":{"content_type":"text","parts":["hello"]}}},
            "n2":{"parent":"n1","message":{"id":"m2","author":{"role":"assistant"},"content":{"content_type":"text","parts":["hi ","there"]}}},
            "n3":{"parent":"n2","message":{"id":"m3","author":{"role":"tool"},"content":{"content_type":"text","parts":["ignored"]}}}
        }
    }"#;
    let conversation = parse_conversation("conv-x", body).expect("parses");
    assert_eq!(conversation.id, "conv-x");
    assert_eq!(conversation.title, "T");
    assert_eq!(conversation.messages.len(), 2);
    assert_eq!(conversation.messages[0].role, "user");
    assert_eq!(conversation.messages[0].text, "hello");
    assert_eq!(conversation.messages[1].role, "assistant");
    assert_eq!(conversation.messages[1].text, "hi \nthere");
}

#[test]
fn drops_hidden_and_empty_messages() {
    let body = r#"{
        "current_node":"n2",
        "mapping":{
            "n1":{"parent":null,"message":{"id":"m1","author":{"role":"assistant"},"content":{"content_type":"text","parts":[""]},"metadata":{}}},
            "n2":{"parent":"n1","message":{"id":"m2","author":{"role":"user"},"content":{"content_type":"text","parts":["visible"]},"metadata":{"is_visually_hidden_from_conversation":true}}}
        }
    }"#;
    let conversation = parse_conversation("c", body).expect("parses");
    assert!(conversation.messages.is_empty());
}

#[test]
fn rejects_an_unsafe_conversation_id() {
    assert!(is_safe_conversation_id(
        "6b001ae7-5a64-83ee-a0dd-4379a74c8a31"
    ));
    assert!(!is_safe_conversation_id("../secrets"));
    assert!(!is_safe_conversation_id("a/b"));
    assert!(!is_safe_conversation_id(""));
}

#[test]
fn keeps_the_text_of_a_message_with_attachments() {
    let body = r#"{
        "current_node":"n1",
        "mapping":{
            "n1":{"parent":null,"message":{"id":"m1","author":{"role":"user"},"recipient":"all","content":{"content_type":"multimodal_text","parts":["사진",{"content_type":"image_asset_pointer","asset_pointer":"file-service://x"},"이게 뭐야?"]}}}
        }
    }"#;
    let conversation = parse_conversation("c", body).expect("parses");
    assert_eq!(conversation.messages.len(), 1);
    assert_eq!(conversation.messages[0].text, "사진\n이게 뭐야?");
}

#[test]
fn drops_assistant_messages_sent_to_a_tool() {
    let body = r#"{
        "current_node":"n2",
        "mapping":{
            "n1":{"parent":null,"message":{"id":"m1","author":{"role":"assistant"},"recipient":"bio","content":{"content_type":"text","parts":["remember this"]}}},
            "n2":{"parent":"n1","message":{"id":"m2","author":{"role":"assistant"},"recipient":"all","content":{"content_type":"text","parts":["저장했습니다."]}}}
        }
    }"#;
    let conversation = parse_conversation("c", body).expect("parses");
    assert_eq!(conversation.messages.len(), 1);
    assert_eq!(conversation.messages[0].id, "m2");
}

#[test]
fn a_conversation_over_the_limit_keeps_the_newest_messages() {
    let count = MAX_CONVERSATION_MESSAGES + 2;
    let nodes: Vec<String> = (0..count)
        .map(|index| {
            let parent = if index == 0 {
                "null".to_string()
            } else {
                format!("\"n{}\"", index - 1)
            };
            format!(
                r#""n{index}":{{"parent":{parent},"message":{{"id":"m{index}","author":{{"role":"user"}},"content":{{"content_type":"text","parts":["t{index}"]}}}}}}"#
            )
        })
        .collect();
    let body = format!(
        r#"{{"current_node":"n{}","mapping":{{{}}}}}"#,
        count,
        nodes.join(",")
            + &format!(
                r#","n{count}":{{"parent":"n{}","message":{{"id":"tool","author":{{"role":"assistant"}},"recipient":"dalle.text2im","content":{{"content_type":"text","parts":["tool-only"]}}}}}}"#,
                count - 1
            )
    );
    let conversation = parse_conversation("c", &body).expect("parses");
    assert_eq!(conversation.messages.len(), MAX_CONVERSATION_MESSAGES);
    assert_eq!(conversation.messages[0].id, "m2");
    assert_eq!(
        conversation.messages.last().expect("has messages").id,
        format!("m{}", count - 1)
    );
}

#[test]
fn a_retry_after_blocks_requests_and_returns_the_remaining_time() {
    let mut cache = ChatsCache::default();
    let now = Instant::now();
    let _initial = cache.fetch(
        || {
            Err(ChatsError::RateLimited {
                retry_after_seconds: Some(15),
            })
        },
        || now,
    );
    let outcome = cache.fetch(
        || panic!("must not call upstream during Retry-After"),
        || now + Duration::from_millis(3_500),
    );
    assert!(matches!(
        outcome,
        Err(ChatsError::RateLimited {
            retry_after_seconds: Some(12)
        })
    ));
}

#[test]
fn requests_resume_after_retry_after_expires() {
    let mut cache = ChatsCache::default();
    let now = Instant::now();
    let _initial = cache.fetch(
        || {
            Err(ChatsError::RateLimited {
                retry_after_seconds: Some(15),
            })
        },
        || now,
    );
    let outcome = cache.fetch(|| Ok("fresh".to_owned()), || now + Duration::from_secs(15));
    assert_eq!(outcome.expect("request allowed at expiration"), "fresh");
}

#[test]
fn changing_accounts_drops_cached_content_and_the_previous_accounts_cooldown() {
    let mut cache = ChatsCache::default();
    cache.select_account("account-a");
    cache.list = Some(vec![ChatSummary {
        id: "c-1".to_owned(),
        title: "Private A".to_owned(),
    }]);
    cache.remember(ChatConversation {
        id: "c-1".to_owned(),
        title: "Private A".to_owned(),
        messages: Vec::new(),
    });
    let now = Instant::now();
    let _initial = cache.fetch(
        || {
            Err(ChatsError::RateLimited {
                retry_after_seconds: Some(15),
            })
        },
        || now,
    );

    cache.select_account("account-b");

    assert!(cache.list.is_none());
    assert!(cache.reads.is_empty());
    assert!(cache.fetch(|| Ok(String::new()), || now).is_ok());
}

#[test]
fn a_full_cache_evicts_only_the_oldest_conversation() {
    let mut cache = ChatsCache::default();
    for index in 0..=MAX_CACHED_CONVERSATIONS {
        cache.remember(ChatConversation {
            id: index.to_string(),
            title: String::new(),
            messages: Vec::new(),
        });
    }
    let retained: Vec<_> = cache.reads.iter().map(|chat| chat.id.clone()).collect();
    let expected: Vec<_> = (1..=MAX_CACHED_CONVERSATIONS)
        .rev()
        .map(|index| index.to_string())
        .collect();
    assert_eq!(retained, expected);
}

#[test]
fn cached_results_carry_the_limit_while_cache_misses_return_the_limit_error() {
    let stale = last_good(Some(&"previous"), Some(10)).expect("last successful response");
    assert_eq!(stale.value, "previous");
    assert_eq!(
        stale.limited.expect("stale flag").retry_after_seconds,
        Some(10)
    );
    assert!(matches!(
        last_good::<String>(None, Some(10)),
        Err(ChatsError::RateLimited {
            retry_after_seconds: Some(10)
        })
    ));
}

/// 실계정 읽기 확인이다 (조회만). 네트워크와 실토큰이 필요하므로 `#[ignore]` 이다. 개수만 찍는다.
/// 수동 실행: `cargo test --lib cloud_chats::tests::live_chat_list -- --ignored --nocapture`.
#[test]
#[ignore = "real ChatGPT account and network; run manually"]
fn live_chat_list() {
    let auth_path = super::super::cloud_upstream::codex_auth_path().expect("auth path");
    match CloudChatsRestAdapter::new(auth_path).list_chats() {
        Ok(chats) => println!("chats: {}", chats.value.len()),
        Err(ChatsError::RateLimited {
            retry_after_seconds,
        }) => println!("rate limited; retry after {retry_after_seconds:?}"),
        Err(error) => panic!("list failed: {error}"),
    }
}
