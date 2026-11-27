use super::*;

fn span(
    field: IssueTextField,
    text: &str,
    needle: &str,
    class: Option<IssueSensitiveSpanClass>,
) -> IssueSensitiveSpan {
    let start = text.find(needle).unwrap();
    IssueSensitiveSpan {
        field,
        range: start..start + needle.len(),
        class,
        user_decision: None,
    }
}

#[test]
fn mixed_overlap_plans_absolute_ranges_without_restorable_plaintext() {
    let title = "홍길동 [[akra-mask:1]]";
    let body = "앞 abcdefghijklmnop 뒤";
    let start = body.find("abcdefghijklmnop").unwrap();
    let body_span = |range: Range<usize>, class| IssueSensitiveSpan {
        field: IssueTextField::Body,
        range: start + range.start..start + range.end,
        class: Some(class),
        user_decision: None,
    };
    let spans = vec![
        body_span(10..16, IssueSensitiveSpanClass::Log),
        body_span(12..14, IssueSensitiveSpanClass::Secret),
        body_span(0..7, IssueSensitiveSpanClass::Name),
        body_span(2..5, IssueSensitiveSpanClass::Secret),
        body_span(5..12, IssueSensitiveSpanClass::Path),
        body_span(4..8, IssueSensitiveSpanClass::Secret),
        body_span(8..10, IssueSensitiveSpanClass::Secret),
        span(
            IssueTextField::Title,
            title,
            "홍길동",
            Some(IssueSensitiveSpanClass::Name),
        ),
    ];
    let plan = plan_issue_masking(title, body, &spans, &IssueDisclosurePolicy::builtin()).unwrap();

    assert_eq!(plan.shared_copy.title, "[[akra-mask:2]] [[akra-mask:1]]");
    assert_eq!(plan.shared_copy.body, "앞 [[akra-mask:3]] 뒤");
    assert_eq!(
        plan.restorable_spans,
        vec![
            IssueRestorableSpanPlan {
                marker: "[[akra-mask:2]]".to_string(),
                field: IssueTextField::Title,
                shared_position: 0,
                source_range: 0.."홍길동".len(),
                removal_ranges: Vec::new(),
            },
            IssueRestorableSpanPlan {
                marker: "[[akra-mask:3]]".to_string(),
                field: IssueTextField::Body,
                shared_position: "앞 ".len(),
                source_range: start..start + 16,
                removal_ranges: vec![start + 2..start + 10, start + 12..start + 14],
            },
        ],
    );
    let debug = format!("{plan:?}");
    for private_text in ["홍길동", "abcdefghijklmnop", "cdefghij"] {
        assert!(!debug.contains(private_text));
    }
}

#[test]
fn adjacent_groups_keep_distinct_markers_and_removal_placeholders() {
    let body = "한글 AABB 이름경로 끝";
    let spans = [
        span(
            IssueTextField::Body,
            body,
            "AA",
            Some(IssueSensitiveSpanClass::Secret),
        ),
        span(
            IssueTextField::Body,
            body,
            "BB",
            Some(IssueSensitiveSpanClass::Secret),
        ),
        span(
            IssueTextField::Body,
            body,
            "이름",
            Some(IssueSensitiveSpanClass::Name),
        ),
        span(
            IssueTextField::Body,
            body,
            "경로",
            Some(IssueSensitiveSpanClass::Path),
        ),
    ];
    let plan = plan_issue_masking("t", body, &spans, &IssueDisclosurePolicy::builtin()).unwrap();

    assert_eq!(
        plan.shared_copy.body,
        "한글 [removed][removed] [[akra-mask:1]][[akra-mask:2]] 끝",
    );
    assert_eq!(plan.restorable_spans.len(), 2);
    let first = &plan.restorable_spans[0];
    let second = &plan.restorable_spans[1];
    assert_eq!(
        first.source_range,
        body.find("이름").unwrap()..body.find("경로").unwrap()
    );
    assert_eq!(
        second.source_range,
        body.find("경로").unwrap()..body.find("경로").unwrap() + "경로".len(),
    );
    assert_eq!(first.shared_position, "한글 [removed][removed] ".len());
    assert_eq!(
        second.shared_position,
        first.shared_position + first.marker.len()
    );
    assert!(first.removal_ranges.is_empty());
    assert!(second.removal_ranges.is_empty());
}

#[test]
fn a_fully_removed_group_keeps_its_restorable_marker() {
    let body = "앞 synthetic-private 뒤";
    let spans = [
        span(
            IssueTextField::Body,
            body,
            "synthetic-private",
            Some(IssueSensitiveSpanClass::Name),
        ),
        span(
            IssueTextField::Body,
            body,
            "synthetic-private",
            Some(IssueSensitiveSpanClass::Secret),
        ),
    ];
    let plan = plan_issue_masking("t", body, &spans, &IssueDisclosurePolicy::builtin()).unwrap();

    assert_eq!(plan.shared_copy.body, "앞 [[akra-mask:1]] 뒤");
    assert_eq!(plan.restorable_spans.len(), 1);
    let group = &plan.restorable_spans[0];
    assert_eq!(group.removal_ranges, vec![group.source_range.clone()]);
}

#[test]
fn removed_input_markers_still_reserve_their_numbers() {
    let title = "[[akra-mask:1]]";
    let body = "alice [[akra-mask:2]]";
    let spans = [
        span(
            IssueTextField::Body,
            body,
            "alice",
            Some(IssueSensitiveSpanClass::Name),
        ),
        span(
            IssueTextField::Body,
            body,
            "[[akra-mask:2]]",
            Some(IssueSensitiveSpanClass::Secret),
        ),
    ];
    let plan = plan_issue_masking(title, body, &spans, &IssueDisclosurePolicy::builtin()).unwrap();

    assert_eq!(plan.shared_copy.title, title);
    assert_eq!(plan.shared_copy.body, "[[akra-mask:3]] [removed]");
    assert_eq!(plan.restorable_spans[0].marker, "[[akra-mask:3]]");
}

#[test]
fn confirmation_decisions_cannot_weaken_a_remove_policy() {
    let title = "alice";
    let body = "synthetic-secret opaque";
    let mut policy = IssueDisclosurePolicy::builtin();
    policy.name = IssueSensitiveSpanAction::UserConfirm;
    let mut spans = [
        span(
            IssueTextField::Title,
            title,
            "alice",
            Some(IssueSensitiveSpanClass::Name),
        ),
        span(
            IssueTextField::Body,
            body,
            "synthetic-secret",
            Some(IssueSensitiveSpanClass::Secret),
        ),
        span(IssueTextField::Body, body, "opaque", None),
    ];
    spans[1].user_decision = Some(IssueSensitiveSpanAction::RestorableMask);
    let blocked = plan_issue_masking(title, body, &spans, &policy).unwrap_err();
    assert_eq!(
        blocked,
        IssueMaskingBlocked::NeedsUserConfirmation {
            span_indexes: vec![0, 2],
        },
    );
    assert!(!format!("{blocked:?}").contains("synthetic-secret"));
    assert!(!blocked.to_string().contains("alice"));

    spans[0].user_decision = Some(IssueSensitiveSpanAction::RestorableMask);
    spans[2].user_decision = Some(IssueSensitiveSpanAction::Remove);
    let plan = plan_issue_masking(title, body, &spans, &policy).unwrap();
    assert_eq!(plan.shared_copy.title, "[[akra-mask:1]]");
    assert_eq!(plan.shared_copy.body, "[removed] [removed]");
    assert_eq!(plan.restorable_spans.len(), 1);
    assert_eq!(plan.restorable_spans[0].field, IssueTextField::Title);
}

#[test]
fn invalid_utf8_ranges_take_precedence_over_pending_confirmation() {
    let body = "한글 본문";
    let spans = [
        IssueSensitiveSpan {
            field: IssueTextField::Body,
            range: 0.."한글".len(),
            class: None,
            user_decision: None,
        },
        IssueSensitiveSpan {
            field: IssueTextField::Body,
            range: 1.."한글".len(),
            class: Some(IssueSensitiveSpanClass::Name),
            user_decision: None,
        },
    ];
    let blocked =
        plan_issue_masking("t", body, &spans, &IssueDisclosurePolicy::builtin()).unwrap_err();
    assert_eq!(blocked, IssueMaskingBlocked::InvalidSpan { span_index: 1 });
    assert!(!format!("{blocked:?}").contains(body));
}
