/*
 * 기본 package binary는 실제 실행 조립을 library crate로 넘기는 가장 얇은 wrapper다.
 * 이 층에서 dead_code를 막아 두면 bin target만 빌드될 때도 끊어진 bootstrap 경로가 조용히
 * 남지 않는다. 구체적인 CLI/TUI 분기는 codex_exec_loop_native::run()이 맡는다.
 */
#![deny(dead_code)]

fn main() {
    /*
     * library run()은 CLI subcommand가 이미 처리한 종료 코드와 기본 TUI 정상 종료를 같은 i32
     * 계약으로 돌려준다. anyhow 오류는 여기서만 stderr와 process failure로 바꿔, 하위 계층이
     * 출력 정책 대신 오류 context 축적에 집중하게 한다.
     */
    let exit_code = match codex_exec_loop_native::run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    // Shell, supervisor, test harness가 같은 방식으로 결과를 보도록 wrapper가 최종 exit code를 확정한다.
    std::process::exit(exit_code);
}
