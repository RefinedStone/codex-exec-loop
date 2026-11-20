use anyhow::Result;

use crate::domain::issue_publish_intent::IssuePublishIntentRecord;

/*
 * 이슈 게시 의도 일지의 바깥 경계다 (#2424). 일지는 작업 폴더의 저장소마다 하나이다. 저장소 안에 두지 않는다:
 * 에이전트가 작업 트리에서 고칠 수 있다.
 *
 * 읽지 못한 일지는 오류이다. 빈 일지로 보지 않는다: 결과를 모르는 의도를 놓치면 같은 글을 다시 보낸다.
 */
/// 조회부터 원격 쓰기와 복원 포인터 저장까지 유지하는 저장소별 프로세스 공유 lease이다.
pub trait IssuePublicationLease {
    /// 잠금 파일과 private runtime의 신원이 바뀌었으면 쓰기 전에 중단한다.
    fn verify_identity(&self) -> Result<()> {
        Ok(())
    }
}

/// 메모리 구현은 프로세스 공유 저장소가 없다. 실제 filesystem 어댑터는 OS lease를 반환한다.
impl IssuePublicationLease for () {}

pub trait IssuePublishIntentJournalPort: Send + Sync {
    /// 서비스의 in-process Mutex 다음에 획득한다. 원격 쓰기와 Vault 확정이 끝날 때까지 보관한다.
    fn acquire_publication_lease(
        &self,
        _workspace_dir: &str,
    ) -> Result<Box<dyn IssuePublicationLease + '_>> {
        Ok(Box::new(()))
    }

    /// 기록을 저장한다. 같은 `intent_id` 의 앞 기록을 대신한다. 돌아왔으면 기록은 디스크에 있다.
    fn save(&self, workspace_dir: &str, record: &IssuePublishIntentRecord) -> Result<()>;

    /// 의도마다 하나인 기록이다. 오래된 것이 앞이다.
    fn load(&self, workspace_dir: &str) -> Result<Vec<IssuePublishIntentRecord>>;
}
