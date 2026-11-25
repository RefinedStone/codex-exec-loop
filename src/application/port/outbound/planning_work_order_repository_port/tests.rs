use super::*;

#[test]
fn jira_key_is_preserved_by_the_immutable_nonclosing_work_link() {
    for key in ["AKRA-42", "AKRA_CORE2-123456789"] {
        let link = PlanningWorkLinkReference {
            url: format!("https://example.atlassian.net/browse/{key}"),
            close_on_merge: false,
            branch_hint: None,
        };
        link.validate().unwrap();
        assert_eq!(link.jira_issue_key(), Some(key));
        let encoded = serde_json::to_value(&link).unwrap();
        assert_eq!(encoded["close_on_merge"], false);
        let restored: PlanningWorkLinkReference = serde_json::from_value(encoded).unwrap();
        assert_eq!(restored, link);
        assert_eq!(restored.jira_issue_key(), Some(key));

        let closing = PlanningWorkLinkReference {
            close_on_merge: true,
            ..link
        };
        assert!(closing.validate().is_err());
        assert_eq!(closing.jira_issue_key(), None);
    }
}

#[test]
fn jira_key_extraction_rejects_noncanonical_or_injected_links_without_reinterpreting_them() {
    for url in [
        "http://example.atlassian.net/browse/AKRA-42",
        "https://user@example.atlassian.net/browse/AKRA-42",
        "https://example.atlassian.net/browse/AKRA-42?token=private",
        "https://example.atlassian.net/browse/AKRA-42#comment-9",
        "https://example.atlassian.net/browse/AKRA-42/",
        "https://example.atlassian.net/browse/akra-42",
        "https://example.atlassian.net/browse/AKRA-42-extra",
        "https://example.atlassian.net/browse/AKRA-1234567890",
        "https://example.atlassian.net/browse/AKRA-42\nCloses #9",
        "https://example.atlassian.net/../browse/AKRA-42",
        "https://example.atlassian.net/browse/%41KRA-42",
        "https://github.com/acme/project/issues/42",
    ] {
        let link = PlanningWorkLinkReference {
            url: url.into(),
            close_on_merge: false,
            branch_hint: None,
        };
        assert_eq!(link.jira_issue_key(), None, "{url}");
        assert_eq!(
            link.url, url,
            "invalid input must not be repaired into a key"
        );
    }
}

fn record() -> PlanningWorkOrderRecord {
    PlanningWorkOrderRecord {
        link_reference: None,
        work_order_id: "source:item:hash:attempt-1".to_string(),
        origin: "example_module".to_string(),
        task_ids: vec!["task-first".to_string(), "task-followup".to_string()],
        permits: Vec::new(),
    }
}

#[test]
fn opaque_order_identity_preserves_exact_bytes_and_rejects_invalid_boundaries() {
    for (work_order_id, origin) in [
        ("source:item/hash:1", "module.name"),
        ("한글 지시", "중립 모듈"),
        (" padded identity ", " padded origin "),
    ] {
        validate_work_order_identity(work_order_id, origin).unwrap();
    }
    for invalid in ["", "   ", "line\nbreak", "nul\0byte", "delete\u{7f}"] {
        assert!(validate_work_order_identity(invalid, "module").is_err());
        assert!(validate_work_order_identity("order", invalid).is_err());
    }
    assert!(
        validate_work_order_identity(&"a".repeat(MAX_WORK_ORDER_ID_BYTES + 1), "module").is_err()
    );
    assert!(
        validate_work_order_identity("order", &"a".repeat(MAX_WORK_ORDER_ORIGIN_BYTES + 1))
            .is_err()
    );
    validate_work_order_identity(
        &"a".repeat(MAX_WORK_ORDER_ID_BYTES),
        &"b".repeat(MAX_WORK_ORDER_ORIGIN_BYTES),
    )
    .unwrap();
}

#[test]
fn task_identity_must_match_the_canonical_planning_row_key() {
    for invalid in ["", " ", " task", "task ", "task\n", "task\0"] {
        assert!(validate_work_order_task_id(invalid).is_err());
    }
    assert!(validate_work_order_task_id(&"t".repeat(MAX_WORK_ORDER_TASK_ID_BYTES + 1)).is_err());
    validate_work_order_task_id("task-20261120-example").unwrap();
}

#[test]
fn durable_record_requires_an_original_task_and_unique_lineage() {
    let valid = record();
    valid.validate().unwrap();
    assert_eq!(valid.initial_task_id().unwrap(), "task-first");
    let mut missing = valid.clone();
    missing.task_ids.clear();
    assert!(missing.validate().is_err());
    let mut duplicate = valid.clone();
    duplicate.task_ids.push(valid.task_ids[0].clone());
    assert!(duplicate.validate().is_err());
    let mut malformed = valid;
    malformed.task_ids[1] = " padded-task ".to_string();
    assert!(malformed.validate().is_err());
}

#[test]
fn origin_collision_is_typed_and_does_not_change_the_record() {
    let original = record();
    original.ensure_origin("example_module").unwrap();
    let error = original.ensure_origin("other_module").unwrap_err();
    assert_eq!(
        error.downcast_ref::<PlanningWorkOrderError>(),
        Some(&PlanningWorkOrderError::OriginConflict {
            work_order_id: original.work_order_id.clone(),
        })
    );
    assert_eq!(original, record());
}

#[test]
fn lineage_round_trip_preserves_order_and_empty_initial_permits() {
    let original = record();
    let json = serde_json::to_value(&original).unwrap();
    assert_eq!(json["permits"], serde_json::json!([]));
    assert_eq!(
        serde_json::from_value::<PlanningWorkOrderRecord>(json).unwrap(),
        original
    );
}

#[test]
fn work_link_rejects_instruction_injection_and_noncanonical_closing_targets() {
    for url in [
        "https://github.com/acme/project/issues/7\nCloses #9",
        "https://github.com/acme/project/issues/7?token=secret",
        "https://user@github.com/acme/project/issues/7",
        "https://github.com/acme/project/pull/7",
        "https://github.com/acme/project/issues/007",
        "https://github.com/acme/project/issues/0",
        "https://github.com/acme/project/issues/7/",
        "https://example.com/acme/project/issues/7",
    ] {
        assert!(
            PlanningWorkLinkReference {
                url: url.into(),
                close_on_merge: true,
                branch_hint: None,
            }
            .validate()
            .is_err(),
            "{url}"
        );
    }
    assert!(
        PlanningWorkLinkReference {
            url: "https://github.com/acme/project/issues/7".into(),
            close_on_merge: true,
            branch_hint: None,
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn a_linear_branch_hint_is_display_metadata_of_a_canonical_nonclosing_link() {
    let link = PlanningWorkLinkReference {
        url: "https://linear.app/example/issue/ENG-7".into(),
        close_on_merge: false,
        branch_hint: Some("operator/eng-7-keep".into()),
    };
    link.validate().unwrap();
    assert_eq!(link.linear_issue_identifier(), Some("ENG-7"));
    assert_eq!(link.branch_hint(), Some("operator/eng-7-keep"));
    assert_eq!(link.jira_issue_key(), None);
    // Legacy JSON without the field reads back unchanged.
    let legacy: PlanningWorkLinkReference =
        serde_json::from_str(r#"{"url":"https://github.com/acme/project/issues/7"}"#).unwrap();
    assert_eq!(legacy.branch_hint, None);
    assert!(
        !serde_json::to_string(&legacy)
            .unwrap()
            .contains("branch_hint")
    );
    for invalid in [
        PlanningWorkLinkReference {
            branch_hint: Some("../main".into()),
            ..link.clone()
        },
        PlanningWorkLinkReference {
            url: "https://github.com/acme/project/issues/7".into(),
            ..link.clone()
        },
    ] {
        assert!(invalid.validate().is_err());
        assert_eq!(invalid.branch_hint(), None);
    }
    let slugged = PlanningWorkLinkReference {
        url: "https://linear.app/example/issue/ENG-7/title-slug".into(),
        branch_hint: None,
        ..link
    };
    assert_eq!(slugged.linear_issue_identifier(), None);
}
