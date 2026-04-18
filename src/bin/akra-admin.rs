#[tokio::main]
async fn main() {
    let exit_code = match codex_exec_loop_native::adapter::inbound::admin_api::run_from_env().await
    {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    std::process::exit(exit_code);
}
