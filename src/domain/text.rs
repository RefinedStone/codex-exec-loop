// 이 함수는 로그, 상태줄, overlay 요약에 들어가는 상세 text를 한 줄짜리 짧은 설명으로 정규화한다.
// domain helper에 두면 adapter마다 whitespace 압축과 ellipsis 규칙을 다시 만들지 않아도 된다.
pub fn compact_whitespace_detail(text: &str, max_len: usize) -> String {
    /*
     * Whitespace compaction happens before length budgeting because callers usually
     * pass stderr, markdown snippets, or multi-line status details. A newline-heavy
     * input should spend the visible budget on words, not on layout artifacts that the
     * one-line UI surface cannot preserve.
     */
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    /*
     * Budgeting uses Unicode scalar count instead of byte length. This keeps Korean
     * text and other multi-byte input from being split inside UTF-8 sequences while
     * still giving status surfaces a deterministic character budget.
     */
    if compact.chars().count() <= max_len {
        /*
         * Text already inside the budget is returned without decoration. Adding an
         * ellipsis in this branch would make complete diagnostics look truncated and
         * would mislead users reading startup or runtime status copy.
         */
        return compact;
    }

    /*
     * An ellipsis must also honor the caller's exact character budget.
     * For tiny limits there is no space for even one content character;
     * return a shortened marker (or empty for zero) rather than exceeding
     * max_len with a three-character marker.
     */
    if max_len <= 3 {
        return ".".repeat(max_len);
    }
    let keep = max_len - 3;
    /*
     * Taking chars after compaction preserves UTF-8 boundaries. This helper is used by
     * adapters and presentation code that should never need to reason about byte
     * slicing to show a safe diagnostic tail.
     */
    let truncated = compact.chars().take(keep).collect::<String>();
    format!("{truncated}...")
}

// 이 module의 test는 text normalization 계약을 domain 가까이에 고정한다. adapter output이 이 helper를
// 신뢰하므로 whitespace 압축과 truncation 예시를 작게 보존한다.
#[cfg(test)]
mod tests {
    // 테스트는 공개 helper만 호출해 외부 caller가 보는 contract와 같은 경로를 검증한다.
    use super::compact_whitespace_detail;

    // 이 test는 여러 whitespace가 하나의 space로 압축되고, 제한 안에 있으면 잘리지 않는다는 happy path를
    // 고정한다.
    #[test]
    fn keeps_compact_text_within_limit() {
        assert_eq!(
            compact_whitespace_detail("alpha   beta\ngamma", 32),
            "alpha beta gamma"
        );
    }

    // 이 test는 whitespace 압축 후 길이 제한을 적용한다는 순서를 검증한다. 원문 기준으로 먼저 자르면
    // `"alpha   beta"`처럼 불필요한 공백이 결과 길이를 잡아먹을 수 있다.
    #[test]
    fn truncates_after_compacting_whitespace() {
        assert_eq!(
            compact_whitespace_detail("alpha   beta gamma", 10),
            "alpha b..."
        );
    }

    // 이 test는 max_len 0~3 경계에서 marker도 예산을 넘지 않는다는 엄격한 상한을 고정한다.
    #[test]
    fn tiny_budgets_never_exceed_maximum_character_count() {
        let markers = ["", ".", "..", "..."];
        for (budget, expected) in markers.iter().enumerate() {
            let result = compact_whitespace_detail("abcd", budget);
            assert_eq!(result, *expected, "budget={budget}");
            assert!(result.chars().count() <= budget);
        }
        assert_eq!(compact_whitespace_detail("가나다라", 2), "..");
        assert_eq!(compact_whitespace_detail("가나다라마", 4), "가...");
    }

    // 이 test는 예산과 길이가 같은 원문은 marker 없이 그대로 남는다는 경계를 고정한다.
    #[test]
    fn exact_budget_preserves_untruncated_text() {
        assert_eq!(compact_whitespace_detail("a", 1), "a");
        assert_eq!(compact_whitespace_detail("ab", 2), "ab");
        assert_eq!(compact_whitespace_detail("abc", 3), "abc");
        assert_eq!(compact_whitespace_detail("", 0), "");
    }
}
