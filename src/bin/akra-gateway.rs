/*
 * gateway bin은 데스크톱 앱 같은 로컬 클라이언트를 codex app-server 에 중계하는 상주 데몬이다.
 * 인자 해석, 실행 디렉터리, upstream 조립은 gateway inbound adapter 가 맡고, 이 파일은 process-level
 * 오류 출력과 exit code 변환만 책임진다.
 */
#[tokio::main]
async fn main() {
    let _diagnostics_guards = codex_exec_loop_native::diagnostics::init_from_env();
    tracing::info!(
        cwd = ?std::env::current_dir().ok(),
        debug_assertions = cfg!(debug_assertions),
        arg_count = std::env::args_os().count(),
        "akra_gateway_process_started"
    );
    let exit_code = match codex_exec_loop_native::adapter::inbound::gateway::run_from_env().await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(exit_code);
}
