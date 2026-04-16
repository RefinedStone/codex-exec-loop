#![deny(dead_code)]

use std::io;

pub mod adapter;
pub mod application;
pub mod domain;

pub fn run() -> anyhow::Result<i32> {
    let mut stdout = io::stdout();
    if let Some(exit_code) = adapter::inbound::cli::run_with_env_args(&mut stdout)? {
        return Ok(exit_code);
    }

    adapter::inbound::tui::app::run()?;
    Ok(0)
}
