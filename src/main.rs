// 학습 주석: 기본 binary는 사용하지 않는 코드가 생기면 즉시 컴파일 오류로 막습니다.
// 실행 진입점이 얇기 때문에 dead_code를 허용하면 실제로 연결되지 않은 bootstrap 코드를 놓치기 쉽습니다.
#![deny(dead_code)]

// 학습 주석: 기본 실행 파일의 역할은 library crate의 `run()`을 호출하고 OS 종료 코드로 변환하는 것입니다.
// 실제 TUI/app-server 흐름은 library 쪽에 두어 bin과 테스트 가능한 application code를 분리합니다.
fn main() {
    // 학습 주석: `run()`이 성공하면 application이 결정한 종료 코드를 그대로 사용합니다.
    // 실패하면 anyhow error chain을 stderr에 출력하고 일반 실패 코드 1로 바꿉니다.
    let exit_code = match codex_exec_loop_native::run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    // 학습 주석: Rust의 `main`은 i32를 직접 반환하지 않으므로 명시적으로 프로세스를 종료해 shell에 결과를 전달합니다.
    std::process::exit(exit_code);
}
