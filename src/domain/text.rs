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
     * The ellipsis marker is always preferred over a silent cut. For tiny budgets,
     * saturating_sub keeps the function total and returns the smallest possible marker
     * rather than panicking or exposing a misleading partial word.
     */
    let keep = max_len.saturating_sub(3);
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
}
