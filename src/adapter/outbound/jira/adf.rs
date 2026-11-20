//! Jira 문서 형식(ADF)의 정해진 부분을 Markdown 으로 옮기고 (#2433), 그 Markdown 을 ADF 로 옮긴다 (#2434).
//! 순수한 변환이다: 같은 문서는 같은 글을 준다.
//!
//! 지원하는 노드: `doc`, `paragraph`, `heading`, `bulletList`, `orderedList`, `listItem`, `codeBlock`, `text`,
//! `hardBreak`. 지원하는 표시(mark): `strong`, `em`, `code`, `strike`, `link`.
//! 그 밖의 노드는 글에 `[unsupported: <type>]` 을 남기고 `unsupported` 에 적는다. 그 밖의 표시는 글을 그대로 두고
//! `unsupported` 에 `mark:<type>` 으로 적는다. 버리지 않는다.
//!
//! Markdown 은 비교하는 표현이다 (1판, `JIRA_BODY_REPRESENTATION_VERSION`). 필드 해시는 이 글로 만든다. 그래서
//! 읽기는 공급자가 바꿀 수 있는 것(표시의 순서, 글 노드의 나눔, 표현에 없는 속성)에 기대지 않는다. Markdown 의
//! 문법 글자가 Jira 의 글에 있으면 `\` 를 붙인다. 쓰기(`writable_adf`)는 ADF 로 옮긴 뒤 다시 읽은 글이 처음 글과
//! 같을 때만 문서를 준다. 같지 않은 글은 쓰지 않는다.

use std::collections::BTreeSet;
use std::ops::Range;

use serde_json::{Value, json};

/// 본문 표현의 판이다. 같은 ADF 가 다른 Markdown 이 되게 바꾸면 올린다.
// shortcut: 판이 필드 해시와 승인에 들어 있지 않다, 표현을 바꾸는 날에 `IssueFieldHash` 와 함께 묶는다.
pub const JIRA_BODY_REPRESENTATION_VERSION: u32 = 1;

/// 쓰기가 `\` 뒤에서 글자 그대로 읽는 글자이다.
const ESCAPABLE: &str = "\\*`~[(#-.";
/// 읽기가 표시를 씌우는 순서이다 (안쪽부터). 공급자가 준 순서를 쓰지 않는다.
const MARK_ORDER: [&str; 5] = ["code", "strike", "em", "strong", "link"];

/// ADF 를 옮긴 결과이다.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct AdfMarkdown {
    pub(super) markdown: String,
    /// 옮기지 못한 노드와 표시의 종류이다. 정렬되어 있고 같은 값은 한 번만 있다.
    pub(super) unsupported: Vec<String>,
}

/// `document` 는 Jira 의 `description` 값이다. `null` 은 빈 본문이다.
pub(super) fn adf_to_markdown(document: &Value) -> AdfMarkdown {
    let mut unsupported = BTreeSet::new();
    let markdown = if document.is_null() {
        String::new()
    } else {
        block(document, &mut unsupported)
    };
    AdfMarkdown {
        markdown,
        unsupported: unsupported.into_iter().collect(),
    }
}

fn node_type(node: &Value) -> &str {
    node["type"].as_str().unwrap_or("unknown")
}

fn children(node: &Value) -> &[Value] {
    node["content"].as_array().map_or(&[], Vec::as_slice)
}

fn blocks(node: &Value, unsupported: &mut BTreeSet<String>) -> Vec<String> {
    children(node)
        .iter()
        .map(|child| block(child, unsupported))
        .collect()
}

fn block(node: &Value, unsupported: &mut BTreeSet<String>) -> String {
    match node_type(node) {
        "doc" => blocks(node, unsupported).join("\n\n"),
        "paragraph" => escape_line_starts(&inline(node, unsupported)),
        "heading" => {
            let level = node["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6);
            format!(
                "{} {}",
                "#".repeat(level as usize),
                inline(node, unsupported)
            )
        }
        "bulletList" => list(node, None, unsupported),
        "orderedList" => {
            let first = node["attrs"]["order"].as_u64().unwrap_or(1);
            list(node, Some(first), unsupported)
        }
        "codeBlock" => {
            let language = node["attrs"]["language"].as_str().unwrap_or_default();
            let code: String = children(node)
                .iter()
                .filter_map(|child| child["text"].as_str())
                .collect();
            format!("```{language}\n{code}\n```")
        }
        other => unsupported_node(other, unsupported),
    }
}

fn unsupported_node(node_type: &str, unsupported: &mut BTreeSet<String>) -> String {
    unsupported.insert(node_type.to_string());
    format!("[unsupported: {node_type}]")
}

/// `first` 가 있으면 번호 목록이다. 항목의 둘째 줄부터는 표식의 너비만큼 들여 쓴다.
fn list(node: &Value, first: Option<u64>, unsupported: &mut BTreeSet<String>) -> String {
    let mut lines = Vec::new();
    for (index, item) in children(node).iter().enumerate() {
        let marker = match first {
            Some(first) => format!("{}. ", first.saturating_add(index as u64)),
            None => "- ".to_string(),
        };
        let text = if node_type(item) == "listItem" {
            blocks(item, unsupported).join("\n")
        } else {
            unsupported_node(node_type(item), unsupported)
        };
        let indent = " ".repeat(marker.len());
        for (line_index, line) in text.split('\n').enumerate() {
            let prefix = if line_index == 0 { &marker } else { &indent };
            lines.push(format!("{prefix}{line}"));
        }
    }
    lines.join("\n")
}

fn inline(node: &Value, unsupported: &mut BTreeSet<String>) -> String {
    let mut text = String::new();
    for child in children(node) {
        let piece = match node_type(child) {
            "text" => marked_text(child, unsupported),
            "hardBreak" => "\n".to_string(),
            other => unsupported_node(other, unsupported),
        };
        // 이웃한 두 노드가 링크의 `](` 를 만들지 않게 한다.
        if text.ends_with(']') && piece.starts_with('(') {
            text.push('\\');
        }
        text.push_str(&piece);
    }
    text
}

fn marked_text(node: &Value, unsupported: &mut BTreeSet<String>) -> String {
    let marks = node["marks"].as_array().map_or(&[][..], Vec::as_slice);
    let has = |kind: &str| marks.iter().find(|mark| node_type(mark) == kind);
    let raw = node["text"].as_str().unwrap_or_default();
    // 코드 표시 안의 글은 글자 그대로이다.
    let mut text = if has("code").is_some() {
        raw.to_string()
    } else {
        escape_text(raw)
    };
    for kind in MARK_ORDER {
        let Some(mark) = has(kind) else { continue };
        text = match kind {
            "code" => format!("`{text}`"),
            "strike" => format!("~~{text}~~"),
            "em" => format!("*{text}*"),
            "strong" => format!("**{text}**"),
            _ => format!(
                "[{text}]({})",
                mark["attrs"]["href"].as_str().unwrap_or_default()
            ),
        };
    }
    for mark in marks {
        if !MARK_ORDER.contains(&node_type(mark)) {
            unsupported.insert(format!("mark:{}", node_type(mark)));
        }
    }
    text
}

/// Jira 의 글에 있는 Markdown 문법 글자에 `\` 를 붙인다. 쓰기가 그 글자를 표시로 읽지 않게 한다.
fn escape_text(text: &str) -> String {
    let mut escaped = String::new();
    let mut previous = None;
    for (index, character) in text.char_indices() {
        let rest = &text[index + character.len_utf8()..];
        let next = rest.chars().next();
        let needs_escape = match character {
            '*' | '`' => true,
            '\\' => next.is_none_or(|next| ESCAPABLE.contains(next)),
            '~' => previous == Some('~') || next == Some('~'),
            // 글자 그대로의 `[unsupported: x]` 는 옮기지 못한 노드의 자리와 다르다.
            '[' => rest.starts_with("unsupported: "),
            '(' => previous == Some(']'),
            _ => false,
        };
        if needs_escape {
            escaped.push('\\');
        }
        escaped.push(character);
        previous = Some(character);
    }
    escaped
}

/// 문단의 줄이 제목이나 목록으로 읽히지 않게 한다.
fn escape_line_starts(text: &str) -> String {
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| match list_marker(line) {
            // `1. ` 은 `1\. ` 이 된다.
            Some((Some(_), width)) => format!("{}\\{}", &line[..width - 2], &line[width - 2..]),
            Some((None, _)) => format!("\\{line}"),
            None if heading(line).is_some() => format!("\\{line}"),
            None => line.to_string(),
        })
        .collect();
    lines.join("\n")
}

/// `# ` 부터 `###### ` 까지로 시작하는 줄의 단계와 나머지 글이다.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    let text = line[level..].strip_prefix(' ')?;
    (1..=6).contains(&level).then_some((level, text))
}

/// 목록 표식(`- `, `12. `)의 번호와 너비이다. 번호가 없으면 글머리 목록이다.
fn list_marker(line: &str) -> Option<(Option<u64>, usize)> {
    if line.starts_with("- ") {
        return Some((None, 2));
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let number = line[..digits].parse().ok()?;
    line[digits..]
        .starts_with(". ")
        .then_some((Some(number), digits + 2))
}

/// 쓸 수 있는 본문의 ADF 문서를 준다. ADF 로 옮긴 뒤 다시 읽은 글이 `markdown` 과 같을 때만 준다. `None` 은 이
/// 표현으로 보존하지 못하는 글이다 (예: `[unsupported: x]` 자리, 닫지 않은 표시, `\r`). 그 글은 쓰지 않는다.
pub(super) fn writable_adf(markdown: &str) -> Option<Value> {
    // 줄 끝의 `\r` 은 공급자가 지울 수 있다. 지워지면 다시 읽은 글이 승인한 글과 다르다.
    if markdown.contains('\r') {
        return None;
    }
    let lines: Vec<&str> = markdown.split('\n').collect();
    let document = json!({ "type": "doc", "version": 1, "content": block_nodes(&lines) });
    let read_back = adf_to_markdown(&document);
    (read_back.markdown == markdown && read_back.unsupported.is_empty()).then_some(document)
}

fn starts_block(line: &str) -> bool {
    line.is_empty()
        || line.starts_with("```")
        || heading(line).is_some()
        || list_marker(line).is_some()
}

// shortcut: 목록 항목의 깊이만큼 재귀한다, 본문의 크기 한도가 없는 입력을 받게 되면 깊이를 막는다.
fn block_nodes(lines: &[&str]) -> Vec<Value> {
    let mut nodes = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.is_empty() {
            index += 1;
        } else if let Some(language) = line.strip_prefix("```") {
            let body = &lines[index + 1..];
            let length = body
                .iter()
                .position(|line| *line == "```")
                .unwrap_or(body.len());
            let code = body[..length].join("\n");
            let mut node = json!({ "type": "codeBlock" });
            if !language.is_empty() {
                node["attrs"] = json!({ "language": language });
            }
            if !code.is_empty() {
                node["content"] = json!([{ "type": "text", "text": code }]);
            }
            nodes.push(node);
            index += length + 2;
        } else if let Some((level, text)) = heading(line) {
            nodes.push(json!({
                "type": "heading",
                "attrs": { "level": level },
                "content": inline_nodes(text),
            }));
            index += 1;
        } else if let Some((first, _)) = list_marker(line) {
            let mut items = Vec::new();
            while let Some((number, width)) = lines.get(index).and_then(|line| list_marker(line)) {
                if number.is_some() != first.is_some() {
                    break;
                }
                // 항목의 둘째 줄부터는 표식의 너비만큼 들여 쓴 줄이다.
                let indent = " ".repeat(width);
                let mut item = vec![&lines[index][width..]];
                index += 1;
                while let Some(continued) = lines
                    .get(index)
                    .and_then(|line| line.strip_prefix(indent.as_str()))
                {
                    item.push(continued);
                    index += 1;
                }
                items.push(json!({ "type": "listItem", "content": block_nodes(&item) }));
            }
            nodes.push(match first {
                Some(order) => {
                    json!({ "type": "orderedList", "attrs": { "order": order }, "content": items })
                }
                None => json!({ "type": "bulletList", "content": items }),
            });
        } else {
            let length = lines[index + 1..]
                .iter()
                .position(|line| starts_block(line))
                .map_or(lines.len() - index, |position| position + 1);
            nodes.push(json!({
                "type": "paragraph",
                "content": inline_nodes(&lines[index..index + length].join("\n")),
            }));
            index += length;
        }
    }
    nodes
}

fn inline_nodes(text: &str) -> Vec<Value> {
    let characters: Vec<char> = text.chars().collect();
    let mut nodes = Vec::new();
    push_inline(&characters, &[], &mut nodes);
    nodes
}

fn text_node(text: String, marks: &[Value]) -> Value {
    let mut node = json!({ "type": "text", "text": text });
    if !marks.is_empty() {
        node["marks"] = Value::Array(marks.to_vec());
    }
    node
}

fn is_escape(characters: &[char], index: usize) -> bool {
    characters[index] == '\\'
        && characters
            .get(index + 1)
            .is_some_and(|next| ESCAPABLE.contains(*next))
}

/// `marks` 는 바깥 표시이다. 안쪽 표시가 앞에 온다.
fn push_inline(characters: &[char], marks: &[Value], nodes: &mut Vec<Value>) {
    let mut plain = String::new();
    let flush = |plain: &mut String, nodes: &mut Vec<Value>| {
        if !plain.is_empty() {
            nodes.push(text_node(std::mem::take(plain), marks));
        }
    };
    let mut index = 0;
    while index < characters.len() {
        if is_escape(characters, index) {
            plain.push(characters[index + 1]);
            index += 2;
        } else if characters[index] == '\n' {
            flush(&mut plain, nodes);
            nodes.push(json!({ "type": "hardBreak" }));
            index += 1;
        } else if let Some((mark, inner, end)) = marked_span(characters, index) {
            flush(&mut plain, nodes);
            let marks = [std::slice::from_ref(&mark), marks].concat();
            if mark["type"] == "code" {
                nodes.push(text_node(characters[inner].iter().collect(), &marks));
            } else {
                push_inline(&characters[inner], &marks, nodes);
            }
            index = end;
        } else {
            plain.push(characters[index]);
            index += 1;
        }
    }
    flush(&mut plain, nodes);
}

/// `index` 에서 시작하는 표시 구간이다: 표시, 안쪽 글의 범위, 구간의 끝. 닫는 표식이 없으면 구간이 아니다.
fn marked_span(characters: &[char], index: usize) -> Option<(Value, Range<usize>, usize)> {
    let at = |position: usize| characters.get(position).copied();
    match characters[index] {
        '`' => {
            let close =
                (index + 1..characters.len()).find(|position| at(*position) == Some('`'))?;
            Some((json!({ "type": "code" }), index + 1..close, close + 1))
        }
        '*' => {
            let width = if star_run(characters, index) == 2 {
                2
            } else {
                1
            };
            let close = closing_stars(characters, index + width, width)?;
            let kind = if width == 2 { "strong" } else { "em" };
            Some((json!({ "type": kind }), index + width..close, close + width))
        }
        '~' if at(index + 1) == Some('~') => {
            let close = find_unescaped(characters, index + 2, |position| {
                at(position) == Some('~') && at(position + 1) == Some('~')
            })?;
            Some((json!({ "type": "strike" }), index + 2..close, close + 2))
        }
        '[' => {
            // 링크의 글은 첫 `]` 에서 끝난다. 그 앞에 다른 `[` 가 있으면 링크가 아니다.
            let close = find_unescaped(characters, index + 1, |position| {
                matches!(at(position), Some('[' | ']'))
            })?;
            if at(close) != Some(']') || at(close + 1) != Some('(') {
                return None;
            }
            let end = (close + 2..characters.len()).find(|position| at(*position) == Some(')'))?;
            let href: String = characters[close + 2..end].iter().collect();
            let mark = json!({ "type": "link", "attrs": { "href": href } });
            Some((mark, index + 1..close, end + 1))
        }
        _ => None,
    }
}

fn find_unescaped(
    characters: &[char],
    from: usize,
    matches: impl Fn(usize) -> bool,
) -> Option<usize> {
    let mut index = from;
    while index < characters.len() {
        if is_escape(characters, index) {
            index += 2;
        } else if matches(index) {
            return Some(index);
        } else {
            index += 1;
        }
    }
    None
}

fn star_run(characters: &[char], index: usize) -> usize {
    characters[index..]
        .iter()
        .take_while(|character| **character == '*')
        .count()
}

/// 너비가 `width` 인 `*` 표식을 닫는 자리이다. 너비가 다른 `*` 는 안쪽 표시이다. 셋 이상 이어진 `*` 는 안쪽 표시를
/// 먼저 닫으므로 그 끝이 닫는 자리이다.
fn closing_stars(characters: &[char], from: usize, width: usize) -> Option<usize> {
    let mut index = from;
    while index < characters.len() {
        if is_escape(characters, index) {
            index += 2;
        } else if characters[index] == '*' {
            let run = star_run(characters, index);
            if run == width || run > 2 {
                return Some(index + run - width);
            }
            index += run;
        } else {
            index += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supported_subset_becomes_markdown() {
        let document: Value =
            serde_json::from_str(include_str!("fixtures/description_supported.json")).unwrap();

        let converted = adf_to_markdown(&document);

        assert_eq!(
            converted.markdown,
            "## Steps\n\n\
             Run **the tool** with `--flag` and *read* the ~~old~~ [guide](https://example.invalid/guide).\nSecond line.\n\n\
             - first\n- second\n  1. nested one\n  2. nested two\n\n\
             3. third\n4. fourth\n\n\
             ```rust\nfn main() {}\nlet x = 1;\n```"
        );
        assert_eq!(converted.unsupported, Vec::<String>::new());
    }

    #[test]
    fn unknown_nodes_and_marks_are_reported_and_never_dropped_silently() {
        let document: Value =
            serde_json::from_str(include_str!("fixtures/description_unsupported.json")).unwrap();

        let converted = adf_to_markdown(&document);

        assert_eq!(
            converted.markdown,
            "Ask [unsupported: mention] about underlined text.\n\n\
             [unsupported: mediaSingle]\n\n\
             [unsupported: table]\n\n\
             - [unsupported: paragraphLike]"
        );
        assert_eq!(
            converted.unsupported,
            [
                "mark:underline",
                "mediaSingle",
                "mention",
                "paragraphLike",
                "table"
            ]
        );
    }

    #[test]
    fn a_missing_description_is_an_empty_body() {
        assert_eq!(
            adf_to_markdown(&Value::Null),
            AdfMarkdown {
                markdown: String::new(),
                unsupported: Vec::new(),
            }
        );
        // 종류가 없는 값은 지원하지 않는 노드이다.
        assert_eq!(
            adf_to_markdown(&serde_json::json!("plain")).unsupported,
            ["unknown"]
        );
    }

    const SUPPORTED_MARKDOWN: &str = "## Steps\n\n\
         Run **the tool** with `--flag` and *read* the ~~old~~ [guide](https://example.invalid/guide).\nSecond line.\n\n\
         - first\n- second\n  1. nested one\n  2. nested two\n\n\
         3. third\n4. fourth\n\n\
         ```rust\nfn main() {}\n\nlet x = 1;\n```";

    #[test]
    fn the_supported_subset_is_written_as_adf_and_reads_back_the_same() {
        assert_eq!(JIRA_BODY_REPRESENTATION_VERSION, 1);
        let document = writable_adf(SUPPORTED_MARKDOWN).expect("writable");
        assert_eq!(adf_to_markdown(&document).markdown, SUPPORTED_MARKDOWN);

        assert_eq!(
            writable_adf("# T\n\nA ***b*** [[akra-mask:1]]\n\\- c\n\n9. x\n10. y\n    `z`"),
            Some(json!({
                "type": "doc",
                "version": 1,
                "content": [
                    {
                        "type": "heading",
                        "attrs": { "level": 1 },
                        "content": [{ "type": "text", "text": "T" }]
                    },
                    {
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "A " },
                            { "type": "text", "text": "b", "marks": [{ "type": "strong" }, { "type": "em" }] },
                            { "type": "text", "text": " [[akra-mask:1]]" },
                            { "type": "hardBreak" },
                            { "type": "text", "text": "- c" }
                        ]
                    },
                    {
                        "type": "orderedList",
                        "attrs": { "order": 9 },
                        "content": [
                            { "type": "listItem", "content": [
                                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }
                            ] },
                            { "type": "listItem", "content": [
                                { "type": "paragraph", "content": [
                                    { "type": "text", "text": "y" },
                                    { "type": "hardBreak" },
                                    { "type": "text", "text": "z", "marks": [{ "type": "code" }] }
                                ] }
                            ] }
                        ]
                    }
                ]
            }))
        );
        // 빈 본문은 빈 문서이다.
        assert_eq!(
            writable_adf(""),
            Some(json!({ "type": "doc", "version": 1, "content": [] }))
        );
    }

    fn paragraph(content: Value) -> Value {
        json!({ "type": "doc", "version": 1, "content": [{ "type": "paragraph", "content": content }] })
    }

    #[test]
    fn markdown_syntax_in_jira_text_is_escaped_and_written_back_as_the_same_text() {
        // Jira 의 글이다. 표시가 없다. Markdown 으로는 강조, 코드, 링크, 제목, 목록처럼 보인다.
        let text =
            r"# not a heading *plain* `tick` ~~kept~~ [a](b) [unsupported: x] C:\dir \* end\";
        let document = paragraph(json!([
            { "type": "text", "text": text },
            { "type": "hardBreak" },
            { "type": "text", "text": "- not a list" },
            { "type": "hardBreak" },
            { "type": "text", "text": "12. not a number ~/home *.rs" }
        ]));

        let converted = adf_to_markdown(&document);

        assert_eq!(
            converted.markdown,
            "\\# not a heading \\*plain\\* \\`tick\\` \\~\\~kept\\~\\~ [a]\\(b) \\[unsupported: x] C:\\dir \\\\\\* end\\\\\n\
             \\- not a list\n\
             12\\. not a number ~/home \\*.rs"
        );
        assert_eq!(converted.unsupported, Vec::<String>::new());
        // 쓰기는 같은 글 노드를 만든다. 표시를 만들지 않는다.
        assert_eq!(writable_adf(&converted.markdown), Some(document));
    }

    #[test]
    fn the_reading_does_not_depend_on_mark_order_or_text_node_splits() {
        let marks = |order: [&str; 2]| {
            paragraph(json!([
                { "type": "text", "text": "a" },
                { "type": "text", "text": "b", "marks": [
                    { "type": order[0], "attrs": { "href": "https://example.invalid/" } },
                    { "type": order[1], "attrs": { "href": "https://example.invalid/" } }
                ] },
                { "type": "text", "text": "]" },
                { "type": "text", "text": "(c)" }
            ]))
        };

        let converted = adf_to_markdown(&marks(["link", "strong"]));

        assert_eq!(
            converted.markdown,
            "a[**b**](https://example.invalid/)]\\(c)"
        );
        assert_eq!(adf_to_markdown(&marks(["strong", "link"])), converted);
        assert!(writable_adf(&converted.markdown).is_some());
    }

    #[test]
    fn a_body_that_does_not_read_back_the_same_is_not_writable() {
        for body in [
            // 옮기지 못한 노드의 자리이다. 글자로 쓰면 원격의 노드를 지운다.
            "Ask [unsupported: mention] first.",
            "[unsupported: table]",
            // 닫지 않은 표시와 `\` 가 없는 문법 글자이다.
            "run *.rs",
            "a ` b",
            "**bold",
            // 표현에 없는 모양이다.
            "line\r\nline",
            "a\n\n\n\nb",
            "paragraph\n- item",
            "1. one\n3. three",
            "```\nnever closed",
            "**[link](https://example.invalid/)**",
        ] {
            assert_eq!(writable_adf(body), None, "{body:?}");
        }
    }
}
