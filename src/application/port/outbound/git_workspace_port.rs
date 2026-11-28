use anyhow::Result;

use crate::domain::git_operation::GitOperationImpact;

/*
 * 작업 트리의 상태를 읽고 저장소를 고치는 바깥 경계다 (plan 27 G3). 고치는 명령은 여기에만 있다. 무엇을 언제
 * 고치는지는 정하지 않는다: 작업 엔진이 계획과 기대 상태를 본 뒤에 부른다.
 */

/// 작업 트리의 지금 형편이다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitWorkspaceCondition {
    /// 이 저장소에서 쓰기 명령을 돌리지 않는 이유이다. 돌려도 되면 `None` 이다. 있으면 아래 값은 읽지 않은 것이다.
    /// 이유는 둘이다: 저장소 설정이 프로그램을 실행한다 (필터, 바깥 diff 등), 또는 파일이 필터를 쓴다 (LFS 등.
    /// 데몬의 git 은 필터를 실행하지 않으므로 체크아웃하면 내용이 틀린다).
    pub refusal: Option<String>,
    /// 추적하는 파일에 커밋하지 않은 변경이 있다 (stage 한 것과 하지 않은 것).
    pub tracked_changes: bool,
    /// 이 작업 트리의 끝나지 않은 작업의 이름이다 (`merge`, `rebase`, `cherry-pick`, `revert`, `bisect`).
    pub operation_in_progress: Option<String>,
    /// 어느 작업 트리의 끝나지 않은 rebase 나 bisect 가 쓰는 브랜치들이다. 그런 작업 트리는 그 브랜치를 체크아웃한
    /// 것으로 보이지 않는다 (HEAD 가 브랜치 없이 있다). 그래도 그 ref 를 옮기면 그 작업이 끝에서 실패한다.
    pub busy_branches: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitConversationWorktree {
    pub head: String,
    pub branch: Option<String>,
}

pub trait GitWorkspacePort: Send + Sync {
    fn preview_picks(
        &self,
        _cwd: &str,
        _head: &str,
        _commits: &[String],
    ) -> Result<Vec<crate::domain::git_operation::cherry_pick::PickPreview>> {
        anyhow::bail!("cherry-pick is unavailable")
    }
    fn pick_one(&self, _cwd: &str, _commit: &str, _receipt: &str) -> Result<()> {
        anyhow::bail!("cherry-pick is unavailable")
    }
    fn finish_pick(
        &self,
        _cwd: &str,
        _action: crate::domain::git_operation::cherry_pick::PickAction,
        _receipt: &str,
    ) -> Result<()> {
        anyhow::bail!("cherry-pick is unavailable")
    }
    fn pick_marker(&self, _cwd: &str) -> Result<Option<String>> {
        anyhow::bail!("cherry-pick is unavailable")
    }
    fn pick_completed(&self, _cwd: &str, _parent: &str, _receipt: &str) -> Result<bool> {
        anyhow::bail!("cherry-pick is unavailable")
    }

    /// 사용자 요청 전용 공간을 새 브랜치에 만든다. 기존 목적지/브랜치를 채택하거나 지우지 않는다.
    fn create_conversation_worktree(
        &self,
        _repository_dir: &str,
        _destination: &str,
        _branch: &str,
        _base_commit: &str,
    ) -> Result<()> {
        anyhow::bail!("conversation worktree creation is unavailable")
    }

    /// 경로·Git common directory·root를 확인한다. 없는 경로만 None이고 알 수 없는 신원은 오류다.
    fn inspect_conversation_worktree(
        &self,
        _repository_dir: &str,
        _destination: &str,
    ) -> Result<Option<GitConversationWorktree>> {
        anyhow::bail!("conversation worktree inspection is unavailable")
    }

    fn read_condition(&self, repository_dir: &str) -> Result<GitWorkspaceCondition>;

    /// `commit` (전체 sha)이 가리키는 커밋의 전체 sha 이다. 태그 객체는 그 커밋으로 푼다. 없으면 `None` 이다.
    fn resolve_commit(&self, repository_dir: &str, commit: &str) -> Result<Option<String>>;

    /// 브랜치 `branch` 를 `from` 에서 `to` 로 옮길 때의 영향이다.
    fn read_move_impact(
        &self,
        repository_dir: &str,
        branch: &str,
        from: &str,
        to: &str,
    ) -> Result<GitOperationImpact>;

    /// ref 를 새로 쓴다. 이미 있으면 실패한다. `full_ref` 는 `refs/` 로 시작하는 전체 이름이다.
    fn create_ref(&self, repository_dir: &str, full_ref: &str, commit: &str) -> Result<()>;

    fn release_backup(&self, _cwd: &str, _reference: &str, _expected: &str) -> Result<()> {
        anyhow::bail!("releasing a pinned backup is unavailable")
    }

    fn delete_ref(&self, repository_dir: &str, full_ref: &str) -> Result<()>;

    /// 현재 브랜치와 작업 트리를 `commit` 으로 옮긴다. 추적하는 파일에 변경이 없을 때만 부른다. 추적하지 않는 파일을
    /// 덮어써야 하면 실패한다. 이 명령과 아래의 두 체크아웃은 그 커밋의 파일이 필터를 쓰면 실패한다.
    fn reset_to(&self, repository_dir: &str, commit: &str) -> Result<()>;

    /// 브랜치의 ref 를 `from` 에서 `to` 로 바꾼다. 지금 값이 `from` 이 아니면 실패한다.
    fn move_branch(&self, repository_dir: &str, branch: &str, to: &str, from: &str) -> Result<()>;

    fn switch_branch(&self, repository_dir: &str, branch: &str) -> Result<()>;

    fn switch_detached(&self, repository_dir: &str, commit: &str) -> Result<()>;
}
