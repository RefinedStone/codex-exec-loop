use super::*;

#[test]
fn merge_revision_is_bounded_for_both_direct_grants_and_persisted_json() {
    for revision in [0, i64::MAX as u64 + 1, u64::MAX] {
        let authority = PlanningWorkDeliveryAuthority::Auto {
            merge_gate_revision: revision,
            source_repository: "owner/repo".into(),
        };
        assert!(authority.validate().is_err());
        assert!(
            serde_json::from_value::<PlanningWorkDeliveryAuthority>(
                serde_json::to_value(authority).unwrap()
            )
            .is_err()
        );
    }
    for revision in [1, i64::MAX as u64] {
        let authority = PlanningWorkDeliveryAuthority::Auto {
            merge_gate_revision: revision,
            source_repository: "owner/repo".into(),
        };
        authority.validate().unwrap();
        assert_eq!(
            serde_json::from_value::<PlanningWorkDeliveryAuthority>(
                serde_json::to_value(&authority).unwrap()
            )
            .unwrap(),
            authority
        );
    }
}

#[test]
fn persisted_repository_validation_matches_canonical_source_metadata_without_normalization() {
    for name in [
        "owner/repo",
        "sample-org/widgets.rs",
        "o/-repo-",
        "owner/repository.git",
        "Owner/repo",
        "owner/Repo",
        "owner._/repo",
        "owner./repo",
        "owner_/repo",
        "-owner/repo",
        "owner-/repo",
        "owner//repo",
        "owner/repo/extra",
        "owner/..",
        "/repo",
        "owner/",
        "owner/repo\n",
        " owner/repo",
        "owner/%2e",
    ] {
        let expected =
            crate::domain::issue_automation::source::validate_repository_name_with_owner(name)
                .is_ok();
        let authority = PlanningWorkDeliveryAuthority::Auto {
            merge_gate_revision: 1,
            source_repository: name.into(),
        };
        assert_eq!(authority.validate().is_ok(), expected, "{name:?}");
        let encoded = serde_json::to_value(&authority).unwrap();
        assert_eq!(
            serde_json::from_value::<PlanningWorkDeliveryAuthority>(encoded).is_ok(),
            expected,
            "{name:?}"
        );
    }
    let oversized = format!("owner/{}", "r".repeat(4091));
    assert!(oversized.len() > 4096);
    assert!(!valid_repository(&oversized));
}
