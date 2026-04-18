fn main() {
    let exit_code = match codex_exec_loop_native::adapter::inbound::telegram_bot::run_from_env() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    std::process::exit(exit_code);
}
