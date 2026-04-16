#![deny(dead_code)]

fn main() -> anyhow::Result<()> {
    codex_exec_loop_native::run()
}
