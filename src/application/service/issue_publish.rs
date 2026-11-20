use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotReadRequest,
};
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::domain::issue_validation::IssueValidationPassed;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssuePublishApproval, IssueReference, IssueWriteOperation,
};

/*
 * 승인한 공유본으로 기존 이슈의 제목과 본문을 고치는 경로이다 (#2423). 순서는 고정이다:
 *
 * 1. 보낼 글은 `IssueValidationPassed` 의 글이다 (필수 검증 #2418 을 지난 글).
 * 2. 승인(`IssuePublishApproval`)의 대상, 글의 해시, 정책 판, 템플릿 판이 지금 보낼 값과 같은지 본다. 다르면
 *    원격을 부르지 않고 거절한다.
 * 3. 원격을 읽는다. 필드 해시가 편집 기준(`edit_basis`)과 다르면 쓰지 않고 충돌을 준다.
 * 4. 제목과 본문만 쓴다. 새 이슈, 댓글, 첨부, 보드 필드는 쓰지 않는다.
 *
 * 남는 경쟁: GitHub 의 쓰기는 조건이 없다. 3의 읽기와 4의 쓰기 사이에 다른 사람이 고친 글은 덮어쓴다. 이 경로는
 * 그 틈을 줄일 뿐이고 없애지 못한다.
 *
 * `Err` 는 "쓰지 않았다"는 뜻이 아니다. 쓰기의 답을 잃으면 결과를 모른다. 게시 의도 기록과 응답 유실 복구(#2424)가
 * 연결될 때까지 composition 은 이 서비스를 만들지 않는다. 실사용 게시 경로는 꺼져 있다.
 */

/// 승인한 공유본으로 이슈를 고치는 요청이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueApprovedPublishRequest {
    pub issue: IssueReference,
    /// 게시 관문(`gate_issue_publish`)을 지난 글이다. 이 글만 보낸다.
    pub validated: IssueValidationPassed,
    pub approval: IssuePublishApproval,
    /// 사용자가 편집을 시작한 원격 글의 필드 해시이다.
    pub edit_basis: IssueFieldHash,
    /// 지금의 이슈 정책 판이다.
    pub policy_version: String,
    /// 지금의 템플릿 판이다. 템플릿이 없으면 `None` 이다.
    pub template_version: Option<String>,
}

/// 승인이 보낼 값과 다른 부분이다. 사용자가 다시 승인해야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuePublishRefusal {
    /// 승인이 다른 이슈의 것이거나 본문 쓰기 승인이 아니다.
    Target,
    /// 보낼 제목이나 본문이 승인한 글과 다르다.
    SharedCopy,
    /// 승인한 뒤에 정책 판이 바뀌었다.
    PolicyVersion,
    /// 승인한 뒤에 템플릿 판이 바뀌었다.
    TemplateVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssuePublishOutcome {
    /// 공급자가 쓰기를 확인했다.
    Published(IssueUpdateResponse),
    /// 원격 글이 편집 기준과 다르다. 쓰지 않았다. `remote` 는 지금 원격의 비교 기준이다.
    Conflict { remote: IssueComparisonBasis },
    /// 승인이 보낼 값과 다르다. 원격을 부르지 않았다.
    Refused(IssuePublishRefusal),
}

pub struct IssuePublishService {
    snapshot: Arc<dyn IssueSnapshotPort>,
    update: Arc<dyn IssueUpdatePort>,
}

impl IssuePublishService {
    pub fn new(snapshot: Arc<dyn IssueSnapshotPort>, update: Arc<dyn IssueUpdatePort>) -> Self {
        Self { snapshot, update }
    }

    /// 승인한 글을 기존 이슈에 쓴다. `Err` 는 원격 읽기나 쓰기의 실패이다. 쓰기의 실패는 결과를 모르는 상태이다.
    pub fn publish_approved(
        &self,
        request: IssueApprovedPublishRequest,
    ) -> Result<IssuePublishOutcome> {
        if let Some(refusal) = approval_refusal(&request) {
            return Ok(IssuePublishOutcome::Refused(refusal));
        }
        let remote = self
            .snapshot
            .read(IssueSnapshotReadRequest {
                issue: request.issue.clone(),
            })?
            .comparison_basis;
        if remote.field_hash != request.edit_basis {
            return Ok(IssuePublishOutcome::Conflict { remote });
        }
        self.update
            .update_title_and_body(IssueUpdateRequest {
                approval: request.approval.write,
                issue: request.issue,
                shared_copy: request.validated.shared_copy().clone(),
            })
            .map(IssuePublishOutcome::Published)
    }
}

fn approval_refusal(request: &IssueApprovedPublishRequest) -> Option<IssuePublishRefusal> {
    let approval = &request.approval;
    let shared_copy = request.validated.shared_copy();
    if !approval
        .write
        .authorizes(&request.issue, IssueWriteOperation::Body)
    {
        Some(IssuePublishRefusal::Target)
    } else if approval.shared_copy_hash
        != IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body)
    {
        Some(IssuePublishRefusal::SharedCopy)
    } else if approval.policy_version != request.policy_version {
        Some(IssuePublishRefusal::PolicyVersion)
    } else if approval.template_version != request.template_version {
        Some(IssuePublishRefusal::TemplateVersion)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use anyhow::anyhow;

    use super::*;
    use crate::application::port::outbound::issue_snapshot_port::{
        IssueRawFields, IssueSnapshotResolveRequest, IssueSnapshotResponse,
    };
    use crate::domain::issue_validation::{IssueRequirements, validate_issue};
    use crate::domain::issue_work::{
        IssueProjectScope, IssueProvider, IssueSharedCopy, IssueWriteApproval,
    };

    /// 메모리에 원격 이슈 한 건을 가진 가짜이다. 읽기와 쓰기의 횟수를 센다. 네트워크가 없다.
    struct FakeRemoteIssue {
        fields: Mutex<IssueSharedCopy>,
        reads: Mutex<usize>,
        updates: Mutex<Vec<IssueUpdateRequest>>,
        fail_read: bool,
    }

    impl IssueSnapshotPort for FakeRemoteIssue {
        fn resolve(&self, _: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
            unreachable!("the publish path reads by stable ID")
        }

        fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
            *self.reads.lock().unwrap() += 1;
            if self.fail_read {
                return Err(anyhow!("gh could not read the issue"));
            }
            let fields = self.fields.lock().unwrap().clone();
            let raw_fields = IssueRawFields::new(fields.title, fields.body);
            Ok(IssueSnapshotResponse {
                issue: request.issue,
                comparison_basis: IssueComparisonBasis {
                    field_hash: raw_fields.field_hash(),
                    provider_updated_at: "2026-11-19T07:19:56Z".to_string(),
                },
                raw_fields,
            })
        }
    }

    impl IssueUpdatePort for FakeRemoteIssue {
        fn update_title_and_body(
            &self,
            request: IssueUpdateRequest,
        ) -> Result<IssueUpdateResponse> {
            request.ensure_authorized()?;
            *self.fields.lock().unwrap() = request.shared_copy.clone();
            self.updates.lock().unwrap().push(request.clone());
            Ok(IssueUpdateResponse {
                issue: request.issue,
                published_copy: request.shared_copy,
            })
        }
    }

    fn issue() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: "R_1".to_string(),
            issue_id: "I_1".to_string(),
            project_scope: IssueProjectScope {
                workspace_dir: "/work/project".to_string(),
            },
        }
    }

    fn copy(title: &str, body: &str) -> IssueSharedCopy {
        IssueSharedCopy {
            title: title.to_string(),
            body: body.to_string(),
        }
    }

    fn validated(shared_copy: &IssueSharedCopy) -> IssueValidationPassed {
        let requirements = IssueRequirements {
            template_fields: Vec::new(),
            required_sections: Vec::new(),
        };
        validate_issue(shared_copy, &requirements).expect("valid")
    }

    /// 원격은 `old` 글이다. 사용자는 `old` 를 보고 `new` 글을 승인했다.
    fn remote(fail_read: bool) -> Arc<FakeRemoteIssue> {
        Arc::new(FakeRemoteIssue {
            fields: Mutex::new(copy("Old title", "old body")),
            reads: Mutex::new(0),
            updates: Mutex::new(Vec::new()),
            fail_read,
        })
    }

    fn service(remote: &Arc<FakeRemoteIssue>) -> IssuePublishService {
        IssuePublishService::new(remote.clone(), remote.clone())
    }

    fn request() -> IssueApprovedPublishRequest {
        IssueApprovedPublishRequest {
            issue: issue(),
            validated: validated(&copy("New title", "new body")),
            approval: IssuePublishApproval {
                write: IssueWriteApproval {
                    issue: issue(),
                    operation: IssueWriteOperation::Body,
                },
                shared_copy_hash: IssueFieldHash::of_fields("New title", "new body"),
                policy_version: "policy-1".to_string(),
                template_version: Some("template-1".to_string()),
            },
            edit_basis: IssueFieldHash::of_fields("Old title", "old body"),
            policy_version: "policy-1".to_string(),
            template_version: Some("template-1".to_string()),
        }
    }

    #[test]
    fn an_approved_copy_is_sent_unchanged_after_one_remote_read() {
        let remote = remote(false);

        let outcome = service(&remote).publish_approved(request()).unwrap();

        assert_eq!(
            outcome,
            IssuePublishOutcome::Published(IssueUpdateResponse {
                issue: issue(),
                published_copy: copy("New title", "new body"),
            })
        );
        assert_eq!(*remote.reads.lock().unwrap(), 1);
        // 쓰기는 한 번이고, 승인한 대상과 글 그대로이다.
        assert_eq!(
            *remote.updates.lock().unwrap(),
            [IssueUpdateRequest {
                approval: request().approval.write,
                issue: issue(),
                shared_copy: copy("New title", "new body"),
            }]
        );
    }

    #[test]
    fn a_value_that_differs_from_the_approval_is_refused_before_any_remote_call() {
        let other_issue = IssueReference {
            issue_id: "I_2".to_string(),
            ..issue()
        };
        let mut board_approval = request();
        board_approval.approval.write.operation = IssueWriteOperation::BoardField;
        let cases = [
            (
                IssueApprovedPublishRequest {
                    issue: other_issue,
                    ..request()
                },
                IssuePublishRefusal::Target,
            ),
            (board_approval, IssuePublishRefusal::Target),
            // 제목만, 본문만 다른 글.
            (
                IssueApprovedPublishRequest {
                    validated: validated(&copy("New title!", "new body")),
                    ..request()
                },
                IssuePublishRefusal::SharedCopy,
            ),
            (
                IssueApprovedPublishRequest {
                    validated: validated(&copy("New title", "new body\n")),
                    ..request()
                },
                IssuePublishRefusal::SharedCopy,
            ),
            (
                IssueApprovedPublishRequest {
                    policy_version: "policy-2".to_string(),
                    ..request()
                },
                IssuePublishRefusal::PolicyVersion,
            ),
            (
                IssueApprovedPublishRequest {
                    template_version: None,
                    ..request()
                },
                IssuePublishRefusal::TemplateVersion,
            ),
        ];
        let remote = remote(false);

        for (request, refusal) in cases {
            assert_eq!(
                service(&remote).publish_approved(request).unwrap(),
                IssuePublishOutcome::Refused(refusal)
            );
        }
        assert_eq!(*remote.reads.lock().unwrap(), 0);
        assert!(remote.updates.lock().unwrap().is_empty());
    }

    #[test]
    fn a_remote_copy_that_left_the_edit_basis_is_a_conflict_and_is_not_overwritten() {
        let remote = remote(false);
        // 사용자가 편집하는 동안 다른 사람이 본문을 고쳤다.
        *remote.fields.lock().unwrap() = copy("Old title", "old body, edited by someone else");

        let outcome = service(&remote).publish_approved(request()).unwrap();

        assert_eq!(
            outcome,
            IssuePublishOutcome::Conflict {
                remote: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields(
                        "Old title",
                        "old body, edited by someone else"
                    ),
                    provider_updated_at: "2026-11-19T07:19:56Z".to_string(),
                },
            }
        );
        assert!(remote.updates.lock().unwrap().is_empty());
        assert_eq!(
            *remote.fields.lock().unwrap(),
            copy("Old title", "old body, edited by someone else")
        );
    }

    #[test]
    fn a_failed_remote_read_writes_nothing() {
        let remote = remote(true);

        assert!(service(&remote).publish_approved(request()).is_err());

        assert_eq!(*remote.reads.lock().unwrap(), 1);
        assert!(remote.updates.lock().unwrap().is_empty());
    }
}
