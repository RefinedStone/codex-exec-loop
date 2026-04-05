pub mod application;
pub mod domain;
pub mod infrastructure;
pub mod ui;

pub fn run() -> anyhow::Result<()> {
    ui::app::run()
}
