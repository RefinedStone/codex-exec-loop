use super::*;
use crate::domain::issue_work::IssueFieldHash;

fn parse(value: Value, meta: Value) -> Result<IssueSourceObservation, IssueSourceFailure> {
    observation::observation(
        value,
        meta,
        &repository(),
        &binding(),
        &Credentials::new().reference(),
        &request(None).disclosure,
    )
}

#[test]
fn execution_sections_and_stable_account_ids_survive_the_shared_copy() {
    let current = parse(issue("10041", "APP-41"), editmeta()).unwrap();
    let IssueSourceContent::Shared(snapshot) = current.content else {
        panic!("expected shared content")
    };
    assert_eq!(
        snapshot.shared_copy.body,
        "## 완료 기준\n\n- 결과가 저장된다"
    );
    assert_eq!(
        snapshot.basis.field_hash,
        IssueFieldHash::of_fields("실행 설명을 보존한다", &snapshot.shared_copy.body)
    );
    assert_eq!(
        snapshot.basis.provider_updated_at,
        "2026-11-20T12:00:00.123+0000"
    );
    assert!(current.content_problems.is_empty());
    assert_eq!(current.assignees.items[0].id, "account-1");
    assert_eq!(current.author.unwrap().id, "creator-1");
    assert_eq!(current.linked_pull_request, IssuePrPresence::Unknown);
    assert_eq!(
        current.label_actors.completeness,
        IssueMetadataCompleteness::Unavailable
    );
    assert!(current.label_actors.items.is_empty());
}

#[test]
fn unsupported_nodes_and_marks_are_typed_even_without_an_unsupported_text_marker() {
    for description in [
        json!({"type":"doc","version":1,"content":[{"type":"panel","content":[]}]}),
        json!({"type":"doc","version":1,"content":[{"type":"paragraph","content":[
            {"type":"text","text":"still visible","marks":[{"type":"textColor","attrs":{"color":"#123456"}}]}]}]}),
        json!({"type":"doc","version":1,"content":[{"type":"codeBlock","content":[
            {"type":"text","text":"visible code","marks":[{"type":"unsupportedCodeMark"}]}]}]}),
    ] {
        let mut value = issue("10041", "APP-41");
        value["fields"]["description"] = description;
        let current = parse(value, editmeta()).unwrap();
        assert!(
            current
                .content_problems
                .contains(&IssueSourceContentProblem::UnsupportedAdf)
        );
        assert!(matches!(current.content, IssueSourceContent::Shared(_)));
    }
}

#[test]
fn oversized_issue_body_becomes_needs_info_without_aborting_the_observation() {
    let text = "x".repeat(1024 * 1024 + 1);
    let mut value = issue("10041", "APP-41");
    value["fields"]["description"] = json!({
        "type": "doc",
        "version": 1,
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]
    });

    let current = parse(value, editmeta()).expect("oversized content remains an issue observation");
    assert_eq!(
        current.content_problems,
        vec![IssueSourceContentProblem::OversizedContent]
    );
    let IssueSourceContent::Withheld { basis, .. } = current.content else {
        panic!("oversized content must not be shared")
    };
    assert_ne!(basis.field_hash, IssueFieldHash::of_fields("title", ""));
}

#[test]
fn unsupported_or_missing_required_fields_are_not_an_execution_ready_empty_template() {
    let mut unsupported = editmeta();
    unsupported["fields"]["customfield_123"] = json!({"required":true,"name":"Approval","schema":{"type":"array","custom":"plugin:approvers"}});
    let current = parse(issue("10041", "APP-41"), unsupported).unwrap();
    assert_eq!(
        current.content_problems,
        vec![IssueSourceContentProblem::UnsupportedRequiredField]
    );

    let mut value = issue("10041", "APP-41");
    value["fields"]["description"] = Value::Null;
    assert_eq!(
        parse(value, editmeta()).unwrap().content_problems,
        vec![IssueSourceContentProblem::MissingRequiredField]
    );

    let mut required_priority = editmeta();
    required_priority["fields"]["priority"] =
        json!({"required":true,"schema":{"type":"priority"},"allowedValues":[{"name":"High"}]});
    assert_eq!(
        parse(issue("10041", "APP-41"), required_priority)
            .unwrap()
            .content_problems,
        vec![IssueSourceContentProblem::UnsupportedRequiredField]
    );
    assert_eq!(
        parse(issue("10041", "APP-41"), json!({})),
        Err(IssueSourceFailure::MalformedResponse)
    );
}

#[test]
fn disclosure_policy_masks_secrets_without_changing_the_remote_comparison_hash() {
    let mut value = issue("10041", "APP-41");
    value["fields"]["description"] = json!({"type":"doc","version":1,"content":[
        {"type":"paragraph","content":[{"type":"text","text":"token=synthetic-secret-sentinel"}]}]});
    let current = parse(value, editmeta()).unwrap();
    let IssueSourceContent::Shared(snapshot) = current.content else {
        panic!("expected shared content")
    };
    assert!(
        !snapshot
            .shared_copy
            .body
            .contains("synthetic-secret-sentinel")
    );
    assert_eq!(
        snapshot.basis.field_hash,
        IssueFieldHash::of_fields("실행 설명을 보존한다", "token=synthetic-secret-sentinel")
    );
}

#[test]
fn category_and_pr_lookup_facts_must_be_coherent_at_the_stored_boundary() {
    let mut value = issue("10041", "APP-41");
    value["fields"]["status"]["statusCategory"]["key"] = json!("done");
    let closed = parse(value, editmeta()).unwrap();
    assert_eq!(closed.remote_state, IssueRemoteState::Closed);
    let mut inconsistent = closed.clone();
    inconsistent.remote_state = IssueRemoteState::Open;
    assert_eq!(
        inconsistent.validate_repository_metadata(),
        Err(IssueSourceFailure::MalformedResponse)
    );
    let mut inconsistent = closed;
    inconsistent.linked_pull_request = IssuePrPresence::Absent;
    assert_eq!(
        inconsistent.validate_repository_metadata(),
        Err(IssueSourceFailure::MalformedResponse)
    );
}

#[test]
fn label_filtering_commits_scanned_progress_without_inventing_a_label_actor() {
    let fixture = Fixture::new();
    fixture.api.page(vec![issue("10041", "APP-41")], true, None);
    let mut selected = request(None);
    selected.selection = IssueDiscoverySelection::LabelsAny(vec!["different-label".into()]);
    let page = fixture.adapter.discover(selected).unwrap();
    assert_eq!(page.scanned_count, 1);
    assert!(page.observations.is_empty());
    assert!(page.next_cursor.is_some());
}
