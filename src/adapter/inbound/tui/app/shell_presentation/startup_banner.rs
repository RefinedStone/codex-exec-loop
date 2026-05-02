// 학습 주석: shell presentation의 Line 타입은 ratatui line을 감싼 표시 단위입니다. startup banner는 일반 conversation line 대신
// 이 타입의 정적 라인 묶음을 반환해 shell entry와 inline terminal 양쪽에서 같은 banner를 재사용합니다.
use super::Line;
// 학습 주석: build_startup_banner_lines_from_context는 현재 테스트에서 shell core context의 banner 활성 조건을 검증하는 입구입니다.
// production 경로는 base overlay의 wrapper를 통해 같은 startup_ascii_art_lines 함수를 호출합니다.
#[cfg(test)]
use super::ShellCorePresentationContext;

// 학습 주석: 기본 startup banner는 AKRA 브랜드를 terminal-safe box drawing glyph로 표현합니다. 이 문자열은 렌더링 중 계산하거나
// 외부 파일에서 읽지 않고, startup 화면을 즉시 그릴 수 있게 compile-time 상수로 둡니다.
const STARTUP_ASCII_ART_DEFAULT: &str = r#"
 █████╗ ██╗  ██╗██████╗  █████╗
██╔══██╗██║ ██╔╝██╔══██╗██╔══██╗
███████║█████╔╝ ██████╔╝███████║
██╔══██║██╔═██╗ ██╔══██╗██╔══██║
██║  ██║██║  ██╗██║  ██║██║  ██║
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝
"#;

// 학습 주석: 이 함수는 shell core context가 "지금 startup banner를 보여도 되는가"를 판단한 뒤 실제 라인을 만듭니다. 테스트 전용인
// 이유는 production 쪽에서는 app state wrapper가 동일한 상태 판단을 더 바깥에서 수행하기 때문입니다.
#[cfg(test)]
pub(super) fn build_startup_banner_lines_from_context(
    // 학습 주석: context는 startup state, overlay, conversation 상태를 모은 presentation snapshot입니다. 여기서는 banner 활성 여부만
    // 읽고 내부 값을 변경하지 않습니다.
    context: &ShellCorePresentationContext<'_>,
    // 학습 주석: max_height는 inline terminal이나 좁은 viewport가 banner 전체를 담지 못할 때 중앙 부분만 남기기 위한 렌더링 제약입니다.
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    // 학습 주석: banner가 비활성 상태이거나 높이가 0이면 None을 반환합니다. 호출자는 None을 보고 일반 shell/prompt rendering으로
    // 넘어가므로, 빈 Vec보다 "banner 없음"을 명시하는 Option이 더 정확한 계약입니다.
    if !context.startup_banner_is_active() || max_height == Some(0) {
        return None;
    }

    // 학습 주석: 활성 상태에서는 공통 ASCII art renderer를 사용합니다. 테스트 함수도 production wrapper도 같은 renderer를 타야
    // crop 규칙과 glyph 안전성 테스트가 실제 표시 경로를 보호합니다.
    Some(startup_ascii_art_lines(max_height))
}

// 학습 주석: startup_ascii_art_lines는 raw banner 문자열을 화면에 올릴 Line 목록으로 정규화합니다. 앞뒤 빈 줄 제거와 max_height
// crop을 여기서 처리해, 호출자는 "현재 화면 높이에 맞는 banner 라인"만 받습니다.
pub(in super::super) fn startup_ascii_art_lines(max_height: Option<u16>) -> Vec<Line<'static>> {
    // 학습 주석: raw string은 보기 좋게 앞뒤 줄바꿈을 포함합니다. 먼저 줄 단위 Vec로 바꿔 양끝의 장식용 빈 줄을 찾아낼 수 있게 합니다.
    let art_lines_vec = STARTUP_ASCII_ART_DEFAULT.lines().collect::<Vec<_>>();
    // 학습 주석: start는 첫 비어 있지 않은 줄입니다. 상수 문자열이 r#"... "# 형태라 첫 줄이 빈 줄이므로, 이 계산이 없으면
    // banner 위에 의도하지 않은 공백 line이 생깁니다.
    let start = art_lines_vec
        .iter()
        .position(|line| !line.trim().is_empty())
        // 학습 주석: 전부 빈 문자열인 경우에도 panic하지 않고 0부터 slice하도록 fallback을 둡니다. 현재 상수에는 실제 glyph가 있습니다.
        .unwrap_or(0);
    // 학습 주석: end는 마지막 비어 있지 않은 줄의 다음 index입니다. Rust slice의 끝 범위가 exclusive라서 index + 1을 저장합니다.
    let end = art_lines_vec
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        // 학습 주석: banner가 비어 있는 비정상 상황에서도 전체 Vec 길이를 end로 써서 slice 범위를 유효하게 유지합니다.
        .unwrap_or(art_lines_vec.len());
    // 학습 주석: art_lines는 실제 표시 대상 slice입니다. 이후 max_height가 들어오면 같은 변수에 더 좁은 중앙 slice를 다시 가리킵니다.
    let mut art_lines = &art_lines_vec[start..end];

    // 학습 주석: max_height가 없으면 banner 전체 6줄을 반환합니다. 값이 있으면 viewport가 허용하는 높이에 맞춰 중앙 부분만 남깁니다.
    if let Some(max_height) = max_height {
        // 학습 주석: UI 높이는 u16으로 들어오지만 slice 계산은 usize를 쓰므로 한 번만 변환합니다.
        let max_height = max_height as usize;
        // 학습 주석: 0은 context 함수에서 이미 None 처리되지만, renderer를 직접 호출하는 경로를 위해 여기서도 crop을 건너뜁니다.
        if max_height > 0 && art_lines.len() > max_height {
            // 학습 주석: 중앙 crop은 로고의 위아래를 균형 있게 줄입니다. saturating_sub는 조건이 바뀌어도 underflow가 나지 않게 합니다.
            let start = art_lines.len().saturating_sub(max_height) / 2;
            art_lines = &art_lines[start..start + max_height];
        }
    }

    // 학습 주석: 마지막 단계에서 &str slice를 ratatui Line으로 변환합니다. Line<'static>이 가능한 이유는 원본 문자열이 static 상수이고,
    // 각 줄도 그 상수에서 빌린 정적 문자열이기 때문입니다.
    art_lines.iter().map(|line| Line::from(*line)).collect()
}

// 학습 주석: 테스트 모듈은 banner가 terminal에서 깨지기 쉬운 색상 코드나 비표준 문자를 포함하지 않는다는 표시 계약을 지킵니다.
#[cfg(test)]
mod tests {
    // 학습 주석: 테스트는 public wrapper가 아니라 순수 renderer를 직접 호출해 trimming/crop 이후의 실제 출력 라인을 검증합니다.
    use super::startup_ascii_art_lines;

    // 학습 주석: 이 테스트는 startup banner가 6줄이고, 의도한 block glyph를 포함하며, 허용된 box drawing 문자만 쓴다는 계약입니다.
    #[test]
    fn startup_ascii_art_uses_plain_terminal_safe_glyphs() {
        // 학습 주석: Line을 String으로 바꾸면 ratatui 타입 세부 구현과 무관하게 최종 사용자에게 보일 텍스트만 검사할 수 있습니다.
        let rendered = startup_ascii_art_lines(None)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        // 학습 주석: 6줄 길이 검증은 앞뒤 빈 줄 trimming이 유지되는지 확인합니다.
        assert_eq!(rendered.len(), 6);
        // 학습 주석: block glyph 포함 검증은 상수가 빈 문자열이나 잘못된 banner로 바뀌는 회귀를 잡습니다.
        assert!(rendered.iter().any(|line| line.contains("██████")));
        // 학습 주석: 허용 문자 검증은 startup 첫 화면이 ANSI escape, emoji, 폭이 불안정한 문자를 포함하지 않게 막습니다.
        assert!(rendered.iter().all(|line| line.chars().all(|ch| {
            matches!(ch, ' ' | '█' | '╗' | '╔' | '╝' | '╚' | '═' | '║')
        })));
    }
}
