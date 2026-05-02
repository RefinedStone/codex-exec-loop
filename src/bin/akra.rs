// 학습 주석: `akra` binary도 기본 main과 같은 bootstrap이므로 사용되지 않는 코드가 남으면 컴파일에서 바로 드러나게 합니다.
#![deny(dead_code)]

// 학습 주석: 명시적인 `akra` binary entrypoint입니다.
// package 기본 binary와 같은 library `run()`을 호출해 CLI/TUI 실행 경로가 한곳으로 모이게 합니다.
fn main() {
    // 학습 주석: library가 반환한 정상 종료 코드는 그대로 shell로 전달하고, 오류는 출력 후 code 1로 표준화합니다.
    let exit_code = match codex_exec_loop_native::run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    // 학습 주석: `std::process::exit`을 사용해 wrapper가 만든 exit_code를 운영체제에 정확히 반영합니다.
    std::process::exit(exit_code);
}
