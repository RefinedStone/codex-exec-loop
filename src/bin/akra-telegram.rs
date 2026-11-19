/*
 * telegram bin은 TUI를 거치지 않고 Telegram inbound adapter만 실행하는 control-plane wrapper다.
 * adapter 쪽에서 token/config, allow-list, planning control facade를 조립하므로 이 파일은
 * process-level 오류 출력과 exit code 변환만 책임진다.
 */
fn main() -> std::process::ExitCode {
    let _diagnostics_guards = codex_exec_loop_native::diagnostics::init_from_env();
    tracing::info!(
        cwd = ?std::env::current_dir().ok(),
        debug_assertions = cfg!(debug_assertions),
        arg_count = std::env::args_os().count(),
        "akra_telegram_process_started"
    );
    /*
     * run_from_env()는 local workspace에 묶인 planning control service를 만들고 long-polling loop를
     * 시작한다. bootstrap 오류와 runner 오류는 모두 anyhow chain으로 올라오므로 여기서 한 번만 출력한다.
     */
    let exit_code = match codex_exec_loop_native::adapter::inbound::telegram_bot::run_from_env() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    // Preserve the supervisor exit status while dropping the trace WorkerGuard.
    std::process::ExitCode::from(exit_code as u8)
}
