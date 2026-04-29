#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptDocument {
    title: String,
    sections: Vec<PromptSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptSection {
    title: String,
    lines: Vec<String>,
}

impl PromptDocument {
    pub(crate) fn builder(title: impl Into<String>) -> PromptDocumentBuilder {
        PromptDocumentBuilder {
            title: title.into(),
            sections: Vec::new(),
        }
    }

    pub(crate) fn render(&self) -> String {
        let mut lines = vec![format!("# {}", self.title.trim())];
        for section in &self.sections {
            if section.lines.is_empty() {
                continue;
            }
            lines.push(String::new());
            lines.push(format!("[{}]", section.title.trim()));
            lines.extend(section.lines.iter().cloned());
        }
        lines.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptDocumentBuilder {
    title: String,
    sections: Vec<PromptSection>,
}

impl PromptDocumentBuilder {
    pub(crate) fn raw_lines(mut self, title: impl Into<String>, lines: Vec<String>) -> Self {
        let lines = lines
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>();
        if lines.iter().any(|line| !line.trim().is_empty()) {
            self.sections.push(PromptSection {
                title: title.into(),
                lines,
            });
        }
        self
    }

    pub(crate) fn lines(mut self, title: impl Into<String>, lines: Vec<String>) -> Self {
        let lines = lines
            .into_iter()
            .map(|line| line.trim_end().to_string())
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            self.sections.push(PromptSection {
                title: title.into(),
                lines,
            });
        }
        self
    }

    pub(crate) fn bullets(self, title: impl Into<String>, bullets: Vec<String>) -> Self {
        self.lines(
            title,
            bullets
                .into_iter()
                .map(|bullet| format!("- {}", bullet.trim()))
                .collect(),
        )
    }

    pub(crate) fn text(self, title: impl Into<String>, text: &str) -> Self {
        let lines = text
            .trim()
            .lines()
            .map(|line| line.trim_end().to_string())
            .collect::<Vec<_>>();
        self.lines(title, lines)
    }

    pub(crate) fn optional_text(self, title: impl Into<String>, text: Option<&str>) -> Self {
        match text.map(str::trim).filter(|value| !value.is_empty()) {
            Some(text) => self.text(title, text),
            None => self,
        }
    }

    pub(crate) fn code_block(self, title: impl Into<String>, language: &str, body: &str) -> Self {
        let body = body.trim();
        let mut lines = vec![format!("```{language}")];
        lines.extend(body.lines().map(|line| line.trim_end().to_string()));
        lines.push("```".to_string());
        self.raw_lines(title, lines)
    }

    pub(crate) fn optional_code_block(
        self,
        title: impl Into<String>,
        language: &str,
        body: Option<&str>,
    ) -> Self {
        match body.map(str::trim).filter(|value| !value.is_empty()) {
            Some(body) => self.code_block(title, language, body),
            None => self,
        }
    }

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
    fn renders_only_non_empty_sections() {
        let prompt = PromptDocument::builder("task")
            .lines("empty", vec![String::new(), "   ".to_string()])
            .bullets("rules", vec!["do this".to_string(), "do that".to_string()])
            .optional_text("missing", None)
            .text("payload", "alpha\nbeta")
            .optional_code_block("missing-code", "json", None)
            .code_block("json", "json", "{\n  \"ok\": true\n}")
            .build()
            .render();

        assert_eq!(
            prompt,
            "# task\n\n[rules]\n- do this\n- do that\n\n[payload]\nalpha\nbeta\n\n[json]\n```json\n{\n  \"ok\": true\n}\n```"
        );
    }
}
