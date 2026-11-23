use super::*;

#[test]
fn legacy_observations_keep_unknown_visibility_and_do_not_change_repository_identity() {
    let original = observation();
    let repository_before = IssueSourceRepository::from_reference(&original.issue);
    let mut encoded = serde_json::to_value(&original).unwrap();
    encoded
        .as_object_mut()
        .unwrap()
        .remove("repositoryVisibility");
    let restored: IssueSourceObservation = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        restored.repository_visibility,
        IssueRepositoryVisibility::Unknown
    );
    assert_eq!(restored.repository_name_with_owner, None);
    assert_eq!(restored, original);

    for visibility in [
        IssueRepositoryVisibility::Unknown,
        IssueRepositoryVisibility::Private,
        IssueRepositoryVisibility::Internal,
        IssueRepositoryVisibility::Public,
    ] {
        let mut current = restored.clone();
        current.repository_visibility = visibility;
        let round_trip: IssueSourceObservation =
            serde_json::from_value(serde_json::to_value(&current).unwrap()).unwrap();
        assert_eq!(round_trip, current);
        assert_eq!(current.content, original.content);
        assert_eq!(
            IssueSourceRepository::from_reference(&current.issue),
            repository_before
        );
    }
    assert_eq!(
        serde_json::to_value(repository_before).unwrap(),
        serde_json::json!({
            "provider": "github",
            "serverId": "github.com",
            "repositoryId": "R_stable",
            "projectScope": { "workspaceDir": "/work/project" }
        })
    );
}

#[test]
fn repository_coordinates_are_explicit_canonical_metadata_and_never_backfilled_from_web() {
    let mut original = observation();
    original.web = Some(IssueSourceWeb {
        number: 17,
        url: "https://github.com/Sample-Org/widgets.rs/issues/17".into(),
    });
    let legacy = serde_json::to_value(&original).unwrap();
    assert!(legacy.get("repositoryNameWithOwner").is_none());
    let restored: IssueSourceObservation = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(restored.repository_name_with_owner, None);
    for invalid in [
        "Sample-Org/widgets.rs",
        "sample/widgets/extra",
        "sample/%2e%2e",
        "sample/..",
        "/widgets",
        "-sample/widgets",
        "sample/widgets\n",
        " sample/widgets",
    ] {
        let mut encoded = legacy.clone();
        encoded["repositoryNameWithOwner"] = serde_json::json!(invalid);
        assert!(
            serde_json::from_value::<IssueSourceObservation>(encoded).is_err(),
            "{invalid}"
        );
        let mut direct = original.clone();
        direct.repository_name_with_owner = Some(invalid.into());
        assert!(direct.validate_repository_metadata().is_err(), "{invalid}");
    }
    for name in ["sample-org/widgets.rs", "sample/repository.git"] {
        let mut current = original.clone();
        current.repository_name_with_owner = Some(name.into());
        current.validate_repository_metadata().unwrap();
        assert_eq!(
            serde_json::from_value::<IssueSourceObservation>(
                serde_json::to_value(&current).unwrap()
            )
            .unwrap(),
            current
        );
    }
}

#[test]
fn persisted_visibility_rejects_unknown_values_and_null_instead_of_assuming_private() {
    for invalid in [
        serde_json::json!("restricted"),
        serde_json::json!("PRIVATE"),
        serde_json::json!("private\n"),
        serde_json::json!(null),
        serde_json::json!(true),
    ] {
        let mut encoded = serde_json::to_value(observation()).unwrap();
        encoded["repositoryVisibility"] = invalid;
        assert!(serde_json::from_value::<IssueSourceObservation>(encoded).is_err());
    }
}
