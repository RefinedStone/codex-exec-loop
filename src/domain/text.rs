pub fn compact_whitespace_detail(text: &str, max_len: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_len {
        return compact;
    }

    let keep = max_len.saturating_sub(3);
    let truncated = compact.chars().take(keep).collect::<String>();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use super::compact_whitespace_detail;

    #[test]
    fn keeps_compact_text_within_limit() {
        assert_eq!(
            compact_whitespace_detail("alpha   beta\ngamma", 32),
            "alpha beta gamma"
        );
    }

    #[test]
    fn truncates_after_compacting_whitespace() {
        assert_eq!(
            compact_whitespace_detail("alpha   beta gamma", 10),
            "alpha b..."
        );
    }
}
