use super::*;
use crate::domain::issue_automation::source::IssueRepositoryVisibility;

#[test]
fn visibility_changes_update_current_metadata_without_rebinding_or_changing_body_basis() {
    let fixture = Fixture::new("collector-current-visibility");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let mut private = observed(&fixture, "I_A", "unchanged body");
    private.repository_visibility = IssueRepositoryVisibility::Private;
    private.repository_name_with_owner = Some("sample/widgets".into());
    private.web = Some(IssueSourceWeb {
        number: 17,
        url: "https://github.com/sample/widgets/issues/17".into(),
    });
    let first = commit(
        adapter
            .apply_discovery_page(
                fixture.cwd(),
                initial.fence(),
                &request,
                &page(&fixture, &request, vec![private.clone()], None),
            )
            .unwrap(),
    );
    let before = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(
        before.items[0].current,
        Some(IssueSourceRead::Observed(Box::new(private.clone())))
    );

    let mut public = private.clone();
    public.repository_visibility = IssueRepositoryVisibility::Public;
    let changed = resync_one(
        &fixture,
        &first.state,
        IssueSourceRead::Observed(Box::new(public.clone())),
    );
    let after = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert!(after.revision > before.revision);
    assert_eq!(after.items[0].binding_id, before.items[0].binding_id);
    assert_eq!(
        after.items[0].current,
        Some(IssueSourceRead::Observed(Box::new(public.clone())))
    );
    assert_eq!(public.content, private.content);
    assert_eq!(public.issue, private.issue);
    assert_eq!(changed.state.binding_id, first.state.binding_id);
    assert_eq!(changed.projection.binding, first.projection.binding);

    resync_one(&fixture, &changed.state, unknown(&public.issue));
    let unreadable = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(unreadable.items[0].binding_id, before.items[0].binding_id);
    assert_eq!(unreadable.items[0].current, Some(unknown(&public.issue)));
}

#[test]
fn stored_legacy_visibility_is_unknown_but_malformed_present_metadata_fails_closed() {
    let fixture = Fixture::new("collector-legacy-visibility");
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(&fixture, &initial);
    let mut value = observed(&fixture, "I_A", "legacy body");
    value.repository_visibility = IssueRepositoryVisibility::Private;
    value.repository_name_with_owner = Some("sample/widgets".into());
    let stored = commit(
        adapter
            .apply_discovery_page(
                fixture.cwd(),
                initial.fence(),
                &request,
                &page(&fixture, &request, vec![value.clone()], None),
            )
            .unwrap(),
    );
    let mut encoded =
        serde_json::to_value(IssueSourceRead::Observed(Box::new(value.clone()))).unwrap();
    encoded["value"]
        .as_object_mut()
        .unwrap()
        .remove("repositoryVisibility");
    encoded["value"]
        .as_object_mut()
        .unwrap()
        .remove("repositoryNameWithOwner");
    {
        let database = Connection::open(fixture.database()).unwrap();
        database
            .execute(
                "UPDATE issue_source_observations SET current_read = ?1",
                [encoded.to_string()],
            )
            .unwrap();
    }
    value.repository_visibility = IssueRepositoryVisibility::Unknown;
    value.repository_name_with_owner = None;
    let view = adapter
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    assert_eq!(
        view.items[0].current,
        Some(IssueSourceRead::Observed(Box::new(value)))
    );
    assert_eq!(
        Some(view.items[0].binding_id.clone()),
        stored.state.binding_id
    );

    encoded["value"]["repositoryVisibility"] = serde_json::json!("trusted_private");
    {
        let database = Connection::open(fixture.database()).unwrap();
        database
            .execute(
                "UPDATE issue_source_observations SET current_read = ?1",
                [encoded.to_string()],
            )
            .unwrap();
    }
    assert!(
        adapter
            .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
            .is_err()
    );
}
