//! 이슈 출처의 읽기 전용 계약이다 (#2537).
//! 발견은 열린 이슈의 페이지를 읽고, 재동기화는 선택 조건과 무관하게 안정 ID 한 건을 읽는다.

use crate::domain::issue_automation::source::{
    IssueDiscoveryCursor, IssueDiscoverySelection, IssueSourceDisclosure, IssueSourceFailure,
    IssueSourceObservation, IssueSourceRead, IssueSourceRepository, validate_page_size,
};
use crate::domain::issue_work::{IssueProjectScope, IssueReference};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueDiscoveryTarget {
    /// 첫 페이지에서만 이름으로 저장소를 찾는다. 예상 ID가 있으면 응답과 반드시 같아야 한다.
    GitHubLocator {
        owner: String,
        name: String,
        project_scope: IssueProjectScope,
        expected_repository_id: Option<String>,
    },
    /// 확인된 바인딩과 후속 페이지는 이름을 다시 해석하지 않는다.
    Repository(IssueSourceRepository),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDiscoveryRequest {
    pub target: IssueDiscoveryTarget,
    pub selection: IssueDiscoverySelection,
    pub cursor: Option<IssueDiscoveryCursor>,
    /// 반환 개수가 아니라 원격에서 검사할 이슈의 상한이다.
    pub page_size: u16,
    pub disclosure: IssueSourceDisclosure,
}

impl IssueDiscoveryRequest {
    pub fn validate(&self) -> Result<(), IssueSourceFailure> {
        validate_page_size(self.page_size)?;
        self.selection.normalized()?;
        self.disclosure.validate()?;
        match &self.target {
            IssueDiscoveryTarget::GitHubLocator {
                owner,
                name,
                project_scope,
                expected_repository_id,
            } => {
                if self.cursor.is_some() {
                    return Err(IssueSourceFailure::InvalidCursor);
                }
                validate_locator_component(owner)?;
                validate_locator_component(name)?;
                validate_text(&project_scope.workspace_dir)?;
                if let Some(expected) = expected_repository_id {
                    validate_text(expected)?;
                }
            }
            IssueDiscoveryTarget::Repository(repository) => {
                repository.validate()?;
                if let Some(cursor) = &self.cursor {
                    cursor.validate_for(
                        repository,
                        &self.selection,
                        self.page_size,
                        &self.disclosure,
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDiscoveryPage {
    pub repository: IssueSourceRepository,
    /// Exact Jira credential generation for the operation, including empty/filtered pages.
    /// This is internal commit metadata; it is not inferred from an opaque provider cursor.
    pub jira_connection: Option<crate::domain::provider_credentials::ConnectionRef>,
    /// The exact binding used by this operation, including its search predicate.
    /// A later policy read cannot relabel a page produced by another binding.
    pub jira_binding: Option<crate::domain::issue_automation::policy::IssueAutomationJiraBinding>,
    /// The exact Linear credential generation and team binding used by this operation.
    pub linear: Option<LinearDiscoveryOperation>,
    pub observations: Vec<IssueSourceObservation>,
    /// 이 페이지의 처리와 장부 저장이 끝난 뒤에만 다음 커서를 저장한다.
    pub next_cursor: Option<IssueDiscoveryCursor>,
    /// 로컬 라벨 필터로 결과가 비어도 검사한 페이지와 다음 위치는 남는다.
    pub scanned_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearDiscoveryOperation {
    pub connection: crate::domain::provider_credentials::ConnectionRef,
    pub binding: crate::domain::issue_automation::policy::IssueAutomationLinearBinding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueResyncRequest {
    pub issue: IssueReference,
    pub disclosure: IssueSourceDisclosure,
}

impl IssueResyncRequest {
    pub fn validate(&self) -> Result<(), IssueSourceFailure> {
        IssueSourceRepository::from_reference(&self.issue).validate()?;
        validate_text(&self.issue.issue_id)?;
        self.disclosure.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssueSourceCapabilities {
    pub discovery: bool,
    pub resync: bool,
    /// 증분 watermark를 지원하는가이다. 단순 페이지 커서는 해당하지 않는다.
    pub incremental: bool,
    // 아래 여섯 값은 쓰기 행동의 능력이다. 라벨/담당자/PR 메타데이터를 읽는다는 뜻이 아니다.
    pub comments: bool,
    pub labels: bool,
    pub assign: bool,
    pub transition: bool,
    pub link_pull_request: bool,
    pub board_fields: bool,
}

impl IssueSourceCapabilities {
    pub fn read_only() -> Self {
        Self {
            discovery: true,
            resync: true,
            incremental: false,
            comments: false,
            labels: false,
            assign: false,
            transition: false,
            link_pull_request: false,
            board_fields: false,
        }
    }
}

pub trait IssueSourcePort: Send + Sync {
    fn capabilities(&self) -> IssueSourceCapabilities;

    /// 잘못된 선택/커서나 읽지 못한 페이지를 빈 성공 페이지로 바꾸지 않는다.
    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure>;

    /// 403, 404, 시간 초과, 파싱 실패는 Unknown이다. 이전 값이나 Closed로 대신하지 않는다.
    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead;
}

fn validate_text(value: &str) -> Result<(), IssueSourceFailure> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(IssueSourceFailure::InvalidRequest);
    }
    Ok(())
}

fn validate_locator_component(value: &str) -> Result<(), IssueSourceFailure> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(IssueSourceFailure::InvalidRequest);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::issue_policy::IssueDisclosurePolicy;
    use crate::domain::issue_work::IssueProvider;

    fn repository() -> IssueSourceRepository {
        IssueSourceRepository {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_owned(),
            repository_id: "R_stable".to_owned(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_owned(),
            },
        }
    }

    fn request() -> IssueDiscoveryRequest {
        IssueDiscoveryRequest {
            target: IssueDiscoveryTarget::GitHubLocator {
                owner: "owner".to_owned(),
                name: "repository".to_owned(),
                project_scope: repository().project_scope,
                expected_repository_id: None,
            },
            selection: IssueDiscoverySelection::LabelsAny(vec!["akra:auto".to_owned()]),
            cursor: None,
            page_size: 25,
            disclosure: IssueSourceDisclosure {
                policy: IssueDisclosurePolicy::builtin(),
                policy_version: "policy-v1".to_owned(),
            },
        }
    }

    fn attach_cursor(request: &mut IssueDiscoveryRequest) {
        request.cursor = Some(
            IssueDiscoveryCursor::new(
                repository(),
                request.selection.clone(),
                request.page_size,
                &request.disclosure,
                "opaque-after",
            )
            .unwrap(),
        );
    }

    #[test]
    fn first_page_accepts_a_locator_and_optional_expected_repository_id() {
        let mut request = request();
        assert_eq!(request.validate(), Ok(()));
        let IssueDiscoveryTarget::GitHubLocator {
            expected_repository_id,
            ..
        } = &mut request.target
        else {
            panic!("fixture should use a locator");
        };
        *expected_repository_id = Some("R_stable".to_owned());
        assert_eq!(request.validate(), Ok(()));
    }

    #[test]
    fn continuation_uses_the_resolved_repository_and_never_reinterprets_a_name() {
        let mut request = request();
        attach_cursor(&mut request);
        assert_eq!(request.validate(), Err(IssueSourceFailure::InvalidCursor));
        request.target = IssueDiscoveryTarget::Repository(repository());
        assert_eq!(request.validate(), Ok(()));

        let IssueDiscoveryTarget::Repository(repository) = &mut request.target else {
            panic!("fixture should use a stable repository");
        };
        repository.repository_id = "R_recreated".to_owned();
        assert_eq!(request.validate(), Err(IssueSourceFailure::InvalidCursor));
    }

    #[test]
    fn invalid_requests_are_rejected_before_a_provider_call() {
        let changes: &[fn(&mut IssueDiscoveryRequest)] = &[
            |request| request.page_size = 0,
            |request| request.page_size = 101,
            |request| request.selection = IssueDiscoverySelection::LabelsAny(Vec::new()),
            |request| request.disclosure.policy_version.clear(),
            |request| {
                request.target = IssueDiscoveryTarget::GitHubLocator {
                    owner: "owner/other".to_owned(),
                    name: "repository".to_owned(),
                    project_scope: repository().project_scope,
                    expected_repository_id: None,
                }
            },
            |request| {
                request.target = IssueDiscoveryTarget::GitHubLocator {
                    owner: "owner".to_owned(),
                    name: "repository".to_owned(),
                    project_scope: repository().project_scope,
                    expected_repository_id: Some(String::new()),
                }
            },
        ];
        for change in changes {
            let mut request = request();
            change(&mut request);
            assert_eq!(request.validate(), Err(IssueSourceFailure::InvalidRequest));
        }
    }

    #[test]
    fn resync_needs_a_stable_issue_and_disclosure_without_discovery_filters() {
        let repository = repository();
        let mut request = IssueResyncRequest {
            issue: IssueReference {
                provider: repository.provider,
                server_id: repository.server_id,
                repository_id: repository.repository_id,
                issue_id: "I_stable".to_owned(),
                project_scope: repository.project_scope,
            },
            disclosure: request().disclosure,
        };
        assert_eq!(request.validate(), Ok(()));
        request.issue.issue_id.clear();
        assert_eq!(request.validate(), Err(IssueSourceFailure::InvalidRequest));
        request.issue.issue_id = "I_stable".to_owned();
        request.disclosure.policy_version.clear();
        assert_eq!(request.validate(), Err(IssueSourceFailure::InvalidRequest));
    }

    #[test]
    fn read_only_capabilities_never_advertise_a_watermark_or_write_action() {
        let capabilities = IssueSourceCapabilities::read_only();
        assert!(capabilities.discovery);
        assert!(capabilities.resync);
        assert!(!capabilities.incremental);
        assert!(!capabilities.comments);
        assert!(!capabilities.labels);
        assert!(!capabilities.assign);
        assert!(!capabilities.transition);
        assert!(!capabilities.link_pull_request);
        assert!(!capabilities.board_fields);
    }
}
