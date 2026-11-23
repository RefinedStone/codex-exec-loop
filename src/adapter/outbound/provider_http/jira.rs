use super::*;

fn issue(value: &Value, site: &str) -> CredentialResult<JiraIssue> {
    object(value)?;
    let key = bounded_text(&value["key"], 60)?;
    if !valid_jira_key(&key) {
        return Err(ProviderCredentialError::Corrupt);
    }
    let fields = object(&value["fields"])?;
    let summary = fields
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if summary.chars().count() > 2000 {
        return Err(ProviderCredentialError::Corrupt);
    }
    let updated = fields
        .get("updated")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if updated.len() > 64 {
        return Err(ProviderCredentialError::Corrupt);
    }
    Ok(JiraIssue {
        url: format!("{site}/browse/{key}"),
        key,
        summary: summary.to_owned(),
        status: nullable_text(&value["fields"]["status"]["name"], 1000)?,
        status_category: nullable_text(&value["fields"]["status"]["statusCategory"]["key"], 128)?,
        issue_type: nullable_text(&value["fields"]["issuetype"]["name"], 1000)?,
        priority: nullable_text(&value["fields"]["priority"]["name"], 1000)?,
        updated_at: updated.to_owned(),
    })
}
pub(super) fn summary(value: &Value, site: &str) -> CredentialResult<JiraSummary> {
    let raw = value["issues"]
        .as_array()
        .ok_or(ProviderCredentialError::Corrupt)?;
    if raw.len() > ISSUE_LIMIT {
        return Err(ProviderCredentialError::TooLarge);
    }
    let issues = raw
        .iter()
        .map(|value| issue(value, site))
        .collect::<CredentialResult<Vec<_>>>()?;
    let unique: std::collections::HashSet<_> = issues.iter().map(|item| &item.key).collect();
    if unique.len() != issues.len() {
        return Err(ProviderCredentialError::Corrupt);
    }
    Ok(JiraSummary {
        site_url: site.to_owned(),
        issues,
    })
}
/// ADF is untrusted. Limit both depth and emitted text independently of the HTTP byte bound.
fn adf(value: &Value, maximum: usize) -> String {
    fn append(value: &str, output: &mut String, remaining: &mut usize) {
        for ch in value.chars().take(*remaining) {
            output.push(ch);
            *remaining -= 1;
        }
    }
    fn walk(value: &Value, depth: usize, output: &mut String, remaining: &mut usize) {
        if depth > 64 || *remaining == 0 {
            return;
        }
        if let Some(text) = value.as_str() {
            append(text, output, remaining);
            return;
        }
        let kind = value["type"].as_str().unwrap_or_default();
        match kind {
            "text" => append(
                value["text"].as_str().unwrap_or_default(),
                output,
                remaining,
            ),
            "hardBreak" => append("\n", output, remaining),
            "mention" | "emoji" => append(
                value["attrs"]["text"].as_str().unwrap_or_default(),
                output,
                remaining,
            ),
            "inlineCard" | "blockCard" => append(
                value["attrs"]["url"].as_str().unwrap_or_default(),
                output,
                remaining,
            ),
            _ => {
                if kind == "listItem" {
                    append("- ", output, remaining);
                }
                if let Some(children) = value["content"].as_array() {
                    for child in children {
                        walk(child, depth + 1, output, remaining);
                    }
                }
                if matches!(
                    kind,
                    "paragraph" | "heading" | "codeBlock" | "rule" | "listItem"
                ) {
                    append("\n", output, remaining);
                }
            }
        }
    }
    let mut result = String::new();
    let mut remaining = maximum + 1;
    walk(value, 0, &mut result, &mut remaining);
    clip(result.trim(), maximum)
}
pub(super) fn prompt(value: &Value, site: &str) -> CredentialResult<String> {
    let item = issue(value, site)?;
    let mut lines = vec![
        "다음 Jira 티켓을 확인하고 작업해 주세요.".to_owned(),
        format!("티켓: {} {} ({})", item.key, item.summary, item.url),
    ];
    let mut facts = Vec::new();
    if let Some(kind) = item.issue_type {
        facts.push(format!("유형: {kind}"));
    }
    if let Some(status) = item.status {
        facts.push(format!("상태: {status}"));
    }
    if let Some(priority) = item.priority {
        facts.push(format!("우선순위: {priority}"));
    }
    if !facts.is_empty() {
        lines.push(facts.join(" · "));
    }
    let description = adf(&value["fields"]["description"], MAX_DESCRIPTION_CHARS);
    lines.extend([
        String::new(),
        "설명:".to_owned(),
        if description.is_empty() {
            "(설명이 없습니다.)".to_owned()
        } else {
            description
        },
    ]);
    if let Some(comments) = value["fields"]["comment"]["comments"].as_array() {
        let recent = &comments[comments.len().saturating_sub(10)..];
        if !recent.is_empty() {
            lines.extend([String::new(), "최근 댓글:".to_owned()]);
            for comment in recent {
                let author = comment["author"]["displayName"].as_str().unwrap_or("?");
                lines.push(format!(
                    "{}: {}",
                    clip(author, 1000),
                    adf(&comment["body"], MAX_COMMENT_CHARS)
                ));
            }
        }
        let total = value["fields"]["comment"]["total"]
            .as_u64()
            .unwrap_or(comments.len() as u64);
        if total > recent.len() as u64 {
            lines.push(format!(
                "(댓글 {total}개 중 최근 {}개만 넣었습니다. 나머지는 티켓에서 확인하세요.)",
                recent.len()
            ));
        }
    }
    Ok(clip(lines.join("\n").trim(), MAX_PROMPT_CHARS))
}
