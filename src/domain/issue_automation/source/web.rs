//! 현재 공급자 관찰에 붙는 표시 좌표다. 안정 ID나 발견 cursor로 사용하지 않는다.
use serde::{Deserialize, Serialize};

use super::IssueSourceFailure;
use crate::domain::issue_work::{IssueProvider, IssueReference};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IssueSourceWeb {
    pub number: u64,
    pub url: String,
}

impl IssueSourceWeb {
    /// 이름과 안정 저장소 ID의 대응은 공급자 adapter가 현재 nameWithOwner로 검증한다.
    /// 이 경계에서는 알려진 공급자·호스트·경로·번호의 정규 형태만 허용한다.
    pub fn validate_for(&self, issue: &IssueReference) -> Result<(), IssueSourceFailure> {
        if issue.provider != IssueProvider::GitHub {
            return Err(IssueSourceFailure::UnsupportedProvider);
        }
        if issue.server_id != "github.com" || self.number == 0 || self.url.len() > 4096 {
            return Err(IssueSourceFailure::InvalidRequest);
        }
        let path = self
            .url
            .strip_prefix("https://github.com/")
            .ok_or(IssueSourceFailure::InvalidRequest)?;
        let parts = path.split('/').collect::<Vec<_>>();
        if parts.len() != 4
            || !valid_owner(parts[0])
            || !valid_repository(parts[1])
            || parts[2] != "issues"
            || parts[3] != self.number.to_string()
        {
            return Err(IssueSourceFailure::InvalidRequest);
        }
        Ok(())
    }
}

/// GitHub 공급자가 확인한 소문자 owner/name 좌표를 검증한다. URL이나 별칭을 정규화하지 않는다.
pub fn validate_repository_name_with_owner(value: &str) -> Result<(), IssueSourceFailure> {
    let Some((owner, name)) = value.split_once('/') else {
        return Err(IssueSourceFailure::InvalidRequest);
    };
    if value.len() > 4096
        || value.bytes().any(|byte| byte.is_ascii_uppercase())
        || !valid_owner(owner)
        || !valid_repository(name)
    {
        return Err(IssueSourceFailure::InvalidRequest);
    }
    Ok(())
}

fn valid_owner(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

fn valid_repository(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_work::IssueProjectScope;

    fn reference() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_owned(),
            repository_id: "R_stable".to_owned(),
            issue_id: "I_stable".to_owned(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_owned(),
            },
        }
    }

    #[test]
    fn issue_web_requires_the_canonical_known_host_path_and_matching_number() {
        let valid = IssueSourceWeb {
            number: 17,
            url: "https://github.com/Sample-Org/widgets.rs/issues/17".to_owned(),
        };
        assert_eq!(valid.validate_for(&reference()), Ok(()));
        for url in [
            "http://github.com/sample/widgets/issues/17",
            "https://github.com.evil.test/sample/widgets/issues/17",
            "https://github.com@evil.test/sample/widgets/issues/17",
            "https://user@github.com/sample/widgets/issues/17",
            "https://github.com:443/sample/widgets/issues/17",
            "https://github.com/sample/widgets/pull/17",
            "https://github.com/sample/widgets/issues/18",
            "https://github.com/sample/widgets/issues/017",
            "https://github.com/sample/widgets/issues/17/",
            "https://github.com/sample/widgets/issues/17?next=evil",
            "https://github.com/sample/widgets/issues/17#comment",
            "https://github.com/sample/%2e%2e/issues/17",
            "https://github.com/sample/../issues/17",
            "https://github.com//widgets/issues/17",
            "https://github.com/-sample/widgets/issues/17",
            "https://github.com/sample/widgets/issues/17\n",
            "javascript:alert(1)",
            "file:///sample/widgets/issues/17",
        ] {
            let web = IssueSourceWeb {
                url: url.to_owned(),
                ..valid.clone()
            };
            assert!(web.validate_for(&reference()).is_err(), "{url}");
        }
        let zero = IssueSourceWeb {
            number: 0,
            url: "https://github.com/sample/widgets/issues/0".to_owned(),
        };
        assert!(zero.validate_for(&reference()).is_err());
        let mut other = reference();
        other.server_id = "enterprise.example".to_owned();
        assert!(valid.validate_for(&other).is_err());
        other.provider = IssueProvider::Jira;
        assert_eq!(
            valid.validate_for(&other),
            Err(IssueSourceFailure::UnsupportedProvider)
        );
    }
}
