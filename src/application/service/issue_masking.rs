use crate::application::port::outbound::evidence_vault_port::VaultPlaintext;
use crate::domain::issue_masking::{IssueRestorableSpanPlan, plan_issue_masking};
use crate::domain::issue_policy::IssueDisclosurePolicy;
use crate::domain::issue_work::IssueSharedCopy;

pub use crate::domain::issue_masking::{
    ISSUE_REMOVED_PLACEHOLDER, IssueMaskingBlocked, IssueSensitiveSpan, IssueTextField,
};

/*
 * 기존 가림 API는 순수한 위치 계획에서 Vault에 맡길 평문만 구성한다 (#2419, #2537).
 * 제거 범위는 읽어 복사하지 않으며, 복원용 평문은 VaultPlaintext 안에만 반환한다.
 */

/// Vault 에 맡길 가림 구간이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMaskedSpan {
    /// 공유본에서 이 구간을 대신하는 표식이다. 제목과 본문 전체에서 하나뿐이다.
    pub marker: String,
    pub field: IssueTextField,
    /// 공유본의 칸에서 표식이 시작하는 바이트 위치이다.
    pub position: usize,
    /// 가린 평문이다. 그 안의 제거 대상은 이미 `ISSUE_REMOVED_PLACEHOLDER` 이다.
    pub plaintext: VaultPlaintext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMaskingOutcome {
    pub shared_copy: IssueSharedCopy,
    pub masked_spans: Vec<IssueMaskedSpan>,
}

/// 공개 정책으로 공유본을 만들고 복원할 구간만 Vault 평문으로 구성한다.
pub fn mask_issue_text(
    title: &str,
    body: &str,
    spans: &[IssueSensitiveSpan],
    policy: &IssueDisclosurePolicy,
) -> Result<IssueMaskingOutcome, IssueMaskingBlocked> {
    let plan = plan_issue_masking(title, body, spans, policy)?;
    let masked_spans = plan
        .restorable_spans
        .into_iter()
        .map(|span| restorable_span(title, body, span))
        .collect();
    Ok(IssueMaskingOutcome {
        shared_copy: plan.shared_copy,
        masked_spans,
    })
}

fn restorable_span(title: &str, body: &str, span: IssueRestorableSpanPlan) -> IssueMaskedSpan {
    let text = match span.field {
        IssueTextField::Title => title,
        IssueTextField::Body => body,
    };
    let mut plaintext = String::new();
    let mut cursor = span.source_range.start;
    for removal in span.removal_ranges {
        plaintext.push_str(&text[cursor..removal.start]);
        plaintext.push_str(ISSUE_REMOVED_PLACEHOLDER);
        cursor = removal.end;
    }
    plaintext.push_str(&text[cursor..span.source_range.end]);
    IssueMaskedSpan {
        marker: span.marker,
        field: span.field,
        position: span.shared_position,
        plaintext: VaultPlaintext::new(plaintext),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_policy::{IssueSensitiveSpanAction, IssueSensitiveSpanClass};
    use std::ops::Range;

    // 시험용으로 만든 값이다. 실제 비밀이 아니다.
    const SYNTHETIC_TOKEN: &str = "synthetic-token-AAAA1111";
    const SYNTHETIC_PASSWORD: &str = "synthetic-password-BBBB2222";

    fn span(
        field: IssueTextField,
        text: &str,
        needle: &str,
        class: Option<IssueSensitiveSpanClass>,
    ) -> IssueSensitiveSpan {
        let start = text.find(needle).expect("needle should be in the text");
        IssueSensitiveSpan {
            field,
            range: start..start + needle.len(),
            class,
            user_decision: None,
        }
    }

    fn mask(
        title: &str,
        body: &str,
        spans: &[IssueSensitiveSpan],
    ) -> Result<IssueMaskingOutcome, IssueMaskingBlocked> {
        mask_issue_text(title, body, spans, &IssueDisclosurePolicy::builtin())
    }

    #[test]
    fn names_and_paths_get_unique_markers_in_title_and_body() {
        let title = "alice 의 빌드 실패";
        let body = "alice 가 /home/alice/app 에서 실행했다";
        let outcome = mask(
            title,
            body,
            &[
                span(
                    IssueTextField::Title,
                    title,
                    "alice",
                    Some(IssueSensitiveSpanClass::Name),
                ),
                span(
                    IssueTextField::Body,
                    body,
                    "alice",
                    Some(IssueSensitiveSpanClass::Name),
                ),
                span(
                    IssueTextField::Body,
                    body,
                    "/home/alice/app",
                    Some(IssueSensitiveSpanClass::Path),
                ),
            ],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.shared_copy.title, "[[akra-mask:1]] 의 빌드 실패");
        assert_eq!(
            outcome.shared_copy.body,
            "[[akra-mask:2]] 가 [[akra-mask:3]] 에서 실행했다"
        );
        let plaintexts: Vec<_> = outcome
            .masked_spans
            .iter()
            .map(|masked| (masked.field, masked.plaintext.expose()))
            .collect();
        assert_eq!(
            plaintexts,
            [
                (IssueTextField::Title, "alice"),
                (IssueTextField::Body, "alice"),
                (IssueTextField::Body, "/home/alice/app"),
            ]
        );
        for masked in &outcome.masked_spans {
            let shared = match masked.field {
                IssueTextField::Title => &outcome.shared_copy.title,
                IssueTextField::Body => &outcome.shared_copy.body,
            };
            assert!(shared[masked.position..].starts_with(&masked.marker));
        }
    }

    #[test]
    fn removed_secrets_stay_out_of_every_output() {
        let title = format!("로그인 실패 {SYNTHETIC_PASSWORD}");
        let body = format!("log: user=alice token={SYNTHETIC_TOKEN} end");
        let log = format!("user=alice token={SYNTHETIC_TOKEN}");
        let outcome = mask(
            &title,
            &body,
            &[
                span(
                    IssueTextField::Title,
                    &title,
                    SYNTHETIC_PASSWORD,
                    Some(IssueSensitiveSpanClass::Secret),
                ),
                // 로그 구간 안에 토큰이 있다. 토큰은 Vault 평문에도 없어야 한다.
                span(
                    IssueTextField::Body,
                    &body,
                    &log,
                    Some(IssueSensitiveSpanClass::Log),
                ),
                span(
                    IssueTextField::Body,
                    &body,
                    SYNTHETIC_TOKEN,
                    Some(IssueSensitiveSpanClass::Secret),
                ),
            ],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.shared_copy.title, "로그인 실패 [removed]");
        assert_eq!(outcome.shared_copy.body, "log: [[akra-mask:1]] end");
        assert_eq!(outcome.masked_spans.len(), 1);
        assert_eq!(
            outcome.masked_spans[0].plaintext.expose(),
            "user=alice token=[removed]"
        );
        // 일반 이벤트와 Gateway 응답은 공유본의 serde 출력이다.
        let event = serde_json::to_string(&outcome.shared_copy).expect("shared copy serializes");
        let debug = format!("{outcome:?}");
        for output in [&event, &debug] {
            assert!(!output.contains(SYNTHETIC_TOKEN));
            assert!(!output.contains(SYNTHETIC_PASSWORD));
            assert!(!output.contains("alice"));
        }
    }

    #[test]
    fn overlapping_spans_become_one_marker() {
        let body = "host build-01.internal path /srv/build-01/out";
        let outcome = mask(
            "t",
            body,
            &[
                span(
                    IssueTextField::Body,
                    body,
                    "build-01.internal path /srv",
                    Some(IssueSensitiveSpanClass::Name),
                ),
                span(
                    IssueTextField::Body,
                    body,
                    "/srv/build-01/out",
                    Some(IssueSensitiveSpanClass::Path),
                ),
            ],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.shared_copy.body, "host [[akra-mask:1]]");
        assert_eq!(
            outcome.masked_spans[0].plaintext.expose(),
            "build-01.internal path /srv/build-01/out"
        );
    }

    #[test]
    fn overlapping_removals_leave_one_placeholder() {
        let body = format!("a {SYNTHETIC_TOKEN}{SYNTHETIC_PASSWORD} b");
        let both = format!("{SYNTHETIC_TOKEN}{SYNTHETIC_PASSWORD}");
        let outcome = mask(
            "t",
            &body,
            &[
                span(
                    IssueTextField::Body,
                    &body,
                    &both,
                    Some(IssueSensitiveSpanClass::Secret),
                ),
                span(
                    IssueTextField::Body,
                    &body,
                    SYNTHETIC_PASSWORD,
                    Some(IssueSensitiveSpanClass::Secret),
                ),
            ],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.shared_copy.body, "a [removed] b");
        assert!(outcome.masked_spans.is_empty());
    }

    #[test]
    fn markers_skip_text_already_in_the_input() {
        let title = "[[akra-mask:1]] 은 예전 표식이다";
        let body = "alice 와 [[akra-mask:2]]";
        let outcome = mask(
            title,
            body,
            &[span(
                IssueTextField::Body,
                body,
                "alice",
                Some(IssueSensitiveSpanClass::Name),
            )],
        )
        .expect("masking should succeed");

        assert_eq!(outcome.masked_spans[0].marker, "[[akra-mask:3]]");
        assert_eq!(outcome.shared_copy.title, title);
        assert_eq!(
            outcome.shared_copy.body,
            "[[akra-mask:3]] 와 [[akra-mask:2]]"
        );
    }

    #[test]
    fn uncertain_spans_need_user_confirmation_and_errors_carry_no_text() {
        let body = format!("alice {SYNTHETIC_TOKEN}");
        let mut policy = IssueDisclosurePolicy::builtin();
        policy.name = IssueSensitiveSpanAction::UserConfirm;
        let mut spans = vec![
            span(
                IssueTextField::Body,
                &body,
                "alice",
                Some(IssueSensitiveSpanClass::Name),
            ),
            span(IssueTextField::Body, &body, SYNTHETIC_TOKEN, None),
        ];

        let blocked =
            mask_issue_text("t", &body, &spans, &policy).expect_err("masking should be blocked");
        assert_eq!(
            blocked,
            IssueMaskingBlocked::NeedsUserConfirmation {
                span_indexes: vec![0, 1]
            }
        );
        for output in [format!("{blocked:?}"), blocked.to_string()] {
            assert!(!output.contains(SYNTHETIC_TOKEN));
            assert!(!output.contains("alice"));
        }
        // 입력 구간의 Debug 에도 글이 없다.
        assert!(!format!("{spans:?}").contains(SYNTHETIC_TOKEN));

        spans[0].user_decision = Some(IssueSensitiveSpanAction::RestorableMask);
        spans[1].user_decision = Some(IssueSensitiveSpanAction::Remove);
        let outcome = mask_issue_text("t", &body, &spans, &policy).expect("masking should succeed");
        assert_eq!(outcome.shared_copy.body, "[[akra-mask:1]] [removed]");
    }

    #[test]
    fn user_decision_does_not_weaken_a_decided_policy() {
        let body = format!("token {SYNTHETIC_TOKEN}");
        let mut secret = span(
            IssueTextField::Body,
            &body,
            SYNTHETIC_TOKEN,
            Some(IssueSensitiveSpanClass::Secret),
        );
        secret.user_decision = Some(IssueSensitiveSpanAction::RestorableMask);

        let outcome = mask("t", &body, &[secret]).expect("masking should succeed");
        assert_eq!(outcome.shared_copy.body, "token [removed]");
        assert!(outcome.masked_spans.is_empty());
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        let body = "한글 본문";
        for range in [0..0, 0..1, Range { start: 3, end: 2 }, 0..100] {
            let blocked = mask(
                "t",
                body,
                &[IssueSensitiveSpan {
                    field: IssueTextField::Body,
                    range,
                    class: Some(IssueSensitiveSpanClass::Name),
                    user_decision: None,
                }],
            )
            .expect_err("masking should be blocked");
            assert_eq!(blocked, IssueMaskingBlocked::InvalidSpan { span_index: 0 });
        }
    }

    #[test]
    fn wrapper_preserves_mixed_overlap_plan_and_vault_text() {
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
        let policy = IssueDisclosurePolicy::builtin();
        let plan = plan_issue_masking(title, body, &spans, &policy).unwrap();
        let outcome = mask_issue_text(title, body, &spans, &policy).unwrap();

        assert_eq!(outcome.shared_copy, plan.shared_copy);
        assert_eq!(outcome.shared_copy.title, "[[akra-mask:2]] [[akra-mask:1]]");
        assert_eq!(outcome.shared_copy.body, "앞 [[akra-mask:3]] 뒤");
        assert_eq!(outcome.masked_spans.len(), plan.restorable_spans.len());
        for (masked, planned) in outcome.masked_spans.iter().zip(&plan.restorable_spans) {
            assert_eq!(masked.marker, planned.marker);
            assert_eq!(masked.field, planned.field);
            assert_eq!(masked.position, planned.shared_position);
        }
        let plaintexts: Vec<_> = outcome
            .masked_spans
            .iter()
            .map(|masked| masked.plaintext.expose())
            .collect();
        assert_eq!(plaintexts, ["홍길동", "ab[removed]kl[removed]op"]);
        assert_eq!(outcome.masked_spans[1].position, "앞 ".len());
        let debug = format!("{outcome:?}");
        for private_text in ["홍길동", "abcdefghijklmnop", "cdefghij"] {
            assert!(!debug.contains(private_text));
        }
    }

    #[test]
    fn wrapper_preserves_adjacent_groups_and_a_fully_removed_mask() {
        let body = "한글 AABB 이름경로 synthetic-private 끝";
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
        let policy = IssueDisclosurePolicy::builtin();
        let plan = plan_issue_masking("t", body, &spans, &policy).unwrap();
        let outcome = mask_issue_text("t", body, &spans, &policy).unwrap();

        assert_eq!(outcome.shared_copy, plan.shared_copy);
        assert_eq!(
            outcome.shared_copy.body,
            "한글 [removed][removed] [[akra-mask:1]][[akra-mask:2]] [[akra-mask:3]] 끝",
        );
        let plaintexts: Vec<_> = outcome
            .masked_spans
            .iter()
            .map(|masked| masked.plaintext.expose())
            .collect();
        assert_eq!(plaintexts, ["이름", "경로", "[removed]"]);
        for (masked, planned) in outcome.masked_spans.iter().zip(&plan.restorable_spans) {
            assert_eq!(masked.marker, planned.marker);
            assert_eq!(masked.position, planned.shared_position);
            assert!(outcome.shared_copy.body[masked.position..].starts_with(&masked.marker));
        }
    }
}
