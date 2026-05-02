// 학습 주석: filesystem helpers는 pool/slot directory 보장에 사용됩니다. 이 module은 service logic에서
// 반복되는 작은 filesystem boundary를 표준화합니다.
use std::fs;
// 학습 주석: Path reference를 받아 caller가 String 변환 없이 workspace/slot path를 넘기게 합니다.
use std::path::Path;

// 학습 주석: UTC timestamp는 parallel mode record들이 서로 다른 process에서 생성되어도 비교 가능한 시간 언어입니다.
use chrono::Utc;

// 학습 주석: git sequence는 rollback처럼 여러 git command를 하나의 diagnostic unit으로 실행할 때 씁니다.
use super::git_sequence::{GitCommandStep, run_git_sequence};
// 학습 주석: slot reset helper는 unstarted branch discard 전에 worktree를 akra baseline으로 되돌립니다.
use super::pool::reset_slot_worktree_to_akra;
// 학습 주석: readiness command runner는 git query 실패를 Option/String 형태로 접는 공통 command boundary입니다.
use super::readiness::run_command;

/*
학습 주석: directory creation은 pool root, lease root, slot parent처럼 여러 모듈에서 반복되는
작은 filesystem boundary입니다. 이미 존재하면 성공으로 보고, 없으면 `create_dir_all`로 부모까지
만듭니다. 호출자는 이 함수를 통해 "디렉터리 보장"이라는 의도를 드러내고, 실패는 각 문맥에
맞는 메시지로 감쌉니다.
*/
// 학습 주석: 이 helper는 "directory가 있으면 계속, 없으면 생성"이라는 pool setup의 공통 intent를 표현합니다.
pub(crate) fn ensure_directory_exists(path: &Path) -> std::io::Result<()> {
    /*
    학습 주석: `exists`를 먼저 보는 것은 이미 디렉터리가 준비된 hot path에서 불필요한 syscall
    error handling을 줄이기 위한 단순 guard입니다. 단, 파일이 같은 path에 있어도 `exists`는 true를
    반환하므로 이 helper는 "path가 directory인지 검증"하는 강한 invariant checker가 아닙니다.
    그런 검증은 pool inspection처럼 operator recovery를 구분해야 하는 모듈에서 따로 수행합니다.
    */
    // 학습 주석: 이미 존재하는 path는 성공으로 봅니다. directory type 검증까지 필요한 caller는 별도 guard를 둡니다.
    if path.exists() {
        // 학습 주석: hot path에서는 create_dir_all을 다시 호출하지 않고 바로 성공을 반환합니다.
        return Ok(());
    }

    // 학습 주석: 없는 경우에는 parent directory까지 모두 만들고, filesystem error는 caller context로 올라갑니다.
    fs::create_dir_all(path)
}

/*
학습 주석: current_timestamp는 lease/session/queue record의 공통 시간 포맷입니다. RFC3339 UTC를
사용하면 문자열 정렬과 사람이 읽는 표시가 모두 안정적이고, 다른 모듈이 별도의 시간 포맷을
만들지 않아도 됩니다.
*/
// 학습 주석: 이 함수는 parallel mode record들이 공유하는 timestamp serialization을 한곳에 고정합니다.
pub(crate) fn current_timestamp() -> String {
    /*
    학습 주석: timestamp는 session detail history, distributor queue record, lease transition이 서로
    같은 시간 언어로 정렬되도록 하는 작은 contract입니다. local timezone을 쓰지 않고 UTC RFC3339를
    쓰면 hidden worker, TUI, recovery process가 다른 환경에서 실행되어도 비교 가능한 문자열을
    공유합니다.
    */
    // 학습 주석: UTC now를 RFC3339 string으로 직렬화해 DB/store/log에 같은 형태로 저장합니다.
    Utc::now().to_rfc3339()
}

/*
학습 주석: current_branch_name은 slot worktree와 integration worktree가 기대 branch에 있는지
확인하는 공통 git query입니다. lease running 전이, cleanup pending 전이, distributor readiness
검사가 모두 이 함수를 통해 branch drift를 감지합니다.
*/
// 학습 주석: 이 함수는 worktree의 현재 git branch name을 조회해 lifecycle guard가 branch drift를 판단하게 합니다.
pub(crate) fn current_branch_name(worktree_path: &Path) -> Option<String> {
    /*
    학습 주석: `rev-parse --abbrev-ref HEAD`는 detached HEAD에서 `HEAD`를 돌려줄 수 있습니다. caller는
    이 값을 그대로 branch 이름처럼 믿지 않고, detached baseline인지 agent branch인지 각 문맥에서
    다시 판정해야 합니다. 여기서는 git query를 표준화하고 실패를 `None`으로 접어 readiness와
    lifecycle guard가 같은 방식으로 branch unknown 상태를 처리하게 합니다.
    */
    // 학습 주석: command runner는 argv에 &str을 받으므로 Path display 값을 String으로 보관해 수명을 맞춥니다.
    let worktree_path_string = worktree_path.display().to_string();
    run_command(
        "git",
        [
            "-C",
            worktree_path_string.as_str(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ],
        None,
    )
}

/*
학습 주석: unstarted slot branch discard는 lease 저장 실패나 stream startup failure처럼 agent가
아직 Running에 들어가기 전의 rollback 경로입니다. 먼저 slot worktree를 baseline으로 되돌리고,
그 다음 repo에서 agent branch를 삭제합니다. 실행 중 작업에 쓰는 cleanup과 달리, 여기서는
"결과가 없는 시작 실패"를 pool 오염 없이 되돌리는 것이 목적입니다.
*/
// 학습 주석: 이 rollback helper는 agent가 작업을 시작하기 전에 만들어진 slot branch만 폐기합니다.
pub(crate) fn discard_unstarted_slot_branch(
    // 학습 주석: repo_root는 branch delete command를 실행할 canonical repository root입니다.
    repo_root: &str,
    // 학습 주석: slot_path는 reset 대상 worktree입니다. branch 삭제 전에 먼저 baseline으로 되돌립니다.
    slot_path: &Path,
    // 학습 주석: branch_name은 unstarted agent branch 이름입니다. Running 이후 branch를 넘기면 안 됩니다.
    branch_name: &str,
) -> bool {
    /*
    학습 주석: 이 rollback은 "lease를 만들려 했지만 agent가 아직 작업을 시작하지 못한" 짧은 실패
    창에만 쓰입니다. Running 이후의 cleanup은 official completion/distributor가 담당하고, 여기서
    그 branch를 삭제하면 실제 산출물을 잃을 수 있습니다. 그래서 이름도 discard_unstarted로 제한해
    호출자가 lifecycle 전제를 의식하게 합니다.
    */
    reset_slot_worktree_to_akra(slot_path).succeeded()
        && run_git_sequence(
            "delete unstarted slot branch",
            vec![GitCommandStep::new(
                "delete agent branch",
                ["-C", repo_root, "branch", "-D", branch_name],
            )],
        )
        // 학습 주석: worktree reset과 branch delete가 모두 성공해야 rollback 성공으로 봅니다.
        .succeeded()
}
