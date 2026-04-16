#![deny(dead_code)]

fn main() {
    let exit_code = match codex_exec_loop_native::run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };

    std::process::exit(exit_code);
}
