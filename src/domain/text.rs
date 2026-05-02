// 학습 주석: 이 함수는 로그, 상태줄, overlay 요약에 들어가는 상세 text를 한 줄짜리 짧은 설명으로
// 정규화합니다. domain helper에 두면 adapter마다 whitespace 압축과 ellipsis 규칙을 다시 만들지 않아도 됩니다.
pub fn compact_whitespace_detail(text: &str, max_len: usize) -> String {
    // 학습 주석: `split_whitespace`는 공백, 탭, 줄바꿈을 모두 구분자로 보고 빈 조각을 버립니다. 다시 single
    // space로 join하면 여러 줄 입력도 UI가 다루기 쉬운 한 줄 detail로 바뀝니다.
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // 학습 주석: 길이 검사는 byte 길이가 아니라 char 개수 기준입니다. 한글 같은 multibyte 문자가 있어도
    // 화면에 보여 줄 문자 수 기준으로 제한하고, 중간 byte를 잘라 깨진 UTF-8을 만들지 않습니다.
    if compact.chars().count() <= max_len {
        // 학습 주석: 이미 제한 안에 들어온 text는 ellipsis를 붙이지 않습니다. caller가 원문 의미를 최대한
        // 보존한 compact string을 받을 수 있습니다.
        return compact;
    }

    // 학습 주석: ellipsis 세 글자(`...`)를 붙일 공간을 먼저 뺍니다. `saturating_sub`라 max_len이 0~2여도
    // underflow 없이 keep이 0이 되고, 결과는 최소한의 `...` truncation marker가 됩니다.
    let keep = max_len.saturating_sub(3);
    // 학습 주석: `chars().take(keep)`로 문자 경계에서 잘라 UTF-8 안전성을 유지합니다. byte slice로 자르면
    // multibyte 문자를 중간에서 끊어 panic이나 잘못된 표시를 만들 수 있습니다.
    let truncated = compact.chars().take(keep).collect::<String>();
    format!("{truncated}...")
}

// 학습 주석: 이 module의 test는 text normalization 계약을 domain 가까이에 고정합니다. adapter output이 이
// helper를 신뢰하므로 whitespace 압축과 truncation 예시를 작게 보존합니다.
#[cfg(test)]
mod tests {
    // 학습 주석: 테스트는 공개 helper만 호출해 외부 caller가 보는 contract와 같은 경로를 검증합니다.
    use super::compact_whitespace_detail;

    // 학습 주석: 이 test는 여러 whitespace가 하나의 space로 압축되고, 제한 안에 있으면 잘리지 않는다는
    // happy path를 고정합니다.
    #[test]
    fn keeps_compact_text_within_limit() {
        assert_eq!(
            compact_whitespace_detail("alpha   beta\ngamma", 32),
            "alpha beta gamma"
        );
    }

    // 학습 주석: 이 test는 whitespace 압축 후 길이 제한을 적용한다는 순서를 검증합니다. 원문 기준으로 먼저
    // 자르면 `"alpha   beta"`처럼 불필요한 공백이 결과 길이를 잡아먹을 수 있습니다.
    #[test]
    fn truncates_after_compacting_whitespace() {
        assert_eq!(
            compact_whitespace_detail("alpha   beta gamma", 10),
            "alpha b..."
        );
    }
}
