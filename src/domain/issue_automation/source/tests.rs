use crate::domain::issue_policy::IssueSensitiveSpanAction as Action;
use crate::domain::issue_work::{IssueFieldHash, IssueSharedCopy};

use super::*;

fn repository() -> IssueSourceRepository {
    IssueSourceRepository {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_stable".to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: "/work/project".to_owned(),
        },
    }
}

fn reference() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_stable".to_owned(),
        issue_id: "I_stable".to_owned(),
        project_scope: repository().project_scope,
    }
}

fn disclosure() -> IssueSourceDisclosure {
    IssueSourceDisclosure {
        policy: IssueDisclosurePolicy::builtin(),
        policy_version: "policy-v1".to_owned(),
    }
}

fn selection() -> IssueDiscoverySelection {
    IssueDiscoverySelection::LabelsAny(vec!["akra:gated".to_owned(), "akra:auto".to_owned()])
}

fn cursor() -> IssueDiscoveryCursor {
    IssueDiscoveryCursor::new(
        repository(),
        selection(),
        25,
        &disclosure(),
        "opaque-after-1",
    )
    .unwrap()
}

fn labels(
    names: &[&str],
    completeness: IssueMetadataCompleteness,
) -> IssueMetadataList<IssueSourceLabel> {
    IssueMetadataList {
        items: names
            .iter()
            .enumerate()
            .map(|(index, name)| IssueSourceLabel {
                id: format!("L_{index}"),
                name: (*name).to_owned(),
            })
            .collect(),
        completeness,
    }
}

fn basis() -> IssueComparisonBasis {
    IssueComparisonBasis {
        field_hash: IssueFieldHash::of_fields("raw-title-sentinel", "raw-secret-sentinel"),
        provider_updated_at: "2026-11-20T12:00:00Z".to_owned(),
    }
}

fn observation() -> IssueSourceObservation {
    IssueSourceObservation {
        issue: reference(),
        web: None,
        remote_state: IssueRemoteState::Open,
        labels: labels(&["akra:auto"], IssueMetadataCompleteness::Complete),
        assignees: IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        author: Some(IssueSourceActor {
            id: "U_author".to_owned(),
            login: "maintainer".to_owned(),
        }),
        label_actors: IssueMetadataList {
            items: vec![IssueLabelAttribution {
                label: IssueSourceLabel {
                    id: "L_0".to_owned(),
                    name: "akra:auto".to_owned(),
                },
                event_id: "LE_1".to_owned(),
                actor: None,
                created_at: "2026-11-20T11:00:00Z".to_owned(),
            }],
            completeness: IssueMetadataCompleteness::Truncated,
        },
        linked_pull_request: IssuePrPresence::Unknown,
        content: IssueSourceContent::Shared(IssueAutomationSnapshot {
            basis: basis(),
            shared_copy: IssueSharedCopy {
                title: "공유 제목".to_owned(),
                body: "[removed]".to_owned(),
            },
            policy_version: "policy-v1".to_owned(),
        }),
    }
}

#[test]
fn labels_any_is_a_union_with_exact_names_and_an_explicit_all_open_choice() {
    let selected = selection();
    assert_eq!(
        selected.matches_labels(&labels(&["akra:auto"], IssueMetadataCompleteness::Complete)),
        Ok(true)
    );
    assert_eq!(
        selected.matches_labels(&labels(
            &["akra:gated"],
            IssueMetadataCompleteness::Complete
        )),
        Ok(true)
    );
    assert_eq!(
        selected.matches_labels(&labels(&["AKRA:AUTO"], IssueMetadataCompleteness::Complete)),
        Ok(false)
    );
    assert_eq!(
        selected.matches_labels(&labels(&[], IssueMetadataCompleteness::Complete)),
        Ok(false)
    );
    assert_eq!(
        IssueDiscoverySelection::AllOpen
            .matches_labels(&labels(&[], IssueMetadataCompleteness::Unavailable)),
        Ok(true)
    );
    assert_eq!(
        IssueDiscoverySelection::LabelsAny(Vec::new()).normalized(),
        Err(IssueSourceFailure::InvalidRequest)
    );
}

#[test]
fn selection_normalizes_order_and_duplicates_without_changing_names() {
    let input = IssueDiscoverySelection::LabelsAny(vec![
        "z".to_owned(),
        "a".to_owned(),
        "z".to_owned(),
        " a".to_owned(),
    ]);
    assert_eq!(
        input.normalized().unwrap(),
        IssueDiscoverySelection::LabelsAny(vec![" a".to_owned(), "a".to_owned(), "z".to_owned()])
    );
    for invalid in ["", " ", "akra\0auto"] {
        assert!(
            IssueDiscoverySelection::LabelsAny(vec![invalid.to_owned()])
                .normalized()
                .is_err()
        );
    }
    let too_many = (0..101).map(|index| format!("label-{index}")).collect();
    assert!(
        IssueDiscoverySelection::LabelsAny(too_many)
            .normalized()
            .is_err()
    );
}

#[test]
fn partial_labels_prove_a_positive_match_but_never_prove_absence() {
    let selected = selection();
    assert_eq!(
        selected.matches_labels(&labels(
            &["akra:auto"],
            IssueMetadataCompleteness::Truncated
        )),
        Ok(true)
    );
    for completeness in [
        IssueMetadataCompleteness::Truncated,
        IssueMetadataCompleteness::Unavailable,
    ] {
        assert_eq!(
            selected.matches_labels(&labels(&["other"], completeness)),
            Err(IssueSourceFailure::MetadataIncomplete)
        );
    }
    assert_eq!(
        selected.matches_labels(&labels(
            &["akra:auto"],
            IssueMetadataCompleteness::Unavailable
        )),
        Err(IssueSourceFailure::MetadataIncomplete)
    );
}

#[test]
fn cursor_round_trip_keeps_scope_and_accepts_equivalent_label_order() {
    let cursor = cursor();
    let record = cursor.to_record();
    assert_eq!(
        record.selection,
        IssueDiscoverySelection::LabelsAny(vec!["akra:auto".to_owned(), "akra:gated".to_owned()])
    );
    let encoded = serde_json::to_string(&record).unwrap();
    let restored = IssueDiscoveryCursor::hydrate(serde_json::from_str(&encoded).unwrap()).unwrap();
    assert_eq!(restored, cursor);
    let equivalent = IssueDiscoverySelection::LabelsAny(vec![
        "akra:auto".to_owned(),
        "akra:gated".to_owned(),
        "akra:auto".to_owned(),
    ]);
    assert_eq!(
        restored.validate_for(&repository(), &equivalent, 25, &disclosure()),
        Ok("opaque-after-1")
    );
}

#[test]
fn cursor_rejects_each_repository_coordinate_change() {
    let changes: &[fn(&mut IssueSourceRepository)] = &[
        |repository| repository.provider = IssueProvider::Jira,
        |repository| repository.server_id = "another-server".to_owned(),
        |repository| repository.repository_id = "R_recreated".to_owned(),
        |repository| repository.project_scope.workspace_dir = "/work/other".to_owned(),
    ];
    for change in changes {
        let cursor = cursor();
        let mut other = repository();
        change(&mut other);
        assert_eq!(
            cursor.validate_for(&other, &selection(), 25, &disclosure()),
            Err(IssueSourceFailure::InvalidCursor)
        );
    }
}

#[test]
fn cursor_binds_selection_page_size_and_policy_version() {
    let cursor = cursor();
    for selected in [
        IssueDiscoverySelection::AllOpen,
        IssueDiscoverySelection::LabelsAny(vec!["akra:auto".to_owned()]),
    ] {
        assert_eq!(
            cursor.validate_for(&repository(), &selected, 25, &disclosure()),
            Err(IssueSourceFailure::InvalidCursor)
        );
    }
    for page_size in [0, 1, 24, 26, 101] {
        assert_eq!(
            cursor.validate_for(&repository(), &selection(), page_size, &disclosure()),
            Err(IssueSourceFailure::InvalidCursor)
        );
    }
    let mut changed = disclosure();
    changed.policy_version = "policy-v2".to_owned();
    assert_eq!(
        cursor.validate_for(&repository(), &selection(), 25, &changed),
        Err(IssueSourceFailure::InvalidCursor)
    );
}

#[test]
fn cursor_binds_all_actual_disclosure_actions_even_when_version_is_reused() {
    let changes: &[fn(&mut IssueDisclosurePolicy)] = &[
        |policy| policy.secret = Action::UserConfirm,
        |policy| policy.name = Action::Remove,
        |policy| policy.path = Action::Remove,
        |policy| policy.log = Action::Remove,
    ];
    for change in changes {
        let cursor = cursor();
        let mut changed = disclosure();
        change(&mut changed.policy);
        assert_eq!(changed.policy_version, disclosure().policy_version);
        assert_eq!(
            cursor.validate_for(&repository(), &selection(), 25, &changed),
            Err(IssueSourceFailure::InvalidCursor)
        );
    }
}

#[test]
fn cursor_hydration_refuses_corrupt_or_noncanonical_records() {
    let changes: &[fn(&mut PersistedIssueDiscoveryCursor)] = &[
        |record| record.version = 0,
        |record| record.version = ISSUE_SOURCE_CURSOR_VERSION + 1,
        |record| record.repository.server_id.clear(),
        |record| record.repository.repository_id.clear(),
        |record| record.repository.project_scope.workspace_dir.clear(),
        |record| record.selection = IssueDiscoverySelection::LabelsAny(Vec::new()),
        |record| record.selection = selection(),
        |record| record.page_size = 0,
        |record| record.page_size = MAX_ISSUE_SOURCE_PAGE_SIZE + 1,
        |record| record.policy_version.clear(),
        |record| record.opaque.clear(),
        |record| record.opaque = "next\npage".to_owned(),
        |record| record.opaque = "next\0page".to_owned(),
        |record| record.opaque = "x".repeat(MAX_CURSOR_BYTES + 1),
    ];
    for change in changes {
        let mut record = cursor().to_record();
        change(&mut record);
        assert_eq!(
            IssueDiscoveryCursor::hydrate(record),
            Err(IssueSourceFailure::InvalidCursor)
        );
    }
}

#[test]
fn cursor_bounds_and_persisted_record_fields_are_explicit() {
    for page_size in [1, MAX_ISSUE_SOURCE_PAGE_SIZE] {
        assert!(
            IssueDiscoveryCursor::new(
                repository(),
                selection(),
                page_size,
                &disclosure(),
                "x".repeat(MAX_CURSOR_BYTES)
            )
            .is_ok()
        );
    }
    for page_size in [0, MAX_ISSUE_SOURCE_PAGE_SIZE + 1] {
        assert_eq!(
            IssueDiscoveryCursor::new(repository(), selection(), page_size, &disclosure(), "after"),
            Err(IssueSourceFailure::InvalidRequest)
        );
    }
    let mut encoded = serde_json::to_value(cursor().to_record()).unwrap();
    encoded["rawBody"] = "must-not-be-a-cursor-field".into();
    assert!(serde_json::from_value::<PersistedIssueDiscoveryCursor>(encoded).is_err());
}

#[test]
fn observation_round_trip_preserves_masked_content_and_distinct_source_hash() {
    let observed = observation();
    let IssueSourceContent::Shared(snapshot) = &observed.content else {
        panic!("fixture should contain a shared copy");
    };
    assert_ne!(
        snapshot.basis.field_hash,
        IssueFieldHash::of_fields(&snapshot.shared_copy.title, &snapshot.shared_copy.body)
    );
    let result = IssueSourceRead::Observed(Box::new(observed));
    let encoded = serde_json::to_string(&result).unwrap();
    assert!(!encoded.contains("raw-title-sentinel"));
    assert!(!encoded.contains("raw-secret-sentinel"));
    assert!(encoded.contains("[removed]"));
    assert_eq!(
        serde_json::from_str::<IssueSourceRead>(&encoded).unwrap(),
        result
    );
}

#[test]
fn withheld_content_keeps_observed_metadata_without_inventing_a_shared_copy() {
    let mut observed = observation();
    observed.remote_state = IssueRemoteState::Closed;
    observed.content = IssueSourceContent::Withheld {
        basis: basis(),
        policy_version: "policy-v1".to_owned(),
        reason: IssueContentWithheldReason::UserConfirmationRequired,
    };
    let encoded = serde_json::to_string(&observed).unwrap();
    assert!(!encoded.contains("sharedCopy"));
    assert!(!encoded.contains("raw-secret-sentinel"));
    assert!(!encoded.contains("raw-title-sentinel"));
    let restored: IssueSourceObservation = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored, observed);
    assert_eq!(restored.remote_state, IssueRemoteState::Closed);

    let mut injected = serde_json::to_value(observed.content).unwrap();
    injected["value"]["sharedCopy"] = serde_json::json!({"title": "unsafe", "body": "unsafe"});
    assert!(serde_json::from_value::<IssueSourceContent>(injected).is_err());
}

#[test]
fn unknown_carries_only_the_requested_identity_and_a_safe_reason() {
    for reason in [
        IssueSourceFailure::InvalidRequest,
        IssueSourceFailure::UnsupportedProvider,
        IssueSourceFailure::ProviderUnavailable,
        IssueSourceFailure::MalformedResponse,
        IssueSourceFailure::IdentityMismatch,
    ] {
        let result = IssueSourceRead::Unknown {
            issue: reference(),
            reason,
        };
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("remoteState"));
        assert!(!encoded.contains("sharedCopy"));
        assert_eq!(
            serde_json::from_str::<IssueSourceRead>(&encoded).unwrap(),
            result
        );

        let mut injected = serde_json::to_value(&result).unwrap();
        injected["value"]["remoteState"] = "closed".into();
        assert!(serde_json::from_value::<IssueSourceRead>(injected).is_err());
    }
}

#[test]
fn metadata_null_and_truncation_do_not_turn_into_empty_or_negative_facts() {
    let mut observed = observation();
    observed.author = None;
    observed.linked_pull_request = IssuePrPresence::Unknown;
    let encoded = serde_json::to_value(&observed).unwrap();
    let restored: IssueSourceObservation = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(restored.author, None);
    assert_eq!(restored.label_actors.items[0].actor, None);
    assert_eq!(
        restored.label_actors.completeness,
        IssueMetadataCompleteness::Truncated
    );
    assert_eq!(restored.linked_pull_request, IssuePrPresence::Unknown);

    for field in ["completeness", "items"] {
        let mut invalid = encoded["labels"].clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<IssueMetadataList<IssueSourceLabel>>(invalid).is_err());
    }
    for invalid in ["missing", "closed", "unchanged"] {
        assert!(serde_json::from_value::<IssueMetadataCompleteness>(invalid.into()).is_err());
        assert!(serde_json::from_value::<IssuePrPresence>(invalid.into()).is_err());
    }
}

#[test]
fn observations_do_not_accept_untyped_content_or_missing_actor_fields() {
    let encoded = serde_json::to_value(observation()).unwrap();
    let mut injected = encoded.clone();
    injected["rawFields"] = serde_json::json!({"title": "raw", "body": "raw"});
    assert!(serde_json::from_value::<IssueSourceObservation>(injected).is_err());
    let mut missing_author = encoded.clone();
    missing_author.as_object_mut().unwrap().remove("author");
    assert!(serde_json::from_value::<IssueSourceObservation>(missing_author).is_err());
    let mut missing_actor = encoded["labelActors"]["items"][0].clone();
    missing_actor.as_object_mut().unwrap().remove("actor");
    assert!(serde_json::from_value::<IssueLabelAttribution>(missing_actor).is_err());
}

#[test]
fn repository_comparison_preserves_the_full_stable_scope() {
    let repository = repository();
    assert!(repository.matches_reference(&reference()));
    assert_eq!(
        IssueSourceRepository::from_reference(&reference()),
        repository
    );
    let mut renamed_display_issue = reference();
    renamed_display_issue.issue_id = "I_another".to_owned();
    assert!(repository.matches_reference(&renamed_display_issue));
    let mut moved_issue = reference();
    moved_issue.repository_id = "R_elsewhere".to_owned();
    assert!(!repository.matches_reference(&moved_issue));
}

#[test]
fn observation_web_is_optional_for_old_json_and_never_fills_an_unknown_read() {
    let legacy = observation();
    let encoded = serde_json::to_value(&legacy).unwrap();
    assert!(encoded.get("web").is_none());
    let restored: IssueSourceObservation = serde_json::from_value(encoded).unwrap();
    assert_eq!(restored.web, None);
    assert_eq!(restored, legacy);

    let mut current = legacy;
    current.web = Some(IssueSourceWeb {
        number: 17,
        url: "https://github.com/sample/widgets/issues/17".to_owned(),
    });
    let encoded = serde_json::to_value(&current).unwrap();
    assert_eq!(
        serde_json::from_value::<IssueSourceObservation>(encoded.clone()).unwrap(),
        current
    );
    let mut unexpected = encoded.clone();
    unexpected["web"]["rawTitle"] = "must-not-enter-web-metadata".into();
    assert!(serde_json::from_value::<IssueSourceObservation>(unexpected).is_err());
    let mut incomplete = encoded;
    incomplete["web"].as_object_mut().unwrap().remove("url");
    assert!(serde_json::from_value::<IssueSourceObservation>(incomplete).is_err());

    let unknown = IssueSourceRead::Unknown {
        issue: reference(),
        reason: IssueSourceFailure::ProviderUnavailable,
    };
    let mut injected = serde_json::to_value(unknown).unwrap();
    injected["value"]["web"] = serde_json::to_value(current.web.unwrap()).unwrap();
    assert!(serde_json::from_value::<IssueSourceRead>(injected).is_err());
}
