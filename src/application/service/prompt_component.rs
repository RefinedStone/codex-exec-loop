#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `PromptDocument`는 agent에게 전달할 긴 prompt를 제목과 named section들의 문서로 조립하는
// 작은 application 계층 값입니다. planning/runtime prompt, queue handoff, admin 문서 생성처럼 여러 흐름이
// 같은 "# title / [section]" 형식을 공유해야 할 때 이 타입을 사용하면 문자열 연결 규칙이 흩어지지 않습니다.
//
// 학습 주석: 이 타입은 Markdown 전체를 모델링하지 않습니다. 프로젝트에서 필요한 최소 형식인
// 최상위 제목, 섹션 제목, 섹션 본문 줄만 다룹니다. 그래서 렌더링 규칙이 예측 가능하고 테스트하기 쉽습니다.
pub(crate) struct PromptDocument {
    // 학습 주석: 최종 prompt의 첫 줄 `# ...`에 들어가는 문서 제목입니다.
    title: String,
    // 학습 주석: 순서가 의미를 가지는 section 목록입니다. builder가 push한 순서대로 render되어
    // system/context/task payload의 우선순위와 읽는 흐름을 유지합니다.
    sections: Vec<PromptSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `PromptSection`은 외부로 공개하지 않는 내부 구성 단위입니다.
// 소비자는 builder 메서드로만 section을 넣게 해서 "빈 section은 렌더링하지 않는다"는 정책을 중앙화합니다.
struct PromptSection {
    // 학습 주석: 렌더링 시 `[title]` 형식으로 출력되는 section 이름입니다.
    title: String,
    // 학습 주석: section 본문을 이미 줄 단위로 정규화한 값입니다. code block, bullet, 일반 텍스트 모두
    // 최종적으로는 이 `Vec<String>`으로 들어와 같은 render 경로를 탑니다.
    lines: Vec<String>,
}

impl PromptDocument {
    // 학습 주석: 문서 생성을 builder로 시작합니다. prompt는 대개 조건부 context와 payload를 순차적으로 붙이므로,
    // 중간 상태를 immutable document로 만들기보다 builder에 section을 누적한 뒤 마지막에 build/render하는 방식이 맞습니다.
    pub(crate) fn builder(title: impl Into<String>) -> PromptDocumentBuilder {
        PromptDocumentBuilder {
            // 학습 주석: `Into<String>`을 받아 호출자가 `&str`과 `String` 모두 편하게 넘기도록 합니다.
            title: title.into(),
            // 학습 주석: 새 builder는 section이 없으며, 이후 fluent 메서드들이 의미 있는 section만 추가합니다.
            sections: Vec::new(),
        }
    }

    // 학습 주석: 내부 구조를 최종 prompt 문자열로 직렬화합니다. agent에게 전달되는 실제 payload는
    // 이 함수의 출력이므로, 공백과 section 구분 규칙은 테스트로 고정되어야 합니다.
    pub(crate) fn render(&self) -> String {
        // 학습 주석: 첫 줄은 항상 `# 제목`입니다. 제목 양끝 공백을 제거해 호출부 실수로 prompt 헤더가 흔들리지 않게 합니다.
        let mut lines = vec![format!("# {}", self.title.trim())];
        for section in &self.sections {
            // 학습 주석: builder에서도 빈 section을 걸러내지만, render에서도 한 번 더 방어합니다.
            // 직접 생성된 테스트 값이나 미래 변경이 빈 section을 넣어도 불필요한 `[empty]` 헤더를 내보내지 않습니다.
            if section.lines.is_empty() {
                continue;
            }
            // 학습 주석: section 사이에는 빈 줄 하나를 넣고, section 제목은 `[name]`으로 감싸 모델이
            // 각 덩어리의 역할을 쉽게 구분하게 합니다.
            lines.push(String::new());
            lines.push(format!("[{}]", section.title.trim()));
            // 학습 주석: 본문 줄은 이미 builder에서 trim_end 처리된 값을 그대로 복사합니다.
            // 앞쪽 공백은 code block이나 들여쓰기 payload에서 의미가 있을 수 있으므로 render에서 제거하지 않습니다.
            lines.extend(section.lines.iter().cloned());
        }
        lines.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `PromptDocumentBuilder`는 section 추가 정책을 소유하는 mutable-like 값입니다.
// 메서드가 `self`를 받아 다시 `Self`를 반환하므로 호출자는 fluent chain으로 prompt 구조를 읽기 좋게 선언할 수 있습니다.
pub(crate) struct PromptDocumentBuilder {
    // 학습 주석: 완성될 document의 제목입니다.
    title: String,
    // 학습 주석: 아직 render 전인 section 누적 목록입니다.
    sections: Vec<PromptSection>,
}

impl PromptDocumentBuilder {
    // 학습 주석: 이미 줄 단위로 준비된 본문을 section으로 추가합니다. `raw_lines`라는 이름은
    // bullet prefix나 text splitting 같은 추가 의미 변환을 하지 않는다는 것을 호출자에게 알려 줍니다.
    pub(crate) fn raw_lines(mut self, title: impl Into<String>, lines: Vec<String>) -> Self {
        // 학습 주석: 오른쪽 공백만 제거합니다. 왼쪽 공백은 code block 내부 indentation이나 payload 정렬에
        // 의미가 있을 수 있으므로 보존합니다.
        let lines = lines
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>();
        // 학습 주석: 모든 줄이 비어 있으면 section 자체를 버립니다. prompt에 빈 헤더가 남으면 모델이
        // 누락된 context가 있는 것으로 오해할 수 있어, "의미 있는 내용이 있을 때만 section 생성"을 기본 정책으로 둡니다.
        if lines.iter().any(|line| !line.trim().is_empty()) {
            self.sections.push(PromptSection {
                title: title.into(),
                lines,
            });
        }
        self
    }

    // 학습 주석: 일반 line section을 추가합니다. 현재는 `raw_lines`와 같은 정규화 규칙을 쓰지만,
    // 호출 의도상 "이미 prompt에 그대로 들어갈 줄들"보다 한 단계 높은 일반 텍스트 라인 API로 남겨 둡니다.
    pub(crate) fn lines(mut self, title: impl Into<String>, lines: Vec<String>) -> Self {
        let lines = lines
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>();
        // 학습 주석: 빈 줄이 섞인 section은 허용하지만, section 전체가 비어 있으면 생략합니다.
        // 그래서 문단 내부 빈 줄은 보존되고, optional context가 비었을 때만 사라집니다.
        if lines.iter().any(|line| !line.trim().is_empty()) {
            self.sections.push(PromptSection {
                title: title.into(),
                lines,
            });
        }
        self
    }

    // 학습 주석: bullet section을 만드는 convenience API입니다. 호출자는 bullet marker를 직접 붙이지 않고
    // 의미 항목만 넘기며, builder가 프로젝트 전반의 "- ..." 형식을 통일합니다.
    pub(crate) fn bullets(self, title: impl Into<String>, bullets: Vec<String>) -> Self {
        self.lines(
            title,
            bullets
                .into_iter()
                // 학습 주석: bullet 값은 양끝을 정리한 뒤 marker를 붙입니다. 빈 bullet은 여기서 제거하지 않으므로,
                // 호출자가 빈 항목을 넣으면 "- "로 보존됩니다. section 전체 empty 판단은 `lines`가 담당합니다.
                .map(|bullet| format!("- {}", bullet.trim()))
                .collect(),
        )
    }

    // 학습 주석: multi-line text를 section으로 추가합니다. 입력 전체의 양끝 공백은 제거하지만,
    // 내부 줄과 빈 줄은 보존해 사람이 작성한 runtime context의 구조를 유지합니다.
    pub(crate) fn text(self, title: impl Into<String>, text: &str) -> Self {
        let lines = text
            .trim()
            .lines()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>();
        self.lines(title, lines)
    }

    // 학습 주석: optional context section을 표현하는 API입니다. planning fragment, queue summary처럼
    // 있을 때만 prompt에 포함해야 하는 값은 호출부에서 `if let`을 반복하지 않고 이 메서드에 맡깁니다.
    pub(crate) fn optional_text(self, title: impl Into<String>, text: Option<&str>) -> Self {
        // 학습 주석: `None`, 빈 문자열, 공백뿐인 문자열을 모두 "section 없음"으로 통일합니다.
        match text.map(str::trim).filter(|value| !value.is_empty()) {
            Some(text) => self.text(title, text),
            None => self,
        }
    }

    // 학습 주석: fenced code block section을 추가합니다. JSON, diff, TOML 같은 payload를 일반 텍스트로
    // 넣으면 모델이 section 설명과 본문을 헷갈릴 수 있으므로, language fence를 포함한 raw section으로 렌더링합니다.
    pub(crate) fn code_block(self, title: impl Into<String>, language: &str, body: &str) -> Self {
        // 학습 주석: block 전체 양끝 공백은 제거해 불필요한 첫/마지막 빈 줄을 없앱니다.
        let body = body.trim();
        // 학습 주석: fence 자체도 section lines에 포함합니다. 그래서 render는 code block을 특별 취급하지 않고
        // raw line 목록을 그대로 출력하기만 하면 됩니다.
        let mut lines = vec![format!("```{language}")];
        lines.extend(body.lines().map(|line| line.trim_end().to_string()));
        lines.push("```".to_string());
        self.raw_lines(title, lines)
    }

    // 학습 주석: optional fenced block입니다. 외부 도구 출력이나 snapshot JSON이 없을 때
    // 빈 code fence를 남기지 않도록 `optional_text`와 같은 생략 정책을 씁니다.
    pub(crate) fn optional_code_block(
        self,
        // 학습 주석: section 제목입니다.
        title: impl Into<String>,
        // 학습 주석: Markdown fence language입니다. 모델과 사람이 payload 종류를 빠르게 구분하게 합니다.
        language: &str,
        // 학습 주석: 존재할 때만 code block으로 렌더링할 본문입니다.
        body: Option<&str>,
    ) -> Self {
        // 학습 주석: 공백뿐인 body도 없는 것으로 취급해 prompt에 빈 fence가 들어가지 않게 합니다.
        match body.map(str::trim).filter(|value| !value.is_empty()) {
            Some(body) => self.code_block(title, language, body),
            None => self,
        }
    }

    // 학습 주석: builder를 immutable document로 닫습니다. 이후 render는 document를 빌려서만 읽으므로,
    // 조립 단계와 출력 단계를 분리해 테스트에서 구조 비교와 문자열 비교를 모두 할 수 있습니다.
    pub(crate) fn build(self) -> PromptDocument {
        PromptDocument {
            title: self.title,
            sections: self.sections,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PromptDocument;

    #[test]
    // 학습 주석: 이 테스트는 builder의 핵심 출력 계약을 한 번에 고정합니다.
    // 빈 section은 빠지고, bullet/text/code block section은 순서를 유지하며, code fence는 raw line으로 보존되어야 합니다.
    fn renders_only_non_empty_sections() {
        // 학습 주석: 일부러 empty/missing section과 실제 section을 섞어 "의미 있는 section만 남는다"는 정책을 검증합니다.
        let prompt = PromptDocument::builder("task")
            .lines("empty", vec![String::new(), "   ".to_string()])
            .bullets("rules", vec!["do this".to_string(), "do that".to_string()])
            .optional_text("missing", None)
            .text("payload", "alpha\n\nbeta")
            .optional_code_block("missing-code", "json", None)
            .code_block("json", "json", "{\n  \"ok\": true\n}")
            .build()
            .render();

        assert_eq!(
            prompt,
            "# task\n\n[rules]\n- do this\n- do that\n\n[payload]\nalpha\n\nbeta\n\n[json]\n```json\n{\n  \"ok\": true\n}\n```"
        );
    }
}
