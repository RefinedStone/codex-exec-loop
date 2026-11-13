use anyhow::Result;

use crate::domain::planning::PlanningResetTarget;

/// 저장소 커널에 맡긴 초기화의 결과이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningResetRouteOutcome {
    /// 커널이 초기화를 실행했고 끝났다.
    Done,
    /*
     * 이 폴더에는 저장소 커널이 있을 수 없다 (대표 checkout 이 없는 저장소, 병렬 slot worktree). 아무것도 실행하지
     * 않았다. 부른 쪽이 이전처럼 직접 초기화한다. 그 초기화는 기록에 남지 않는다.
     */
    NoRepositoryKernel,
}

/*
 * 입구(CLI, admin, Telegram)가 planning 초기화를 저장소 커널에 맡기는 경계이다 (plan 18 K4-9a,
 * 설계 `docs/plan/18-k4-command-spec.md` §3.7 의 순서 1).
 *
 * 구현은 커널 명령 `reset_planning` 을 보내고 그 연산이 끝날 때까지 기다린다. 실패이면 커널이 댄 원인이 오류의 글이다.
 * 확인 규칙은 부른 쪽이 먼저 검사한다. 그래서 이 경계는 대상만 받는다. 초기화가 바꾸는 파일의 목록은 대상마다 정해져
 * 있어서 이 경계가 돌려주지 않는다 (`reset_result`).
 *
 * 커널 자신(런타임의 효과 실행기, 명령 전용 조립)은 이 경계를 쓰지 않는다. 쓰면 명령이 자기에게 돌아온다.
 */
pub trait PlanningResetRoutePort: Send + Sync {
    fn reset_workspace(
        &self,
        workspace_dir: &str,
        target: PlanningResetTarget,
    ) -> Result<PlanningResetRouteOutcome>;
}
