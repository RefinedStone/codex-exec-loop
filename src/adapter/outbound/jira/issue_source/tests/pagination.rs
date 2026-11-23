use super::*;

fn opaque(cursor: &IssueDiscoveryCursor) -> &str {
    cursor
        .validate_for(
            &repository(),
            &request(None).selection,
            2,
            &request(None).disclosure,
        )
        .unwrap()
}

#[test]
fn continuation_keeps_its_window_and_replay_is_deterministic() {
    let fixture = Fixture::new();
    fixture.api.page(vec![], false, Some("page-two"));
    let first = fixture
        .adapter
        .discover(request(None))
        .unwrap()
        .next_cursor
        .unwrap();
    fixture.now.store(NOW + 60_000, Ordering::SeqCst);
    fixture.api.page(vec![], true, None);
    let second = fixture
        .adapter
        .discover(request(Some(first.clone())))
        .unwrap()
        .next_cursor
        .unwrap();
    fixture.api.page(vec![], true, None);
    let replay = fixture
        .adapter
        .discover(request(Some(first)))
        .unwrap()
        .next_cursor
        .unwrap();
    assert_eq!(second, replay);
    let queries = fixture.queries();
    assert_eq!(queries[0].0, queries[1].0);
    assert_eq!(queries[1], queries[2]);
    assert_eq!(queries[0].1, None);
    assert_eq!(queries[1].1.as_deref(), Some("page-two"));
    assert!(
        queries[0]
            .0
            .contains("project = 10001 AND (assignee = currentUser())")
    );
    assert!(queries[0].0.ends_with("ORDER BY updated ASC, id ASC"));
    assert_eq!(
        serde_json::from_str::<Value>(opaque(&second)).unwrap()["watermark"],
        NOW
    );
}

#[test]
fn completed_cursor_progresses_with_the_same_or_a_backwards_clock() {
    let fixture = Fixture::new();
    let mut previous = None;
    for now in [NOW, NOW, NOW - 10_000] {
        fixture.now.store(now, Ordering::SeqCst);
        fixture.api.page(vec![], true, None);
        let page = fixture.adapter.discover(request(previous.clone())).unwrap();
        assert!(page.observations.is_empty());
        assert_eq!(page.jira_connection, Some(fixture.credentials.reference()));
        assert_eq!(page.jira_binding, Some(fixture.adapter.binding.clone()));
        let next = page.next_cursor;
        assert_ne!(next, previous);
        let value: Value = serde_json::from_str(opaque(next.as_ref().unwrap())).unwrap();
        assert_eq!(value["watermark"], NOW);
        previous = next;
    }
}

#[test]
fn overlap_and_periodic_full_backfill_find_late_indexed_older_issues() {
    let fixture = Fixture::new();
    fixture.api.page(vec![], true, None);
    let first = fixture.adapter.discover(request(None)).unwrap().next_cursor;
    fixture.now.store(NOW + 60_000, Ordering::SeqCst);
    fixture.api.page(vec![], true, None);
    let second = fixture
        .adapter
        .discover(request(first))
        .unwrap()
        .next_cursor;
    fixture
        .now
        .store(NOW + cursor::BACKFILL_MS + 1, Ordering::SeqCst);
    fixture.api.page(vec![issue("10041", "APP-41")], true, None);
    let third = fixture.adapter.discover(request(second)).unwrap();
    assert_eq!(third.observations[0].issue.issue_id, "10041");
    let queries = fixture.queries();
    assert!(queries[0].0.contains("updated >= 0"));
    assert!(
        queries[1]
            .0
            .contains(&format!("updated >= {}", NOW - 10 * 60 * 1000))
    );
    assert!(queries[2].0.contains("updated >= 0"));
}

#[test]
fn changed_connection_or_filter_restarts_from_a_full_page_without_reusing_the_token() {
    for change_connection in [true, false] {
        let mut fixture = Fixture::new();
        fixture.api.page(vec![], false, Some("old-account-page"));
        let first = fixture.adapter.discover(request(None)).unwrap().next_cursor;
        if change_connection {
            fixture.credentials.rotate();
        } else {
            fixture.adapter.binding.jql = "labels = ready".into();
        }
        fixture.api.page(vec![], true, None);
        let page = fixture.adapter.discover(request(first)).unwrap();
        assert_eq!(page.jira_binding, Some(fixture.adapter.binding.clone()));
        let queries = fixture.queries();
        assert!(queries[1].0.contains("updated >= 0"));
        assert!(queries[1].1.is_none());
    }
}

#[test]
fn expired_continuation_restarts_once_but_transport_failure_keeps_the_checkpoint() {
    for error in [
        ProviderCredentialError::InvalidRequest,
        ProviderCredentialError::NotFound,
    ] {
        let fixture = Fixture::new();
        fixture.api.page(vec![], false, Some("expired"));
        let first = fixture.adapter.discover(request(None)).unwrap().next_cursor;
        fixture.api.prefix();
        fixture.api.fail(error);
        fixture.api.answer(json!({"issues":[],"isLast":true}));
        fixture.adapter.discover(request(first)).unwrap();
        let queries = fixture.queries();
        assert_eq!(queries.len(), 3);
        assert_eq!(queries[1].1.as_deref(), Some("expired"));
        assert!(queries[2].1.is_none());
        assert!(queries[2].0.contains("updated >= 0"));
    }
    let fixture = Fixture::new();
    fixture.api.page(vec![], false, Some("retained"));
    let original = fixture.adapter.discover(request(None)).unwrap().next_cursor;
    fixture.api.prefix();
    fixture.api.fail(ProviderCredentialError::Unavailable);
    assert_eq!(
        fixture.adapter.discover(request(original.clone())),
        Err(IssueSourceFailure::ProviderUnavailable)
    );
    assert_eq!(fixture.queries().len(), 2);
    fixture.api.page(vec![], true, None);
    fixture.adapter.discover(request(original)).unwrap();
    assert_eq!(fixture.queries()[1], fixture.queries()[2]);
}

#[test]
fn malformed_duplicate_or_nonadvancing_pages_never_publish_a_next_cursor() {
    for value in [
        json!({"issues":[]}),
        json!({"issues":[],"isLast":false}),
        json!({"issues":[],"isLast":false,"nextPageToken":""}),
        json!({"issues":[issue("1","APP-1"),issue("2","APP-2"),issue("3","APP-3")],"isLast":true}),
    ] {
        let fixture = Fixture::new();
        fixture.api.prefix();
        fixture.api.answer(value);
        assert_eq!(
            fixture.adapter.discover(request(None)),
            Err(IssueSourceFailure::MalformedResponse)
        );
    }
    let fixture = Fixture::new();
    fixture
        .api
        .page(vec![issue("1", "APP-1"), issue("1", "APP-1")], true, None);
    assert_eq!(
        fixture.adapter.discover(request(None)),
        Err(IssueSourceFailure::MalformedResponse)
    );
    let fixture = Fixture::new();
    fixture.api.page(vec![], false, Some("same-token"));
    let first = fixture.adapter.discover(request(None)).unwrap().next_cursor;
    fixture.api.page(vec![], false, Some("same-token"));
    assert_eq!(
        fixture.adapter.discover(request(first)),
        Err(IssueSourceFailure::MalformedResponse)
    );
}

#[test]
fn tampered_inner_identity_and_exhausted_revision_are_rejected() {
    let fixture = Fixture::new();
    fixture.api.page(vec![], true, None);
    let first = fixture
        .adapter
        .discover(request(None))
        .unwrap()
        .next_cursor
        .unwrap();
    for (field, value) in [
        ("revision", json!(9_007_199_254_740_991u64)),
        ("version", json!(2)),
    ] {
        let mut inner: Value = serde_json::from_str(opaque(&first)).unwrap();
        inner[field] = value;
        let cursor = IssueDiscoveryCursor::new(
            repository(),
            request(None).selection,
            2,
            &request(None).disclosure,
            inner.to_string(),
        )
        .unwrap();
        fixture.api.prefix();
        if field == "revision" {
            fixture.api.answer(json!({"issues":[],"isLast":true}));
        }
        assert_eq!(
            fixture.adapter.discover(request(Some(cursor))),
            Err(IssueSourceFailure::InvalidCursor)
        );
    }
}
