use anyhow::Result;

use crate::domain::planning::PlanningResetRequest;

/*
 * 입구(CLI, admin, Telegram)가 planning 초기화를 저장소 커널에 맡기는 경계이다 (plan 18 K4-9a,
 * 설계 `docs/plan/18-k4-command-spec.md` §3.7 의 순서 1).
 *
 * 구현은 커널 명령 `reset_planning` 을 보내고 그 연산이 끝날 때까지 기다린다. 성공이면 `Ok(())` 이다. 실패이면 커널이
 * 댄 원인이 오류의 글이다. 초기화가 바꾸는 파일의 목록은 대상마다 정해져 있어서 이 경계가 돌려주지 않는다
 * (`PlanningWorkspaceResetResult::of`).
 *
 * 커널 자신(런타임의 효과 실행기, 명령 전용 조립)은 이 경계를 쓰지 않는다. 쓰면 명령이 자기에게 돌아온다.
 */
pub trait PlanningResetRoutePort: Send + Sync {
    fn reset_workspace(&self, workspace_dir: &str, request: PlanningResetRequest) -> Result<()>;
}
