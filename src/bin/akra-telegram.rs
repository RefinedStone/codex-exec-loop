// 학습 주석: telegram binary는 Telegram inbound adapter만 띄우는 얇은 프로세스 wrapper입니다.
// 실제 bot 설정 읽기와 실행은 adapter의 `run_from_env()`가 담당합니다.
fn main() {
    // 학습 주석: bot runner가 정상 종료하면 code 0을, 환경/실행 오류를 반환하면 stderr 출력 후 code 1을 사용합니다.
    let exit_code = match codex_exec_loop_native::adapter::inbound::telegram_bot::run_from_env() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    // 학습 주석: supervisor나 shell script가 bot bootstrap 실패를 감지하도록 프로세스 종료 코드를 명시합니다.
    std::process::exit(exit_code);
}
