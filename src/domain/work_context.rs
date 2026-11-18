/*
 * 작업 컨텍스트(plan 22)의 값 타입과 외부 문서 인식 규칙이다. Akra 는 스레드에 나온 외부 문서(Notion 페이지,
 * Jira 티켓, GitHub PR 등)를 모델 없이 코드로 찾아 장부에 적는다. 이 모듈은 I/O 없이 "글에서 주소를 찾고, 그
 * 주소가 어떤 서비스의 어떤 문서인지 정하는" 일만 한다.
 *
 * 새 외부 서비스는 `ExternalResourceRecognizer` 구현 하나를 `external_resource_recognizers` 에 더하면 된다.
 * 장부, 수집, Gateway, Desktop 은 종류 이름(`kind`)만 다루므로 바꾸지 않는다.
 */
use serde::Serialize;

/// 장부에 남기는 제목 힌트의 최대 길이이다.
const TITLE_HINT_MAX_CHARS: usize = 120;

/// 외부 문서 하나를 가리킨다. `(kind, key)` 가 같으면 주소가 달라도 같은 문서이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalResourceRef {
    /// 서비스 종류이다. 인식기의 `kind()` 값이다.
    pub kind: String,
    /// 서비스 안에서 문서를 구분하는 값이다. 주소의 쿼리나 제목 부분이 달라도 같다.
    pub key: String,
    /// 처음 본 주소이다. 사람이 열 때 쓴다.
    pub url: String,
    /// 주소에서 읽은 이름이다. 서비스에 묻지 않고 보여 줄 때 쓴다.
    pub title_hint: Option<String>,
}

/// 주소의 호스트와 경로이다. 쿼리와 조각은 뺀다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpsUrlParts<'a> {
    pub url: &'a str,
    pub host: &'a str,
    /// `/` 로 시작한다. 경로가 없으면 빈 문자열이다.
    pub path: &'a str,
}

/// 외부 서비스 하나의 주소 규칙이다.
pub trait ExternalResourceRecognizer: Send + Sync {
    /// 장부와 화면에 쓰는 종류 이름이다. 소문자 한 단어이다.
    fn kind(&self) -> &'static str;

    /// 이 서비스의 문서 주소이면 문서를 돌려준다. 목록이나 홈처럼 문서 하나가 아닌 주소는 `None` 이다.
    fn recognize(&self, url: &HttpsUrlParts<'_>) -> Option<ExternalResourceRef>;
}

/// 지원하는 외부 서비스들이다. 순서대로 물어보고 처음 인식한 것을 쓴다.
pub fn external_resource_recognizers() -> Vec<Box<dyn ExternalResourceRecognizer>> {
    vec![
        Box::new(NotionRecognizer),
        Box::new(JiraRecognizer),
        Box::new(GitHubRecognizer),
        Box::new(LinearRecognizer),
    ]
}

/// 글에서 외부 문서를 찾는다. 같은 문서는 한 번만 돌려주고, 처음 나온 순서를 지킨다.
pub fn extract_external_resources(
    text: &str,
    recognizers: &[Box<dyn ExternalResourceRecognizer>],
) -> Vec<ExternalResourceRef> {
    let mut found: Vec<ExternalResourceRef> = Vec::new();
    for url in https_urls(text) {
        let Some(parts) = https_url_parts(url) else {
            continue;
        };
        let Some(resource) = recognizers
            .iter()
            .find_map(|recognizer| recognizer.recognize(&parts))
        else {
            continue;
        };
        if !found
            .iter()
            .any(|known| known.kind == resource.kind && known.key == resource.key)
        {
            found.push(resource);
        }
    }
    found
}

/// 글 안의 `https://` 주소들이다. 공백, 따옴표, 괄호에서 끊고 끝의 문장 부호를 뗀다.
fn https_urls(text: &str) -> Vec<&str> {
    const SCHEME: &str = "https://";
    let mut urls = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(SCHEME) {
        let candidate = &rest[start..];
        let end = candidate
            .find(|character: char| {
                character.is_whitespace()
                    || character.is_control()
                    || matches!(
                        character,
                        '"' | '\''
                            | '<'
                            | '>'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '\\'
                            | '`'
                            | '|'
                    )
            })
            .unwrap_or(candidate.len());
        let url = candidate[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', '*']);
        if url.len() > SCHEME.len() {
            urls.push(url);
        }
        rest = &candidate[end.max(SCHEME.len())..];
    }
    urls
}

fn https_url_parts(url: &str) -> Option<HttpsUrlParts<'_>> {
    let after_scheme = url.strip_prefix("https://")?;
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    // 사용자 정보가 있는 주소는 다루지 않는다. 포트는 뗀다.
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = authority.split(':').next().unwrap_or(authority);
    let after_authority = &after_scheme[authority_end..];
    let path_end = after_authority
        .find(['?', '#'])
        .unwrap_or(after_authority.len());
    Some(HttpsUrlParts {
        url,
        host,
        path: &after_authority[..path_end],
    })
}

fn title_hint(text: &str) -> Option<String> {
    let title = text.trim();
    if title.is_empty() {
        return None;
    }
    Some(title.chars().take(TITLE_HINT_MAX_CHARS).collect())
}

struct NotionRecognizer;

impl ExternalResourceRecognizer for NotionRecognizer {
    fn kind(&self) -> &'static str {
        "notion"
    }

    /// 페이지 주소의 마지막 경로 조각은 `<제목>-<32자리 id>` 또는 id 만이다. id 가 없으면 문서가 아니다.
    fn recognize(&self, url: &HttpsUrlParts<'_>) -> Option<ExternalResourceRef> {
        let host = url.host.to_ascii_lowercase();
        let is_notion = matches!(
            host.as_str(),
            "notion.so" | "www.notion.so" | "app.notion.com" | "notion.com" | "www.notion.com"
        ) || host.ends_with(".notion.site");
        if !is_notion {
            return None;
        }
        let segment = url.path.rsplit('/').find(|segment| !segment.is_empty())?;
        let compact: String = segment
            .chars()
            .filter(|character| *character != '-')
            .collect();
        if compact.len() < 32 || !compact.is_char_boundary(compact.len() - 32) {
            return None;
        }
        let id = &compact[compact.len() - 32..];
        if !id.chars().all(|character| character.is_ascii_hexdigit()) {
            return None;
        }
        // 제목은 id 앞부분이다. id 에 하이픈이 섞인 주소도 있으므로 뒤에서부터 id 글자를 세어 자른다.
        let mut id_chars_left = 32;
        let mut title_end = segment.len();
        for (index, character) in segment.char_indices().rev() {
            if id_chars_left == 0 {
                break;
            }
            if character != '-' {
                id_chars_left -= 1;
            }
            title_end = index;
        }
        Some(ExternalResourceRef {
            kind: self.kind().to_string(),
            key: id.to_ascii_lowercase(),
            url: url.url.to_string(),
            title_hint: title_hint(&segment[..title_end].replace('-', " ")),
        })
    }
}

struct JiraRecognizer;

/// Normalize a Jira installation URL, including its non-default port and context path.
pub fn normalize_jira_site(site: &str) -> Option<String> {
    if site.contains(['?', '#', '@', '\\'])
        || site
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }
    let uri: http::Uri = site.parse().ok()?;
    if uri.scheme_str() != Some("https") {
        return None;
    }
    let authority = uri.authority()?;
    let host = authority.host().to_ascii_lowercase();
    if host.starts_with('[') {
        host.strip_prefix('[')?
            .strip_suffix(']')?
            .parse::<std::net::Ipv6Addr>()
            .ok()?;
    } else if host.is_empty()
        || !host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
    {
        return None;
    }
    let port = if authority.as_str().len() > host.len() {
        let port = authority.port_u16()?;
        if port == 0 {
            return None;
        }
        (port != 443).then_some(port)
    } else {
        None
    };
    let path = uri.path().trim_end_matches('/');
    if !path.is_empty()
        && !path.strip_prefix('/')?.split('/').all(|segment| {
            !segment.is_empty()
                && !matches!(segment, "." | "..")
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '~')
                })
        })
    {
        return None;
    }
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    Some(format!("https://{authority}{path}"))
}

fn jira_url_identity(url: &str) -> Option<(String, &str)> {
    if url.contains('\\')
        || url
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }
    let base = url.split(['?', '#']).next()?;
    let (site, issue) = base.rsplit_once("/browse/")?;
    let site = normalize_jira_site(site)?;
    let (project, number) = issue.split_once('-')?;
    if !project.starts_with(|character: char| character.is_ascii_uppercase())
        || !project.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
        || number.is_empty()
        || !number.chars().all(|character| character.is_ascii_digit())
    {
        return None;
    }
    Some((site, issue))
}

/// Return the installation only when the Jira URL and stored identity agree.
pub fn jira_resource_site(resource: &ExternalResourceRef) -> Option<String> {
    if resource.kind != "jira" {
        return None;
    }
    let (site, issue) = jira_url_identity(&resource.url)?;
    (resource.key == format!("{}/{issue}", site.strip_prefix("https://")?)).then_some(site)
}

impl ExternalResourceRecognizer for JiraRecognizer {
    fn kind(&self) -> &'static str {
        "jira"
    }

    /// Cloud 와 Server 모두 티켓 주소가 `<사이트>/browse/<프로젝트>-<번호>` 이다. 사이트가 다르면 다른 문서이다.
    fn recognize(&self, url: &HttpsUrlParts<'_>) -> Option<ExternalResourceRef> {
        let (site, issue) = jira_url_identity(url.url)?;
        Some(ExternalResourceRef {
            kind: self.kind().to_string(),
            key: format!("{}/{issue}", site.strip_prefix("https://")?),
            url: url.url.to_string(),
            title_hint: title_hint(issue),
        })
    }
}

struct GitHubRecognizer;

impl ExternalResourceRecognizer for GitHubRecognizer {
    fn kind(&self) -> &'static str {
        "github"
    }

    /// PR 과 issue 만 문서로 본다: `github.com/<owner>/<repo>/(pull|issues)/<번호>`.
    fn recognize(&self, url: &HttpsUrlParts<'_>) -> Option<ExternalResourceRef> {
        if !url.host.eq_ignore_ascii_case("github.com") {
            return None;
        }
        let segments = url
            .path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        let [owner, repo, section, number, ..] = segments.as_slice() else {
            return None;
        };
        let label = match *section {
            "pull" => "PR",
            "issues" => "issue",
            _ => return None,
        };
        if number.is_empty() || !number.chars().all(|character| character.is_ascii_digit()) {
            return None;
        }
        Some(ExternalResourceRef {
            kind: self.kind().to_string(),
            key: format!("{owner}/{repo}#{number}").to_ascii_lowercase(),
            // 댓글이나 파일 탭 주소여도 문서의 기본 주소로 연다.
            url: format!("https://github.com/{owner}/{repo}/{section}/{number}"),
            title_hint: title_hint(&format!("{owner}/{repo} {label} #{number}")),
        })
    }
}

struct LinearRecognizer;

impl ExternalResourceRecognizer for LinearRecognizer {
    fn kind(&self) -> &'static str {
        "linear"
    }

    fn recognize(&self, url: &HttpsUrlParts<'_>) -> Option<ExternalResourceRef> {
        let authority = url
            .url
            .strip_prefix("https://")?
            .split(['/', '?', '#'])
            .next()?;
        if !url.host.eq_ignore_ascii_case("linear.app")
            || !authority.eq_ignore_ascii_case("linear.app")
        {
            return None;
        }
        let segments = url.path.strip_prefix('/')?.split('/').collect::<Vec<_>>();
        let (workspace, issue) = match segments.as_slice() {
            [workspace, "issue", issue] => (*workspace, *issue),
            [workspace, "issue", issue, slug] if !slug.is_empty() => (*workspace, *issue),
            _ => return None,
        };
        let valid_workspace = workspace
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
            && workspace
                .chars()
                .last()
                .is_some_and(|last| last.is_ascii_alphanumeric())
            && workspace
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-');
        let (team, number) = issue.split_once('-')?;
        let valid_team = team
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
            && team
                .chars()
                .all(|character| character.is_ascii_alphanumeric());
        let valid_number =
            !number.is_empty() && number.chars().all(|character| character.is_ascii_digit());
        if !valid_workspace || !valid_team || !valid_number {
            return None;
        }
        let identifier = issue.to_ascii_uppercase();
        Some(ExternalResourceRef {
            kind: self.kind().to_string(),
            key: format!("{}/{identifier}", workspace.to_ascii_lowercase()),
            url: url.url.to_string(),
            title_hint: title_hint(&identifier),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(text: &str) -> Vec<ExternalResourceRef> {
        extract_external_resources(text, &external_resource_recognizers())
    }

    #[test]
    fn notion_pages_are_keyed_by_page_id() {
        let found = extract(
            "스펙: https://www.notion.so/team/Login-Spec-0123456789abcdef0123456789ABCDEF?pvs=4 참고.",
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "notion");
        assert_eq!(found[0].key, "0123456789abcdef0123456789abcdef");
        assert_eq!(found[0].title_hint.as_deref(), Some("Login Spec"));
        assert_eq!(
            found[0].url,
            "https://www.notion.so/team/Login-Spec-0123456789abcdef0123456789ABCDEF?pvs=4"
        );
    }

    #[test]
    fn notion_accepts_dashed_ids_and_sites_and_skips_non_pages() {
        let dashed = extract("https://app.notion.com/p/01234567-89ab-cdef-0123-456789abcdef");
        assert_eq!(dashed[0].key, "0123456789abcdef0123456789abcdef");
        assert_eq!(dashed[0].title_hint, None);
        let site = extract("https://team.notion.site/Roadmap-0123456789abcdef0123456789abcdef");
        assert_eq!(site[0].title_hint.as_deref(), Some("Roadmap"));
        assert!(extract("https://www.notion.so/ https://www.notion.so/product/ai").is_empty());
        assert!(
            extract("https://notion.so.evil.com/x-0123456789abcdef0123456789abcdef").is_empty()
        );
    }

    #[test]
    fn jira_issues_are_keyed_by_site_and_issue() {
        let found = extract(
            "(https://Acme.atlassian.net/browse/ABC-123), https://jira.corp.example/jira/browse/OPS_2-7#c1",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].kind, "jira");
        assert_eq!(found[0].key, "acme.atlassian.net/ABC-123");
        assert_eq!(found[0].title_hint.as_deref(), Some("ABC-123"));
        assert_eq!(found[1].key, "jira.corp.example/jira/OPS_2-7");
        assert!(
            extract(
                "https://acme.atlassian.net/browse/abc-1 https://acme.atlassian.net/browse/ABC-"
            )
            .is_empty()
        );
    }

    #[test]
    fn jira_identity_keeps_ports_and_contexts_but_normalizes_default_port() {
        let found = extract(
            "https://Jira.example/browse/ABC-1 https://jira.example:443/browse/ABC-1 \
             https://jira.example:8443/browse/ABC-1 https://jira.example/one/browse/ABC-1 \
             https://jira.example/two/browse/ABC-1",
        );
        assert_eq!(
            found
                .iter()
                .map(|resource| resource.key.as_str())
                .collect::<Vec<_>>(),
            [
                "jira.example/ABC-1",
                "jira.example:8443/ABC-1",
                "jira.example/one/ABC-1",
                "jira.example/two/ABC-1"
            ]
        );
        for resource in &found {
            assert_eq!(
                jira_resource_site(resource),
                Some(format!(
                    "https://{}",
                    resource.key.strip_suffix("/ABC-1").unwrap()
                ))
            );
        }
        let mut mismatch = found[2].clone();
        mismatch.key = "jira.example/ABC-1".into();
        assert_eq!(jira_resource_site(&mismatch), None);
        mismatch = found[0].clone();
        mismatch.kind = "github".into();
        assert_eq!(jira_resource_site(&mismatch), None);
    }

    #[test]
    fn jira_rejects_unsafe_sites_and_non_issue_paths() {
        for site in [
            "https://user@jira.example",
            "https://jira.example:bad",
            "https://jira.example:65536",
            "https://jira.example:0",
            "https://jira.example:",
            "https://jira.example/jira?site=other",
            "https://jira.example/jira#other",
            "https://jira.example/a/../b",
            "https://jira.example/a%2fb",
            "http://jira.example",
        ] {
            assert_eq!(normalize_jira_site(site), None, "{site}");
        }
        assert_eq!(
            normalize_jira_site("https://JIRA.example:443/jira/"),
            Some("https://jira.example/jira".into())
        );
        for url in [
            "https://jira.example:bad/browse/ABC-1",
            "https://jira.example:65536/browse/ABC-1",
            "https://user@jira.example/browse/ABC-1",
            "https://jira.example/browse/ABC-1/extra",
            "https://jira.example/?next=/browse/ABC-1",
        ] {
            assert!(extract(url).is_empty(), "{url}");
        }
    }

    #[test]
    fn github_pull_requests_and_issues_use_the_base_url() {
        let found = extract(
            "https://github.com/Acme/Widgets/pull/42/files and https://github.com/acme/widgets/issues/7",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].key, "acme/widgets#42");
        assert_eq!(found[0].url, "https://github.com/Acme/Widgets/pull/42");
        assert_eq!(found[0].title_hint.as_deref(), Some("Acme/Widgets PR #42"));
        assert_eq!(
            found[1].title_hint.as_deref(),
            Some("acme/widgets issue #7")
        );
        assert!(
            extract("https://github.com/acme/widgets/blob/main/a.rs https://github.com/acme")
                .is_empty()
        );
    }

    #[test]
    fn the_same_document_is_reported_once_in_first_seen_order() {
        let found = extract(
            "https://github.com/a/b/pull/1 https://x.atlassian.net/browse/K-1 https://github.com/a/b/pull/1#discussion",
        );
        assert_eq!(
            found
                .iter()
                .map(|resource| resource.key.as_str())
                .collect::<Vec<_>>(),
            vec!["a/b#1", "x.atlassian.net/K-1"]
        );
    }

    #[test]
    fn linear_issues_use_workspace_and_identifier_without_the_slug() {
        let found = extract(
            "Fix [login](https://LINEAR.APP/Acme-Team/issue/eng2-123/fix-login?view=board#comment). \
             Then https://linear.app/other-team/issue/ENG2-123!",
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].kind, "linear");
        assert_eq!(found[0].key, "acme-team/ENG2-123");
        assert_eq!(found[0].title_hint.as_deref(), Some("ENG2-123"));
        assert_eq!(
            found[0].url,
            "https://LINEAR.APP/Acme-Team/issue/eng2-123/fix-login?view=board#comment"
        );
        assert_eq!(found[1].key, "other-team/ENG2-123");
    }

    #[test]
    fn linear_issues_deduplicate_case_slug_query_and_fragment_variants() {
        let found = extract(
            "https://linear.app/Acme/issue/eng-42/original-title?view=all#comment \
             https://linear.app/acme/issue/ENG-42/renamed-title \
             https://linear.app/ACME/issue/Eng-42?view=mine \
             https://linear.app/acme/issue/ENG-43.",
        );
        assert_eq!(
            found
                .iter()
                .map(|resource| resource.key.as_str())
                .collect::<Vec<_>>(),
            vec!["acme/ENG-42", "acme/ENG-43"]
        );
        assert_eq!(
            found[0].url,
            "https://linear.app/Acme/issue/eng-42/original-title?view=all#comment"
        );
    }

    #[test]
    fn linear_rejects_unsafe_hosts_non_issue_paths_and_malformed_identifiers() {
        for url in [
            "http://linear.app/acme/issue/ENG-1",
            "https://linear.app.evil.com/acme/issue/ENG-1",
            "https://evil-linear.app/acme/issue/ENG-1",
            "https://www.linear.app/acme/issue/ENG-1",
            "https://user:secret@linear.app/acme/issue/ENG-1",
            "https://linear.app@evil.com/acme/issue/ENG-1",
            "https://linear.app:443/acme/issue/ENG-1",
            "https://linear.app/acme",
            "https://linear.app/acme/project/ENG-1",
            "https://linear.app/acme/issue",
            "https://linear.app//issue/ENG-1",
            "https://linear.app/acme//issue/ENG-1",
            "https://linear.app/acme/issue/ENG-1/title/extra",
            "https://linear.app/acme/issue/ENG-1/",
            "https://linear.app/acme/issue/-1",
            "https://linear.app/acme/issue/1ENG-1",
            "https://linear.app/acme/issue/EN_G-1",
            "https://linear.app/acme/issue/ENG-",
            "https://linear.app/acme/issue/ENG-one",
            "https://linear.app/acme/issue/ENG-1-2",
            "https://linear.app/acme/issue/ENG-%31",
            "https://linear.app/acme/issue/ENG-１２",
            "https://linear.app/-acme/issue/ENG-1",
            "https://linear.app/acme.example/issue/ENG-1",
        ] {
            assert!(extract(url).is_empty(), "unexpected recognition: {url}");
        }
    }

    #[test]
    fn urls_end_at_quotes_brackets_and_json_escapes() {
        let json = r#"{"id":"https://github.com/a/b/issues/3","note":"see [link](https://github.com/a/b/pull/4)\nnext"}"#;
        let found = extract(json);
        assert_eq!(
            found
                .iter()
                .map(|resource| resource.key.as_str())
                .collect::<Vec<_>>(),
            vec!["a/b#3", "a/b#4"]
        );
        assert!(
            extract("http://github.com/a/b/pull/1 https://user@github.com/a/b/pull/1 https://")
                .is_empty()
        );
    }
}
