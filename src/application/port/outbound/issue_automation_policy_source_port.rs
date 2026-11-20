use std::path::PathBuf;

use anyhow::Result;

/// 파일의 정체성이다. renderer가 보낸 임의 경로나 공개 정책 파일을 대신하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAutomationPolicyFileScope {
    User,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueAutomationPolicyFileStatus {
    Missing,
    Readable,
    Unavailable,
}

/// 원문 설정은 이 outbound 경계 안에서만 전달한다. 조회 DTO로 직렬화하지 않는다.
pub enum IssueAutomationPolicySourceRead {
    Missing,
    Contents(String),
    Unavailable,
}

/// location과 read는 같은 파일 경로를 한 번 읽은 결과다.
/// 읽기 오류의 원문은 보존하지 않으며, 위치를 결정하지 못하면 None이다.
pub struct IssueAutomationPolicySourceDocument {
    pub location: Option<PathBuf>,
    pub read: IssueAutomationPolicySourceRead,
}

impl IssueAutomationPolicySourceDocument {
    pub fn from_read(location: Option<PathBuf>, result: Result<Option<String>>) -> Self {
        Self {
            location,
            read: match result {
                Ok(Some(contents)) => IssueAutomationPolicySourceRead::Contents(contents),
                Ok(None) => IssueAutomationPolicySourceRead::Missing,
                Err(_) => IssueAutomationPolicySourceRead::Unavailable,
            },
        }
    }
}

pub struct IssueAutomationPolicySourceSnapshot {
    pub user: IssueAutomationPolicySourceDocument,
    pub project: IssueAutomationPolicySourceDocument,
}

/// 자동화 정책 파일만 읽는 경계다 (#2540). 기존 issue-policy와 별도로 실패한다.
/// 파일 없음은 None, 안전하게 읽지 못한 파일은 Err이다. application은 경로를 열지 않는다.
pub trait IssueAutomationPolicySourcePort: Send + Sync {
    fn load_user_policy_toml(&self) -> Result<Option<String>>;
    fn load_project_policy_toml(&self, workspace_dir: &str) -> Result<Option<String>>;

    /// 기존 source는 위치를 모르는 읽기 결과만 제공한다. 파일 adapter는 같은 경로로
    /// location/read를 함께 만든다. 조회 때문에 파일 생성이나 권한 변경을 하지 않는다.
    fn read_policy_snapshot(&self, workspace_dir: &str) -> IssueAutomationPolicySourceSnapshot {
        IssueAutomationPolicySourceSnapshot {
            user: IssueAutomationPolicySourceDocument::from_read(
                None,
                self.load_user_policy_toml(),
            ),
            project: IssueAutomationPolicySourceDocument::from_read(
                None,
                self.load_project_policy_toml(workspace_dir),
            ),
        }
    }
}
