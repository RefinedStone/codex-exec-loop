//! Bounded work-panel values and link identities. No HTTP or credential material.
use super::provider_credentials::{ConnectionRef, CredentialResult, ProviderCredentialError};
use serde::{Deserialize, Serialize};

pub const ISSUE_LIMIT: usize = 30;
pub const MAX_PROMPT_CHARS: usize = 20_000;
pub const MAX_DESCRIPTION_CHARS: usize = 8_000;
pub const MAX_COMMENT_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JiraIssue {
    pub key: String,
    pub summary: String,
    pub url: String,
    pub status: Option<String>,
    pub status_category: Option<String>,
    pub issue_type: Option<String>,
    pub priority: Option<String>,
    pub updated_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JiraSummary {
    pub site_url: String,
    pub issues: Vec<JiraIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedEntity {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearState {
    pub id: String,
    pub name: String,
    pub r#type: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearTeam {
    pub id: String,
    pub key: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearCycle {
    pub id: String,
    pub number: u64,
    pub name: Option<String>,
    pub is_active: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub priority: u8,
    pub updated_at: String,
    pub branch_name: Option<String>,
    pub state: LinearState,
    pub team: LinearTeam,
    pub project: Option<NamedEntity>,
    pub cycle: Option<LinearCycle>,
}
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearFilters {
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    pub current_cycle: bool,
}
impl LinearFilters {
    pub fn validate(&self) -> CredentialResult<()> {
        if self
            .team_id
            .as_deref()
            .is_some_and(|id| !valid_linear_id(id))
            || self
                .project_id
                .as_deref()
                .is_some_and(|id| !valid_linear_id(id))
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinearSummary {
    pub issues: Vec<LinearIssue>,
    pub teams: Vec<NamedEntity>,
    pub projects: Vec<NamedEntity>,
    pub total_cached: usize,
    pub limit: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderLinkRequest {
    pub connection: ConnectionRef,
    pub url: String,
}

pub fn valid_jira_key(value: &str) -> bool {
    let Some((project, number)) = value.split_once('-') else {
        return false;
    };
    !project.is_empty()
        && project.len() <= 50
        && project.as_bytes()[0].is_ascii_uppercase()
        && project
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        && !number.is_empty()
        && number.len() <= 9
        && number.bytes().all(|byte| byte.is_ascii_digit())
}
pub fn valid_linear_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}
pub fn valid_linear_identifier(value: &str) -> bool {
    let Some((project, number)) = value.split_once('-') else {
        return false;
    };
    !project.is_empty()
        && value.len() <= 128
        && project.as_bytes()[0].is_ascii_alphabetic()
        && project
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !number.is_empty()
        && number.bytes().all(|byte| byte.is_ascii_digit())
}
pub fn jira_issue_key_for_link(site: &str, url: &str) -> Option<String> {
    if url.len() > 2048
        || url.contains(['\\', '@'])
        || url.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return None;
    }
    let base = url.split(['?', '#']).next()?;
    let (candidate, key) = base.rsplit_once("/browse/")?;
    (super::work_context::normalize_jira_site(candidate).as_deref() == Some(site)
        && valid_jira_key(key))
    .then(|| key.to_owned())
}
pub fn valid_linear_issue_url(value: &str) -> bool {
    if value.len() > 2048
        || !value.starts_with("https://linear.app/")
        || value.contains(['\\', '@'])
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return false;
    }
    let base = value.split(['?', '#']).next().unwrap_or_default();
    let Some(path) = base.strip_prefix("https://linear.app/") else {
        return false;
    };
    let segments: Vec<_> = path.trim_end_matches('/').split('/').collect();
    if !(segments.len() == 3 || segments.len() == 4)
        || !valid_linear_id(segments[0])
        || segments[1] != "issue"
        || !valid_linear_identifier(segments[2])
    {
        return false;
    }
    if let Some(slug) = segments.get(3) {
        if slug.is_empty()
            || matches!(*slug, "." | "..")
            || !slug
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || b"_%.-".contains(&ch))
        {
            return false;
        }
        let lower = slug.to_ascii_lowercase();
        if ["%2f", "%5c", "%00", "%0a", "%0d"]
            .iter()
            .any(|part| lower.contains(part))
        {
            return false;
        }
    }
    true
}
/// Jira sometimes uses +HHmm; require millisecond-or-coarser precision without truncation.
pub fn normalized_timestamp(value: &str) -> CredentialResult<String> {
    if !(20..=64).contains(&value.len()) || !value.is_ascii() {
        return Err(ProviderCredentialError::Corrupt);
    }
    let bytes = value.as_bytes();
    if !bytes[..19]
        .iter()
        .enumerate()
        .all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        })
        || bytes[17] > b'5'
    {
        return Err(ProviderCredentialError::Corrupt);
    }
    let mut normalized = value.to_owned();
    if matches!(bytes[value.len() - 5], b'+' | b'-')
        && bytes[value.len() - 4..].iter().all(u8::is_ascii_digit)
    {
        normalized.insert(normalized.len() - 2, ':');
    }
    let mut suffix = &normalized.as_bytes()[19..];
    if suffix.first() == Some(&b'.') {
        let digits = suffix[1..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if !(1..=3).contains(&digits) {
            return Err(ProviderCredentialError::Corrupt);
        }
        suffix = &suffix[digits + 1..];
    }
    let valid_zone = match suffix {
        [b'Z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => {
            [h1, h2, m1, m2].iter().all(|byte| byte.is_ascii_digit())
        }
        _ => false,
    };
    if !valid_zone {
        return Err(ProviderCredentialError::Corrupt);
    }
    let parsed = chrono::DateTime::parse_from_rfc3339(&normalized)
        .map_err(|_| ProviderCredentialError::Corrupt)?;
    Ok(parsed
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_bind_site_port_context_and_key() {
        let site = "https://jira.example:8443/jira";
        assert_eq!(
            jira_issue_key_for_link(site, "https://jira.example:8443/jira/browse/APP-42"),
            Some("APP-42".to_owned())
        );
        for bad in [
            "https://jira.example/jira/browse/APP-42",
            "https://jira.example:8443/browse/APP-42",
            "https://evil.example/jira/browse/APP-42",
            "https://jira.example:8443/jira/browse/APP-42/extra",
            "https://jira.example:8443/jira/browse/APP-42\\other",
            "https://jira.example:8443/jira/browse/APP-42%2fother",
        ] {
            assert!(jira_issue_key_for_link(site, bad).is_none(), "{bad}");
        }
    }
    #[test]
    fn linear_links_are_fixed_host_issue_paths() {
        assert!(valid_linear_issue_url(
            "https://linear.app/team/issue/APP-42/some-title"
        ));
        for bad in [
            "http://linear.app/team/issue/APP-42",
            "https://linear.app.evil/team/issue/APP-42",
            "https://linear.app:443/team/issue/APP-42",
            "https://linear.app/team/issue/APP-42/%2f",
            "https://linear.app/team/issue/APP-42/..",
            "https://linear.app/team/issue/APP-42/a/b",
        ] {
            assert!(!valid_linear_issue_url(bad), "{bad}");
        }
    }
    #[test]
    fn calendar_errors_are_not_versions() {
        assert_eq!(
            normalized_timestamp("2026-11-23T13:04:05.123+0900").unwrap(),
            "2026-11-23T04:04:05.123Z"
        );
        for (value, expected) in [
            ("2026-11-23T13:04:05.1Z", "2026-11-23T13:04:05.100Z"),
            ("2026-11-23T13:04:05.12Z", "2026-11-23T13:04:05.120Z"),
        ] {
            assert_eq!(normalized_timestamp(value).unwrap(), expected);
        }
        for bad in [
            "2026-02-30T13:04:05Z",
            "2026-11-23T99:04:05Z",
            "not-a-time",
            "2026-11-23T13:04:60Z",
            "2026-11-23T13:04:05.1234Z",
            "2026-11-23T13:04:05z",
            "2026-11-23T13:04:05.Z",
            "2026-11-23T13:04:05+24:00",
            "2026-11-23T13:04:05+09:99",
            "2026-11-23T13:04:05한글",
            "2026-11-23 13:04:05Z",
        ] {
            assert!(normalized_timestamp(bad).is_err(), "{bad}");
        }
    }
}
