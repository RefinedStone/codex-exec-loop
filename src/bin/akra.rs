/*
 * 명시적 akra bin target도 package 기본 binary와 같은 library bootstrap을 탄다.
 * 별도 target을 유지하되 실행 경로를 하나로 모아, `cargo run --bin akra`와 기본 실행이
 * CLI subcommand 판별, TUI startup, 오류 출력 규칙을 공유하게 한다.
 */
#![deny(dead_code)]

fn main() {
    /*
     * 이 wrapper는 명시적 binary 이름만 제공하고 정책은 추가하지 않는다. 하위 library가 반환한
     * 정상 종료 코드는 그대로 전달하고, 오류만 `{:#}` chain으로 펼쳐 사람이 bootstrap 실패 지점을
     * 바로 볼 수 있게 한다.
     */
    let exit_code = match codex_exec_loop_native::run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    // 모든 bin wrapper가 같은 exit-code contract를 지키도록 마지막 변환은 여기서만 한다.
    std::process::exit(exit_code);
}
