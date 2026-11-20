//! Jira 문서 형식(ADF)의 정해진 부분을 Markdown 으로 옮긴다 (#2433). 순수한 변환이다: 같은 문서는 같은 글을 준다.
//!
//! 지원하는 노드: `doc`, `paragraph`, `heading`, `bulletList`, `orderedList`, `listItem`, `codeBlock`, `text`,
//! `hardBreak`. 지원하는 표시(mark): `strong`, `em`, `code`, `strike`, `link`.
//! 그 밖의 노드는 글에 `[unsupported: <type>]` 을 남기고 `unsupported` 에 적는다. 그 밖의 표시는 글을 그대로 두고
//! `unsupported` 에 `mark:<type>` 으로 적는다. 버리지 않는다.

use std::collections::BTreeSet;

use serde_json::Value;

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
        "paragraph" => inline(node, unsupported),
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
    children(node)
        .iter()
        .map(|child| match node_type(child) {
            "text" => marked_text(child, unsupported),
            "hardBreak" => "\n".to_string(),
            other => unsupported_node(other, unsupported),
        })
        .collect()
}

// shortcut: the text is not escaped for Markdown, so `*` in Jira text reads as emphasis; escape when a write
// path (#2434) must round-trip the text.
fn marked_text(node: &Value, unsupported: &mut BTreeSet<String>) -> String {
    let mut text = node["text"].as_str().unwrap_or_default().to_string();
    for mark in node["marks"].as_array().map_or(&[][..], Vec::as_slice) {
        text = match node_type(mark) {
            "strong" => format!("**{text}**"),
            "em" => format!("*{text}*"),
            "code" => format!("`{text}`"),
            "strike" => format!("~~{text}~~"),
            "link" => format!(
                "[{text}]({})",
                mark["attrs"]["href"].as_str().unwrap_or_default()
            ),
            other => {
                unsupported.insert(format!("mark:{other}"));
                text
            }
        };
    }
    text
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
}
