use super::*;

#[test]
fn current_visibility_is_bound_to_the_stable_repository_and_canonical_issue_url() {
    let mut previous_content = None;
    for (wire, expected) in [
        ("PRIVATE", IssueRepositoryVisibility::Private),
        ("INTERNAL", IssueRepositoryVisibility::Internal),
        ("PUBLIC", IssueRepositoryVisibility::Public),
    ] {
        let mut value = issue("I_1");
        value["repository"]["visibility"] = json!(wire);
        let (adapter, calls) = fake(vec![
            Reply::Json(page(vec![value.clone()], false, false, None)),
            Reply::Json(read_reply(value)),
        ]);
        let page = adapter.discover(discovery_request()).unwrap();
        let current = observed(read(&adapter));
        assert_eq!(page.repository, repository());
        assert_eq!(page.observations, vec![current.clone()]);
        assert_eq!(current.repository_visibility, expected);
        assert_eq!(
            current.repository_name_with_owner.as_deref(),
            Some("sample/widgets")
        );
        assert_eq!(current.issue, reference("I_1"));
        assert_eq!(
            current.web.as_ref().unwrap().url,
            "https://github.com/sample/widgets/issues/17"
        );
        if let Some(previous) = &previous_content {
            assert_eq!(
                &current.content, previous,
                "visibility must not alter the v1 body basis"
            );
        }
        previous_content = Some(current.content);
        for call in calls.lock().unwrap().iter() {
            assert!(
                argument(call, "query")
                    .unwrap()
                    .contains("repository { id nameWithOwner visibility }")
            );
        }
    }
}

#[test]
fn canonical_repository_coordinate_uses_the_same_response_and_preserves_display_case() {
    let mut value = issue("I_1");
    value["repository"]["nameWithOwner"] = json!("Sample-Org/Widgets.rs");
    value["url"] = json!("https://github.com/Sample-Org/Widgets.rs/issues/17");
    let (adapter, _) = fake(vec![Reply::Json(read_reply(value))]);
    let current = observed(read(&adapter));
    assert_eq!(
        current.repository_name_with_owner.as_deref(),
        Some("sample-org/widgets.rs")
    );
    assert_eq!(
        current.web.unwrap().url,
        "https://github.com/Sample-Org/Widgets.rs/issues/17"
    );
    assert_eq!(current.issue, reference("I_1"));
}

#[test]
fn provider_visibility_is_required_and_never_defaults_to_a_private_observation() {
    let mut missing = issue("I_1");
    missing["repository"]
        .as_object_mut()
        .unwrap()
        .remove("visibility");
    let mut cases = vec![missing];
    for visibility in [
        json!(null),
        json!(true),
        json!("UNKNOWN"),
        json!("private"),
        json!("PRIVATE\n"),
    ] {
        let mut value = issue("I_1");
        value["repository"]["visibility"] = visibility;
        cases.push(value);
    }
    for value in cases {
        let (adapter, _) = fake(vec![
            Reply::Json(page(vec![value.clone()], false, false, None)),
            Reply::Json(read_reply(issue("I_1"))),
            Reply::Json(read_reply(value)),
        ]);
        assert_eq!(
            adapter.discover(discovery_request()).unwrap_err(),
            IssueSourceFailure::MalformedResponse
        );
        assert_eq!(
            observed(read(&adapter)).repository_visibility,
            IssueRepositoryVisibility::Private
        );
        assert_eq!(
            read(&adapter),
            IssueSourceRead::Unknown {
                issue: reference("I_1"),
                reason: IssueSourceFailure::MalformedResponse,
            }
        );
    }
}

#[test]
fn a_private_fork_or_unmatched_repository_name_cannot_replace_the_public_issue_source() {
    let mut public = issue("I_1");
    public["repository"]["visibility"] = json!("PUBLIC");
    let mut private_fork = issue("I_1");
    private_fork["repository"]["id"] = json!("R_private_fork");
    private_fork["repository"]["nameWithOwner"] = json!("operator/private-fork");
    private_fork["url"] = json!("https://github.com/operator/private-fork/issues/17");
    let mut mismatched_name = issue("I_1");
    mismatched_name["repository"]["nameWithOwner"] = json!("operator/private-fork");
    let (adapter, _) = fake(vec![
        Reply::Json(read_reply(public)),
        Reply::Json(read_reply(private_fork)),
        Reply::Json(read_reply(mismatched_name)),
    ]);
    assert_eq!(
        observed(read(&adapter)).repository_visibility,
        IssueRepositoryVisibility::Public
    );
    for _ in 0..2 {
        assert_eq!(
            read(&adapter),
            IssueSourceRead::Unknown {
                issue: reference("I_1"),
                reason: IssueSourceFailure::IdentityMismatch,
            }
        );
    }
}
