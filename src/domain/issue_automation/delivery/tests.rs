use super::*;

fn text(path: &str, added: u64, deleted: u64) -> IssueDeliveryDiffFile {
    IssueDeliveryDiffFile {
        path: path.into(),
        lines: IssueDeliveryDiffLines::Text { added, deleted },
    }
}

fn diff(files: Vec<IssueDeliveryDiffFile>) -> IssueDeliveryDiff {
    IssueDeliveryDiff {
        base_commit_sha: "a".repeat(40),
        source_commit_sha: "b".repeat(40),
        files,
    }
}

fn assess(files: Vec<IssueDeliveryDiffFile>) -> IssueDeliveryDiffVerdict {
    assess_delivery_diff(&IssueAutomationPolicy::builtin(), &diff(files)).unwrap()
}

#[test]
fn an_ordinary_diff_uses_added_and_deleted_lines_without_requiring_a_person() {
    let verdict = assess(vec![text("src/lib.rs", 7, 4), text("README.md", 0, 3)]);
    assert_eq!(verdict.changed_files, 2);
    assert_eq!(verdict.changed_lines, Some(14));
    assert!(!verdict.requires_human());
}

#[test]
fn automatic_size_boundaries_are_inclusive_and_one_more_requires_a_person() {
    let at_limit = (0..MAX_AUTOMATIC_DELIVERY_FILES)
        .map(|index| text(&format!("src/file-{index}.rs"), 10, 10))
        .collect::<Vec<_>>();
    let verdict = assess(at_limit.clone());
    assert_eq!(verdict.changed_lines, Some(1_000));
    assert!(!verdict.requires_human());

    let mut extra_file = at_limit;
    extra_file.push(text("src/mode-only.rs", 0, 0));
    assert_eq!(
        assess(extra_file).reasons,
        [IssueDeliveryRiskReason::DiffTooLarge],
    );
    assert_eq!(
        assess(vec![text("src/lib.rs", 500, 501)]).reasons,
        [IssueDeliveryRiskReason::DiffTooLarge],
    );
}

#[test]
fn every_builtin_path_family_is_enforced_on_actual_paths_including_deletions() {
    let protected_paths = [
        ".github/workflows/check.yml",
        "scripts/release/publish.sh",
        ".akra/issue-automation.toml",
        ".akra/issue-policy.toml",
        "AGENTS.md",
        "src/AGENTS.md",
        ".env",
        "service/.env.local",
        "config/credentials.json",
        "secrets/server.pem",
        "secrets/server.key",
    ];
    for path in protected_paths {
        let verdict = assess(vec![text(path, 0, 1)]);
        assert_eq!(
            verdict.reasons,
            [IssueDeliveryRiskReason::ProtectedPath],
            "{path}"
        );
    }
}

#[test]
fn added_protection_patterns_remain_additive_and_match_unicode_and_spaces() {
    let mut policy = IssueAutomationPolicy::builtin();
    policy
        .protection
        .paths
        .extend(["src/payments/**".into(), "문서/검토 *.md".into()]);
    for path in ["src/payments/refund.rs", "문서/검토 결과.md", "AGENTS.md"] {
        let verdict = assess_delivery_diff(&policy, &diff(vec![text(path, 1, 0)])).unwrap();
        assert_eq!(
            verdict.reasons,
            [IssueDeliveryRiskReason::ProtectedPath],
            "{path}"
        );
    }
    policy.protection.paths.clear();
    assert!(assess_delivery_diff(&policy, &diff(vec![text("src/lib.rs", 1, 0)])).is_err());
}

#[test]
fn rename_inventory_checks_both_old_and_new_paths_without_hiding_a_delete() {
    for files in [
        vec![
            text(".github/workflows/check.yml", 0, 4),
            text("docs/check.txt", 4, 0),
        ],
        vec![
            text("src/check.txt", 0, 4),
            text(".github/workflows/check.yml", 4, 0),
        ],
    ] {
        let verdict = assess(files);
        assert_eq!(verdict.changed_files, 2);
        assert_eq!(verdict.changed_lines, Some(8));
        assert_eq!(verdict.reasons, [IssueDeliveryRiskReason::ProtectedPath]);
    }
}

#[test]
fn binary_files_always_require_a_person_and_do_not_claim_a_complete_line_count() {
    let verdict = assess(vec![IssueDeliveryDiffFile {
        path: "assets/icon.bin".into(),
        lines: IssueDeliveryDiffLines::Binary,
    }]);
    assert_eq!(verdict.changed_lines, None);
    assert_eq!(verdict.reasons, [IssueDeliveryRiskReason::BinaryDiff]);

    let verdict = assess(vec![
        text("src/generated.rs", 1_001, 0),
        IssueDeliveryDiffFile {
            path: "config/credentials.bin".into(),
            lines: IssueDeliveryDiffLines::Binary,
        },
    ]);
    assert_eq!(verdict.changed_lines, None);
    assert_eq!(
        verdict.reasons,
        [
            IssueDeliveryRiskReason::ProtectedPath,
            IssueDeliveryRiskReason::DiffTooLarge,
            IssueDeliveryRiskReason::BinaryDiff,
        ]
    );
}

#[test]
fn incomplete_inventory_and_arithmetic_overflow_never_become_a_safe_verdict() {
    let policy = IssueAutomationPolicy::builtin();
    for files in [
        vec![],
        vec![text("src/lib.rs", 1, 0), text("src/lib.rs", 0, 1)],
        vec![text("src/lib.rs", u64::MAX, 1)],
        vec![text("a", u64::MAX, 0), text("b", 1, 0)],
        (0..=MAX_ISSUE_DELIVERY_DIFF_FILES)
            .map(|index| text(&format!("src/file-{index}.rs"), 0, 0))
            .collect(),
    ] {
        assert!(assess_delivery_diff(&policy, &diff(files)).is_err());
    }
}

#[test]
fn unsupported_paths_and_noncanonical_commit_ranges_are_rejected() {
    let policy = IssueAutomationPolicy::builtin();
    for path in [
        "",
        "/root/file",
        "../secret",
        "src/../secret",
        "src//file",
        "src\\file",
        "C:/file",
        "tab\tfile",
        "line\nfile",
    ] {
        assert!(
            assess_delivery_diff(&policy, &diff(vec![text(path, 1, 0)])).is_err(),
            "{path:?}"
        );
    }
    let long_path = "x".repeat(MAX_ISSUE_DELIVERY_DIFF_PATH_BYTES + 1);
    assert!(assess_delivery_diff(&policy, &diff(vec![text(&long_path, 1, 0)])).is_err());
    for (base, source) in [
        ("a".repeat(40), "a".repeat(40)),
        ("a".repeat(7), "b".repeat(40)),
        ("a".repeat(40), "b".repeat(64)),
        ("a".repeat(40), "B".repeat(40)),
        ("a".repeat(40), "x".repeat(40)),
    ] {
        let mut value = diff(vec![text("src/lib.rs", 1, 0)]);
        value.base_commit_sha = base;
        value.source_commit_sha = source;
        assert!(assess_delivery_diff(&policy, &value).is_err());
    }
}

#[test]
fn sha256_ranges_are_supported_without_shortening_the_identity() {
    let mut value = diff(vec![text("src/lib.rs", 1, 0)]);
    value.base_commit_sha = "a".repeat(64);
    value.source_commit_sha = "b".repeat(64);
    assert!(
        !assess_delivery_diff(&IssueAutomationPolicy::builtin(), &value)
            .unwrap()
            .requires_human()
    );
}

#[test]
fn persisted_verdicts_cannot_hide_binary_or_known_large_diffs() {
    let normal = assess(vec![text("src/lib.rs", 1, 0)]);
    let mut invalid = normal.clone();
    invalid.changed_lines = None;
    assert!(invalid.validate().is_err());
    invalid = normal.clone();
    invalid.changed_files = 51;
    assert!(invalid.validate().is_err());
    invalid = normal.clone();
    invalid.changed_lines = Some(1_001);
    assert!(invalid.validate().is_err());
    invalid = normal.clone();
    invalid.reasons = vec![IssueDeliveryRiskReason::DiffTooLarge];
    assert!(invalid.validate().is_err());
    invalid = normal.clone();
    invalid.reasons = vec![
        IssueDeliveryRiskReason::ProtectedPath,
        IssueDeliveryRiskReason::ProtectedPath,
    ];
    assert!(invalid.validate().is_err());
    invalid = normal.clone();
    invalid.changed_files = 0;
    assert!(invalid.validate().is_err());
    invalid = normal;
    invalid.changed_files = 4_097;
    invalid.reasons = vec![IssueDeliveryRiskReason::DiffTooLarge];
    assert!(invalid.validate().is_err());
}
