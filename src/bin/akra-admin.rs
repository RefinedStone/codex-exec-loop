/*
 * admin API는 axum 서버를 띄우는 별도 inbound process라 Tokio runtime이 entrypoint에서
 * 필요하다. CLI의 `akra admin` 경로와 달리 이 bin은 곧바로 admin_api adapter로 들어가
 * 현재 workspace 기준의 planning admin surface를 연다.
 */
#[tokio::main]
async fn main() {
    codex_exec_loop_native::diagnostics::trace_event_log::init_from_env();
    tracing::info!(
        cwd = ?std::env::current_dir().ok(),
        debug_assertions = cfg!(debug_assertions),
        arg_count = std::env::args_os().count(),
        "akra_admin_process_started"
    );
    /*
     * run_from_env() 아래에서 args/env, workspace canonicalization, facade wiring, HTTP bind가
     * 모두 수행된다. bin wrapper는 그 결과를 process contract로만 변환해 systemd나 shell script가
     * startup 실패를 일반 code 1로 감지하게 한다.
     */
    let exit_code = match codex_exec_loop_native::adapter::inbound::admin_api::run_from_env().await
    {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    // async runtime이 정상적으로 내려온 뒤에도 명시적 exit code를 남겨 supervisor 관측을 단순하게 유지한다.
    std::process::exit(exit_code);
}
