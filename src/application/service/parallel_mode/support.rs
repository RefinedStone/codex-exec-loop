// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::fs;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use std::path::Path;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use chrono::Utc;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::git_sequence::{GitCommandStep, run_git_sequence};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::pool::reset_slot_worktree_to_akra;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::readiness::run_command;

/*
학습 주석: directory creation은 pool root, lease root, slot parent처럼 여러 모듈에서 반복되는
작은 filesystem boundary입니다. 이미 존재하면 성공으로 보고, 없으면 `create_dir_all`로 부모까지
만듭니다. 호출자는 이 함수를 통해 "디렉터리 보장"이라는 의도를 드러내고, 실패는 각 문맥에
맞는 메시지로 감쌉니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(crate) fn ensure_directory_exists(path: &Path) -> std::io::Result<()> {
    /*
    학습 주석: `exists`를 먼저 보는 것은 이미 디렉터리가 준비된 hot path에서 불필요한 syscall
    error handling을 줄이기 위한 단순 guard입니다. 단, 파일이 같은 path에 있어도 `exists`는 true를
    반환하므로 이 helper는 "path가 directory인지 검증"하는 강한 invariant checker가 아닙니다.
    그런 검증은 pool inspection처럼 operator recovery를 구분해야 하는 모듈에서 따로 수행합니다.
    */
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if path.exists() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return Ok(());
    }

    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    fs::create_dir_all(path)
}

/*
학습 주석: current_timestamp는 lease/session/queue record의 공통 시간 포맷입니다. RFC3339 UTC를
사용하면 문자열 정렬과 사람이 읽는 표시가 모두 안정적이고, 다른 모듈이 별도의 시간 포맷을
만들지 않아도 됩니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(crate) fn current_timestamp() -> String {
    /*
    학습 주석: timestamp는 session detail history, distributor queue record, lease transition이 서로
    같은 시간 언어로 정렬되도록 하는 작은 contract입니다. local timezone을 쓰지 않고 UTC RFC3339를
    쓰면 hidden worker, TUI, recovery process가 다른 환경에서 실행되어도 비교 가능한 문자열을
    공유합니다.
    */
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    Utc::now().to_rfc3339()
}

/*
학습 주석: current_branch_name은 slot worktree와 integration worktree가 기대 branch에 있는지
확인하는 공통 git query입니다. lease running 전이, cleanup pending 전이, distributor readiness
검사가 모두 이 함수를 통해 branch drift를 감지합니다.
*/
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(crate) fn current_branch_name(worktree_path: &Path) -> Option<String> {
    /*
    학습 주석: `rev-parse --abbrev-ref HEAD`는 detached HEAD에서 `HEAD`를 돌려줄 수 있습니다. caller는
    이 값을 그대로 branch 이름처럼 믿지 않고, detached baseline인지 agent branch인지 각 문맥에서
    다시 판정해야 합니다. 여기서는 git query를 표준화하고 실패를 `None`으로 접어 readiness와
    lifecycle guard가 같은 방식으로 branch unknown 상태를 처리하게 합니다.
    */
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
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
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(crate) fn discard_unstarted_slot_branch(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    repo_root: &str,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    slot_path: &Path,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
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
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .succeeded()
}
