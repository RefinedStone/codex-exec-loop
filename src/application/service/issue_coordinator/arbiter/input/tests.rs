use super::*;
use crate::domain::issue_policy::IssuePolicy;

#[test]
fn arbiter_evidence_selection_ignores_bare_identifiers_and_keeps_explicit_paths() {
    assert_eq!(
        selected_paths(
            "`gated` `timeout_arbiter` `field_hash` `due_at` `src/path` `src/lib.rs` `README.md`"
        )
        .unwrap(),
        [
            "Cargo.toml",
            "README.md",
            "package.json",
            "src/lib.rs",
            "src/path"
        ]
    );
}

#[test]
fn arbiter_bare_identifiers_do_not_consume_the_evidence_path_budget() {
    let body = (0..32)
        .map(|index| format!("`field_{index}`"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        selected_paths(&body).unwrap(),
        ["Cargo.toml", "README.md", "package.json"]
    );
}

#[test]
fn arbiter_malformed_or_incomplete_mentions_still_reject_the_whole_selection() {
    for body in [
        "`gated` `../outside.rs`",
        "`timeout_arbiter` `src/**`",
        "`field_hash` `src/unclosed.rs",
        "`due_at` `desktop/src/../private.ts`",
    ] {
        assert_eq!(
            selected_paths(body),
            Err(IssueArbiterWaitReason::EvidenceUnavailable),
            "{body}"
        );
    }
}

#[test]
fn arbiter_windows_workspace_is_rejected_before_json_escaping_hides_it() {
    let workspace = r"C:\Users\fixture\private-repository";
    let input = serde_json::json!({
        "repository": {
            "entries": [{"kind":"file", "path":"README.md", "text":format!("local checkout: {workspace}\\src\\lib.rs")}]
        }
    });
    assert!(
        !serde_json::to_string(&input).unwrap().contains(workspace),
        "the old serialized-string check misses escaped Windows paths"
    );
    assert_eq!(
        validate_disclosed_input(&input, &IssuePolicy::builtin(), workspace),
        Err(IssueArbiterWaitReason::EvidenceUnavailable)
    );
}

#[test]
fn arbiter_nested_policy_and_option_strings_share_the_same_disclosure_check() {
    for input in [
        serde_json::json!({"policy":{"labels":{"hold":"token=private-fixture-value"}}}),
        serde_json::json!({"policy":{"protection":{"paths":["docs/ghp_SYNTHETIC_ASSESSMENT_VALUE_000000.md"]}}}),
        serde_json::json!({"decision":{"options":[{"label":"token=private-fixture-value"}]}}),
    ] {
        assert_eq!(
            validate_disclosed_input(&input, &IssuePolicy::builtin(), "/fixture/workspace"),
            Err(IssueArbiterWaitReason::EvidenceUnavailable)
        );
    }
}

#[test]
fn arbiter_disclosed_scalars_and_unrelated_relative_paths_remain_usable() {
    let input = serde_json::json!({
        "schema_version": 1, "optional":null, "enabled":true,
        "issue":{"title":"공개 이슈", "body":"변경 경로: `src/lib.rs`"},
        "policy":{"arbiter":{"min_confidence":9000}},
        "repository":{"entries":[{"kind":"absent", "path":"Cargo.toml"}]}
    });
    assert_eq!(
        validate_disclosed_input(
            &input,
            &IssuePolicy::builtin(),
            r"C:\Users\fixture\private-repository"
        ),
        Ok(())
    );
}
