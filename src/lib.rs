#![deny(dead_code)]

pub mod adapter;
pub mod application;
pub mod domain;

pub fn run() -> anyhow::Result<()> {
    adapter::inbound::tui::app::run()
}
